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
    let maxrate = prefs.bitrate_kbps * 5 / 4;
    let bufsize = prefs.bitrate_kbps * 2;

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
    fn builds_argv_for_medium_quality_1080p_30fps() {
        let prefs = VideoPreferences::from_quality(QualityPreset::Medium);
        let argv = build_ffmpeg_argv(&prefs, 1920, 1080, Path::new("/tmp/out.mp4"));
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
        let i_count = argv.iter().filter(|a| a.as_str() == "-i").count();
        assert_eq!(i_count, 3);
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
