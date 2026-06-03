use super::{CameraCapture, CameraInfo};
use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub struct MacosCameraCapture;

impl MacosCameraCapture {
    pub fn new() -> Self { Self }
}

impl CameraCapture for MacosCameraCapture {
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError> {
        let devices = nokhwa::query(nokhwa::utils::ApiBackend::Auto)
            .map_err(|e| VideoRecordingError::CameraCaptureFailed(e.to_string()))?;
        Ok(devices
            .into_iter()
            .map(|d| CameraInfo {
                id: d.index().to_string(),
                name: d.human_name(),
            })
            .collect())
    }
    fn start(&mut self, _camera_id: &str, _frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        Err(VideoRecordingError::CameraCaptureFailed("macOS camera capture not yet implemented".into()))
    }
    fn stop(&mut self) {}
}
