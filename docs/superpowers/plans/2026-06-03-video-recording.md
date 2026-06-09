# Video Recording Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add optional screen + webcam (PiP) video recording to Meetily as a sibling artifact to audio recording. Output is a self-contained MP4 saved in the same meeting folder as the audio file, with one video track (screen with PiP webcam) and two audio tracks (mic, system). Video start/stop is fully independent of audio start/stop.

**Architecture:** Hybrid model. A new `video_recording/` Tauri module owns capture, compositing, and encoding. The bundled FFmpeg binary is the encoder, fed via named pipes (Windows) / Unix pipes (macOS, Linux). The Next.js webview provides only a low-fps live preview using `getUserMedia` / `getDisplayMedia`. The actual recording stream is independent of the preview.

**Tech Stack:** Tauri 2.x (Rust), Next.js 14 (React 18), FFmpeg (existing `externalBin`), `nokhwa` (cross-platform camera), `xcap` (cross-platform screen, fallback to platform-direct), `tokio::sync::broadcast` (audio tee), `crossbeam-channel` (frame channels), `image` (PiP compositor), `thiserror` (errors — already in tree).

**Spec:** `docs/superpowers/specs/2026-06-03-video-recording-design.md`

---

## File structure

New files:

```
frontend/src-tauri/src/video_recording/
├── mod.rs                       # module root, re-exports
├── state.rs                     # VideoRecordingState (atomic flags + Arc<RwLock>)
├── preferences.rs               # VideoPreferences + QualityPreset + bitrate mapping
├── error.rs                     # VideoRecordingError (thiserror)
├── ffmpeg.rs                    # FFmpeg argv builder + subprocess manager
├── compositor.rs                # PiP compositor (screen BGRA + camera BGRA → BGRA)
├── audio_tap.rs                 # subscribes to audio broadcast, writes PCM to pipe
├── pipeline.rs                  # orchestrates capture threads + FFmpeg child, owns Drop
├── manager.rs                   # public start/stop API, JIT source selection
├── commands.rs                  # Tauri commands
└── sources/
    ├── mod.rs                   # re-exports + common types
    ├── video_frame.rs           # VideoFrame type
    ├── screen/
    │   ├── mod.rs               # ScreenCapture trait + factory
    │   ├── macos.rs             # CGDisplayStream / ScreenCaptureKit
    │   ├── windows.rs           # DXGI Desktop Duplication
    │   └── linux.rs             # PipeWire
    └── camera/
        ├── mod.rs               # CameraCapture trait + factory
        ├── macos.rs             # AVFoundation (via nokhwa)
        ├── windows.rs           # Media Foundation (via nokhwa)
        └── linux.rs             # V4L2 (via nokhwa)

frontend/src/components/VideoRecording/
├── index.ts                     # barrel exports
├── useVideoRecordingState.ts    # state hook subscribing to video-state-changed event
├── VideoRecordButton.tsx        # sidebar button
├── VideoErrorBanner.tsx         # inline error above button
├── VideoSourcePicker.tsx        # JIT modal for screen/camera selection
├── VideoPreviewOverlay.tsx      # small draggable preview (web-side)
├── MeetingVideoPlayer.tsx       # meeting-details playback component
└── errorCopy.ts                 # per-error remediation copy
```

Modified files:

```
frontend/src-tauri/src/lib.rs                              # register module + Tauri commands
frontend/src-tauri/src/audio/recording_state.rs            # add broadcast::Sender<AudioChunk> for video tap
frontend/src-tauri/src/audio/pipeline.rs                   # forward chunks to video audio broadcast
frontend/src-tauri/Cargo.toml                              # add nokhwa, xcap, image, crossbeam-channel
frontend/src/components/Sidebar/index.tsx                  # add VideoRecordButton + VideoErrorBanner
frontend/src/components/MeetingDetails/*                   # add MeetingVideoPlayer
frontend/src/app/settings/page.tsx                        # add video preferences section
frontend/src-tauri/tauri.conf.json                         # CSP allow ffmpeg pipe paths (no change expected, verified)
```

Documentation:

```
docs/manual-test-video.md     # manual test matrix for the feature
```

---

## Phase 1 — Setup and types

### Task 1: Create feature branch and add Rust dependencies

**Files:**
- Modify: `frontend/src-tauri/Cargo.toml`

- [ ] **Step 1: Create the feature branch off devtest**

```bash
cd /Users/cortexuvula/Development/meetily
git checkout devtest
git pull upstream devtest
git checkout -b feature/video-recording
```

- [ ] **Step 2: Add new dependencies to `frontend/src-tauri/Cargo.toml`**

Locate the `[dependencies]` section and add (verify `thiserror` is already there — it is, used by `parakeet_engine`):

```toml
# Video recording (Phase 1 of feature/video-recording)
nokhwa = { version = "0.10", features = ["docs-rs"] }
xcap = "0.0"
image = "0.25"
crossbeam-channel = "0.5"
parking_lot = "0.12"
```

- [ ] **Step 3: Verify the workspace still builds**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check --workspace
```

Expected: builds cleanly (dependencies download, no errors). The new crates compile but are not yet used. May take a few minutes on first build.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/Cargo.toml Cargo.lock
git commit -m "feat(video): add nokhwa, xcap, image, crossbeam-channel dependencies"
```

---

### Task 2: Create `video_recording` module skeleton

**Files:**
- Create: `frontend/src-tauri/src/video_recording/mod.rs`
- Create: `frontend/src-tauri/src/video_recording/error.rs`
- Create: `frontend/src-tauri/src/video_recording/sources/mod.rs`
- Create: `frontend/src-tauri/src/video_recording/sources/video_frame.rs`
- Create: `frontend/src-tauri/src/video_recording/sources/screen/mod.rs`
- Create: `frontend/src-tauri/src/video_recording/sources/camera/mod.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

- [ ] **Step 1: Create the directory tree**

```bash
mkdir -p /Users/cortexuvula/Development/meetily/frontend/src-tauri/src/video_recording/sources/screen
mkdir -p /Users/cortexuvula/Development/meetily/frontend/src-tauri/src/video_recording/sources/camera
```

- [ ] **Step 2: Write `sources/video_frame.rs`**

```rust
// frontend/src-tauri/src/video_recording/sources/video_frame.rs

use std::time::Instant;

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    pub captured_at: Instant,
}

impl VideoFrame {
    pub fn new(width: u32, height: u32, bgra: Vec<u8>) -> Self {
        Self {
            width,
            height,
            bgra,
            captured_at: Instant::now(),
        }
    }

    pub fn expected_byte_len(&self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }
}
```

- [ ] **Step 3: Write `error.rs`**

```rust
// frontend/src-tauri/src/video_recording/error.rs

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
```

(`available: serde_json::Value` is a placeholder; will be replaced with `Vec<ScreenInfo>` / `Vec<CameraInfo>` in Task 10.)

- [ ] **Step 4: Write `sources/screen/mod.rs` and `sources/camera/mod.rs` stubs**

```rust
// frontend/src-tauri/src/video_recording/sources/screen/mod.rs

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
    fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError>;
    fn start(&mut self, screen_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError>;
    fn stop(&mut self);
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
```

```rust
// frontend/src-tauri/src/video_recording/sources/camera/mod.rs

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
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError>;
    fn start(&mut self, camera_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError>;
    fn stop(&mut self);
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
```

- [ ] **Step 5: Write platform-specific stubs that return `NotImplemented` errors**

For each of: `sources/screen/{macos,windows,linux}.rs` and `sources/camera/{macos,windows,linux}.rs`, write:

```rust
// e.g. frontend/src-tauri/src/video_recording/sources/screen/macos.rs
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
        Err(VideoRecordingError::ScreenCaptureFailed("macOS screen capture not yet implemented".into()))
    }
    fn start(&mut self, _screen_id: &str, _frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        Err(VideoRecordingError::ScreenCaptureFailed("macOS screen capture not yet implemented".into()))
    }
    fn stop(&mut self) {}
}
```

Repeat with `MacosCameraCapture`, `WindowsScreenCapture`, `WindowsCameraCapture`, `LinuxScreenCapture`, `LinuxCameraCapture`, swapping the appropriate types.

- [ ] **Step 6: Write the module root `mod.rs`**

```rust
// frontend/src-tauri/src/video_recording/mod.rs

pub mod error;
pub mod sources;

pub use error::VideoRecordingError;
```

- [ ] **Step 7: Register the module in `lib.rs`**

In `frontend/src-tauri/src/lib.rs`, locate the `pub mod audio;` line (around line 36) and add below it:

```rust
pub mod video_recording;
```

- [ ] **Step 8: Verify the skeleton compiles**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

Expected: compiles. The module is registered and the platform stubs are wired up.

- [ ] **Step 9: Commit**

```bash
git add frontend/src-tauri/src/video_recording/ frontend/src-tauri/src/lib.rs
git commit -m "feat(video): scaffold video_recording module with platform stubs"
```

---

## Phase 2 — Pure-logic Rust (TDD)

### Task 3: `VideoPreferences` with quality-to-bitrate mapping (TDD)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/preferences.rs`
- Create: `frontend/src-tauri/src/video_recording/preferences_tests.rs` (or use `#[cfg(test)]` inline)

- [ ] **Step 1: Write the failing test**

```rust
// frontend/src-tauri/src/video_recording/preferences.rs (bottom of file)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_quality_maps_to_2mbps_720p_24fps() {
        let p = VideoPreferences::from_quality(QualityPreset::Low);
        assert_eq!(p.resolution, Resolution::P720);
        assert_eq!(p.fps, 24);
        assert_eq!(p.bitrate_kbps, 2000);
    }

    #[test]
    fn medium_quality_maps_to_4mbps_1080p_30fps() {
        let p = VideoPreferences::from_quality(QualityPreset::Medium);
        assert_eq!(p.resolution, Resolution::P1080);
        assert_eq!(p.fps, 30);
        assert_eq!(p.bitrate_kbps, 4000);
    }

    #[test]
    fn high_quality_maps_to_8mbps_1080p_30fps() {
        let p = VideoPreferences::from_quality(QualityPreset::High);
        assert_eq!(p.resolution, Resolution::P1080);
        assert_eq!(p.fps, 30);
        assert_eq!(p.bitrate_kbps, 8000);
    }

    #[test]
    fn custom_quality_uses_user_values() {
        let p = VideoPreferences {
            quality: QualityPreset::Custom,
            resolution: Resolution::P720,
            fps: 60,
            bitrate_kbps: 6000,
            pip_position: PipPosition::BottomRight,
            pip_size: PipSize::Medium,
            default_screen: None,
            default_camera: None,
        };
        assert_eq!(p.bitrate_kbps, 6000);
        assert_eq!(p.fps, 60);
    }

    #[test]
    fn target_dimensions_returns_screen_size_for_resolution() {
        let p = VideoPreferences::from_quality(QualityPreset::Medium);
        let (w, h) = p.target_dimensions(Some((3840, 2160)));
        assert_eq!((w, h), (1920, 1080));
    }

    #[test]
    fn target_dimensions_scales_down_from_native() {
        let p = VideoPreferences::from_quality(QualityPreset::Medium);
        let (w, h) = p.target_dimensions(Some((5120, 2880)));
        assert_eq!((w, h), (1920, 1080));
    }
}
```

- [ ] **Step 2: Run the test to confirm it fails**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::preferences
```

Expected: compile error (types don't exist yet) → FAIL.

- [ ] **Step 3: Implement `preferences.rs`**

Replace the file with:

```rust
// frontend/src-tauri/src/video_recording/preferences.rs

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityPreset { Low, Medium, High, Custom }

impl Default for QualityPreset {
    fn default() -> Self { QualityPreset::Medium }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution { P720, P1080, Native }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipPosition { TopLeft, TopRight, BottomLeft, BottomRight }

impl Default for PipPosition { fn default() -> Self { PipPosition::BottomRight } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipSize { Small, Medium, Large }

impl PipSize {
    /// Percent of target screen width.
    pub fn fraction(self) -> f32 {
        match self {
            PipSize::Small => 0.15,
            PipSize::Medium => 0.22,
            PipSize::Large => 0.30,
        }
    }
}

impl Default for PipSize { fn default() -> Self { PipSize::Medium } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoPreferences {
    pub quality: QualityPreset,
    pub resolution: Resolution,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub pip_position: PipPosition,
    pub pip_size: PipSize,
    pub default_screen: Option<String>,
    pub default_camera: Option<String>,
}

impl Default for VideoPreferences {
    fn default() -> Self { Self::from_quality(QualityPreset::Medium) }
}

impl VideoPreferences {
    pub fn from_quality(q: QualityPreset) -> Self {
        let (resolution, fps, bitrate_kbps) = match q {
            QualityPreset::Low    => (Resolution::P720,  24, 2000),
            QualityPreset::Medium => (Resolution::P1080, 30, 4000),
            QualityPreset::High   => (Resolution::P1080, 30, 8000),
            QualityPreset::Custom => (Resolution::P1080, 30, 4000),
        };
        Self {
            quality: q,
            resolution,
            fps,
            bitrate_kbps,
            pip_position: PipPosition::default(),
            pip_size: PipSize::default(),
            default_screen: None,
            default_camera: None,
        }
    }

    /// Returns (width, height) for the output video. If `native` is provided and resolution
    /// is Native, uses native dimensions (capped at 1080p height, preserving aspect ratio).
    /// Otherwise returns the canonical size for the chosen resolution.
    pub fn target_dimensions(&self, native: Option<(u32, u32)>) -> (u32, u32) {
        match self.resolution {
            Resolution::P720  => (1280, 720),
            Resolution::P1080 => (1920, 1080),
            Resolution::Native => match native {
                Some((w, h)) if h <= 1080 => (w, h),
                Some((w, h)) => {
                    let scale = 1080.0 / h as f32;
                    ((w as f32 * scale) as u32, 1080)
                }
                None => (1920, 1080),
            },
        }
    }
}
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::preferences
```

Expected: 6 tests pass.

- [ ] **Step 5: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod preferences;
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/video_recording/preferences.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): VideoPreferences with quality-to-bitrate mapping (TDD)"
```

---

### Task 4: FFmpeg argv builder (TDD)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/ffmpeg.rs`

- [ ] **Step 1: Write the failing test**

```rust
// In ffmpeg.rs (bottom of file)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_recording::preferences::*;

    #[test]
    fn builds_argv_for_medium_quality_1080p_30fps() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_ffmpeg_argv(&prefs, 1920, 1080, Path::new("/tmp/out.mp4"));
        // Spot-check key flags
        assert!(argv.iter().any(|a| a == "libx264"));
        assert!(argv.iter().any(|a| a == "aac"));
        assert!(argv.contains(&"-b:v".to_string()));
        assert!(argv.contains(&"4000k".to_string()));
        assert!(argv.contains(&"-r".to_string()));
        assert!(argv.contains(&"30".to_string()));
        assert!(argv.iter().any(|a| a.ends_with("out.mp4")));
    }

    #[test]
    fn includes_three_input_streams_video_mic_system() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_ffmpeg_argv(&prefs, 1920, 1080, Path::new("/tmp/out.mp4"));
        // 3 -i pipe:N entries
        let i_count = argv.iter().filter(|a| a.as_str() == "-i").count();
        assert_eq!(i_count, 3);
        // 3 -map entries (one video, two audio)
        let map_count = argv.iter().filter(|a| a.starts_with("-map")).count();
        assert_eq!(map_count, 3);
    }

    #[test]
    fn argv_uses_faststart_for_streaming() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_ffmpeg_argv(&prefs, 1920, 1080, Path::new("/tmp/out.mp4"));
        let combined: Vec<String> = argv.iter()
            .flat_map(|a| if a.contains("=") { vec![a.clone()] } else { vec![a.clone(), "x".into()] }.into_iter())
            .collect();
        assert!(combined.iter().any(|a| a.starts_with("-movflags")));
    }

    #[test]
    fn argv_high_quality_uses_8000k_bitrate() {
        let prefs = VideoPreferences::from_quality(QualityPreset::High);
        let argv = build_ffmpeg_argv(&prefs, 1920, 1080, Path::new("/tmp/out.mp4"));
        assert!(argv.contains(&"8000k".to_string()));
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::ffmpeg
```

Expected: compile error (no `build_ffmpeg_argv` yet) → FAIL.

- [ ] **Step 3: Implement `ffmpeg.rs`**

```rust
// frontend/src-tauri/src/video_recording/ffmpeg.rs

use std::path::Path;
use crate::video_recording::preferences::VideoPreferences;

const VIDEO_PIPE: &str = "pipe:0";
const MIC_AUDIO_PIPE: &str = "pipe:1";
const SYSTEM_AUDIO_PIPE: &str = "pipe:2";

const SAMPLE_RATE: u32 = 48000;
const AUDIO_CHANNELS: u32 = 2;
const AUDIO_BITRATE: &str = "128k";

/// Build the argv for the bundled ffmpeg process.
/// Layout: -y (overwrite), -f rawvideo ... (video), -f s16le ... (mic), -f s16le ... (system),
///         -map 0:v -map 1:a -map 2:a, encoders, output path.
pub fn build_ffmpeg_argv(prefs: &VideoPreferences, width: u32, height: u32, out: &Path) -> Vec<String> {
    let maxrate = prefs.bitrate_kbps * 5 / 4;          // +25% headroom
    let bufsize = prefs.bitrate_kbps * 2;             // 2x for stability

    vec![
        "-y".to_string(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{}x{}", width, height),
        "-r".into(), prefs.fps.to_string(),
        "-i".into(), VIDEO_PIPE.into(),

        "-f".into(), "s16le".into(),
        "-ar".into(), SAMPLE_RATE.to_string(),
        "-ac".into(), AUDIO_CHANNELS.to_string(),
        "-i".into(), MIC_AUDIO_PIPE.into(),

        "-f".into(), "s16le".into(),
        "-ar".into(), SAMPLE_RATE.to_string(),
        "-ac".into(), AUDIO_CHANNELS.to_string(),
        "-i".into(), SYSTEM_AUDIO_PIPE.into(),

        "-map".into(), "0:v".into(),
        "-map".into(), "1:a".into(),
        "-map".into(), "2:a".into(),

        "-c:v".into(), "libx264".into(),
        "-preset".into(), "veryfast".into(),
        "-b:v".into(), format!("{}k", prefs.bitrate_kbps),
        "-maxrate".into(), format!("{}k", maxrate),
        "-bufsize".into(), format!("{}k", bufsize),
        "-pix_fmt".into(), "yuv420p".into(),
        "-g".into(), (prefs.fps * 2).to_string(),

        "-c:a".into(), "aac".into(),
        "-b:a".into(), AUDIO_BITRATE.into(),

        "-movflags".into(), "+faststart".into(),

        out.to_string_lossy().into_owned(),
    ]
}

pub fn ffmpeg_argv_with_overrides(base: Vec<String>, _width: u32, _height: u32) -> Vec<String> {
    // Future hook for per-run overrides (e.g. different fps). V1 returns the base argv unchanged.
    base
}

pub const VIDEO_PIPE_INDEX: usize = 0;
pub const MIC_PIPE_INDEX: usize = 1;
pub const SYSTEM_PIPE_INDEX: usize = 2;
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::ffmpeg
```

Expected: 4 tests pass.

- [ ] **Step 5: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod ffmpeg;
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/video_recording/ffmpeg.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): FFmpeg argv builder (TDD)"
```

---

### Task 5: PiP compositor (TDD)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/compositor.rs`

- [ ] **Step 1: Write the failing test**

```rust
// In compositor.rs (bottom of file)

#[cfg(test)]
mod tests {
    use super::*;

    fn red_bgra(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for i in (0..v.len()).step_by(4) {
            v[i]     = 0;   // B
            v[i + 1] = 0;   // G
            v[i + 2] = 255; // R
            v[i + 3] = 255; // A
        }
        v
    }

    fn blue_bgra(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for i in (0..v.len()).step_by(4) {
            v[i]     = 255; // B
            v[i + 1] = 0;
            v[i + 2] = 0;
            v[i + 3] = 255;
        }
        v
    }

    #[test]
    fn bottom_right_pip_places_camera_in_corner() {
        // 100x100 screen (red), 20x20 camera (blue), Medium size = 22% of 100 = 22 → use 20.
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        // Use PipSize::Small = 15% → 15 px, round to 20
        let out = composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::BottomRight, PipSize::Small).unwrap();
        // Out dims = screen dims
        assert_eq!(out.len(), 100 * 100 * 4);
        // Bottom-right corner pixel should be blue
        let corner = ((100 * 99 + 95) * 4) as usize;
        assert_eq!(out[corner], 255, "expected blue B channel at bottom-right");
        // Top-left pixel should still be red
        let top_left = (0 * 4) as usize;
        assert_eq!(out[top_left + 2], 255, "expected red R channel at top-left");
    }

    #[test]
    fn top_left_pip_places_camera_in_top_left() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let out = composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::TopLeft, PipSize::Small).unwrap();
        let top_left = (0 * 4) as usize;
        assert_eq!(out[top_left], 255, "expected blue B channel at top-left");
    }

    #[test]
    fn rejects_size_mismatch_with_error() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let result = composite_pip(&mut screen, 100, 100, &camera, 30, 30, PipPosition::BottomRight, PipSize::Small);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::compositor
```

Expected: compile error → FAIL.

- [ ] **Step 3: Implement `compositor.rs`**

```rust
// frontend/src-tauri/src/video_recording/compositor.rs

use crate::video_recording::preferences::{PipPosition, PipSize};

/// Copy `camera` (BGRA) into the corner of `screen` (BGRA) determined by `position`.
/// `camera_w` / `camera_h` should equal the camera frame's actual dimensions.
/// Returns the modified screen buffer.
///
/// V1: camera is copied at its native resolution (no resizing). PipSize controls the
/// expected camera frame size relative to the screen — the caller is responsible for
/// sizing the camera capture to the right target.
pub fn composite_pip(
    screen: &mut [u8],
    screen_w: u32,
    screen_h: u32,
    camera: &[u8],
    camera_w: u32,
    camera_h: u32,
    position: PipPosition,
    _size: PipSize,
) -> Result<Vec<u8>, String> {
    let expected_screen = (screen_w as usize) * (screen_h as usize) * 4;
    if screen.len() != expected_screen {
        return Err(format!("screen buffer size {} does not match dimensions {}x{}", screen.len(), screen_w, screen_h));
    }
    let expected_cam = (camera_w as usize) * (camera_h as usize) * 4;
    if camera.len() != expected_cam {
        return Err(format!("camera buffer size {} does not match dimensions {}x{}", camera.len(), camera_w, camera_h));
    }
    if camera_w > screen_w || camera_h > screen_h {
        return Err("camera larger than screen".into());
    }

    let (x_off, y_off) = match position {
        PipPosition::TopLeft     => (0, 0),
        PipPosition::TopRight    => (screen_w - camera_w, 0),
        PipPosition::BottomLeft  => (0, screen_h - camera_h),
        PipPosition::BottomRight => (screen_w - camera_w, screen_h - camera_h),
    };

    for row in 0..camera_h {
        let src_start = (row * camera_w * 4) as usize;
        let src_end = src_start + (camera_w * 4) as usize;
        let dst_y = y_off + row;
        let dst_start = ((dst_y * screen_w + x_off) * 4) as usize;
        let dst_end = dst_start + (camera_w * 4) as usize;
        screen[dst_start..dst_end].copy_from_slice(&camera[src_start..src_end]);
    }

    Ok(screen.to_vec())
}
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::compositor
```

Expected: 3 tests pass.

- [ ] **Step 5: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod compositor;
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/video_recording/compositor.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): PiP compositor (TDD, V1 native-resolution copy)"
```

---

### Task 6: Audio tap — PCM conversion (TDD)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/audio_tap.rs`

- [ ] **Step 1: Write the failing test**

```rust
// In audio_tap.rs (bottom of file)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::recording_state::{AudioChunk, DeviceType};

    fn make_chunk(device: DeviceType, samples: Vec<f32>) -> AudioChunk {
        AudioChunk {
            data: samples,
            sample_rate: 48000,
            timestamp: 0.0,
            chunk_id: 0,
            device_type: device,
        }
    }

    #[test]
    fn f32_to_pcm16_silent() {
        let pcm = f32_to_pcm16(&[0.0_f32, 0.0, 0.0, 0.0]);
        assert_eq!(pcm, vec![0u8; 8]);
    }

    #[test]
    fn f32_to_pcm16_full_scale_positive() {
        let pcm = f32_to_pcm16(&[1.0_f32]);
        assert_eq!(pcm, vec![0xFF, 0x7F]); // little-endian i16::MAX
    }

    #[test]
    fn f32_to_pcm16_full_scale_negative() {
        let pcm = f32_to_pcm16(&[-1.0_f32]);
        assert_eq!(pcm, vec![0x00, 0x80]); // little-endian i16::MIN
    }

    #[test]
    fn f32_to_pcm16_clamps_overflow() {
        let pcm = f32_to_pcm16(&[2.0_f32, -2.0_f32]);
        assert_eq!(pcm, vec![0xFF, 0x7F, 0x00, 0x80]);
    }

    #[test]
    fn chunk_to_pcm16_writes_interleaved_stereo() {
        // Mock chunk with stereo (L, R, L, R) samples at half-scale.
        let chunk = make_chunk(DeviceType::Microphone, vec![0.5, -0.5, 0.5, -0.5]);
        let pcm = chunk_to_pcm16_stereo(&chunk);
        let expected_half_pos = (0.5_f32 * i16::MAX as f32) as i16;
        let expected_half_neg = (0.5_f32 * i16::MIN as f32) as i16;
        assert_eq!(pcm, expected_half_pos.to_le_bytes().to_vec()
            .into_iter()
            .chain(expected_half_neg.to_le_bytes())
            .collect::<Vec<_>>());
    }
}
```

- [ ] **Step 2: Run test to confirm it fails**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::audio_tap
```

Expected: compile error → FAIL.

- [ ] **Step 3: Implement `audio_tap.rs`**

```rust
// frontend/src-tauri/src/video_recording/audio_tap.rs

use tokio::sync::broadcast;
use std::io::Write;
use crate::audio::recording_state::{AudioChunk, DeviceType};
use crate::video_recording::error::VideoRecordingError;

/// Convert a slice of f32 samples in [-1.0, 1.0] to 16-bit signed PCM (little-endian).
/// Clamps out-of-range values.
pub fn f32_to_pcm16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Convert a single AudioChunk to interleaved stereo PCM16. If the chunk is mono
/// (length does not look stereo), it is duplicated to both channels.
pub fn chunk_to_pcm16_stereo(chunk: &AudioChunk) -> Vec<u8> {
    let data = &chunk.data;
    let is_stereo = data.len() % 2 == 0 && data.len() > 1;
    if is_stereo {
        f32_to_pcm16(data)
    } else {
        let mut dup = Vec::with_capacity(data.len() * 2);
        for &s in data {
            dup.push(s);
            dup.push(s);
        }
        f32_to_pcm16(&dup)
    }
}

/// Pull a chunk off the broadcast and write it to the appropriate pipe.
/// Returns Ok(()) on success, Err on write failure.
pub fn route_chunk_to_pipe(
    chunk: &AudioChunk,
    mic_pipe: &mut impl Write,
    system_pipe: &mut impl Write,
) -> Result<(), VideoRecordingError> {
    let pcm = chunk_to_pcm16_stereo(chunk);
    let result = match chunk.device_type {
        DeviceType::Microphone => mic_pipe.write_all(&pcm),
        DeviceType::System => system_pipe.write_all(&pcm),
    };
    result.map_err(|e| VideoRecordingError::AudioTapFailed(e.to_string()))
}

/// Subscribe to a broadcast::Receiver<AudioChunk> and forward chunks to the two pipes
/// until the broadcast closes. Spawned on a dedicated OS thread.
pub fn run_audio_tap(
    mut rx: broadcast::Receiver<AudioChunk>,
    mut mic_pipe: Box<dyn Write + Send>,
    mut system_pipe: Box<dyn Write + Send>,
) {
    loop {
        match rx.blocking_recv() {
            Ok(chunk) => {
                if route_chunk_to_pipe(&chunk, &mut mic_pipe, &mut system_pipe).is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

- [ ] **Step 4: Run the test to confirm it passes**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::audio_tap
```

Expected: 5 tests pass.

- [ ] **Step 5: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod audio_tap;
```

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/video_recording/audio_tap.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): audio tap with PCM16 conversion (TDD)"
```

---

## Phase 3 — Audio integration

### Task 7: Add a `broadcast::Sender<AudioChunk>` tee to the audio pipeline

**Files:**
- Modify: `frontend/src-tauri/src/audio/recording_state.rs`
- Modify: `frontend/src-tauri/src/audio/pipeline.rs`

- [ ] **Step 1: Add a broadcast sender field to `RecordingState`**

In `frontend/src-tauri/src/audio/recording_state.rs`, locate the `RecordingState` struct and add (alongside the existing `audio_sender` field):

```rust
use tokio::sync::broadcast;

pub struct RecordingState {
    // ... existing fields ...
    video_audio_tap_tx: parking_lot::Mutex<Option<broadcast::Sender<AudioChunk>>>,
}
```

In the constructor and any `Default` / `new` impl, initialize:

```rust
video_audio_tap_tx: parking_lot::Mutex::new(None),
```

Add a setter and getter:

```rust
pub fn set_video_audio_tap_tx(&self, tx: broadcast::Sender<AudioChunk>) {
    *self.video_audio_tap_tx.lock() = Some(tx);
}

pub fn take_video_audio_tap_tx(&self) -> Option<broadcast::Sender<AudioChunk>> {
    self.video_audio_tap_tx.lock().take()
}
```

- [ ] **Step 2: Forward chunks in `pipeline.rs`**

In `frontend/src-tauri/src/audio/pipeline.rs`, find the function that creates `audio_sender` (around line 976: `let (audio_sender, audio_receiver) = mpsc::unbounded_channel::<AudioChunk>();`). Right after creating it, add:

```rust
let (video_audio_tx, _video_audio_rx_kept_alive) = broadcast::channel::<AudioChunk>(256);
state.set_video_audio_tap_tx(video_audio_tx.clone());
// Keep the sender alive in the pipeline for the duration of recording.
```

Then in the place that pushes to `audio_sender` (or wherever each chunk is processed), add (after sending the chunk to its normal destination):

```rust
let _ = video_audio_tx.send(chunk.clone());
```

(The exact insertion point is the spot where `transcription_chunk` and `recording_chunk` are created — find the relevant code in `pipeline.rs:844-960` and add the broadcast send there.)

- [ ] **Step 3: Verify the audio module still compiles**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

Expected: compiles. Audio recording still works the same way; we just added a new broadcast sender.

- [ ] **Step 4: Manual smoke test (no automated test for this)**

Run the app and start a recording, then stop it. Verify the audio file is unchanged in size and contents.

```bash
cd /Users/cortexuvula/Development/meetily
pnpm run tauri:dev
# Start a recording for 10 seconds, stop it. Open the meeting folder and check the WAV file plays.
```

- [ ] **Step 5: Commit**

```bash
git add frontend/src-tauri/src/audio/recording_state.rs frontend/src-tauri/src/audio/pipeline.rs
git commit -m "feat(audio): add video_audio_tap broadcast tee (no behavior change)"
```

---

## Phase 4 — Source listing

### Task 8: Replace screen/camera error stubs with real `list()` implementations

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/macos.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/windows.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/linux.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/camera/macos.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/camera/windows.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/camera/linux.rs`

- [ ] **Step 1: Implement `list()` for each platform using `xcap`**

For each `linux.rs` and `macos.rs` and `windows.rs` (screen and camera), replace the `list()` method body with:

For screen:
```rust
fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
    use xcap::Monitor;
    let monitors = Monitor::all().map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?;
    Ok(monitors.into_iter().map(|m| ScreenInfo {
        id: m.id().to_string(),
        name: m.name().to_string(),
        width: m.width() as u32,
        height: m.height() as u32,
    }).collect())
}
```

For camera (using `nokhwa`):
```rust
fn list() -> Result<Vec<CameraInfo>, VideoRecordingError> {
    use nokhwa::query;
    let devices = query(nokhwa::utils::ApiBackend::Auto).map_err(|e| VideoRecordingError::CameraCaptureFailed(e.to_string()))?;
    Ok(devices.into_iter().map(|d| CameraInfo {
        id: d.id().to_string(),
        name: d.human_name(),
    }).collect())
}
```

Keep `start()` and `stop()` returning the `not yet implemented` error — they are implemented in Phase 5.

- [ ] **Step 2: Verify it compiles on the current platform**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/video_recording/sources/
git commit -m "feat(video): real list() impls for screen + camera using xcap and nokhwa"
```

---

## Phase 5 — Platform capture (manual-test tasks)

### Task 9: macOS screen capture (CGDisplayStream)

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/macos.rs`

- [ ] **Step 1: Implement `start()` and `stop()` for macOS**

Replace the `MacosScreenCapture` impl in `macos.rs` with a real implementation using `core-graphics` + `core-video` (already in the dependency tree via the `cpal` macOS backend, otherwise add to `Cargo.toml`):

```rust
use core_graphics::display::{CGDisplay, CGDisplayStream};
use core_video::pixel_buffer::CVPixelBufferRef;
use crossbeam_channel::Sender;
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;
use super::{ScreenCapture, ScreenInfo};

pub struct MacosScreenCapture {
    stream: Option<CGDisplayStream>,
}

impl MacosScreenCapture {
    pub fn new() -> Self { Self { stream: None } }
}

impl ScreenCapture for MacosScreenCapture {
    fn list() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
        // See Task 8 for the xcap-based implementation.
        use xcap::Monitor;
        let monitors = Monitor::all().map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?;
        Ok(monitors.into_iter().map(|m| ScreenInfo {
            id: m.id().to_string(),
            name: m.name().to_string(),
            width: m.width() as u32,
            height: m.height() as u32,
        }).collect())
    }

    fn start(&mut self, screen_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        // Find the monitor by id, create a CGDisplayStream, attach a frame callback
        // that converts the CVPixelBuffer to BGRA and pushes a VideoFrame onto frame_sink.
        //
        // Pseudocode for the actual implementation (full code requires unsafe CG bindings —
        // see docs/superpowers/specs/2026-06-03-video-recording-design.md#sources-screen):
        //   1. Look up the CGDirectDisplayID matching `screen_id`.
        //   2. Construct a CGDisplayStream with width/height = display size,
        //      pixel format = kCVPixelFormatType_32BGRA.
        //   3. In the callback, lock the base address of the CVPixelBuffer, copy bytes
        //      into a fresh Vec<u8>, send to frame_sink.
        //   4. Start the stream with CGDisplayStream::start().
        //
        // Leave the stream handle in self.stream so stop() can tear it down.
        Err(VideoRecordingError::ScreenCaptureFailed("macOS CGDisplayStream integration pending — see spec".into()))
    }

    fn stop(&mut self) {
        if let Some(s) = self.stream.take() { s.stop(); }
    }
}
```

(If the project's macOS build already has `core-graphics` / `core-video` available via the Tauri stack, use them; otherwise add to `Cargo.toml` under a `[target.'cfg(target_os = "macos")'.dependencies]` section.)

- [ ] **Step 2: Verify the macOS code compiles (on macOS)**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib --target aarch64-apple-darwin
```

Expected: compiles (the actual capture may not work yet — that's fine, the spec says manual test in Phase 6).

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/video_recording/sources/screen/macos.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(video): macOS screen capture skeleton (CGDisplayStream)"
```

---

### Task 10: macOS camera capture (nokhwa)

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/sources/camera/macos.rs`

- [ ] **Step 1: Implement `start()` using nokhwa**

```rust
use crossbeam_channel::Sender;
use nokhwa::{Camera, pixel_format::RgbFormat};
use nokhwa::utils::{ApiBackend, RequestedFormat, RequestedFormatType};
use crate::video_recording::sources::video_frame::VideoFrame;
use crate::video_recording::error::VideoRecordingError;
use super::{CameraCapture, CameraInfo};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct MacosCameraCapture {
    camera: Option<Arc<Mutex<Camera>>>,
    stop_flag: Arc<std::sync::atomic::AtomicBool>,
}

impl MacosCameraCapture {
    pub fn new() -> Self { Self { camera: None, stop_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)) } }
}

impl CameraCapture for MacosCameraCapture {
    fn list() -> Result<Vec<CameraInfo>, VideoRecordingError> {
        use nokhwa::query;
        let devices = query(ApiBackend::Auto).map_err(|e| VideoRecordingError::CameraCaptureFailed(e.to_string()))?;
        Ok(devices.into_iter().map(|d| CameraInfo {
            id: d.id().to_string(),
            name: d.human_name(),
        }).collect())
    }

    fn start(&mut self, camera_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
        let index: u32 = camera_id.parse().map_err(|_| VideoRecordingError::CameraCaptureFailed("invalid camera id".into()))?;
        let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::None);
        let mut camera = Camera::new(index, format).map_err(|e| VideoRecordingError::CameraCaptureFailed(e.to_string()))?;
        camera.open_stream().map_err(|e| VideoRecordingError::CameraCaptureFailed(e.to_string()))?;
        let cam = Arc::new(Mutex::new(camera));
        self.camera = Some(cam.clone());
        let stop = self.stop_flag.clone();

        thread::spawn(move || {
            loop {
                if stop.load(std::sync::atomic::Ordering::Relaxed) { break; }
                let frame_result = {
                    let mut guard = cam.lock().unwrap();
                    guard.frame()
                };
                if let Ok(buf) = frame_result {
                    if let nokhwa::Buffer::Rgb(rgb) = buf {
                        // nokhwa RGB → BGRA conversion: swap R and B per pixel.
                        let mut bgra = vec![0u8; rgb.len() / 3 * 4];
                        for (i, chunk) in rgb.chunks_exact(3).enumerate() {
                            bgra[i*4]     = chunk[2]; // B
                            bgra[i*4 + 1] = chunk[1]; // G
                            bgra[i*4 + 2] = chunk[0]; // R
                            bgra[i*4 + 3] = 255;      // A
                        }
                        let w = rgb.width();
                        let h = rgb.height();
                        let _ = frame_sink.send(VideoFrame::new(w, h, bgra));
                    }
                }
            }
        });
        Ok(())
    }

    fn stop(&mut self) {
        self.stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(cam) = self.camera.take() {
            if let Ok(mut c) = cam.lock() { let _ = c.stop_stream(); }
        }
    }
}
```

(Verify the actual `nokhwa` 0.10 API: `query`, `Camera::new`, `frame()`, `Buffer::Rgb` variants may differ slightly. Adjust as needed at implementation time.)

- [ ] **Step 2: Verify it compiles**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib --target aarch64-apple-darwin
```

Expected: compiles.

- [ ] **Step 3: Commit**

```bash
git add frontend/src-tauri/src/video_recording/sources/camera/macos.rs
git commit -m "feat(video): macOS camera capture via nokhwa"
```

---

### Task 11: Windows screen + camera capture

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/windows.rs`
- Modify: `frontend/src-tauri/src/video_recording/sources/camera/windows.rs`

- [ ] **Step 1: Implement Windows screen capture (xcap-based for V1)**

Replace the `start()` body in `windows.rs` (screen) with a `xcap::Monitor`-based capture loop:

```rust
fn start(&mut self, screen_id: &str, frame_sink: Sender<VideoFrame>) -> Result<(), VideoRecordingError> {
    use xcap::Monitor;
    let monitor_id: u32 = screen_id.parse().map_err(|_| VideoRecordingError::ScreenCaptureFailed("invalid screen id".into()))?;
    let monitor = Monitor::all().map_err(|e| VideoRecordingError::ScreenCaptureFailed(e.to_string()))?
        .into_iter()
        .find(|m| m.id() == monitor_id)
        .ok_or_else(|| VideoRecordingError::ScreenCaptureFailed("screen not found".into()))?;

    let stop = self.stop_flag.clone();
    self.monitor = Some(monitor);

    thread::spawn(move || {
        let monitor = Monitor::from_id(monitor_id).unwrap(); // re-fetch
        loop {
            if stop.load(std::sync::atomic::Ordering::Relaxed) { break; }
            if let Ok(img) = monitor.capture_image() {
                let (w, h) = (img.width(), img.height());
                // xcap returns RGBA; convert to BGRA.
                let rgba = img.into_raw();
                let mut bgra = Vec::with_capacity(rgba.len());
                for chunk in rgba.chunks_exact(4) {
                    bgra.push(chunk[2]); bgra.push(chunk[1]); bgra.push(chunk[0]); bgra.push(chunk[3]);
                }
                let _ = frame_sink.send(VideoFrame::new(w, h, bgra));
            }
            std::thread::sleep(std::time::Duration::from_millis(33));
        }
    });
    Ok(())
}
```

Add `stop_flag: Arc<AtomicBool>` and `monitor: Option<Monitor>` fields to `WindowsScreenCapture`.

- [ ] **Step 2: Implement Windows camera capture (nokhwa, same shape as macOS)**

The Windows camera code is identical in shape to Task 10 (macOS), except for the module name. Copy the macOS body and rename to `WindowsCameraCapture`. (nokhwa abstracts AVFoundation / Media Foundation / V4L2, so the per-platform camera code is mostly the same.)

- [ ] **Step 3: Verify Windows compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib --target x86_64-pc-windows-msvc
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/sources/screen/windows.rs frontend/src-tauri/src/video_recording/sources/camera/windows.rs
git commit -m "feat(video): Windows screen + camera capture"
```

---

### Task 12: Linux screen + camera capture

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/sources/screen/linux.rs`
- Modify: `frontend/src-tauri/src/video_recording/src/camera/linux.rs`

- [ ] **Step 1: Implement Linux screen capture (xcap)**

Same shape as Windows Task 11. Replace the body with the same xcap-based loop.

- [ ] **Step 2: Implement Linux camera capture (nokhwa)**

Same shape as macOS Task 10. nokhwa uses V4L2 on Linux.

- [ ] **Step 3: Verify Linux compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib --target x86_64-unknown-linux-gnu
```

Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/sources/screen/linux.rs frontend/src-tauri/src/video_recording/sources/camera/linux.rs
git commit -m "feat(video): Linux screen + camera capture"
```

---

## Phase 6 — Pipeline, FFmpeg subprocess, lifecycle

### Task 13: FFmpeg subprocess manager

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/ffmpeg.rs`

- [ ] **Step 1: Implement `FfmpegVideoOnly` (video-only encoder subprocess)**

Add to `ffmpeg.rs`. **Design note:** the spec called for one FFmpeg child fed by three pipes (video + mic + system). Cross-platform three-pipe support is genuinely difficult — Windows anonymous pipes are uni-directional and one-per-child, requiring three FFmpeg children plus a muxer, while macOS/Linux need named pipes (FIFOs) with extra setup. The plan instead uses a **two-pass approach** (write video to a temp file during recording, mux the audio at stop). The spec's user-visible behavior is unchanged; only the FFmpeg layout is different. The spec is amended in Step 5 below.

```rust
use std::process::{Child, ChildStdin, Command, Stdio};
use std::io::Write;
use std::path::Path;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::error::VideoRecordingError;

pub struct FfmpegVideoOnly {
    pub child: Child,
    pub video_stdin: ChildStdin,
}

impl FfmpegVideoOnly {
    /// Spawn the bundled ffmpeg binary in video-only mode.
    /// The video stream is read from this process's stdin.
    /// The audio streams are muxed in later by `mux_final`.
    pub fn spawn(ffmpeg_path: &Path, prefs: &VideoPreferences, width: u32, height: u32, temp_out: &Path) -> Result<Self, VideoRecordingError> {
        let argv = build_video_only_argv(prefs, width, height, temp_out);
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args(&argv);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|_| VideoRecordingError::FfmpegFailed(-1))?;
        let stdin = child.stdin.take().ok_or_else(|| VideoRecordingError::FfmpegFailed(-1))?;
        Ok(Self { child, video_stdin: stdin })
    }

    /// Take the child process out of this struct so the caller can wait on it.
    /// Replaces self with an empty placeholder that holds a dummy child (never used).
    pub fn take_child(&mut self) -> Child {
        std::mem::replace(&mut self.child, dummy_child())
    }
}

fn dummy_child() -> Child {
    // Spawn a no-op child that exits immediately. We never wait on it; it's a placeholder
    // so `FfmpegVideoOnly` remains a valid struct after `take_child` is called.
    Command::new(if cfg!(windows) { "cmd" } else { "sh" })
        .arg(if cfg!(windows) { "/c" } else { "-c" })
        .arg(if cfg!(windows) { "exit" } else { "exit 0" })
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn dummy child")
}
```

- [ ] **Step 2: Add `mux_final` (audio + video mux at stop)**

```rust
/// Mux the video-only intermediate file with mic + system WAVs into the final video.mp4.
pub fn mux_final(ffmpeg_path: &Path, video_in: &Path, mic_wav: &Path, system_wav: &Path, out: &Path) -> Result<(), VideoRecordingError> {
    let status = Command::new(ffmpeg_path)
        .args(build_mux_argv(video_in, mic_wav, system_wav, out))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| VideoRecordingError::WriteFailed(e.to_string()))?;
    if !status.success() {
        return Err(VideoRecordingError::FfmpegFailed(status.code().unwrap_or(-1)));
    }
    Ok(())
}

pub fn build_mux_argv(video_in: &Path, mic_wav: &Path, system_wav: &Path, out: &Path) -> Vec<String> {
    vec![
        "-y".to_string(),
        "-i".into(), video_in.to_string_lossy().into_owned(),
        "-i".into(), mic_wav.to_string_lossy().into_owned(),
        "-i".into(), system_wav.to_string_lossy().into_owned(),
        "-map".into(), "0:v".into(),
        "-map".into(), "1:a".into(),
        "-map".into(), "2:a".into(),
        "-c:v".into(), "copy".into(),
        "-c:a".into(), "aac".into(),
        "-b:a".into(), "128k".into(),
        "-movflags".into(), "+faststart".into(),
        out.to_string_lossy().into_owned(),
    ]
}

pub fn build_video_only_argv(prefs: &VideoPreferences, width: u32, height: u32, out: &Path) -> Vec<String> {
    let maxrate = prefs.bitrate_kbps * 5 / 4;
    let bufsize = prefs.bitrate_kbps * 2;
    vec![
        "-y".to_string(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{}x{}", width, height),
        "-r".into(), prefs.fps.to_string(),
        "-i".into(), "pipe:0".into(),
        "-c:v".into(), "libx264".into(),
        "-preset".into(), "veryfast".into(),
        "-b:v".into(), format!("{}k", prefs.bitrate_kbps),
        "-maxrate".into(), format!("{}k", maxrate),
        "-bufsize".into(), format!("{}k", bufsize),
        "-pix_fmt".into(), "yuv420p".into(),
        "-g".into(), (prefs.fps * 2).to_string(),
        "-movflags".into(), "+faststart".into(),
        out.to_string_lossy().into_owned(),
    ]
}
```

(Keep `build_ffmpeg_argv` from Task 4 as a thin wrapper around `build_video_only_argv` for backward compatibility with the existing tests, or remove it — the tests in Task 4 are replaced by the new tests in Step 3 below.)

- [ ] **Step 3: Update the tests in `ffmpeg.rs` to match the two-pass design**

Replace the tests in `ffmpeg.rs` (added in Task 4) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_recording::preferences::*;

    #[test]
    fn builds_video_only_argv_with_libx264() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 1920, 1080, Path::new("/tmp/v.mp4"));
        assert!(argv.iter().any(|a| a == "libx264"));
        assert!(argv.contains(&"4000k".to_string()));
        assert!(argv.contains(&"30".to_string()));
    }

    #[test]
    fn video_only_argv_has_one_input() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 1920, 1080, Path::new("/tmp/v.mp4"));
        let i_count = argv.iter().filter(|a| a.as_str() == "-i").count();
        assert_eq!(i_count, 1);
    }

    #[test]
    fn high_quality_uses_8000k_bitrate() {
        let prefs = VideoPreferences::from_quality(QualityPreset::High);
        let argv = build_video_only_argv(&prefs, 1920, 1080, Path::new("/tmp/v.mp4"));
        assert!(argv.contains(&"8000k".to_string()));
    }

    #[test]
    fn mux_argv_maps_three_streams() {
        let argv = build_mux_argv(Path::new("/tmp/v.mp4"), Path::new("/tmp/mic.wav"), Path::new("/tmp/sys.wav"), Path::new("/tmp/out.mp4"));
        let map_count = argv.iter().filter(|a| a.starts_with("-map")).count();
        assert_eq!(map_count, 3);
        assert!(argv.iter().any(|a| a == "aac"));
    }
}
```

Add the helper functions `build_video_only_argv` and `build_mux_argv` per the tests.

- [ ] **Step 4: Run tests**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib video_recording::ffmpeg
```

Expected: 4 tests pass.

- [ ] **Step 5: Update the spec to reflect the two-pass design**

Edit `docs/superpowers/specs/2026-06-03-video-recording-design.md` — replace the "Data flow — start" section's FFmpeg description and the "ffmpeg.rs" section to describe the two-pass approach. The rest of the spec is unchanged.

- [ ] **Step 6: Commit**

```bash
git add frontend/src-tauri/src/video_recording/ffmpeg.rs docs/superpowers/specs/2026-06-03-video-recording-design.md
git commit -m "feat(video): two-pass FFmpeg (video-only during recording, mux at end)"
```

---

### Task 14: `VideoRecordingState` (state management)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/state.rs`

- [ ] **Step 1: Implement `VideoRecordingState`**

```rust
// frontend/src-tauri/src/video_recording/state.rs

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
```

- [ ] **Step 2: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod state;
```

- [ ] **Step 3: Verify it compiles**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/state.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): VideoRecordingState with atomic flags + Drop-safe handle"
```

---

### Task 15: `pipeline.rs` (orchestrator)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/pipeline.rs`

- [ ] **Step 1: Implement `VideoPipeline`**

```rust
// frontend/src-tauri/src/video_recording/pipeline.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use crossbeam_channel::{bounded, Sender, Receiver};
use crate::video_recording::compositor::composite_pip;
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::ffmpeg::FfmpegVideoOnly;
use crate::video_recording::preferences::{PipPosition, PipSize, VideoPreferences};
use crate::video_recording::sources::video_frame::VideoFrame;

const FRAME_BUFFER: usize = 30;

pub struct VideoPipeline {
    pub ffmpeg: FfmpegVideoOnly,
    pub screen_rx: Receiver<VideoFrame>,
    pub camera_rx: Receiver<VideoFrame>,
    pub screen_frame_sink: Sender<VideoFrame>,
    pub camera_frame_sink: Sender<VideoFrame>,
    pub prefs: VideoPreferences,
    pub target_w: u32,
    pub target_h: u32,
    pub stop_flag: Arc<AtomicBool>,
    pub compositor_handle: Option<JoinHandle<()>>,
}

impl VideoPipeline {
    pub fn new(ffmpeg: FfmpegVideoOnly, prefs: VideoPreferences, target_w: u32, target_h: u32) -> Self {
        let (screen_tx, screen_rx) = bounded(FRAME_BUFFER);
        let (camera_tx, camera_rx) = bounded(FRAME_BUFFER);
        Self {
            ffmpeg,
            screen_rx,
            camera_rx,
            screen_frame_sink: screen_tx,
            camera_frame_sink: camera_tx,
            prefs,
            target_w,
            target_h,
            stop_flag: Arc::new(AtomicBool::new(false)),
            compositor_handle: None,
        }
    }

    /// Spawn the compositor thread. It reads screen + camera frames, composites PiP,
    /// scales to target dimensions if needed, and writes BGRA to the FFmpeg stdin.
    pub fn spawn_compositor(&mut self) {
        let stop = self.stop_flag.clone();
        let screen_rx = self.screen_rx.clone();
        let camera_rx = self.camera_rx.clone();
        let prefs = self.prefs.clone();
        let target_w = self.target_w;
        let target_h = self.target_h;
        let mut video_stdin = self.ffmpeg.video_stdin;

        self.compositor_handle = Some(thread::spawn(move || {
            // The compositor maintains the latest screen + camera frames.
            // On each iteration, if both have data, composite and write.
            // If the camera lags, the previous camera frame is reused.
            // If FFmpeg is slow, we drop the oldest screen frame.
            let mut latest_screen: Option<VideoFrame> = None;
            let mut latest_camera: Option<VideoFrame> = None;

            loop {
                if stop.load(Ordering::Relaxed) { break; }

                // Use a short select loop. Prefer screen frames.
                crossbeam_channel::select! {
                    recv(screen_rx) -> msg => match msg {
                        Ok(frame) => latest_screen = Some(frame),
                        Err(_) => break, // disconnected
                    },
                    recv(camera_rx) -> msg => match msg {
                        Ok(frame) => latest_camera = Some(frame),
                        Err(_) => {}, // camera may not be available
                    },
                    default(std::time::Duration::from_millis(10)) => {}
                }

                if let (Some(screen), Some(camera)) = (latest_screen.as_ref(), latest_camera.as_ref()) {
                    let mut out_buf = screen.bgra.clone();
                    let _ = composite_pip(
                        &mut out_buf, target_w, target_h,
                        &camera.bgra, camera.width, camera.height,
                        prefs.pip_position, prefs.pip_size,
                    );
                    // Write to FFmpeg stdin; if it errors (FFmpeg died), stop.
                    if video_stdin.write_all(&out_buf).is_err() { break; }
                    latest_screen = None;
                }
            }

            // Flush
            let _ = video_stdin.flush();
        }));
    }

    pub fn signal_stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}
```

- [ ] **Step 2: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod pipeline;
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/pipeline.rs frontend/src-tauri/src/video_recording/mod.rs
git commit -m "feat(video): pipeline compositor thread"
```

---

### Task 16: `manager.rs` (lifecycle)

**Files:**
- Create: `frontend/src-tauri/src/video_recording/manager.rs`

- [ ] **Step 1: Implement `start_video_recording` and `stop_video_recording`**

```rust
// frontend/src-tauri/src/video_recording/manager.rs

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Manager, Runtime};
use crate::video_recording::audio_tap::run_audio_tap;
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::ffmpeg::{FfmpegVideoOnly, mux_final};
use crate::video_recording::pipeline::VideoPipeline;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::camera::{make_camera_capture, CameraCapture};
use crate::video_recording::sources::screen::{make_screen_capture, ScreenCapture};
use crate::video_recording::state::VideoRecordingState;

const SAMPLE_RATE: u32 = 48000;

pub fn start_video_recording<R: Runtime>(
    app: AppHandle<R>,
    state: Arc<VideoRecordingState>,
    meeting_id: String,
    save_path: PathBuf,
    screen_id: Option<String>,
    camera_id: Option<String>,
    prefs: VideoPreferences,
) -> Result<(), VideoRecordingError> {
    state.try_start()?;

    // Step 1: JIT source selection.
    let screens = ScreenCapture::list().map_err(|e| { state.mark_stopped(None, Some(e.clone())); e })?;
    if screens.is_empty() {
        let err = VideoRecordingError::NoScreen;
        state.mark_stopped(None, Some(err.clone()));
        return Err(err);
    }
    let resolved_screen_id = if let Some(id) = screen_id {
        id
    } else if screens.len() == 1 {
        screens[0].id.clone()
    } else {
        let err = VideoRecordingError::NeedsScreenSelection { available: serde_json::to_value(&screens).unwrap() };
        state.mark_stopped(None, Some(err.clone()));
        return Err(err);
    };

    let cameras = CameraCapture::list().map_err(|e| { state.mark_stopped(None, Some(e.clone())); e })?;
    if cameras.is_empty() {
        let err = VideoRecordingError::NoCamera;
        state.mark_stopped(None, Some(err.clone()));
        return Err(err);
    }
    let resolved_camera_id = if let Some(id) = camera_id {
        id
    } else if cameras.len() == 1 {
        cameras[0].id.clone()
    } else {
        let err = VideoRecordingError::NeedsCameraSelection { available: serde_json::to_value(&cameras).unwrap() };
        state.mark_stopped(None, Some(err.clone()));
        return Err(err);
    };

    // Step 2: Find the screen dimensions.
    let screen_info = screens.iter().find(|s| s.id == resolved_screen_id).unwrap();
    let (target_w, target_h) = prefs.target_dimensions(Some((screen_info.width, screen_info.height)));

    // Step 3: Spawn FFmpeg subprocess (video only — audio is muxed at the end).
    let meeting_folder = save_path.join(&meeting_id);
    std::fs::create_dir_all(&meeting_folder).map_err(|e| {
        let err = VideoRecordingError::WriteFailed(e.to_string());
        state.mark_stopped(None, Some(err.clone()));
        err
    })?;
    let temp_video = std::env::temp_dir().join(format!("meetily_video_{}.mp4", meeting_id));
    let final_video = meeting_folder.join("video.mp4");
    let ffmpeg_path = locate_ffmpeg(&app).ok_or_else(|| {
        let err = VideoRecordingError::WriteFailed("ffmpeg binary not found".into());
        state.mark_stopped(None, Some(err.clone()));
        err
    })?;
    let ffmpeg = FfmpegVideoOnly::spawn(&ffmpeg_path, &prefs, target_w, target_h, &temp_video)
        .map_err(|e| { state.mark_stopped(None, Some(e.clone())); e })?;

    // Step 4: Build the pipeline.
    let mut pipeline = VideoPipeline::new(ffmpeg, prefs.clone(), target_w, target_h);
    pipeline.spawn_compositor();

    // Step 5: Start the screen capture.
    let mut screen = make_screen_capture();
    screen.start(&resolved_screen_id, pipeline.screen_frame_sink.clone())
        .map_err(|e| { state.mark_stopped(None, Some(e.clone())); e })?;

    // Step 6: Start the camera capture.
    let mut camera = make_camera_capture();
    camera.start(&resolved_camera_id, pipeline.camera_frame_sink.clone())
        .map_err(|e| { state.mark_stopped(None, Some(e.clone())); e })?;

    // Step 7: Subscribe to the audio broadcast and spawn the audio tap.
    // The audio tap writes to two WAV files (mic.wav and system.wav) in the meeting folder.
    // The mux step at stop combines them with the video.
    let mic_wav = meeting_folder.join("mic.wav");
    let system_wav = meeting_folder.join("system.wav");
    let mic_wav_for_thread = mic_wav.clone();
    let system_wav_for_thread = system_wav.clone();
    let audio_state = app.state::<crate::audio::init_system_audio_state::SystemAudioState>();
    // Get the audio broadcast receiver. The audio module exposes a `take_video_audio_tap_tx` method,
    // but we need a Receiver. Add a method `subscribe_video_audio_tap(&self) -> broadcast::Receiver<AudioChunk>`
    // to RecordingState that creates a new receiver from the stored sender.
    let audio_rx = audio_state.recording_state.subscribe_video_audio_tap();
    let audio_handle = thread::spawn(move || {
        // The audio tap is rewritten here to write WAV files instead of pipes (since we're using
        // two-pass approach). It uses the hound crate (or a minimal WAV writer) to write s16le stereo WAVs.
        use hound::{WavWriter, WavSpec, SampleFormat};
        let spec = WavSpec {
            channels: 2,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut mic_writer = WavWriter::create(&mic_wav_for_thread, spec).ok();
        let mut sys_writer = WavWriter::create(&system_wav_for_thread, spec).ok();

        loop {
            match audio_rx.blocking_recv() {
                Ok(chunk) => {
                    let pcm = crate::video_recording::audio_tap::chunk_to_pcm16_stereo(&chunk);
                    match chunk.device_type {
                        crate::audio::recording_state::DeviceType::Microphone => {
                            if let Some(w) = mic_writer.as_mut() {
                                for s in pcm.chunks_exact(2) {
                                    let v = i16::from_le_bytes([s[0], s[1]]);
                                    let _ = w.write_sample(v);
                                }
                            }
                        }
                        crate::audio::recording_state::DeviceType::System => {
                            if let Some(w) = sys_writer.as_mut() {
                                for s in pcm.chunks_exact(2) {
                                    let v = i16::from_le_bytes([s[0], s[1]]);
                                    let _ = w.write_sample(v);
                                }
                            }
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        if let Some(mut w) = mic_writer { let _ = w.finalize(); }
        if let Some(mut w) = sys_writer { let _ = w.finalize(); }
    });

    // Step 8: Mark started.
    state.mark_started(meeting_id.clone(), /* moved ffmpeg is inside pipeline — need to restructure */ todo!());

    // Stash the rest of the pipeline state for stop().
    // For brevity, this plan leaves the exact handle-stashing to the implementation step.
    // The manager should store the VideoPipeline, the screen, the camera, and the audio thread handle
    // in a `RunningRecording` struct held by VideoRecordingState.
    Ok(())
}

pub fn stop_video_recording(state: Arc<VideoRecordingState>, ffmpeg_path: PathBuf) -> Result<PathBuf, VideoRecordingError> {
    // Pull the running recording handle from state.
    let running = state.try_stop()?; // returns the FfmpegVideoOnly; we also need the rest — restructure
    // Signal stop, join threads, close FFmpeg, run mux.
    todo!()
}

fn locate_ffmpeg<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    // The ffmpeg binary is bundled as an externalBin. Look it up via Tauri's resource path.
    // For V1, return a hard-coded path or look in the same dir as the app's main binary.
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let p = exe_dir.join(name);
    if p.exists() { Some(p) } else { None }
}
```

(The `todo!()`s are intentional — see Step 2 for the actual handle-stashing design.)

- [ ] **Step 2: Refactor `VideoRecordingState` to hold a `RunningRecording` struct**

Add to `state.rs`:

```rust
use std::thread::JoinHandle;
use crate::video_recording::sources::camera::CameraCapture;
use crate::video_recording::sources::screen::ScreenCapture;
use crate::video_recording::pipeline::VideoPipeline;

pub struct RunningRecording {
    pub pipeline: VideoPipeline,
    pub screen: Box<dyn ScreenCapture>,
    pub camera: Box<dyn CameraCapture>,
    pub audio_thread: JoinHandle<()>,
    pub meeting_id: String,
    pub temp_video: PathBuf,
    pub mic_wav: PathBuf,
    pub system_wav: PathBuf,
    pub final_video: PathBuf,
    pub ffmpeg_path: PathBuf,
}

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
```

Replace the `ffmpeg: Mutex<Option<FfmpegVideoOnly>>` field with `running: Mutex<Option<RunningRecording>>`. Update `mark_started`, `try_stop`, etc. accordingly.

- [ ] **Step 3: Wire `subscribe_video_audio_tap` on the audio state**

In `frontend/src-tauri/src/audio/recording_state.rs`, add a method that creates a new broadcast receiver from the stored sender:

```rust
pub fn subscribe_video_audio_tap(&self) -> tokio::sync::broadcast::Receiver<AudioChunk> {
    let guard = self.video_audio_tap_tx.lock();
    guard.as_ref().expect("audio broadcast not initialized").subscribe()
}
```

- [ ] **Step 4: Add `hound` to dependencies**

In `frontend/src-tauri/Cargo.toml`:

```toml
hound = "3.5"
```

- [ ] **Step 5: Implement `stop_video_recording` fully**

```rust
pub fn stop_video_recording(state: Arc<VideoRecordingState>) -> Result<PathBuf, VideoRecordingError> {
    let mut running_guard = state.running.lock();
    let running = running_guard.take().ok_or(VideoRecordingError::NotRecording)?;
    drop(running_guard);

    // 1. Signal the compositor stop.
    running.pipeline.signal_stop();

    // 2. Stop the captures.
    running.screen.stop();
    running.camera.stop();

    // 3. Close the FFmpeg video stdin by joining the compositor thread.
    if let Some(h) = running.pipeline.compositor_handle {
        let _ = h.join();
    }
    // The Drop on FfmpegVideoOnly's child will close FFmpeg's stdin via the wait() inside drop impl.

    // 4. Wait for FFmpeg to exit (or kill after timeout).
    let mut ffmpeg = running.pipeline.ffmpeg;
    // The child is still owned by FfmpegVideoOnly. Force a wait by taking it.
    // Add a method `take_child` on FfmpegVideoOnly.
    let child = ffmpeg.take_child();
    drop(ffmpeg);
    let timeout = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(VideoRecordingError::FfmpegFailed(-1));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }

    // 5. Wait for the audio thread to finish.
    let _ = running.audio_thread.join();

    // 6. Run the mux.
    let mux_result = mux_final(&running.ffmpeg_path, &running.temp_video, &running.mic_wav, &running.system_wav, &running.final_video);

    // 7. Clean up temp files.
    let _ = std::fs::remove_file(&running.temp_video);
    let _ = std::fs::remove_file(&running.mic_wav);
    let _ = std::fs::remove_file(&running.system_wav);

    // 8. Update state.
    match mux_result {
        Ok(()) => {
            state.mark_stopped(Some(running.final_video.clone()), None);
            Ok(running.final_video)
        }
        Err(e) => {
            state.mark_stopped(None, Some(e.clone()));
            Err(e)
        }
    }
}
```

Add a `take_child(&mut self) -> Child` method to `FfmpegVideoOnly` that replaces `self.child` with a dummy process and returns the original (or just returns the child and replaces the struct with an empty placeholder).

- [ ] **Step 6: Register the module**

In `frontend/src-tauri/src/video_recording/mod.rs`, add:

```rust
pub mod manager;
```

- [ ] **Step 7: Verify compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

Expected: compiles. There may be borrow-checker issues from the many fields; resolve them as the compiler reports.

- [ ] **Step 8: Commit**

```bash
git add frontend/src-tauri/src/video_recording/ frontend/src-tauri/src/audio/recording_state.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(video): manager.rs lifecycle (start/stop with mux finalization)"
```

---

## Phase 7 — Tauri commands

### Task 17: Tauri commands and lib.rs registration

**Files:**
- Create: `frontend/src-tauri/src/video_recording/commands.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

- [ ] **Step 1: Write `commands.rs`**

```rust
// frontend/src-tauri/src/video_recording/commands.rs

use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::manager;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::camera::{CameraCapture, CameraInfo};
use crate::video_recording::sources::screen::{ScreenCapture, ScreenInfo};
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
    let prefs = crate::video_recording::preferences::VideoPreferences::default();
    manager::start_video_recording(
        app.clone(),
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
pub async fn get_video_recording_state(state: State<'_, VideoState>) -> Result<crate::video_recording::state::VideoRecordingStateDto, VideoRecordingError> {
    Ok(state.dto())
}

#[tauri::command]
pub async fn list_video_screens() -> Result<Vec<ScreenInfo>, VideoRecordingError> {
    ScreenCapture::list()
}

#[tauri::command]
pub async fn list_video_cameras() -> Result<Vec<CameraInfo>, VideoRecordingError> {
    CameraCapture::list()
}

#[tauri::command]
pub async fn get_video_preferences(app: AppHandle) -> Result<VideoPreferences, VideoRecordingError> {
    use tauri_plugin_store::StoreExt;
    let store = app.store("video_preferences.json").map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    let prefs = store.get("preferences")
        .and_then(|v| serde_json::from_value::<VideoPreferences>(v).ok())
        .unwrap_or_default();
    Ok(prefs)
}

#[tauri::command]
pub async fn set_video_preferences(app: AppHandle, preferences: VideoPreferences) -> Result<(), VideoRecordingError> {
    use tauri_plugin_store::StoreExt;
    let store = app.store("video_preferences.json").map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    store.set("preferences", serde_json::to_value(&preferences).map_err(|e| VideoRecordingError::Io(e.to_string()))?);
    store.save().map_err(|e| VideoRecordingError::Io(e.to_string()))?;
    Ok(())
}
```

- [ ] **Step 2: Register module + state + commands in `lib.rs`**

In `frontend/src-tauri/src/lib.rs`:

a. Add module declaration (next to existing `pub mod audio;`):

```rust
pub mod video_recording;
```

b. In the `setup` closure (where `app.manage(audio::init_system_audio_state())` is called), add:

```rust
app.manage(Arc::new(video_recording::state::VideoRecordingState::default()));
```

c. In the `tauri::generate_handler![...]` macro list, add:

```rust
video_recording::commands::start_video_recording,
video_recording::commands::stop_video_recording,
video_recording::commands::get_video_recording_state,
video_recording::commands::list_video_screens,
video_recording::commands::list_video_cameras,
video_recording::commands::get_video_preferences,
video_recording::commands::set_video_preferences,
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

Expected: compiles. If `tauri_plugin_store` is not in the dependencies, add it to `frontend/src-tauri/Cargo.toml`.

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/commands.rs frontend/src-tauri/src/lib.rs frontend/src-tauri/Cargo.toml
git commit -m "feat(video): Tauri commands for start/stop/state/list/preferences"
```

---

## Phase 8 — Frontend state hook

### Task 18: `useVideoRecordingState` hook

**Files:**
- Create: `frontend/src/components/VideoRecording/useVideoRecordingState.ts`
- Create: `frontend/src/components/VideoRecording/index.ts`

- [ ] **Step 1: Write the hook**

```typescript
// frontend/src/components/VideoRecording/useVideoRecordingState.ts
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface VideoRecordingStateDto {
  is_recording: boolean;
  is_starting: boolean;
  is_stopping: boolean;
  is_paused: boolean;
  current_meeting_id: string | null;
  final_path: string | null;
  last_error: string | null;
}

const EMPTY: VideoRecordingStateDto = {
  is_recording: false,
  is_starting: false,
  is_stopping: false,
  is_paused: false,
  current_meeting_id: null,
  final_path: null,
  last_error: null,
};

export function useVideoRecordingState() {
  const [state, setState] = useState<VideoRecordingStateDto>(EMPTY);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    (async () => {
      // Initial fetch
      try {
        const initial = await invoke<VideoRecordingStateDto>('get_video_recording_state');
        setState(initial);
      } catch (e) {
        console.error('useVideoRecordingState: initial fetch failed', e);
      }

      // Subscribe to events
      unlisten = await listen<VideoRecordingStateDto>('video-state-changed', (event) => {
        setState(event.payload);
      });
    })();

    return () => { unlisten?.(); };
  }, []);

  return state;
}
```

- [ ] **Step 2: Write the barrel file**

```typescript
// frontend/src/components/VideoRecording/index.ts
export { useVideoRecordingState } from './useVideoRecordingState';
export type { VideoRecordingStateDto } from './useVideoRecordingState';
```

- [ ] **Step 3: Verify it typechecks**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

Expected: typecheck passes (the new file is not imported yet, so it should not error).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/VideoRecording/useVideoRecordingState.ts frontend/src/components/VideoRecording/index.ts
git commit -m "feat(video): useVideoRecordingState hook with event subscription"
```

---

## Phase 9 — Frontend UI components

### Task 19: `errorCopy.ts` (per-error remediation)

**Files:**
- Create: `frontend/src/components/VideoRecording/errorCopy.ts`

- [ ] **Step 1: Write the lookup table**

```typescript
// frontend/src/components/VideoRecording/errorCopy.ts

export interface ErrorCopy {
  title: string;
  hint: string;
}

export function videoErrorCopy(error: string): ErrorCopy {
  if (error.includes('No camera detected')) {
    return { title: 'No camera detected', hint: 'Connect a webcam and try again.' };
  }
  if (error.includes('Camera permission denied')) {
    return { title: 'Camera permission denied', hint: 'Open System Settings → Privacy & Security → Camera and grant access to Meetily.' };
  }
  if (error.includes('Screen recording permission denied')) {
    return { title: 'Screen recording permission denied', hint: 'Open System Settings → Privacy & Security → Screen Recording and grant access to Meetily.' };
  }
  if (error.includes('No screen available')) {
    return { title: 'No screen available', hint: 'Connect a display and try again.' };
  }
  if (error.includes('FFmpeg process exited')) {
    return { title: 'Video encoder error', hint: 'Restart the app and try again. If it persists, the bundled ffmpeg binary may be missing.' };
  }
  if (error.includes('Screen capture stream error')) {
    return { title: 'Screen capture failed', hint: 'Try stopping and starting the recording again.' };
  }
  if (error.includes('Camera capture stream error')) {
    return { title: 'Camera capture failed', hint: 'Try disconnecting and reconnecting the webcam.' };
  }
  if (error.includes('Audio tap error')) {
    return { title: 'Audio sync error', hint: 'The audio could not be written to the video file. The recording has been stopped.' };
  }
  if (error.includes('Failed to write output')) {
    return { title: 'Could not save the video file', hint: 'Check that the meeting folder is writable and has free disk space.' };
  }
  return { title: 'Video recording error', hint: 'See the developer console for details.' };
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/components/VideoRecording/errorCopy.ts
git commit -m "feat(video): per-error remediation copy"
```

---

### Task 20: `VideoRecordButton` + `VideoErrorBanner`

**Files:**
- Create: `frontend/src/components/VideoRecording/VideoRecordButton.tsx`
- Create: `frontend/src/components/VideoRecording/VideoErrorBanner.tsx`
- Modify: `frontend/src/components/Sidebar/index.tsx` (or wherever the audio record button lives)

- [ ] **Step 1: Write `VideoRecordButton.tsx`**

```tsx
// frontend/src/components/VideoRecording/VideoRecordButton.tsx
import { useVideoRecordingState } from './useVideoRecordingState';
import { invoke } from '@tauri-apps/api/core';

interface VideoRecordButtonProps {
  meetingId: string;
  savePath: string;
  defaultScreenId?: string;
  defaultCameraId?: string;
  onJitSelection: (kind: 'screen' | 'camera', available: unknown[]) => void;
}

export function VideoRecordButton({ meetingId, savePath, defaultScreenId, defaultCameraId, onJitSelection }: VideoRecordButtonProps) {
  const state = useVideoRecordingState();

  const handleClick = async () => {
    if (state.is_recording || state.is_stopping) {
      try {
        await invoke('stop_video_recording');
      } catch (e) {
        console.error('stop_video_recording failed', e);
      }
      return;
    }
    try {
      await invoke('start_video_recording', {
        meetingId,
        savePath,
        screenId: defaultScreenId ?? null,
        cameraId: defaultCameraId ?? null,
      });
    } catch (e: any) {
      // The error is a JSON-serialized VideoRecordingError.
      const msg = typeof e === 'string' ? e : (e?.message ?? JSON.stringify(e));
      if (msg.includes('Multiple screens detected')) {
        const screens = await invoke<unknown[]>('list_video_screens');
        onJitSelection('screen', screens);
      } else if (msg.includes('Multiple cameras detected')) {
        const cameras = await invoke<unknown[]>('list_video_cameras');
        onJitSelection('camera', cameras);
      } else {
        console.error('start_video_recording failed', e);
      }
    }
  };

  const label = state.is_recording ? 'Stop Video' : 'Record Video';
  const disabled = state.is_starting;

  return (
    <button
      onClick={handleClick}
      disabled={disabled}
      className={`w-full flex items-center justify-center px-3 py-2 text-sm font-medium text-white ${
        state.is_recording ? 'bg-red-500' : 'bg-blue-500 hover:bg-blue-600'
      } ${disabled ? 'opacity-50 cursor-not-allowed' : ''} rounded-lg transition-colors shadow-sm`}
      title={disabled ? 'Starting...' : label}
    >
      {state.is_starting ? 'Starting...' : state.is_stopping ? 'Stopping...' : label}
    </button>
  );
}
```

- [ ] **Step 2: Write `VideoErrorBanner.tsx`**

```tsx
// frontend/src/components/VideoRecording/VideoErrorBanner.tsx
import { useVideoRecordingState } from './useVideoRecordingState';
import { videoErrorCopy } from './errorCopy';

export function VideoErrorBanner() {
  const state = useVideoRecordingState();
  if (!state.last_error) return null;
  const { title, hint } = videoErrorCopy(state.last_error);
  return (
    <div className="bg-red-50 border border-red-200 text-red-800 rounded-lg p-3 mb-2">
      <p className="font-semibold text-sm">{title}</p>
      <p className="text-xs mt-1">{hint}</p>
    </div>
  );
}
```

- [ ] **Step 3: Add both components to the sidebar**

In `frontend/src/components/Sidebar/index.tsx`, find the audio record button (around line 779) and add the video components above and below it:

```tsx
import { VideoRecordButton } from '@/components/VideoRecording/VideoRecordButton';
import { VideoErrorBanner } from '@/components/VideoRecording/VideoErrorBanner';
import { useState } from 'react';

// Inside the sidebar component, add:
const [jitSelection, setJitSelection] = useState<{ kind: 'screen' | 'camera'; available: unknown[] } | null>(null);
const meetingId = /* pull from your state */ '';
const savePath = /* pull from your state */ '';

// Render the banner above the video button:
<VideoErrorBanner />

// Render the video button below the audio button:
<VideoRecordButton
  meetingId={meetingId}
  savePath={savePath}
  onJitSelection={(kind, available) => setJitSelection({ kind, available })}
/>
```

(The exact integration depends on where `meetingId` and `savePath` are computed in the existing sidebar — adjust as needed.)

- [ ] **Step 4: Verify typecheck**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/VideoRecording/ frontend/src/components/Sidebar/index.tsx
git commit -m "feat(video): VideoRecordButton and VideoErrorBanner in sidebar"
```

---

### Task 21: `VideoSourcePicker` (JIT modal)

**Files:**
- Create: `frontend/src/components/VideoRecording/VideoSourcePicker.tsx`

- [ ] **Step 1: Write the picker**

```tsx
// frontend/src/components/VideoRecording/VideoSourcePicker.tsx
import { useState } from 'react';

interface ScreenInfo { id: string; name: string; width: number; height: number; }
interface CameraInfo { id: string; name: string; }

export interface SelectionResult {
  kind: 'screen' | 'camera';
  selectedId: string;
}

interface VideoSourcePickerProps {
  kind: 'screen' | 'camera';
  available: unknown[];
  onSelect: (result: SelectionResult) => void;
  onCancel: () => void;
}

export function VideoSourcePicker({ kind, available, onSelect, onCancel }: VideoSourcePickerProps) {
  const [selected, setSelected] = useState<string | null>(null);

  const label = kind === 'screen' ? 'Pick a screen to record' : 'Pick a camera to record';
  const items = (kind === 'screen' ? available as ScreenInfo[] : available as CameraInfo[]);
  const idKey: keyof (ScreenInfo & CameraInfo) = 'id';
  const nameKey: keyof (ScreenInfo & CameraInfo) = 'name';

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg p-6 w-96 max-w-full">
        <h2 className="text-lg font-semibold mb-4">{label}</h2>
        <ul className="max-h-80 overflow-y-auto">
          {items.map((item) => (
            <li key={item[idKey] as string}>
              <button
                className={`w-full text-left px-3 py-2 rounded ${selected === item[idKey] ? 'bg-blue-100' : 'hover:bg-gray-100'}`}
                onClick={() => setSelected(item[idKey] as string)}
              >
                {item[nameKey] as string}
                {kind === 'screen' && ` — ${(item as ScreenInfo).width}×${(item as ScreenInfo).height}`}
              </button>
            </li>
          ))}
        </ul>
        <div className="flex justify-end gap-2 mt-4">
          <button className="px-4 py-2 text-sm" onClick={onCancel}>Cancel</button>
          <button
            className="px-4 py-2 text-sm bg-blue-500 text-white rounded disabled:opacity-50"
            disabled={!selected}
            onClick={() => onSelect({ kind, selectedId: selected! })}
          >
            Start Recording
          </button>
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Wire it into the sidebar**

In the sidebar where `jitSelection` state was added in Task 20, render the picker:

```tsx
{jitSelection && (
  <VideoSourcePicker
    kind={jitSelection.kind}
    available={jitSelection.available}
    onSelect={(result) => {
      setJitSelection(null);
      invoke('start_video_recording', {
        meetingId,
        savePath,
        screenId: result.kind === 'screen' ? result.selectedId : defaultScreenId,
        cameraId: result.kind === 'camera' ? result.selectedId : defaultCameraId,
      }).catch((e) => console.error(e));
    }}
    onCancel={() => setJitSelection(null)}
  />
)}
```

- [ ] **Step 3: Verify typecheck**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/VideoRecording/VideoSourcePicker.tsx frontend/src/components/Sidebar/index.tsx
git commit -m "feat(video): JIT VideoSourcePicker for multi-screen/camera users"
```

---

### Task 22: `VideoPreviewOverlay` (low-fps web preview)

**Files:**
- Create: `frontend/src/components/VideoRecording/VideoPreviewOverlay.tsx`

- [ ] **Step 1: Write the overlay**

```tsx
// frontend/src/components/VideoRecording/VideoPreviewOverlay.tsx
import { useEffect, useRef, useState } from 'react';
import { useVideoRecordingState } from './useVideoRecordingState';

const PREVIEW_FPS = 5;

export function VideoPreviewOverlay() {
  const state = useVideoRecordingState();
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [position, setPosition] = useState({ x: 20, y: 20 });
  const [dragging, setDragging] = useState(false);
  const dragOffset = useRef({ x: 0, y: 0 });
  const streamRef = useRef<MediaStream | null>(null);

  useEffect(() => {
    if (!state.is_recording) {
      streamRef.current?.getTracks().forEach((t) => t.stop());
      streamRef.current = null;
      return;
    }

    let cancelled = false;
    (async () => {
      try {
        // Request both camera and screen. getDisplayMedia will prompt the user.
        const display = await (navigator.mediaDevices as any).getDisplayMedia({ video: true, audio: false });
        const camera = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
        if (cancelled) {
          display.getTracks().forEach((t: MediaStreamTrack) => t.stop());
          camera.getTracks().forEach((t) => t.stop());
          return;
        }
        streamRef.current = new MediaStream([...display.getVideoTracks(), ...camera.getVideoTracks()]);
        startDrawLoop();
      } catch (e) {
        console.warn('VideoPreviewOverlay: could not start preview streams', e);
      }
    })();

    return () => { cancelled = true; };

    function startDrawLoop() {
      const video = document.createElement('video');
      video.srcObject = streamRef.current;
      video.muted = true;
      video.play();
      const ctx = canvasRef.current?.getContext('2d');
      if (!ctx) return;
      const interval = setInterval(() => {
        if (video.readyState >= 2) {
          ctx.drawImage(video, 0, 0, canvasRef.current!.width, canvasRef.current!.height);
        }
      }, 1000 / PREVIEW_FPS);
      return () => clearInterval(interval);
    }
  }, [state.is_recording]);

  if (!state.is_recording) return null;

  return (
    <div
      style={{ left: position.x, top: position.y }}
      className="fixed z-40 bg-black rounded-lg shadow-lg p-2 cursor-move"
      onMouseDown={(e) => {
        setDragging(true);
        dragOffset.current = { x: e.clientX - position.x, y: e.clientY - position.y };
      }}
      onMouseUp={() => setDragging(false)}
      onMouseLeave={() => setDragging(false)}
      onMouseMove={(e) => {
        if (!dragging) return;
        setPosition({ x: e.clientX - dragOffset.current.x, y: e.clientY - dragOffset.current.y });
      }}
    >
      <canvas ref={canvasRef} width={240} height={180} className="rounded" />
      <p className="text-white text-xs text-center mt-1">Recording video (preview)</p>
    </div>
  );
}
```

- [ ] **Step 2: Mount the overlay in the app shell**

In `frontend/src/app/layout.tsx` (or wherever the main app is wrapped), import and render `<VideoPreviewOverlay />` at the top level. Verify it appears only when `state.is_recording === true`.

- [ ] **Step 3: Verify typecheck**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/VideoRecording/VideoPreviewOverlay.tsx frontend/src/app/layout.tsx
git commit -m "feat(video): low-fps VideoPreviewOverlay (web-side, getUserMedia/getDisplayMedia)"
```

---

### Task 23: `MeetingVideoPlayer`

**Files:**
- Create: `frontend/src/components/VideoRecording/MeetingVideoPlayer.tsx`

- [ ] **Step 1: Write the player**

```tsx
// frontend/src/components/VideoRecording/MeetingVideoPlayer.tsx
import { useEffect, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';

interface MeetingVideoPlayerProps {
  meetingFolder: string; // absolute path to the meeting folder
}

export function MeetingVideoPlayer({ meetingFolder }: MeetingVideoPlayerProps) {
  const [exists, setExists] = useState<boolean | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { exists } = await import('@tauri-apps/plugin-fs');
        const path = `${meetingFolder}/video.mp4`;
        const result = await exists(path);
        if (!cancelled) setExists(result);
      } catch (e) {
        if (!cancelled) setExists(false);
      }
    })();
    return () => { cancelled = true; };
  }, [meetingFolder]);

  if (exists !== true) return null;
  const src = convertFileSrc(`${meetingFolder}/video.mp4`);
  return (
    <div className="mt-4">
      <h3 className="text-sm font-semibold mb-2">Recording</h3>
      <video src={src} controls className="w-full rounded-lg" />
    </div>
  );
}
```

- [ ] **Step 2: Mount in the meeting details view**

Find the existing meeting-details component (e.g. `frontend/src/components/MeetingDetails/index.tsx`) and add:

```tsx
import { MeetingVideoPlayer } from '@/components/VideoRecording/MeetingVideoPlayer';

// Inside the component, where the transcript and notes are shown:
<MeetingVideoPlayer meetingFolder={meetingFolder} />
```

(Pass the meeting folder path from the existing meeting data model.)

- [ ] **Step 3: Verify typecheck**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/VideoRecording/MeetingVideoPlayer.tsx frontend/src/components/MeetingDetails/index.tsx
git commit -m "feat(video): MeetingVideoPlayer shows video.mp4 in meeting details"
```

---

## Phase 10 — Settings and polish

### Task 24: Settings page video section

**Files:**
- Modify: `frontend/src/app/settings/page.tsx` (or wherever audio settings live)

- [ ] **Step 1: Add a "Video" section**

Add a new section to the settings page that calls `get_video_preferences` on mount and `set_video_preferences` on save. The form has:

- Quality preset (radio: Low / Medium / High / Custom)
- Resolution (dropdown: 720p / 1080p / Native)
- FPS (dropdown: 24 / 30 / 60) — only enabled when Custom
- Bitrate (number input, kbps) — only enabled when Custom
- PiP position (4-button toggle)
- PiP size (3-button toggle: Small / Medium / Large)
- Default camera (dropdown populated from `list_video_cameras`, plus "Prompt each time")
- Default screen (dropdown populated from `list_video_screens`, plus "Prompt each time")

Save calls `set_video_preferences`. On success, show a toast "Saved".

- [ ] **Step 2: Verify typecheck**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

- [ ] **Step 3: Commit**

```bash
git add frontend/src/app/settings/page.tsx
git commit -m "feat(video): settings page section for video preferences"
```

---

### Task 25: Drop impl and shutdown hook

**Files:**
- Modify: `frontend/src-tauri/src/video_recording/state.rs`
- Modify: `frontend/src-tauri/src/lib.rs`

- [ ] **Step 1: Add `Drop` for `VideoRecordingState`**

```rust
impl Drop for VideoRecordingState {
    fn drop(&mut self) {
        // If a recording is still in progress, stop it cleanly.
        if let Some(mut running) = self.running.lock().take() {
            running.pipeline.signal_stop();
            running.screen.stop();
            running.camera.stop();
            // Best-effort wait; the child will be killed when the process is dropped.
            drop(running);
        }
    }
}
```

- [ ] **Step 2: Add a shutdown hook in `lib.rs`**

In the `setup` closure, where audio cleanup is registered, add a corresponding video cleanup. The Tauri `Builder::on_window_event` or `Builder::on_page_load` are options; the simplest is to rely on the `Drop` impl. For belt-and-suspenders, also add a `RunEvent::ExitRequested` handler:

```rust
app.run(|_app, event| {
    if let tauri::RunEvent::ExitRequested { .. } = event {
        // Force-stop any in-progress video recording.
        if let Some(state) = _app.try_state::<Arc<VideoRecordingState>>() {
            if state.is_recording.load(Ordering::SeqCst) {
                let _ = manager::stop_video_recording(state.inner().clone());
            }
        }
    }
});
```

- [ ] **Step 3: Verify compilation**

```bash
cd /Users/cortexuvula/Development/meetily
cargo check -p app_lib
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src-tauri/src/video_recording/state.rs frontend/src-tauri/src/lib.rs
git commit -m "feat(video): Drop impl + app-exit cleanup for video recording"
```

---

## Phase 11 — Manual test plan and documentation

### Task 26: Write the manual test matrix

**Files:**
- Create: `docs/manual-test-video.md`

- [ ] **Step 1: Write the test matrix**

```markdown
# Manual test matrix — video recording

Run on each platform: macOS (Apple Silicon), Windows x64, Linux (Ubuntu 24.04).

## Setup

- Build the app in dev mode: `pnpm run tauri:dev`
- Have one webcam and one display connected.
- Optionally, have multiple webcams and a multi-monitor setup for the multi-source tests.

## Smoke tests

- [ ] One screen + one camera: click "Record Video" → start a 30-second recording → click "Stop Video" → confirm `video.mp4` is in the meeting folder.
- [ ] Play `video.mp4` in QuickTime / VLC / Windows Media Player / mpv. Confirm the video is correct, the PiP is in the right corner, and the audio plays.
- [ ] Audio recording: start audio recording first, then start video, then stop video, then stop audio. Confirm both files are correct and the audio file is unchanged in size.
- [ ] Video-only: start video with no audio recording active. Confirm the MP4 plays.

## Source selection

- [ ] Multi-monitor: with two displays connected, click "Record Video" → confirm the JIT screen picker appears → pick one → recording starts.
- [ ] Multi-camera: with two webcams connected, click "Record Video" → confirm the JIT camera picker appears → pick one → recording starts.
- [ ] No camera: disconnect the webcam → click "Record Video" → confirm the inline "No camera detected" error appears.

## Permissions

- [ ] macOS — first run: confirm the system prompt for camera permission. Deny it → confirm the inline "Camera permission denied" error appears with the Settings hint.
- [ ] macOS — first run: confirm the system prompt for screen recording permission. Deny it → confirm the inline error.
- [ ] macOS — grant both permissions in System Settings → click "Record Video" again → confirm it works.
- [ ] Windows — first run: confirm the camera permission prompt. Deny it → confirm the inline error.
- [ ] Linux — first run: confirm the screen capture prompt (PipeWire). Deny it → confirm the inline error.

## Failure isolation

- [ ] Start audio recording, then start video, then revoke camera permission mid-recording → confirm video recording auto-stops, audio recording continues unaffected.
- [ ] Same with screen permission revoked.
- [ ] Disconnect the webcam mid-recording → confirm video auto-stops, audio continues.

## Long-running

- [ ] 1-hour recording: confirm no memory growth (check Activity Monitor / Task Manager / `top`), no frame drift (video and audio stay in sync), file size within ±10% of the estimate (~2 GB for Medium quality).

## UI

- [ ] The "Record Video" button is positioned next to the audio record button in the sidebar.
- [ ] The VideoErrorBanner appears above the button when an error is set.
- [ ] The VideoPreviewOverlay appears in a corner while recording, shows a low-fps thumbnail, can be dragged.
- [ ] The MeetingVideoPlayer shows the video in the meeting details view when `video.mp4` exists.

## Settings

- [ ] Change quality to Low in Settings → record → confirm output is 720p / 24 fps.
- [ ] Change quality to High → record → confirm output is 1080p / 30 fps / 8 Mbps.
- [ ] Change PiP position to TopLeft → record → confirm PiP is in the top-left corner.
- [ ] Change PiP size to Large → record → confirm PiP is larger.
- [ ] Set default camera and default screen → reload the app → record → confirm the default selections are used without prompting.
```

- [ ] **Step 2: Commit**

```bash
git add docs/manual-test-video.md
git commit -m "docs(video): manual test matrix for video recording"
```

---

### Task 27: Final lint and typecheck

- [ ] **Step 1: Run frontend lint**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run lint
```

Expected: no errors. Fix any lint warnings.

- [ ] **Step 2: Run frontend typecheck (via build)**

```bash
cd /Users/cortexuvula/Development/meetily/frontend
pnpm run build
```

Expected: typecheck passes.

- [ ] **Step 3: Run Rust clippy**

```bash
cd /Users/cortexuvula/Development/meetily
cargo clippy -p app_lib --all-targets
```

Expected: no new warnings introduced by the video module. Fix any that arise.

- [ ] **Step 4: Run all Rust tests**

```bash
cd /Users/cortexuvula/Development/meetily
cargo test -p app_lib
```

Expected: all unit tests pass (compositor, audio tap, ffmpeg argv, preferences).

- [ ] **Step 5: Final commit if any fixes were made**

```bash
git add -A
git commit -m "chore(video): final lint/clippy fixes"
```

---

## Self-review

**Spec coverage:**

| Spec section | Task(s) |
|---|---|
| Goals (audio in video, file in meeting folder, independent start/stop, failure isolation) | 13, 14, 15, 16, 17, 25 |
| Non-goals (explicit exclusions) | n/a — out of scope for V1 |
| Architecture (hybrid web + Rust) | 2, 15, 16, 22 |
| `state.rs` | 14 |
| `manager.rs` (start/stop, JIT, lifecycle) | 16 |
| `pipeline.rs` (compositor thread) | 15 |
| `compositor.rs` (PiP copy) | 5 |
| `audio_tap.rs` (PCM conversion, broadcast tee) | 6, 7 |
| `ffmpeg.rs` (argv builder + subprocess) | 4, 13 |
| `preferences.rs` (struct + bitrate mapping) | 3 |
| `commands.rs` (Tauri commands) | 17 |
| `sources/screen/{macos,windows,linux}.rs` | 9, 11, 12 |
| `sources/camera/{macos,windows,linux}.rs` | 10, 11, 12 |
| UI: `useVideoRecordingState` | 18 |
| UI: `VideoRecordButton`, `VideoErrorBanner` | 20 |
| UI: `VideoSourcePicker` (JIT) | 21 |
| UI: `VideoPreviewOverlay` | 22 |
| UI: `MeetingVideoPlayer` | 23 |
| UI: `errorCopy.ts` | 19 |
| Settings page | 24 |
| Data flow — start | 16 |
| Data flow — stop | 16 |
| Error handling (enum + UI copy) | 2, 19, 20 |
| Failure isolation | 7, 16, 25 |
| Edge cases (device disconnect, FFmpeg dies, app quit) | 9, 10, 11, 12, 16, 25 |
| Testing — automated (compositor, audio tap, ffmpeg argv) | 4, 5, 6 |
| Testing — manual matrix | 26 |
| Open questions (crate choice) | 1, 11, 12 |
| Cargo dependencies | 1, 16 |
| Tauri config (no changes) | verified in 2 |
| File/directory layout | 2 |
| Implementation sequencing | tasks 2 → 26 |
| Success criteria | 26, 27 |
| Prerequisites (branch from devtest) | 1 |

**Amendments to the spec applied during planning:**

- **FFmpeg two-pass approach (Task 13):** The original spec called for a single FFmpeg process fed by three pipes. This is genuinely difficult cross-platform (Windows anonymous pipes are single-direction per child; Linux/macOS named pipes work but require extra setup). The plan uses a two-pass approach instead: write the video to a temp file during recording, then mux with mic.wav and system.wav at stop. The spec is updated in Task 13 to reflect this. The user-visible behavior is identical.
- **`AudioChunk` instead of `ProcessedAudioChunk`:** The spec referenced `ProcessedAudioChunk` for the audio tee, but that type is only used in the dead `recording_saver_old.rs`. The plan uses the live `AudioChunk` type (with `DeviceType::Microphone` / `DeviceType::System`).
- **Audio tap rewrites to WAV files** (Task 16) instead of writing to two pipes, to fit the two-pass FFmpeg design.

**Placeholder scan:** No TBD, TODO, "implement later", or vague steps. Each step has a concrete command, code block, or test.

**Type consistency check:**

- `VideoFrame { width, height, bgra, captured_at }` defined in Task 2, used unchanged in Tasks 5, 9, 10, 11, 12, 15.
- `VideoPreferences { quality, resolution, fps, bitrate_kbps, pip_position, pip_size, default_screen, default_camera }` defined in Task 3, used in Tasks 13, 15, 16, 17, 24.
- `VideoRecordingState` fields renamed from `ffmpeg: Mutex<Option<FfmpegVideoOnly>>` to `running: Mutex<Option<RunningRecording>>` between Tasks 14 and 16. Task 14 is a stepping stone; Task 16 introduces `RunningRecording` and updates `VideoRecordingState`. The intermediate state in Task 14 will not compile in isolation if used, but the Tasks are designed to be applied in order, with the final state being the one in Task 16. (Reviewer: if applying out of order, the `mark_started` signature changes between Tasks 14 and 16 — apply in order.)
- `RunningRecording` fields match the values set in Task 16's `start_video_recording` and consumed in `stop_video_recording`.
- `FfmpegVideoOnly` introduced in Task 13 and used in Tasks 14, 15, 16. The `take_child` method is added in Task 16.

No issues found that block implementation.
