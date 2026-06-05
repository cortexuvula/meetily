use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub mod macos;
pub mod windows;
pub mod linux;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScreenInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
}

pub trait ScreenCapture: Send {
    fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError>
    where
        Self: Sized;
    fn start(&mut self, screen_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError>;
    fn stop(&mut self);
    /// Returns the last error reported by the capture thread, if any.
    /// Cleared at the start of each `start()` call.
    fn last_error(&self) -> Option<String>;
}

pub fn make_screen_capture() -> Box<dyn ScreenCapture> {
    #[cfg(target_os = "macos")]
    { return Box::new(macos::MacosScreenCapture::new()); }
    #[cfg(target_os = "windows")]
    { return Box::new(windows::WindowsScreenCapture::new()); }
    #[cfg(target_os = "linux")]
    { return Box::new(linux::LinuxScreenCapture::new()); }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { compile_error!("video_recording::sources::screen: unsupported platform"); }
}

pub fn list_screens() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
    #[cfg(target_os = "macos")]
    { return macos::MacosScreenCapture::list(); }
    #[cfg(target_os = "windows")]
    { return windows::WindowsScreenCapture::list(); }
    #[cfg(target_os = "linux")]
    { return linux::LinuxScreenCapture::list(); }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { compile_error!("video_recording::sources::screen: unsupported platform"); }
}
