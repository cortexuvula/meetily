use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;
use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{CameraIndex, RequestedFormat, RequestedFormatType};
use nokhwa::Camera;

use super::{CameraCapture, CameraInfo};
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::sources::video_frame::VideoFrame;

pub struct MacosCameraCapture {
    stop_flag: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MacosCameraCapture {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            thread: None,
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

        self.stop_flag.store(false, Ordering::Relaxed);
        let stop = self.stop_flag.clone();

        let handle = thread::spawn(move || {
            let format = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
            let mut camera = match Camera::new(CameraIndex::Index(index), format) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("camera open failed: {}", e);
                    return;
                }
            };
            if let Err(e) = camera.open_stream() {
                log::warn!("camera open_stream failed: {}", e);
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
                        }
                    },
                    Err(e) => {
                        log::warn!("camera frame error: {}", e);
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
