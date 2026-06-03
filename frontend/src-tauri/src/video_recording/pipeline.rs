use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver, Sender};
use crate::video_recording::compositor::composite_pip;
use crate::video_recording::ffmpeg::FfmpegVideoOnly;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::video_frame::VideoFrame;

const FRAME_BUFFER: usize = 30;

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

    pub fn spawn_compositor(&mut self) {
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
