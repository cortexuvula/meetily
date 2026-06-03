use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;

use super::{ScreenCapture, ScreenInfo};
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::sources::video_frame::VideoFrame;

pub struct WindowsScreenCapture {
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WindowsScreenCapture {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl ScreenCapture for WindowsScreenCapture {
    fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
        let monitors = xcap::Monitor::all()
            .map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?;
        let to_err = |e: xcap::XCapError| VideoRecordingError::ScreenCaptureFailed(e.to_string());
        monitors
            .into_iter()
            .map(|m| {
                Ok(ScreenInfo {
                    id: m.id().map_err(to_err)?.to_string(),
                    name: m.name().map_err(to_err)?,
                    width: m.width().map_err(to_err)?,
                    height: m.height().map_err(to_err)?,
                })
            })
            .collect()
    }

    fn start(
        &mut self,
        screen_id: &str,
        frame_sink: Sender<VideoFrame>,
    ) -> Result<(), VideoRecordingError> {
        let id: u32 = screen_id
            .parse()
            .map_err(|_| VideoRecordingError::ScreenCaptureFailed("invalid screen id".into()))?;

        let monitor = xcap::Monitor::all()
            .map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?
            .into_iter()
            .find(|m| m.id().ok() == Some(id))
            .ok_or_else(|| VideoRecordingError::ScreenCaptureFailed("screen not found".into()))?;

        let width = monitor
            .width()
            .map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?;
        let height = monitor
            .height()
            .map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?;

        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = self.stop_flag.clone();
        let monitor_for_thread = monitor.clone();

        let handle = thread::spawn(move || {
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match monitor_for_thread.capture_image() {
                    Ok(img) => {
                        let bgra = img.into_raw_bgra();
                        if frame_sink.send(VideoFrame::new(width, height, bgra)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("screen capture error: {}", e);
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(33));
            }
        });

        self.thread = Some(handle);
        Ok(())
    }

    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}
