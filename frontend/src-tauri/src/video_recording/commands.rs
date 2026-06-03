use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::manager;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::camera::{CameraInfo, list_cameras};
use crate::video_recording::sources::screen::{ScreenInfo, list_screens};
use crate::video_recording::state::VideoRecordingState;

pub type VideoState = Arc<VideoRecordingState>;

#[tauri::command]
pub async fn start_video_recording<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, VideoState>,
    meeting_id: String,
    save_path: String,
    screen_id: Option<String>,
    camera_id: Option<String>,
) -> Result<(), VideoRecordingError> {
    let prefs = VideoPreferences::default();
    manager::start_video_recording(
        app,
        state.inner().clone(),
        meeting_id,
        PathBuf::from(save_path),
        screen_id,
        camera_id,
        prefs,
    )
}

#[tauri::command]
pub async fn stop_video_recording(state: State<'_, VideoState>) -> Result<PathBuf, VideoRecordingError> {
    manager::stop_video_recording(state.inner().clone())
}

#[tauri::command]
pub async fn get_video_recording_state(
    state: State<'_, VideoState>,
) -> Result<crate::video_recording::state::VideoRecordingStateDto, VideoRecordingError> {
    Ok(state.dto())
}

#[tauri::command]
pub async fn list_video_screens() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
    list_screens()
}

#[tauri::command]
pub async fn list_video_cameras() -> Result<Vec<CameraInfo>, VideoRecordingError> {
    list_cameras()
}

#[tauri::command]
pub async fn get_video_preferences(app: AppHandle) -> Result<VideoPreferences, VideoRecordingError> {
    use tauri_plugin_store::StoreExt;
    let store = app
        .store("video_preferences.json")
        .map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    let prefs = store
        .get("preferences")
        .and_then(|v| serde_json::from_value::<VideoPreferences>(v).ok())
        .unwrap_or_default();
    Ok(prefs)
}

#[tauri::command]
pub async fn set_video_preferences(
    app: AppHandle,
    preferences: VideoPreferences,
) -> Result<(), VideoRecordingError> {
    use tauri_plugin_store::StoreExt;
    let store = app
        .store("video_preferences.json")
        .map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    store.set(
        "preferences",
        serde_json::to_value(&preferences).map_err(|e| VideoRecordingError::Io(e.to_string()))?,
    );
    store.save().map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    Ok(())
}
