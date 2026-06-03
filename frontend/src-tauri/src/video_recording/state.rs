use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::ffmpeg::FfmpegVideoOnly;

#[derive(Default)]
pub struct VideoRecordingState {
    is_recording: AtomicBool,
    is_starting: AtomicBool,
    is_stopping: AtomicBool,
    is_paused: AtomicBool,
    current_meeting_id: Mutex<Option<String>>,
    ffmpeg: Mutex<Option<FfmpegVideoOnly>>,
    last_error: Mutex<Option<VideoRecordingError>>,
    final_path: Mutex<Option<PathBuf>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoRecordingStateDto {
    pub is_recording: bool,
    pub is_starting: bool,
    pub is_stopping: bool,
    pub is_paused: bool,
    pub current_meeting_id: Option<String>,
    pub final_path: Option<PathBuf>,
    pub last_error: Option<String>,
}

impl VideoRecordingState {
    pub fn dto(&self) -> VideoRecordingStateDto {
        VideoRecordingStateDto {
            is_recording: self.is_recording.load(Ordering::SeqCst),
            is_starting: self.is_starting.load(Ordering::SeqCst),
            is_stopping: self.is_stopping.load(Ordering::SeqCst),
            is_paused: self.is_paused.load(Ordering::SeqCst),
            current_meeting_id: self.current_meeting_id.lock().clone(),
            final_path: self.final_path.lock().clone(),
            last_error: self.last_error.lock().as_ref().map(|e| e.to_string()),
        }
    }

    pub fn try_start(&self) -> Result<(), VideoRecordingError> {
        if self.is_recording.load(Ordering::SeqCst) {
            return Err(VideoRecordingError::AlreadyRecording);
        }
        self.is_starting.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn mark_started(&self, meeting_id: String, ffmpeg: FfmpegVideoOnly) {
        self.ffmpeg.lock().replace(ffmpeg);
        self.current_meeting_id.lock().replace(meeting_id);
        self.is_starting.store(false, Ordering::SeqCst);
        self.is_recording.store(true, Ordering::SeqCst);
        self.last_error.lock().take();
    }

    pub fn try_stop(&self) -> Result<FfmpegVideoOnly, VideoRecordingError> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Err(VideoRecordingError::NotRecording);
        }
        self.is_stopping.store(true, Ordering::SeqCst);
        self.ffmpeg.lock().take().ok_or(VideoRecordingError::NotRecording)
    }

    pub fn mark_stopped(&self, final_path: Option<PathBuf>, error: Option<VideoRecordingError>) {
        self.is_stopping.store(false, Ordering::SeqCst);
        self.is_recording.store(false, Ordering::SeqCst);
        self.current_meeting_id.lock().take();
        if let Some(p) = final_path { self.final_path.lock().replace(p); }
        if let Some(e) = error { self.last_error.lock().replace(e); }
    }
}
