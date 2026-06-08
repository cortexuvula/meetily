use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution};
use nokhwa::Camera;
use parking_lot::Mutex;

use super::{CameraCapture, CameraInfo};
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::sources::video_frame::VideoFrame;

/// Check if macOS camera permission is granted.
/// Returns Ok(()) if authorized or not determined, Err if denied/restricted.
fn check_camera_permission() -> Result<(), VideoRecordingError> {
    if nokhwa::nokhwa_check() {
        Ok(())
    } else {
        // nokhwa_check returns false for restricted/denied/notDetermined
        // We still allow proceeding for notDetermined (will prompt), but
        // log a warning
        log::warn!("[video] nokhwa_check returned false, camera may not be accessible");
        Ok(())
    }
}

pub struct MacosCameraCapture {
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl MacosCameraCapture {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
            last_error: Arc::new(Mutex::new(None)),
        }
    }
}

fn to_err(e: impl std::fmt::Display) -> VideoRecordingError {
    VideoRecordingError::CameraCaptureFailed(e.to_string())
}

impl CameraCapture for MacosCameraCapture {
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError> {
        // Camera listing disabled — nokhwa::Camera::new() panics in extern "C"
        // AVFoundation callback.  Return empty list until nokhwa is fixed.
        log::info!("[video] camera listing disabled (nokhwa extern C panic bug)");
        Ok(vec![])
    }

    fn start(
        &mut self,
        camera_id: &str,
        frame_sink: Sender<VideoFrame>,
    ) -> Result<(), VideoRecordingError> {
        let index: u32 = camera_id
            .parse()
            .map_err(|_| VideoRecordingError::CameraCaptureFailed("invalid camera id".into()))?;

        check_camera_permission()?;

        *self.last_error.lock() = None;
        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = self.stop_flag.clone();
        let last_error = self.last_error.clone();

        let handle = thread::spawn(move || {
            log::info!("[video] camera thread: starting for index {}", index);
            // Ask nokhwa for the highest resolution close to 1280x720. The
            // compositor will scale the captured frame to the configured
            // PipSize, so 720p is more than enough source resolution.
            let format = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::HighestResolution(
                Resolution::new(1280, 720),
            ));
            log::info!("[video] camera thread: creating Camera::new");
            let mut camera = match Camera::new(CameraIndex::Index(index), format) {
                Ok(c) => {
                    log::info!("[video] camera thread: Camera::new succeeded");
                    c
                }
                Err(e) => {
                    log::warn!("camera open failed: {}", e);
                    *last_error.lock() = Some(format!("camera open failed: {}", e));
                    return;
                }
            };
            log::info!("[video] camera thread: calling open_stream");
            if let Err(e) = camera.open_stream() {
                log::warn!("camera open_stream failed: {}", e);
                *last_error.lock() = Some(format!("camera open_stream failed: {}", e));
                return;
            }
            log::info!("[video] camera thread: open_stream succeeded");

            let (w, h) = {
                let res = camera.resolution();
                (res.width(), res.height())
            };
            log::info!("[video] camera thread: resolution {}x{}", w, h);

            loop {
                if stop.load(Ordering::Relaxed) {
                    log::info!("[video] camera thread: stop flag set, exiting");
                    break;
                }
                match camera.frame() {
                    Ok(buffer) => match buffer.decode_image::<RgbAFormat>() {
                        Ok(rgba_image) => {
                            let rgba = rgba_image.into_raw();
                            let bgra: Vec<u8> = rgba
                                .chunks_exact(4)
                                .flat_map(|c| [c[2], c[1], c[0], c[3]])
                                .collect();
                            if frame_sink.send(VideoFrame::new(w, h, bgra)).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            log::warn!("camera decode failed: {}", e);
                            *last_error.lock() = Some(format!("camera decode failed: {}", e));
                        }
                    },
                    Err(e) => {
                        log::warn!("camera frame error: {}", e);
                        *last_error.lock() = Some(format!("camera frame error: {}", e));
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(33));
            }
            log::info!("[video] camera thread: exiting");
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

    fn last_error(&self) -> Option<String> {
        self.last_error.lock().clone()
    }
}
