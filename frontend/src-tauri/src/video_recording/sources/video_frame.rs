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
