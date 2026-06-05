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
        let devices = nokhwa::query(nokhwa::utils::ApiBackend::Auto).map_err(to_err)?;
        Ok(devices
            .into_iter()
            .map(|d| CameraInfo {
                id: d.index().to_string(),
                name: d.human_name(),
            })
            .collect())
    }

    fn start(
        &mut self,
        camera_id: &str,
        frame_sink: Sender<VideoFrame>,
    ) -> Result<(), VideoRecordingError> {
        let index: u32 = camera_id
            .parse()
            .map_err(|_| VideoRecordingError::CameraCaptureFailed("invalid camera id".into()))?;

        *self.last_error.lock() = None;
        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = self.stop_flag.clone();
        let last_error = self.last_error.clone();

        let handle = thread::spawn(move || {
            // Ask nokhwa for the highest resolution close to 1280x720. The
            // compositor will scale the captured frame to the configured
            // PipSize, so 720p is more than enough source resolution.
            let format = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::HighestResolution(
                Resolution::new(1280, 720),
            ));
            let mut camera = match Camera::new(CameraIndex::Index(index), format) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("camera open failed: {}", e);
                    *last_error.lock() = Some(format!("camera open failed: {}", e));
                    return;
                }
            };
            if let Err(e) = camera.open_stream() {
                log::warn!("camera open_stream failed: {}", e);
                *last_error.lock() = Some(format!("camera open_stream failed: {}", e));
                return;
            }

            let (w, h) = {
                let res = camera.resolution();
                (res.width(), res.height())
            };

            loop {
                if stop.load(Ordering::Relaxed) {
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
