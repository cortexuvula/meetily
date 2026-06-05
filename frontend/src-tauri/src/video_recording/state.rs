use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use parking_lot::Mutex;
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::pipeline::VideoPipeline;
use crate::video_recording::sources::camera::CameraCapture;
use crate::video_recording::sources::screen::ScreenCapture;

pub struct RunningRecording {
    pub pipeline: VideoPipeline,
    pub screen: Box<dyn ScreenCapture>,
    pub camera: Box<dyn CameraCapture>,
    pub audio_thread: Option<std::thread::JoinHandle<()>>,
    pub audio_stop: std::sync::Arc<AtomicBool>,
    pub meeting_id: String,
    pub temp_video: PathBuf,
    pub mic_wav: PathBuf,
    pub system_wav: PathBuf,
    pub final_video: PathBuf,
    pub ffmpeg_path: PathBuf,
}

#[derive(Default)]
pub struct VideoRecordingState {
    is_recording: AtomicBool,
    is_starting: AtomicBool,
    is_stopping: AtomicBool,
    is_paused: AtomicBool,
    current_meeting_id: Mutex<Option<String>>,
    running: Mutex<Option<RunningRecording>>,
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

    /// Attempt to start a recording. Returns AlreadyRecording if a recording is
    /// active or already starting, and AlreadyStopping if a stop is in progress
    /// (so callers can't race a stop with a fresh start).
    pub fn try_start(&self) -> Result<(), VideoRecordingError> {
        if self.is_recording.load(Ordering::SeqCst) || self.is_starting.load(Ordering::SeqCst) {
            return Err(VideoRecordingError::AlreadyRecording);
        }
        if self.is_stopping.load(Ordering::SeqCst) {
            return Err(VideoRecordingError::AlreadyRecording);
        }
        self.is_starting.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn mark_started(&self, meeting_id: String) {
        self.current_meeting_id.lock().replace(meeting_id);
        self.is_starting.store(false, Ordering::SeqCst);
        self.is_recording.store(true, Ordering::SeqCst);
        self.last_error.lock().take();
    }

    pub fn mark_failed(&self, error: VideoRecordingError) {
        self.is_starting.store(false, Ordering::SeqCst);
        self.is_stopping.store(false, Ordering::SeqCst);
        self.last_error.lock().replace(error);
    }

    pub fn mark_stopping(&self) {
        self.is_stopping.store(true, Ordering::SeqCst);
    }

    pub fn take_running(&self) -> Option<RunningRecording> {
        self.running.lock().take()
    }

    pub fn store_running(&self, running: RunningRecording) {
        self.running.lock().replace(running);
    }

    pub fn mark_stopped(&self, final_path: Option<PathBuf>, error: Option<VideoRecordingError>) {
        self.is_stopping.store(false, Ordering::SeqCst);
        self.is_recording.store(false, Ordering::SeqCst);
        self.is_starting.store(false, Ordering::SeqCst);
        self.current_meeting_id.lock().take();
        if let Some(p) = final_path { self.final_path.lock().replace(p); }
        if let Some(e) = error { self.last_error.lock().replace(e); }
    }
}

impl Drop for RunningRecording {
    fn drop(&mut self) {
        // Best-effort cleanup. The stop function takes the audio_thread via
        // .take() and takes the pipeline's compositor_handle + ffmpeg_child,
        // so after a proper stop most of this is a no-op. This handles the
        // case where the start function fails partway through (no stop
        // ever called) or the app exits mid-recording.
        self.audio_stop.store(true, std::sync::atomic::Ordering::Relaxed);
        self.screen.stop();
        self.camera.stop();
        if let Some(handle) = self.audio_thread.take() {
            let _ = handle.join();
        }
        // Clean up temp files (best-effort). final_video is intentionally
        // left alone — if it exists the user may want it.
        let _ = std::fs::remove_file(&self.temp_video);
        let _ = std::fs::remove_file(&self.mic_wav);
        let _ = std::fs::remove_file(&self.system_wav);
    }
}

impl Drop for VideoRecordingState {
    fn drop(&mut self) {
        // Dropping the running triggers RunningRecording::drop, which does
        // the full cleanup (audio thread, captures, ffmpeg child, temp
        // files).
        let _ = self.running.lock().take();
    }
}
