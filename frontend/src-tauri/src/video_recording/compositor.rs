use crate::video_recording::preferences::{PipPosition, PipSize};

/// Copy `camera` (BGRA) into the corner of `screen` (BGRA) determined by `position`.
/// Returns the modified screen buffer.
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

    #[test]
    fn bottom_right_pip_places_camera_in_corner() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let out = composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::BottomRight, PipSize::Small).unwrap();
        assert_eq!(out.len(), 100 * 100 * 4);
        let corner = ((100 * 99 + 95) * 4) as usize;
        assert_eq!(out[corner], 255, "expected blue B channel at bottom-right");
        let top_left = 0_usize;
        assert_eq!(out[top_left + 2], 255, "expected red R channel at top-left");
    }

    #[test]
    fn top_left_pip_places_camera_in_top_left() {
        let mut screen = red_bgra(100, 100);
        let camera = blue_bgra(20, 20);
        let out = composite_pip(&mut screen, 100, 100, &camera, 20, 20, PipPosition::TopLeft, PipSize::Small).unwrap();
        let top_left = 0_usize;
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
