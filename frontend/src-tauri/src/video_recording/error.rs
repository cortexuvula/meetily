use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum VideoRecordingError {
    #[error("No camera detected. Please connect a webcam and try again.")]
    NoCamera,

    #[error("Camera permission denied. Open System Settings → Privacy & Security → Camera and grant access to Meetily.")]
    CameraPermissionDenied,

    #[error("Screen recording permission denied. Open System Settings → Privacy & Security → Screen Recording and grant access to Meetily.")]
    ScreenPermissionDenied,

    #[error("No screen available to capture.")]
    NoScreen,

    #[error("Multiple screens detected. Please pick which one to record.")]
    NeedsScreenSelection { available: serde_json::Value },

    #[error("Multiple cameras detected. Please pick which one to record.")]
    NeedsCameraSelection { available: serde_json::Value },

    #[error("FFmpeg process exited with code {0}.")]
    FfmpegFailed(i32),

    #[error("Screen capture stream error: {0}")]
    ScreenCaptureFailed(String),

    #[error("Camera capture stream error: {0}")]
    CameraCaptureFailed(String),

    #[error("Audio tap error: {0}")]
    AudioTapFailed(String),

    #[error("Failed to write output: {0}")]
    WriteFailed(String),

    #[error("Video recording is already in progress.")]
    AlreadyRecording,

    #[error("Video recording is not in progress.")]
    NotRecording,

    #[error("IO error: {0}")]
    Io(String),
}

impl From<std::io::Error> for VideoRecordingError {
    fn from(e: std::io::Error) -> Self {
        VideoRecordingError::Io(e.to_string())
    }
}
