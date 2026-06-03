use super::{ScreenCapture, ScreenInfo};
use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub struct MacosScreenCapture;

impl MacosScreenCapture {
    pub fn new() -> Self { Self }
}

impl ScreenCapture for MacosScreenCapture {
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
    fn start(&mut self, _screen_id: &str, _frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        Err(VideoRecordingError::ScreenCaptureFailed("macOS screen capture not yet implemented".into()))
    }
    fn stop(&mut self) {}
}
