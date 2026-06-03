use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use crate::video_recording::preferences::VideoPreferences;
use crate::video_recording::error::VideoRecordingError;

/// Subprocess for the video-only FFmpeg pass.
/// Video frames are written to `video_stdin` (BGRA rawvideo).
/// The audio streams are muxed in later by `mux_final`.
pub struct FfmpegVideoOnly {
    pub child: Child,
    pub video_stdin: Option<ChildStdin>,
}

impl FfmpegVideoOnly {
    /// Spawn the bundled ffmpeg binary in video-only mode.
    pub fn spawn(
        ffmpeg_path: &Path,
        prefs: &VideoPreferences,
        width: u32,
        height: u32,
        temp_out: &Path,
    ) -> Result<Self, VideoRecordingError> {
        let argv = build_video_only_argv(prefs, width, height, temp_out);
        let mut cmd = Command::new(ffmpeg_path);
        cmd.args(&argv);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| {
            VideoRecordingError::WriteFailed(format!("failed to spawn ffmpeg: {}", e))
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            VideoRecordingError::WriteFailed("ffmpeg stdin not available".into())
        })?;
        Ok(Self { child, video_stdin: Some(stdin) })
    }

    /// Take the child process out of this struct so the caller can wait on it.
    /// Replaces self.child with a long-running no-op placeholder.
    pub fn take_child(&mut self) -> Child {
        std::mem::replace(&mut self.child, dummy_long_running_child())
    }

    /// Take the video stdin out of this struct so the caller can write to it.
    pub fn take_video_stdin(&mut self) -> ChildStdin {
        self.video_stdin
            .take()
            .expect("ffmpeg video stdin already taken")
    }
}

fn dummy_long_running_child() -> Child {
    #[cfg(windows)]
    let mut cmd = { let mut c = Command::new("cmd"); c.arg("/c").arg("ping").arg("127.0.0.1").arg("-n").arg("9999"); c };
    #[cfg(not(windows))]
    let mut cmd = { let mut c = Command::new("sh"); c.arg("-c").arg("sleep 999999"); c };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().expect("failed to spawn placeholder child")
}

/// Build the FFmpeg argv for the video-only first pass.
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

/// Run the mux pass. Returns Ok(()) on success, Err otherwise.
pub fn mux_final(
    ffmpeg_path: &Path,
    video_in: &Path,
    mic_wav: &Path,
    system_wav: &Path,
    out: &Path,
) -> Result<(), VideoRecordingError> {
    let argv = build_mux_argv(video_in, mic_wav, system_wav, out);
    let status = Command::new(ffmpeg_path)
        .args(&argv)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| VideoRecordingError::WriteFailed(format!("failed to spawn ffmpeg: {}", e)))?;
    if !status.success() {
        return Err(VideoRecordingError::FfmpegFailed(status.code().unwrap_or(-1)));
    }
    Ok(())
}

/// Backward-compat shim. Returns the video-only argv. (Kept so existing callers/tests compile.)
pub fn build_ffmpeg_argv(prefs: &VideoPreferences, width: u32, height: u32, out: &Path) -> Vec<String> {
    build_video_only_argv(prefs, width, height, out)
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
}
