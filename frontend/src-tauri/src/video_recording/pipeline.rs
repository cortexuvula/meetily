use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
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
    pub target_w: u32,
    pub target_h: u32,
    pub stop_flag: Arc<AtomicBool>,
    pub compositor_handle: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    pub fn new(ffmpeg: FfmpegVideoOnly, prefs: VideoPreferences, target_w: u32, target_h: u32) -> Self {
        let (screen_tx, screen_rx) = bounded(FRAME_BUFFER);
        let (camera_tx, camera_rx) = bounded(FRAME_BUFFER);
        Self {
            ffmpeg,
            screen_rx,
            camera_rx,
            screen_frame_sink: screen_tx,
            camera_frame_sink: camera_tx,
            prefs,
            target_w,
            target_h,
            stop_flag: Arc::new(AtomicBool::new(false)),
            compositor_handle: None,
        }
    }

    pub fn spawn_compositor<R: Runtime>(&mut self, app: AppHandle<R>) {
        let stop = self.stop_flag.clone();
        let prefs = self.prefs.clone();
        let target_w = self.target_w;
        let target_h = self.target_h;
        let mut video_stdin = self.ffmpeg.take_video_stdin();
        let screen_rx = std::mem::replace(&mut self.screen_rx, crossbeam_channel::never());
        let camera_rx = std::mem::replace(&mut self.camera_rx, crossbeam_channel::never());

        self.compositor_handle = Some(thread::spawn(move || {
            let mut latest_screen: Option<VideoFrame> = None;
            let mut latest_camera: Option<VideoFrame> = None;
            let mut frame_counter: u32 = 0;

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                crossbeam_channel::select! {
                    recv(screen_rx) -> msg => match msg {
                        Ok(frame) => latest_screen = Some(frame),
                        Err(_) => break,
                    },
                    recv(camera_rx) -> msg => match msg {
                        Ok(frame) => latest_camera = Some(frame),
                        Err(_) => {},
                    },
                    default(Duration::from_millis(10)) => {}
                }

                if let (Some(screen), Some(camera)) = (latest_screen.as_ref(), latest_camera.as_ref()) {
                    let mut out_buf = screen.bgra.clone();
                    let _ = composite_pip(
                        &mut out_buf,
                        target_w,
                        target_h,
                        &camera.bgra,
                        camera.width,
                        camera.height,
                        prefs.pip_position,
                        prefs.pip_size,
                    );
                    if video_stdin.write_all(&out_buf).is_err() {
                        break;
                    }

                    // Emit a low-res preview every Nth frame so the webview
                    // can show the user what's actually being recorded.
                    frame_counter = frame_counter.wrapping_add(1);
                    if frame_counter % PREVIEW_FRAME_INTERVAL == 0 {
                        let preview = downscale_to_preview(&out_buf, target_w, target_h);
                        let _ = app.emit(
                            PREVIEW_EVENT,
                            PreviewFrame {
                                width: PREVIEW_WIDTH,
                                height: PREVIEW_HEIGHT,
                                bgra: preview,
                            },
                        );
                    }

                    latest_screen = None;
                }
            }

            let _ = video_stdin.flush();
        }));
    }

    pub fn signal_stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}
