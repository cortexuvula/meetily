use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tauri::{AppHandle, Emitter, Runtime};
use hound::{SampleFormat, WavSpec, WavWriter};
use crate::video_recording::audio_tap::chunk_to_pcm16_stereo;
use crate::video_recording::error::VideoRecordingError;
use crate::video_recording::ffmpeg::FfmpegVideoOnly;
use crate::video_recording::ffmpeg::mux_final;
use crate::video_recording::ffmpeg::read_stderr_to_tail;
use crate::video_recording::pipeline::VideoPipeline;
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::sources::camera::{make_camera_capture, list_cameras};
use crate::video_recording::sources::screen::{list_screens, make_screen_capture};
use crate::video_recording::state::{RunningRecording, VideoRecordingState};

const SAMPLE_RATE: u32 = 48000;
const AUDIO_CHANNELS: u16 = 2;
const SILENT_WAV_SECONDS: u32 = 1;

/// Create a silent stereo WAV file at 48 kHz of the given duration. Used as a
/// placeholder when the audio recording is not active so the two-pass FFmpeg
/// mux can still include an audio track of meaningful duration.
fn create_silent_wav(path: &Path, seconds: u32) -> Result<(), VideoRecordingError> {
    let spec = WavSpec {
        channels: AUDIO_CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)
        .map_err(|e| VideoRecordingError::WriteFailed(format!("create silent wav: {}", e)))?;
    let total_samples = (SAMPLE_RATE as usize) * (seconds as usize) * (AUDIO_CHANNELS as usize);
    for _ in 0..total_samples {
        writer
            .write_sample(0i16)
            .map_err(|e| VideoRecordingError::WriteFailed(format!("write silent sample: {}", e)))?;
    }
    writer
        .finalize()
        .map_err(|e| VideoRecordingError::WriteFailed(format!("finalize silent wav: {}", e)))?;
    Ok(())
}

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

    // Helper: every mark_failed path also emits the state change so the UI updates
    // and logs the error so we can see which failure mode fired.
    let notify_failed = |err: &VideoRecordingError| {
        log::warn!("[video] start: failure — {}", err);
        state.mark_failed(err.clone());
        let _ = app.emit("video-state-changed", state.dto());
    };

    // 1. Resolve screen and camera.
    let screens = list_screens()
        .map_err(|e| { notify_failed(&e); e })?;
    log::info!("[video] start: found {} screen(s)", screens.len());
    if screens.is_empty() {
        let err = VideoRecordingError::NoScreen;
        notify_failed(&err);
        return Err(err);
    }
    let resolved_screen_id = match screen_id {
        Some(id) => id,
        None if screens.len() == 1 => screens[0].id.clone(),
        None => {
            let err = VideoRecordingError::NeedsScreenSelection {
                available: serde_json::to_value(&screens).unwrap_or(serde_json::Value::Null),
            };
            notify_failed(&err);
            return Err(err);
        }
    };

    let cameras = list_cameras()
        .map_err(|e| { notify_failed(&e); e })?;
    log::info!("[video] start: found {} camera(s)", cameras.len());
    if cameras.is_empty() {
        let err = VideoRecordingError::NoCamera;
        notify_failed(&err);
        return Err(err);
    }
    let resolved_camera_id = match camera_id {
        Some(id) => id,
        None if cameras.len() == 1 => cameras[0].id.clone(),
        None => {
            let err = VideoRecordingError::NeedsCameraSelection {
                available: serde_json::to_value(&cameras).unwrap_or(serde_json::Value::Null),
            };
            notify_failed(&err);
            return Err(err);
        }
    };

    // 2. Compute capture and target dimensions. The screen capture is always
    //    at its native resolution; ffmpeg downscales to the user's target.
    let screen_info = screens.iter().find(|s| s.id == resolved_screen_id).unwrap();
    let capture_w = screen_info.width;
    let capture_h = screen_info.height;
    let (target_w, target_h) = prefs.target_dimensions(Some((capture_w, capture_h)));

    // 3. Set up paths.
    let meeting_folder = save_path.join(&meeting_id);
    std::fs::create_dir_all(&meeting_folder).map_err(|e| {
        let err = VideoRecordingError::WriteFailed(e.to_string());
        notify_failed(&err);
        err
    })?;
    let temp_video = std::env::temp_dir().join(format!("meetily_video_{}.mp4", meeting_id));
    let final_video = meeting_folder.join("video.mp4");
    let mic_wav = meeting_folder.join("mic.wav");
    let system_wav = meeting_folder.join("system.wav");

    // 4. Locate FFmpeg binary.
    let ffmpeg_path = locate_ffmpeg().ok_or_else(|| {
        let err = VideoRecordingError::WriteFailed("ffmpeg binary not found".into());
        notify_failed(&err);
        err
    })?;

    // 5. Spawn the video-only FFmpeg subprocess.
    let ffmpeg = FfmpegVideoOnly::spawn(
        &ffmpeg_path,
        &prefs,
        capture_w,
        capture_h,
        target_w,
        target_h,
        &temp_video,
    )
    .map_err(|e| { notify_failed(&e); e })?;

    // 6. Build the pipeline and spawn the compositor.
    let mut pipeline = VideoPipeline::new(ffmpeg, prefs.clone(), capture_w, capture_h);
    let video_stdin = pipeline.take_video_stdin().ok_or_else(|| {
        let err = VideoRecordingError::WriteFailed("ffmpeg video stdin already taken".into());
        notify_failed(&err);
        err
    })?;
    pipeline.spawn_compositor(app.clone(), video_stdin);

    // 7. Start the screen capture.
    let mut screen = make_screen_capture();
    screen.start(&resolved_screen_id, pipeline.screen_frame_sink.clone())
        .map_err(|e| { notify_failed(&e); e })?;

    // 8. Start the camera capture.
    let mut camera = make_camera_capture();
    camera.start(&resolved_camera_id, pipeline.camera_frame_sink.clone())
        .map_err(|e| { notify_failed(&e); e })?;

    // 9. Subscribe to the audio broadcast and spawn a thread that writes WAV files.
    //    If the audio recording is not active, generate silent placeholder WAVs
    //    and skip the audio thread — the video will have silent audio tracks.
    let audio_stop = Arc::new(AtomicBool::new(false));
    let audio_thread = match crate::audio::recording_commands::current_recording_state() {
        Some(audio_state) => {
            log::info!("[video] start: audio recording is active, will mux live audio");
            let mut audio_rx = audio_state.subscribe_video_audio_tap();
            let audio_stop_for_thread = audio_stop.clone();

            let mic_wav_for_thread = mic_wav.clone();
            let system_wav_for_thread = system_wav.clone();
            Some(thread::spawn(move || {
                let spec = WavSpec {
                    channels: AUDIO_CHANNELS,
                    sample_rate: SAMPLE_RATE,
                    bits_per_sample: 16,
                    sample_format: SampleFormat::Int,
                };
                let mut mic_writer = WavWriter::create(&mic_wav_for_thread, spec).ok();
                let mut sys_writer = WavWriter::create(&system_wav_for_thread, spec).ok();

                loop {
                    if audio_stop_for_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    match audio_rx.try_recv() {
                        Ok(chunk) => {
                            let pcm = chunk_to_pcm16_stereo(&chunk);
                            // The audio pipeline currently broadcasts a single
                            // mixed stream tagged as Microphone. Write the same
                            // PCM data to both the mic and system tracks so the
                            // mux pass has two valid audio inputs to combine.
                            for w in [mic_writer.as_mut(), sys_writer.as_mut()].into_iter().flatten() {
                                for s in pcm.chunks_exact(2) {
                                    let v = i16::from_le_bytes([s[0], s[1]]);
                                    let _ = w.write_sample(v);
                                }
                            }
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                            thread::sleep(std::time::Duration::from_millis(20));
                        }
                    }
                }
                if let Some(w) = mic_writer.take() { let _ = w.finalize(); }
                if let Some(w) = sys_writer.take() { let _ = w.finalize(); }
            }))
        }
        None => {
            log::warn!("[video] start: audio recording is not active, recording video with silent audio tracks");
            create_silent_wav(&mic_wav, SILENT_WAV_SECONDS)?;
            create_silent_wav(&system_wav, SILENT_WAV_SECONDS)?;
            None
        }
    };

    // 10. Mark started and store the running recording.
    state.mark_started(meeting_id.clone());
    log::info!("[video] start: mark_started called, dto={:?}", state.dto());
    let _ = app.emit("video-state-changed", state.dto());
    log::info!("[video] start: emit dispatched");
    state.store_running(RunningRecording {
        pipeline,
        screen,
        camera,
        audio_thread,
        audio_stop,
        meeting_id,
        temp_video,
        mic_wav,
        system_wav,
        final_video,
        ffmpeg_path,
    });

    Ok(())
}

pub fn stop_video_recording<R: Runtime>(app: AppHandle<R>, state: Arc<VideoRecordingState>) -> Result<PathBuf, VideoRecordingError> {
    log::info!("[video] stop: stop_video_recording called");
    let mut running = state.take_running().ok_or(VideoRecordingError::NotRecording)?;

    state.mark_stopping();
    let _ = app.emit("video-state-changed", state.dto());

    // 1. Signal the audio thread to stop. Using a flag + polling means the
    //    thread will exit even if the audio broadcast sender is still alive
    //    (which it will be when audio recording is concurrently active).
    running.audio_stop.store(true, Ordering::Relaxed);

    // 2. Signal the compositor stop.
    running.pipeline.signal_stop();

    // 3. Stop the captures (these also signal their threads via the stop_flag).
    running.screen.stop();
    running.camera.stop();

    // 4. Wait for the compositor thread to drain its last frames and exit.
    //    When the thread exits, its owned `video_stdin` is dropped, which
    //    closes the underlying pipe and signals EOF to ffmpeg — so ffmpeg
    //    finalizes the temp video.
    if let Some(handle) = running.pipeline.compositor_handle.take() {
        let _ = handle.join();
    }

    // 5. Take the FFmpeg child and stderr handle out so we can wait on it
    //    and read its diagnostic output. After we have them, drop the
    //    wrapper — both fields are `Option`, so no dummy is spawned.
    let mut child = running.pipeline.take_ffmpeg_child();
    let stderr_handle = running.pipeline.take_ffmpeg_stderr();

    // 6. Wait for FFmpeg to finalize the temp video (with a 2-second timeout).
    let timeout = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    let exit_status = match child.as_mut() {
        Some(c) => loop {
            match c.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = c.kill();
                        let _ = c.wait();
                        break None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => break None,
            }
        },
        None => None,
    };
    let first_pass_stderr = stderr_handle.map(read_stderr_to_tail).unwrap_or_default();
    drop(child);
    if exit_status.is_none() {
        let err = VideoRecordingError::FfmpegFailed {
            code: -1,
            message: if first_pass_stderr.is_empty() {
                "ffmpeg first pass did not exit within 2s; killed".into()
            } else {
                first_pass_stderr
            },
        };
        state.mark_stopped(None, Some(err.clone()));
        let _ = app.emit("video-state-changed", state.dto());
        return Err(err);
    }

    // 7. Wait for the audio thread to finish (so WAV files are finalized).
    //    Use take() so the Drop for RunningRecording has nothing left to do
    //    for this handle.
    if let Some(handle) = running.audio_thread.take() {
        let _ = handle.join();
    }

    // 8. Surface capture/compositor errors to the state. Screen failure is
    //    most severe (no video at all), then compositor failure (ffmpeg
    //    write failed or screen channel closed), then camera failure
    //    (recording continues with screen only). If a severe capture
    //    error is present, skip the mux — the temp file is unusable.
    let screen_err = running.screen.last_error();
    let compositor_err = running.pipeline.last_error.lock().clone();
    let camera_err = running.camera.last_error();
    let capture_failure = screen_err
        .or(compositor_err)
        .map(VideoRecordingError::ScreenCaptureFailed)
        .or_else(|| camera_err.map(VideoRecordingError::CameraCaptureFailed));

    // 9. Run the mux (only if no severe capture failure).
    let mux_result: Result<(), VideoRecordingError> = if capture_failure.is_some() {
        Err(VideoRecordingError::WriteFailed(
            "skipping mux due to capture failure".into(),
        ))
    } else {
        mux_final(
            &running.ffmpeg_path,
            &running.temp_video,
            &running.mic_wav,
            &running.system_wav,
            &running.final_video,
        )
    };

    // 10. Clean up temp files (best-effort).
    let _ = std::fs::remove_file(&running.temp_video);
    let _ = std::fs::remove_file(&running.mic_wav);
    let _ = std::fs::remove_file(&running.system_wav);

    // 11. Update state. Capture failure takes priority over mux failure.
    let mux_err = mux_result.as_ref().err().cloned();
    match mux_result {
        Ok(()) => {
            let final_path = running.final_video.clone();
            let final_error = capture_failure.or(mux_err);
            state.mark_stopped(Some(final_path.clone()), final_error.clone());
            let _ = app.emit("video-state-changed", state.dto());
            log::info!("[video] stop: emit dispatched (success)");
            if let Some(e) = final_error {
                Err(e)
            } else {
                Ok(final_path)
            }
        }
        Err(e) => {
            let final_error = capture_failure.unwrap_or(e);
            state.mark_stopped(None, Some(final_error.clone()));
            let _ = app.emit("video-state-changed", state.dto());
            log::info!("[video] stop: emit dispatched (error)");
            Err(final_error)
        }
    }
}

fn locate_ffmpeg() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let p = exe_dir.join(name);
    if p.exists() { Some(p) } else { None }
}
