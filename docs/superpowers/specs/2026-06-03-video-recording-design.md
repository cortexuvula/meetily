# Video Recording — Design Spec

**Date:** 2026-06-03
**Status:** Approved — ready for implementation planning
**Author:** OpenCode brainstorming session

## Overview

Add optional **screen + webcam (picture-in-picture) video recording** to Meetily as a sibling artifact to the existing audio recording. The output is a self-contained MP4 file saved in the same meeting folder as the audio file, playable alongside the transcript in the meeting details view. The user can start and stop video recording independently of audio recording.

The capture, compositing, and encoding are done in the Tauri Rust process using the FFmpeg binary that is already bundled as an `externalBin`. The Next.js webview provides only a low-fps live preview, using `getUserMedia` / `getDisplayMedia` for UI feedback. The actual recording stream is independent of the preview.

## Goals

- A user can record the screen plus a small webcam overlay (PiP) while in a meeting, with a single click.
- The resulting MP4 is self-contained: it contains both the screen video, the webcam video composited as PiP, and the same mic + system audio tracks that the audio file has.
- The MP4 lives in the existing meeting folder (`<save_folder>/<MeetingName>/video.mp4`) so it can be played back next to the transcript with no new data model.
- Video start/stop is independent of audio start/stop. Either can run alone or together.
- Video failure must never affect audio recording.

## Non-goals (V1)

- Live streaming, RTMP, WebRTC.
- Per-frame LLM visual analysis.
- Multi-camera simultaneous recording.
- Cloud upload, sharing, trimming, watermarking.
- Webcam-only or screen-only mode (V1 is screen + PiP webcam only).
- Configurable PiP shape (circle, etc.).
- Speaker detection or face tracking.

## Design decisions (locked during brainstorming)

| Decision | Choice |
|---|---|
| Video source | Screen + webcam composited (PiP) |
| Purpose | Saved file, played back alongside the transcript |
| Capture architecture | Hybrid: web preview + Rust capture/encode |
| Start/stop | Independent button, parallel to the audio button |
| Source selection | Just-in-time prompts only when ambiguous (multiple screens or cameras) |
| Audio in video | Both mic and system audio muxed into the MP4 |
| Quality | 1080p / 30 fps / ~4 Mbps H.264, configurable in Settings |
| File location | Same meeting folder as audio, no DB change |
| Live preview | Small in-app overlay (~240×180 px) while recording |
| Error UX | Per-failure-mode inline error with remediation hint |
| Capture pipeline | Approach A — real-time FFmpeg streaming via pipes |

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│  Tauri webview (Next.js / React)                                   │
│  ┌──────────────────────────────────────┐  ┌────────────────────┐  │
│  │ VideoPreviewOverlay (getUserMedia / │  │ VideoRecordButton  │  │
│  │ getDisplayMedia — preview only)      │  │ + state hook       │  │
│  └──────────────┬───────────────────────┘  └─────────┬──────────┘  │
└─────────────────┼─────────────────────────────────────┼────────────┘
   tauri.invoke() │   tauri event: video-state-changed   │
                  ▼                                     ▼
┌────────────────────────────────────────────────────────────────────┐
│  Tauri Rust (app_lib)                                              │
│                                                                    │
│  video_recording/                                                  │
│  ├─ state.rs            — VideoRecordingState (Arc<RwLock>)        │
│  │                          + atomic is_recording / is_paused      │
│  ├─ manager.rs          — orchestrates start/stop/lifecycle        │
│  ├─ pipeline.rs         — spawns capture threads, owns FFmpeg proc │
│  ├─ compositor.rs       — PiP compositing (BGRA → BGRA)            │
│  ├─ audio_tap.rs        — taps existing mic + system streams       │
│  ├─ ffmpeg.rs           — FFmpeg subprocess management             │
│  ├─ preferences.rs      — quality / resolution / fps / PiP prefs   │
│  ├─ commands.rs         — Tauri commands                            │
│  └─ sources/                                                   │   │
│     ├─ screen/{macos,windows,linux}.rs — native screen capture     │
│     └─ camera/{macos,windows,linux}.rs — native camera capture     │
│                                                                    │
│  audio/  (existing)        — exposes broadcast tee for audio_tap   │
└────────────────────────────────────────────────────────────────────┘
                              │
                              ▼ named pipe (Windows) / Unix pipe
                       ┌──────────────────┐
                       │  FFmpeg (binary) │
                       └──────────────────┘
                              │
                              ▼
                    <save_folder>/<MeetingName>/video.mp4
```

## Rust-side components

### `video_recording/state.rs`

Mirrors the shape of `audio/recording_state.rs`. Contains `VideoRecordingState` with:

- `is_recording: AtomicBool`
- `is_paused: AtomicBool` (reserved for V2; always false in V1)
- `is_starting: AtomicBool`, `is_stopping: AtomicBool` for UI feedback
- `current_meeting_id: Mutex<Option<String>>`
- `ffmpeg_child: Mutex<Option<std::process::Child>>`
- `last_error: Mutex<Option<VideoRecordingError>>`
- `final_path: Mutex<Option<PathBuf>>`

Independent of `RecordingState`. App-managed via `app.manage(VideoRecordingState::default())` in `lib.rs`.

### `video_recording/manager.rs`

Public API for start/stop. Coordinates:

1. Resolve the meeting folder from `save_path` and `meeting_id`.
2. Validate the requested screen and camera. If a source is missing and more than one is available, return `NeedsScreenSelection` / `NeedsCameraSelection` so the UI can prompt.
3. Verify the bundled `ffmpeg` binary is reachable.
4. Spawn two capture threads (screen + camera), each pushing BGRA frames into a `crossbeam_channel::bounded(30)`.
5. Spawn the audio tap thread subscribed to the audio broadcast tee.
6. Spawn the compositor thread (compositor reads from both source channels, writes a single BGRA stream to the FFmpeg pipe).
7. Spawn the FFmpeg subprocess with the args described below.
8. Update state to `is_recording = true` and emit `video-state-changed`.

On stop: signal all threads, close the FFmpeg stdin, wait for clean exit (with a 2-second timeout, then `kill`), update state, emit the event with the final path.

### `video_recording/pipeline.rs`

Owns the lifecycle of the capture threads, the audio tap, the compositor, and the FFmpeg child. Implements `Drop` so that a panic or early return in `manager.rs` cannot leak child processes or threads.

### `video_recording/compositor.rs`

Reads BGRA frames from the two source channels, scales the camera frame to the configured PiP size, and pastes it into a corner of the screen frame. Output is a single BGRA stream. Pure CPU composition using `image` for scaling. Drops the oldest frame if the consumer (FFmpeg pipe) is slow — never blocks the producers.

### `video_recording/audio_tap.rs`

Subscribes to the existing `ProcessedAudioChunk` broadcast. The audio pipeline gets a small change: its current mpsc channels get wrapped in a `tokio::sync::broadcast::Sender<ProcessedAudioChunk>`, with the existing audio saver plus this new tap as subscribers.

The tap thread:
1. Receives `ProcessedAudioChunk` (mono or stereo, `f32`).
2. Converts to interleaved 16-bit PCM at 48 kHz (the audio pipeline's native rate).
3. Writes to its end of the FFmpeg pipe.
4. Catches all errors, logs via `perf_error!`, and exits. The original audio saver is unaffected by anything the tap does.

### `video_recording/ffmpeg.rs`

**Two-pass design.** The single-pass 3-pipe approach (one FFmpeg child reading video + mic PCM + system PCM simultaneously) was abandoned because it doesn't work cross-platform: Windows anonymous pipes are uni-directional, and macOS/Linux named pipes complicate lifecycle. Instead, the recording is split into two FFmpeg invocations:

**Pass 1 (during recording) — `FfmpegVideoOnly`.** A single FFmpeg child process reads composited BGRA video frames from its stdin (`pipe:0`) and writes a video-only `.mp4` to a temp file (e.g. `<meeting_folder>/.video-tmp.mp4`). The video encoder runs at the configured bitrate / fps / resolution. No audio is involved in this pass.

```rust
pub struct FfmpegVideoOnly {
    pub child: Child,
    pub video_stdin: ChildStdin,
}
impl FfmpegVideoOnly {
    pub fn spawn(ffmpeg_path: &Path, prefs: &VideoPreferences, width: u32, height: u32, temp_out: &Path) -> Result<Self, VideoRecordingError>;
    pub fn take_child(&mut self) -> Child;
}
pub fn build_video_only_argv(prefs: &VideoPreferences, width: u32, height: u32, out: &Path) -> Vec<String>;
```

Pass 1 argv (illustrative):

```
ffmpeg -y \
  -f rawvideo -pix_fmt bgra -s {W}x{H} -r {fps} -i pipe:0 \
  -c:v libx264 -preset veryfast -b:v {bitrate}k -maxrate {max}k -bufsize {buf}k \
  -pix_fmt yuv420p -g {fps*2} -movflags +faststart \
  <meeting_folder>/.video-tmp.mp4
```

**Pass 2 (at stop) — `mux_final`.** A second FFmpeg invocation muxes the temp video with two WAV files written by the audio tap (`mic.wav`, `system.wav`) into the final `video.mp4`. The video stream is copied (`-c:v copy`) — no re-encoding, so this is fast. The audio is re-encoded to AAC at 128 kbps.

```rust
pub fn build_mux_argv(video_in: &Path, mic_wav: &Path, system_wav: &Path, out: &Path) -> Vec<String>;
pub fn mux_final(ffmpeg_path: &Path, video_in: &Path, mic_wav: &Path, system_wav: &Path, out: &Path) -> Result<(), VideoRecordingError>;
```

Pass 2 argv:

```
ffmpeg -y \
  -i <meeting_folder>/.video-tmp.mp4 \
  -i <meeting_folder>/mic.wav \
  -i <meeting_folder>/system.wav \
  -map 0:v -map 1:a -map 2:a \
  -c:v copy -c:a aac -b:a 128k \
  -movflags +faststart \
  <meeting_folder>/video.mp4
```

- `-map 0:v -map 1:a -map 2:a` — one video track from the temp file, two audio tracks (mic first, system second)
- `-c:v copy` — the temp `.mp4` is already H.264 in `yuv420p`, so the video stream is copied without re-encoding
- `-c:a aac -b:a 128k` — re-encode the WAV PCM tracks to AAC
- The WAV files are written by `audio_tap.rs` in parallel with Pass 1; see that section for the WAV header layout.
- The `metadata` flag for embedding the meeting title into the MP4 container is **deferred to V2** to keep the V1 FFmpeg argv minimal.
- Both arg builders are unit-tested without invoking FFmpeg.

### `video_recording/sources/screen/{macos,windows,linux}.rs`

Platform-specific screen capture behind a common trait:

```rust
pub trait ScreenCapture: Send {
    fn start(&mut self, display_id: DisplayId, frame_sink: mpsc::Sender<VideoFrame>) -> Result<()>;
    fn stop(&mut self);
    fn list_displays() -> Result<Vec<ScreenInfo>>;
}
```

- **macOS** — `CGDisplayStream` (10.15+) or `ScreenCaptureKit` (12.3+). Per-app screen-recording permission required.
- **Windows** — DXGI Desktop Duplication API. Returns a stream of BGRA frames.
- **Linux** — PipeWire screen capture via `pipewire` crate.

The implementation choice between `xcap` and platform-specific bindings is a plan-time decision; the trait keeps the rest of the system clean either way.

### `video_recording/sources/camera/{macos,windows,linux}.rs`

Platform-specific camera capture behind a common trait:

```rust
pub trait CameraCapture: Send {
    fn start(&mut self, device_id: DeviceId, frame_sink: mpsc::Sender<VideoFrame>) -> Result<()>;
    fn stop(&mut self);
    fn list_devices() -> Result<Vec<CameraInfo>>;
}
```

- **macOS** — AVFoundation via `nokhwa` (which wraps it on macOS).
- **Windows** — Media Foundation via `nokhwa`.
- **Linux** — V4L2 via `nokhwa`.

### `video_recording/preferences.rs`

Defines `VideoPreferences`:

```rust
pub struct VideoPreferences {
    pub quality: QualityPreset,        // Low / Medium / High / Custom
    pub resolution: Resolution,        // P720 / P1080 / Native
    pub fps: u32,                      // 24, 30, 60
    pub bitrate_kbps: u32,             // ignored unless quality == Custom
    pub pip_position: PipPosition,     // TopLeft, TopRight, BottomLeft, BottomRight
    pub pip_size: PipSize,             // percent of screen width: Small (15%), Medium (22%), Large (30%)
    pub default_screen: Option<DisplayId>,
    pub default_camera: Option<DeviceId>,
}
```

Quality → bitrate mapping (locked):

| Quality | Resolution | FPS | Video bitrate | Maxrate | Bufsize |
|---|---|---|---|---|---|
| Low | 720p | 24 | 2 Mbps | 2.5 Mbps | 4 Mbps |
| Medium | 1080p | 30 | 4 Mbps | 5 Mbps | 8 Mbps |
| High | 1080p | 30 | 8 Mbps | 10 Mbps | 16 Mbps |
| Custom | (user value) | (user value) | (user value) | (bitrate × 1.25) | (bitrate × 2) |

Persisted in Tauri's `store` plugin (already in use, no new capability needed). Default: `Medium` (1080p / 30 fps / 4 Mbps / BottomRight / Medium size).

`is_paused` exists in `state.rs` for forward-compatibility but is not wired into any Tauri command in V1; the recording is always running or stopped.

### `video_recording/commands.rs` — Tauri commands

| Command | Args | Returns |
|---|---|---|
| `start_video_recording` | `meeting_id`, `save_path`, `screen_id?`, `camera_id?` | `Result<(), VideoRecordingError>` |
| `stop_video_recording` | — | `Result<PathBuf, VideoRecordingError>` (the final MP4 path) |
| `get_video_recording_state` | — | `VideoRecordingStateDto` |
| `list_video_screens` | — | `Vec<ScreenInfo>` |
| `list_video_cameras` | — | `Vec<CameraInfo>` |
| `get_video_preferences` | — | `VideoPreferences` |
| `set_video_preferences` | `VideoPreferences` | `Result<(), VideoRecordingError>` |
| `trigger_video_permission` | — | `Result<VideoPermissions, VideoRecordingError>` |

All registered in `tauri::generate_handler![...]` in `frontend/src-tauri/src/lib.rs`.

## UI components

All in `frontend/src/components/VideoRecording/`:

### `VideoRecordButton.tsx`

Sits next to the existing audio record button in the sidebar. Independent toggle. Shows "Record Video" / "Stop Video". Disabled with tooltip while `isStarting` / `isStopping`.

### `VideoPreviewOverlay.tsx`

Small (~240×180 px) draggable overlay, only mounted when `videoState.isRecording === true`. Uses `getUserMedia` (camera) and `getDisplayMedia` (screen) on the JS side for preview only — independent of the Rust-side capture. Renders a low-fps (~5 fps) `<canvas>` thumbnail. The user copy clarifies that this is "what's roughly being captured," not a pixel-perfect mirror.

### `VideoSourcePicker.tsx`

JIT modal triggered when the backend returns `NeedsScreenSelection` or `NeedsCameraSelection`. Renders the list returned by the corresponding `list_*` command. Returns the selection and retries the original start command.

### `VideoErrorBanner.tsx`

Inline error component shown above the video button when state has an error. Uses platform-specific remediation copy from a small lookup table in `frontend/src/components/VideoRecording/errorCopy.ts`.

### `MeetingVideoPlayer.tsx`

Added to the meeting details view. Checks for `video.mp4` in the meeting folder (path from the existing meeting-folder convention). If present, shows a `<video controls>` element with native controls and a small "PiP / Fullscreen" toggle. If absent, the component renders nothing.

### `useVideoRecordingState.ts`

Hook mirroring `useRecordingState()`. Subscribes to the `video-state-changed` Tauri event, exposes `{ isRecording, isStarting, isStopping, error, finalPath }`.

## Settings additions

In the existing Settings page (wherever the audio preferences are), add a "Video" section with:

- Video Quality preset (Low / Medium / High / Custom)
- Resolution (720p / 1080p / Native)
- FPS (24 / 30 / 60)
- PiP position (4 corners)
- PiP size (Small / Medium / Large)
- Default camera (or "Prompt each time")
- Default screen (or "Prompt each time")

Stored via the existing `store:default` Tauri plugin.

## Data flow — start

1. UI calls `start_video_recording({ meeting_id, save_path, screen_id?, camera_id? })`.
2. `manager.rs` validates prefs and sources. If a JIT selection is needed, returns the typed `NeedsXxxSelection` error.
3. `manager.rs` spawns:
   - Screen capture thread → `screen_tx: crossbeam::Sender<VideoFrame>`
   - Camera capture thread → `camera_tx: crossbeam::Sender<VideoFrame>`
   - Audio tap thread → reads from `audio_broadcast_rx`, writes mic.wav and system.wav to the meeting folder
4. `manager.rs` spawns the compositor thread, which reads from both `screen_tx` / `camera_tx` (wrapped as receivers) and writes to the FFmpeg video pipe.
5. `manager.rs` spawns the FFmpeg subprocess (`FfmpegVideoOnly`) in video-only mode. **Video goes to a temp `.mp4` during recording** (e.g. `<meeting_folder>/.video-tmp.mp4`); the audio tracks are muxed in later at stop.
6. State flips to `is_recording = true`. UI receives `video-state-changed`.

## Data flow — stop

1. UI calls `stop_video_recording()`.
2. `pipeline.rs` sets a shared `stop_flag: AtomicBool = true`.
3. Capture threads see the flag, drop their channel ends, exit.
4. Audio tap flushes remaining buffered chunks, closes `mic.wav` and `system.wav`, exits.
5. Compositor flushes its last frame, drops the FFmpeg video pipe (EOF on pipe 0).
6. `manager.rs` `wait()`s on the FFmpeg child with a 2-second timeout; on timeout, `kill()`. Pass 1 finalizes the temp `.mp4`.
7. `manager.rs` calls `mux_final(...)`. **FFmpeg muxes the temp video with mic.wav and system.wav into the final video.mp4** using stream copy for the video track and AAC re-encode for audio. On success, the temp file is removed; on failure, `FfmpegFailed(code)` or `WriteFailed(msg)` is surfaced.
8. `manager.rs` verifies the final file exists and is non-empty; on success, emits `video-state-changed` with `final_path`; on failure, surfaces the mux error.
9. The meeting folder now contains `video.mp4` alongside the existing audio file and `metadata.json`.

## Error handling

A single `VideoRecordingError` enum returned from every Tauri command. Each variant maps to a specific UI message via `frontend/src/components/VideoRecording/errorCopy.ts`.

```rust
#[derive(Debug, thiserror::Error)]
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
    NeedsScreenSelection { available: Vec<ScreenInfo> },

    #[error("Multiple cameras detected. Please pick which one to record.")]
    NeedsCameraSelection { available: Vec<CameraInfo> },

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
}
```

## Failure isolation (the most important property)

**Video recording failure must never affect audio recording.** Concretely:

- The audio tap is a passive broadcast subscriber; if its pipe closes, the FFmpeg process dies, or the tap thread panics, the original audio stream keeps flowing to the audio saver.
- The capture threads own their own crossbeam channels. If FFmpeg is slow, the bounded channel applies back-pressure and the compositor drops the oldest frame. The audio pipeline is never blocked.
- The state machines are independent. `stop_recording` does not call `stop_video_recording`, and vice versa.
- `pipeline.rs` implements `Drop` so that any unwind in `manager.rs` cleanly tears down the FFmpeg child and the capture threads.

## Edge cases

- **No webcam** — `manager.rs` returns `NoCamera` immediately. Audio continues.
- **Webcam disconnects mid-recording** — capture thread sees the device-gone signal, emits `CameraCaptureFailed("device disconnected")` on the state, recording is auto-stopped. The MP4 is finalized cleanly with whatever frames made it through.
- **Screen capture fails (e.g. user revoked permission mid-recording)** — same as above but with `ScreenCaptureFailed`.
- **FFmpeg dies unexpectedly** — `pipeline.rs` sees the exit code, surfaces `FfmpegFailed(code)`, auto-stops, finalizes a partial MP4 or removes it.
- **Disk fills up** — write errors propagate to `WriteFailed`. Recording stops. The meeting folder has whatever was saved.
- **App quits while recording** — `lib.rs` shutdown hook calls into the audio cleanup; we add a parallel `stop_video_recording` call in the same hook that signals the FFmpeg subprocess, waits up to 2 seconds for clean exit, then kills it.

## Testing

### Automated (Rust)

- `compositor::tests` — unit tests for the PiP compositor: known BGRA inputs → known composited outputs. Pure function, fast.
- `audio_tap::tests` — round-trip a `ProcessedAudioChunk` through the PCM conversion and assert the bytes match a known WAV header.
- `ffmpeg::tests::arg_building` — verify the FFmpeg argv is correct for given preferences. (No actual FFmpeg execution.)

### Manual test matrix (documented in `docs/manual-test-video.md`)

- Start video on macOS, Windows, Linux with one screen + one camera → recording finalizes.
- Multi-monitor user is prompted to pick a screen.
- Multi-camera user is prompted to pick a camera.
- Revoke camera permission mid-recording → recording auto-stops cleanly.
- Revoke screen permission mid-recording → recording auto-stops cleanly.
- Start audio-only → start video → stop video first → stop audio → both files are correct.
- Start video-only (no audio) → stop → MP4 has no audio, plays fine.
- 1-hour recording → no memory growth, no frame drift, file size within ±10% of estimate.
- Play back the resulting MP4 in QuickTime / VLC / Windows Media Player / mpv.

## Open questions (to resolve at plan time, not design time)

- Exact platform-screen-capture crate (`xcap` vs `windows-capture` + `cocoa` + `pipewire`).
- Whether to use `nokhwa` for camera or platform-direct (V4L2 / Media Foundation / AVFoundation). `nokhwa` is the default plan; revisit if it has issues.

## Tauri configuration changes

`frontend/src-tauri/tauri.conf.json` capabilities list: **no changes**. The native screen + camera capture happens via our own Rust code, not via a Tauri plugin. We use only what is already in the capability list (`fs:write-all`, `store:default`, `core:event:default`, etc.).

## Cargo dependencies (new)

- `nokhwa` (camera, cross-platform; AVFoundation / MediaFoundation / V4L2)
- `xcap` (screen, cross-platform; **or** platform-specific bindings — plan-time decision)
- `crossbeam-channel` (likely already in tree) for the frame channels
- `image` (PiP compositor scaling)
- `thiserror` (already in tree)

## File / directory layout

```
frontend/src-tauri/src/video_recording/
├── mod.rs
├── state.rs
├── manager.rs
├── pipeline.rs
├── compositor.rs
├── audio_tap.rs
├── ffmpeg.rs
├── preferences.rs
├── commands.rs
└── sources/
    ├── mod.rs
    ├── screen/
    │   ├── mod.rs
    │   ├── macos.rs
    │   ├── windows.rs
    │   └── linux.rs
    └── camera/
        ├── mod.rs
        ├── macos.rs
        ├── windows.rs
        └── linux.rs

frontend/src/components/VideoRecording/
├── VideoRecordButton.tsx
├── VideoPreviewOverlay.tsx
├── VideoSourcePicker.tsx
├── VideoErrorBanner.tsx
├── MeetingVideoPlayer.tsx
├── useVideoRecordingState.ts
└── errorCopy.ts
```

## Implementation sequencing (hint for the plan)

1. Rust skeleton: `state.rs`, `manager.rs`, `pipeline.rs`, `ffmpeg.rs` (arg builder only, no capture yet).
2. Audio tap and audio broadcast tee.
3. Compositor (unit-testable).
4. Camera capture per platform.
5. Screen capture per platform.
6. Wire all threads into the FFmpeg subprocess.
7. Tauri commands and `lib.rs` registration.
8. Frontend: `useVideoRecordingState` hook, `VideoRecordButton`, `VideoErrorBanner`.
9. Frontend: `VideoSourcePicker` (JIT flow).
10. Frontend: `VideoPreviewOverlay` (web-side).
11. Frontend: `MeetingVideoPlayer` (read-only playback).
12. Settings page additions.
13. Manual test matrix execution.

## Success criteria

- A user can click "Record Video" in the sidebar, record a meeting with screen + PiP webcam, and find a `video.mp4` in the meeting folder that plays back in standard players.
- Video start/stop is fully independent of audio start/stop. Either can run alone.
- Failures in video recording never affect audio recording, and vice versa.
- On macOS / Windows / Linux, the JIT source-selection flow works when the user has multiple screens or cameras.
- All entries in the manual test matrix pass on each platform.

## Prerequisites

- Per `CONTRIBUTING.md` and `AGENTS.md`, feature work forks from `devtest`, not `main`. Implementation must begin by branching from the latest `devtest`:
  ```
  git checkout devtest
  git pull upstream devtest
  git checkout -b feature/video-recording
  ```
- The repo's submodule `backend/whisper.cpp` is unrelated to this feature and does not need to be initialized if only working on the Tauri frontend.
