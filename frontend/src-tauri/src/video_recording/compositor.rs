use crate::video_recording::preferences::{PipPosition, PipSize};

/// Nearest-neighbor resize of a BGRA buffer.
fn scale_bgra_nearest(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    let src_w_us = src_w as usize;
    let src_h_us = src_h as usize;
    let dst_w_us = dst_w as usize;
    let dst_h_us = dst_h as usize;
    for y in 0..dst_h_us {
        let src_y = (y * src_h_us / dst_h_us) as usize;
        for x in 0..dst_w_us {
            let src_x = (x * src_w_us / dst_w_us) as usize;
            let src_idx = (src_y * src_w_us + src_x) * 4;
            let dst_idx = (y * dst_w_us + x) * 4;
            dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    dst
}

/// Copy `camera` (BGRA) into the corner of `screen` (BGRA) determined by
/// `position`, scaled to `PipSize`. Modifies `screen` in place.
pub fn composite_pip(
    screen: &mut [u8],
    screen_w: u32,
    screen_h: u32,
    camera: &[u8],
    camera_w: u32,
    camera_h: u32,
    position: PipPosition,
    size: PipSize,
) -> Result<(), String> {
    let expected_screen = (screen_w as usize) * (screen_h as usize) * 4;
    if screen.len() != expected_screen {
        return Err(format!("screen buffer size {} does not match dimensions {}x{}", screen.len(), screen_w, screen_h));
    }
    let expected_cam = (camera_w as usize) * (camera_h as usize) * 4;
    if camera.len() != expected_cam {
        return Err(format!("camera buffer size {} does not match dimensions {}x{}", camera.len(), camera_w, camera_h));
    }
    if camera_w == 0 || camera_h == 0 || screen_w == 0 || screen_h == 0 {
        return Err("zero dimension".into());
    }

    let pip_w = ((screen_w as f32) * size.fraction()) as u32;
    let pip_w = pip_w.max(1).min(screen_w);
    let pip_h = ((camera_h as f32) * (pip_w as f32 / camera_w as f32)).round() as u32;
    let pip_h = pip_h.max(1).min(screen_h);

    let scaled = scale_bgra_nearest(camera, camera_w, camera_h, pip_w, pip_h);

    let (x_off, y_off) = match position {
        PipPosition::TopLeft     => (0, 0),
        PipPosition::TopRight    => (screen_w - pip_w, 0),
        PipPosition::BottomLeft  => (0, screen_h - pip_h),
        PipPosition::BottomRight => (screen_w - pip_w, screen_h - pip_h),
    };

    for row in 0..pip_h {
        let src_start = (row * pip_w * 4) as usize;
        let src_end = src_start + (pip_w * 4) as usize;
        let dst_y = y_off + row;
        let dst_start = ((dst_y * screen_w + x_off) * 4) as usize;
        let dst_end = dst_start + (pip_w * 4) as usize;
        screen[dst_start..dst_end].copy_from_slice(&scaled[src_start..src_end]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red_bgra(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for i in (0..v.len()).step_by(4) {
            v[i]     = 0;
            v[i + 1] = 0;
            v[i + 2] = 255;
            v[i + 3] = 255;
        }
        v
    }

    fn blue_bgra(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for i in (0..v.len()).step_by(4) {
            v[i]     = 255;
            v[i + 1] = 0;
            v[i + 2] = 0;
            v[i + 3] = 255;
        }
        v
    }

    fn pixel_bgra(screen: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * w + x) * 4) as usize;
        [screen[idx], screen[idx + 1], screen[idx + 2], screen[idx + 3]]
    }

    #[test]
    fn bottom_right_pip_places_camera_in_corner() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::BottomRight, PipSize::Small).unwrap();
        let pip_w = ((100.0 * 0.15) as u32).max(1).min(100);
        let pip_h = ((20.0 * (pip_w as f32 / 20.0)) as u32).max(1).min(100);
        let pip_x = 100 - pip_w;
        let pip_y = 100 - pip_h;
        assert_eq!(pixel_bgra(&screen, 100, pip_x, pip_y), [255, 0, 0, 255], "top-left of PIP should be blue");
        assert_eq!(pixel_bgra(&screen, 100, 0, 0), [0, 0, 255, 255], "top-left of screen should be red");
        assert_eq!(pixel_bgra(&screen, 100, pip_x + pip_w / 2, pip_y + pip_h / 2), [255, 0, 0, 255], "PIP center should be blue");
    }

    #[test]
    fn top_left_pip_places_camera_in_top_left() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::TopLeft, PipSize::Small).unwrap();
        assert_eq!(pixel_bgra(&screen, 100, 0, 0), [255, 0, 0, 255], "top-left of PIP should be blue");
        assert_eq!(pixel_bgra(&screen, 100, 99, 99), [0, 0, 255, 255], "bottom-right of screen should be red");
    }

    #[test]
    fn rejects_size_mismatch_with_error() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let result = composite_pip(&mut screen, 100, 100, &camera, 30, 30, PipPosition::BottomRight, PipSize::Small);
        assert!(result.is_err());
    }

    #[test]
    fn pip_size_affects_composited_area() {
        let mut screen_a = red_bgra(1000, 1000);
        let mut screen_b = red_bgra(1000, 1000);
        let camera = blue_bgra(640, 480);
        composite_pip(&mut screen_a, 1000, 1000, &camera, 640, 480, PipPosition::TopLeft, PipSize::Small).unwrap();
        composite_pip(&mut screen_b, 1000, 1000, &camera, 640, 480, PipPosition::TopLeft, PipSize::Large).unwrap();
        let small_uses_blue = (10..149).any(|x| pixel_bgra(&screen_a, 1000, x, 10) == [255, 0, 0, 255]);
        let large_uses_blue = (10..299).any(|x| pixel_bgra(&screen_b, 1000, x, 10) == [255, 0, 0, 255]);
        assert!(small_uses_blue, "Small PIP should place blue pixels near the corner");
        assert!(large_uses_blue, "Large PIP should place blue pixels near the corner");
        assert!(small_uses_blue && large_uses_blue, "PIP size must affect area");
    }
}
