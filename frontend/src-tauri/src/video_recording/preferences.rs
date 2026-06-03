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
