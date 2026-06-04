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
