use super::{CameraCapture, CameraInfo};
use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub struct LinuxCameraCapture;

impl LinuxCameraCapture {
    pub fn new() -> Self { Self }
}

impl CameraCapture for LinuxCameraCapture {
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError> {
        Err(VideoRecordingError::CameraCaptureFailed("Linux camera capture not yet implemented".into()))
    }
    fn start(&mut self, _camera_id: &str, _frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        Err(VideoRecordingError::CameraCaptureFailed("Linux camera capture not yet implemented".into()))
    }
    fn stop(&mut self) {}
}
