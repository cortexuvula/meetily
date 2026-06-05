use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;

pub mod macos;
pub mod windows;
pub mod linux;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CameraInfo {
    pub id: String,
    pub name: String,
}

pub trait CameraCapture: Send {
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError>
    where
        Self: Sized;
    fn start(&mut self, camera_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError>;
    fn stop(&mut self);
    /// Returns the last error reported by the capture thread, if any.
    /// Cleared at the start of each `start()` call.
    fn last_error(&self) -> Option<String>;
}

pub fn make_camera_capture() -> Box<dyn CameraCapture> {
    #[cfg(target_os = "macos")]
    { return Box::new(macos::MacosCameraCapture::new()); }
    #[cfg(target_os = "windows")]
    { return Box::new(windows::WindowsCameraCapture::new()); }
    #[cfg(target_os = "linux")]
    { return Box::new(linux::LinuxCameraCapture::new()); }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { compile_error!("video_recording::sources::camera: unsupported platform"); }
}

pub fn list_cameras() -> Result<Vec<CameraInfo>, VideoRecordingError> {
    #[cfg(target_os = "macos")]
    { return macos::MacosCameraCapture::list(); }
    #[cfg(target_os = "windows")]
    { return windows::WindowsCameraCapture::list(); }
    #[cfg(target_os = "linux")]
    { return linux::LinuxCameraCapture::list(); }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    { compile_error!("video_recording::sources::camera: unsupported platform"); }
}
