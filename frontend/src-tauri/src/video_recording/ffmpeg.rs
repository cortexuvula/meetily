use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::error::VideoRecordingError;

/// Maximum size of stderr we keep from a failed ffmpeg invocation. Anything
/// beyond the last `MAX_STDERR_BYTES` is dropped (with a marker) to keep the
/// surfaced diagnostic string bounded.
pub const MAX_STDERR_BYTES: usize = 8 * 1024;

/// Subprocess for the video-only FFmpeg pass.
/// Video frames are written to `video_stdin` (BGRA rawvideo).
/// The audio streams are muxed in later by `mux_final`.
pub struct FfmpegVideoOnly {
    pub child: Option<Child>,
    pub video_stdin: Option<ChildStdin>,
    pub stderr: Option<ChildStderr>,
}

impl FfmpegVideoOnly {
    /// Spawn the bundled ffmpeg binary in video-only mode.
    pub fn spawn(
        ffmpeg_path: &Path,
        prefs: &VideoPreferences,
        width: u32,
        height: u32,
        target_w: u32,
        target_h: u32,
        temp_out: &Path,
    ) -> Result<Self, VideoRecordingError> {
        let argv = build_video_only_argv(prefs, width, height, target_w, target_h, temp_out);
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args(&argv);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            VideoRecordingError::WriteFailed(format!("failed to spawn ffmpeg: {}", e))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            VideoRecordingError::WriteFailed("ffmpeg stdin not available".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            VideoRecordingError::WriteFailed("ffmpeg stderr not available".into())
        })?;
        Ok(Self {
            child: Some(child),
            video_stdin: Some(stdin),
            stderr: Some(stderr),
        })
    }

    /// Take the child process out of this struct so the caller can wait on it.
    /// Returns None if it was already taken.
    pub fn take_child(&mut self) -> Option<Child> {
        self.child.take()
    }

    /// Take the video stdin out so the caller can write to it. Returns None if
    /// it was already taken.
    pub fn take_video_stdin(&mut self) -> Option<ChildStdin> {
        self.video_stdin.take()
    }

    /// Take the stderr handle out so the caller can read it after waiting on
    /// the child. Returns None if it was already taken.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }
}

/// Drain a ffmpeg stderr pipe to a String, then truncate to the last
/// `MAX_STDERR_BYTES` bytes (on a UTF-8 char boundary) so the surfaced
/// diagnostic stays bounded.
pub fn read_stderr_to_tail<R: Read>(stderr: R) -> String {
    let mut buf = String::new();
    let mut reader = std::io::BufReader::new(stderr);
    let _ = reader.read_to_string(&mut buf);
    if buf.len() > MAX_STDERR_BYTES {
        let start = buf.len() - MAX_STDERR_BYTES;
        let mut idx = start;
        while idx < buf.len() && !buf.is_char_boundary(idx) {
            idx += 1;
        }
        buf = format!("...[truncated]...\n{}", &buf[idx..]);
    }
    buf
}

impl Drop for FfmpegVideoOnly {
    fn drop(&mut self) {
        // Best-effort kill if the child is still running. The stop function
        // takes the child via `take_child()`, so after a proper stop this
        // is a no-op. This handles the case where the start function
        // returns Err after spawn — without it, the ffmpeg process would
        // be leaked.
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Build the FFmpeg argv for the video-only first pass.
/// Frames are read at `width × height` (the capture's native size) and
/// scaled to `target_w × target_h` before encoding so the output matches
/// the user's resolution preference.
pub fn build_video_only_argv(prefs: &VideoPreferences, width: u32, height: u32, target_w: u32, target_h: u32, out: &Path) -> Vec<String> {
    let maxrate = prefs.bitrate_kbps * 5 / 4;
    let bufsize = prefs.bitrate_kbps * 2;
    let mut argv = vec![
        "-y".to_string(),
        "-f".into(), "rawvideo".into(),
        "-pix_fmt".into(), "bgra".into(),
        "-s".into(), format!("{}x{}", width, height),
        "-r".into(), prefs.fps.to_string(),
        "-i".into(), "pipe:0".into(),
    ];
    if target_w != width || target_h != height {
        argv.push("-vf".into());
        argv.push(format!("scale={}:{}", target_w, target_h));
    }
    argv.extend([
        "-c:v".into(), "libx264".into(),
        "-preset".into(), "veryfast".into(),
        "-b:v".into(), format!("{}k", prefs.bitrate_kbps),
        "-maxrate".into(), format!("{}k", maxrate),
        "-bufsize".into(), format!("{}k", bufsize),
        "-pix_fmt".into(), "yuv420p".into(),
        "-g".into(), (prefs.fps * 2).to_string(),
        "-movflags".into(), "+faststart".into(),
        out.to_string_lossy().into_owned(),
    ]);
    argv
}

/// Build the FFmpeg argv for the mux pass (video + mic.wav + system.wav → final video.mp4).
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

/// Run the mux pass. Returns Ok(()) on success, Err with stderr tail otherwise.
pub fn mux_final(
    ffmpeg_path: &Path,
    video_in: &Path,
    mic_wav: &Path,
    system_wav: &Path,
    out: &Path,
) -> Result<(), VideoRecordingError> {
    let argv = build_mux_argv(video_in, mic_wav, system_wav, out);
    let mut child = Command::new(ffmpeg_path)
        .args(&argv)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| VideoRecordingError::WriteFailed(format!("failed to spawn ffmpeg: {}", e)))?;
    let stderr = child.stderr.take();
    let status = child.wait().map_err(|e| {
        VideoRecordingError::WriteFailed(format!("failed to wait for ffmpeg: {}", e))
    })?;
    if !status.success() {
        let message = stderr.map(read_stderr_to_tail).unwrap_or_default();
        return Err(VideoRecordingError::FfmpegFailed {
            code: status.code().unwrap_or(-1),
            message,
        });
    }
    Ok(())
}

/// Backward-compat shim. Returns the video-only argv. (Kept so existing callers/tests compile.)
pub fn build_ffmpeg_argv(prefs: &VideoPreferences, width: u32, height: u32, out: &Path) -> Vec<String> {
    build_video_only_argv(prefs, width, height, width, height, out)
}

pub fn ffmpeg_argv_with_overrides(base: Vec<String>, _width: u32, _height: u32) -> Vec<String> {
    base
}

pub const VIDEO_PIPE_INDEX: usize = 0;
pub const MIC_PIPE_INDEX: usize = 1;
pub const SYSTEM_PIPE_INDEX: usize = 2;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_recording::preferences::*;

    #[test]
    fn builds_video_only_argv_with_libx264() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 1920, 1080, 1920, 1080, Path::new("/tmp/v.mp4"));
        assert!(argv.iter().any(|a| a == "libx264"));
        assert!(argv.contains(&"4000k".to_string()));
        assert!(argv.contains(&"30".to_string()));
    }

    #[test]
    fn video_only_argv_has_one_input() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 1920, 1080, 1920, 1080, Path::new("/tmp/v.mp4"));
        let i_count = argv.iter().filter(|a| a.as_str() == "-i").count();
        assert_eq!(i_count, 1);
    }

    #[test]
    fn high_quality_uses_8000k_bitrate() {
        let prefs = VideoPreferences::from_quality(QualityPreset::High);
        let argv = build_video_only_argv(&prefs, 1920, 1080, 1920, 1080, Path::new("/tmp/v.mp4"));
        assert!(argv.contains(&"8000k".to_string()));
    }

    #[test]
    fn mux_argv_maps_three_streams() {
        let argv = build_mux_argv(
            Path::new("/tmp/v.mp4"),
            Path::new("/tmp/mic.wav"),
            Path::new("/tmp/sys.wav"),
            Path::new("/tmp/out.mp4"),
        );
        let map_count = argv.iter().filter(|a| a.starts_with("-map")).count();
        assert_eq!(map_count, 3);
        assert!(argv.iter().any(|a| a == "aac"));
    }

    #[test]
    fn adds_scale_filter_when_target_differs_from_input() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 2560, 1440, 1920, 1080, Path::new("/tmp/v.mp4"));
        let vf_idx = argv.iter().position(|a| a == "-vf").expect("missing -vf");
        assert_eq!(argv[vf_idx + 1], "scale=1920:1080");
    }

    #[test]
    fn omits_scale_filter_when_target_matches_input() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_video_only_argv(&prefs, 1920, 1080, 1920, 1080, Path::new("/tmp/v.mp4"));
        assert!(!argv.iter().any(|a| a == "-vf"));
    }

    #[test]
    fn read_stderr_to_tail_truncates_long_output() {
        let long = "x".repeat(MAX_STDERR_BYTES + 1000);
        let cursor = std::io::Cursor::new(long.into_bytes());
        let truncated = read_stderr_to_tail(cursor);
        assert!(truncated.starts_with("...[truncated]..."));
        assert!(truncated.len() <= MAX_STDERR_BYTES + 32);
    }
}
