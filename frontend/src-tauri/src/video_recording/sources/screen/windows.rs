use super::{ScreenCapture, ScreenInfo};
use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub struct WindowsScreenCapture;

impl WindowsScreenCapture {
    pub fn new() -> Self { Self }
}

impl ScreenCapture for WindowsScreenCapture {
    fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
        Err(VideoRecordingError::ScreenCaptureFailed("Windows screen capture not yet implemented".into()))
    }
    fn start(&mut self, _screen_id: &str, _frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        Err(VideoRecordingError::ScreenCaptureFailed("Windows screen capture not yet implemented".into()))
    }
    fn stop(&mut self) {}
}
