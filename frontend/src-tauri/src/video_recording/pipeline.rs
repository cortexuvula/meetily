use std::io::Write;
use std::process::{Child, ChildStdin};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use crate::video_recording::compositor::composite_pip;
use crate::video_recording::ffmpeg::FfmpegVideoOnly;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::video_frame::VideoFrame;

const FRAME_BUFFER: usize = 30;
pub const PREVIEW_EVENT: &str = "video-preview-frame";
pub const PREVIEW_WIDTH: u32 = 240;
pub const PREVIEW_HEIGHT: u32 = 180;
const PREVIEW_FRAME_INTERVAL: u32 = 6; // emit ~5 fps at 30 fps recording
const CAMERA_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Downscale a BGRA frame to the preview size by nearest-neighbor sampling.
fn downscale_to_preview(src: &[u8], src_w: u32, src_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize];
    for y in 0..PREVIEW_HEIGHT {
        let src_y = (y * src_h / PREVIEW_HEIGHT) as usize;
        for x in 0..PREVIEW_WIDTH {
            let src_x = (x * src_w / PREVIEW_WIDTH) as usize;
            let src_idx = (src_y * src_w as usize + src_x) * 4;
            let dst_idx = (y * PREVIEW_WIDTH + x) as usize * 4;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    dst
}

pub struct VideoPipeline {
    pub ffmpeg: FfmpegVideoOnly,
    pub screen_rx: Receiver<VideoFrame>,
    pub camera_rx: Receiver<VideoFrame>,
    pub screen_frame_sink: Sender<VideoFrame>,
    pub camera_frame_sink: Sender<VideoFrame>,
    pub prefs: VideoPreferences,
    pub input_w: u32,
    pub input_h: u32,
    pub stop_flag: Arc<AtomicBool>,
    pub compositor_handle: Option<JoinHandle<()>>,
    pub last_error: Arc<Mutex<Option<String>>>,
}

impl VideoPipeline {
    pub fn new(ffmpeg: FfmpegVideoOnly, prefs: VideoPreferences, input_w: u32, input_h: u32) -> Self {
        let (screen_tx, screen_rx) = bounded(FRAME_BUFFER);
        let (camera_tx, camera_rx) = bounded(FRAME_BUFFER);
        Self {
            ffmpeg,
            screen_rx,
            camera_rx,
            screen_frame_sink: screen_tx,
            camera_frame_sink: camera_tx,
            prefs,
            input_w,
            input_h,
            stop_flag: Arc::new(AtomicBool::new(false)),
            compositor_handle: None,
            last_error: Arc::new(Mutex::new(None)),
        }
    }

    /// Take the FFmpeg child out so the caller can wait on it. Returns None
    /// if it was already taken.
    pub fn take_ffmpeg_child(&mut self) -> Option<Child> {
        self.ffmpeg.take_child()
    }

    /// Take the ffmpeg stderr handle out so the caller can drain it for
    /// diagnostics after the child exits.
    pub fn take_ffmpeg_stderr(&mut self) -> Option<std::process::ChildStderr> {
        self.ffmpeg.take_stderr()
    }

    /// Take the video stdin out so the caller can hand it to the compositor.
    pub fn take_video_stdin(&mut self) -> Option<ChildStdin> {
        self.ffmpeg.take_video_stdin()
    }

    pub fn spawn_compositor<R: Runtime>(&mut self, app: AppHandle<R>, mut video_stdin: ChildStdin) {
        let stop = self.stop_flag.clone();
        let prefs = self.prefs.clone();
        let input_w = self.input_w;
        let input_h = self.input_h;
        let screen_rx = std::mem::replace(&mut self.screen_rx, crossbeam_channel::never());
        let camera_rx = std::mem::replace(&mut self.camera_rx, crossbeam_channel::never());
        let last_error = self.last_error.clone();

        self.compositor_handle = Some(thread::spawn(move || {
            let mut latest_screen: Option<VideoFrame> = None;
            let mut latest_camera: Option<VideoFrame> = None;
            let mut frame_counter: u32 = 0;
            let start = Instant::now();
            let mut camera_warned = false;
            // Reusable output buffer. Sized to the first screen frame and then
            // swapped in place on every subsequent frame to avoid the per-frame
            // ~8 MB allocation that `screen.bgra.clone()` would cause.
            let mut out_buf: Vec<u8> = Vec::new();

            'main: loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                crossbeam_channel::select! {
                    recv(screen_rx) -> msg => match msg {
                        Ok(frame) => latest_screen = Some(frame),
                        Err(_) => {
                            if !stop.load(Ordering::Relaxed) {
                                *last_error.lock() = Some("screen capture stream ended unexpectedly".into());
                            }
                            break 'main;
                        }
                    },
                    recv(camera_rx) -> msg => match msg {
                        Ok(frame) => latest_camera = Some(frame),
                        Err(_) => {
                            log::warn!("[video] compositor: camera stream ended");
                        }
                    },
                    default(Duration::from_millis(10)) => {}
                }

                if let Some(mut screen) = latest_screen.take() {
                    if out_buf.len() != screen.bgra.len() {
                        // First frame or monitor swap — allocate the buffer.
                        out_buf = screen.bgra;
                    } else {
                        // Swap the screen data into out_buf; the previous
                        // out_buf is dropped with the VideoFrame.
                        std::mem::swap(&mut out_buf, &mut screen.bgra);
                    }
                    if let Some(camera) = latest_camera.as_ref() {
                        let _ = composite_pip(
                            &mut out_buf,
                            input_w,
                            input_h,
                            &camera.bgra,
                            camera.width,
                            camera.height,
                            prefs.pip_position,
                            prefs.pip_size,
                        );
                    } else if !camera_warned && start.elapsed() > CAMERA_FIRST_FRAME_TIMEOUT {
                        log::warn!(
                            "[video] compositor: no camera frame received within {:?}, recording screen only",
                            CAMERA_FIRST_FRAME_TIMEOUT
                        );
                        camera_warned = true;
                    }
                    if let Err(e) = video_stdin.write_all(&out_buf) {
                        if !stop.load(Ordering::Relaxed) {
                            *last_error.lock() = Some(format!("ffmpeg stdin write failed: {}", e));
                        }
                        break;
                    }

                    frame_counter = frame_counter.wrapping_add(1);
                    if frame_counter % PREVIEW_FRAME_INTERVAL == 0 {
                        let preview = downscale_to_preview(&out_buf, input_w, input_h);
                        let _ = app.emit(
                            PREVIEW_EVENT,
                            PreviewFrame {
                                width: PREVIEW_WIDTH,
                                height: PREVIEW_HEIGHT,
                                bgra: preview,
                            },
                        );
                    }
                }
            }

            let _ = video_stdin.flush();
        }));
    }

    pub fn signal_stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Drop for VideoPipeline {
    fn drop(&mut self) {
        // Best-effort cleanup for the case where the pipeline is dropped
        // without going through the manager's stop path. The stop function
        // takes the compositor_handle via .take(), so after a proper stop
        // the join below is a no-op. The FfmpegVideoOnly inside self.ffmpeg
        // has its own Drop that kills the child if it's still alive.
        self.signal_stop();
        if let Some(handle) = self.compositor_handle.take() {
            let _ = handle.join();
        }
    }
}
