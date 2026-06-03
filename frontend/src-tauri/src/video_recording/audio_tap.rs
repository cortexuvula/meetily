use tokio::sync::broadcast;
use std::io::Write;
use crate::audio::recording_state::{AudioChunk, DeviceType};
use crate::video_recording::error::VideoRecordingError;

/// Convert a slice of f32 samples in [-1.0, 1.0] to 16-bit signed PCM (little-endian).
/// Clamps out-of-range values. Uses asymmetric scaling (i16::MAX for positive, -i16::MIN
/// for negative) to correctly cover the full i16 range including i16::MIN for -1.0.
pub fn f32_to_pcm16(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = if clamped >= 0.0 {
            (clamped * i16::MAX as f32) as i16
        } else {
            (clamped * -(i16::MIN as f32)) as i16
        };
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Convert a single AudioChunk to interleaved stereo PCM16. If the chunk is mono
/// (length does not look stereo), it is duplicated to both channels.
pub fn chunk_to_pcm16_stereo(chunk: &AudioChunk) -> Vec<u8> {
    let data = &chunk.data;
    let is_stereo = data.len() % 2 == 0 && data.len() > 1;
    if is_stereo {
        f32_to_pcm16(data)
    } else {
        let mut dup = Vec::with_capacity(data.len() * 2);
        for &s in data {
            dup.push(s);
            dup.push(s);
        }
        f32_to_pcm16(&dup)
    }
}

/// Pull a chunk off the broadcast and write it to the appropriate pipe.
pub fn route_chunk_to_pipe(
    chunk: &AudioChunk,
    mic_pipe: &mut impl Write,
    system_pipe: &mut impl Write,
) -> Result<(), VideoRecordingError> {
    let pcm = chunk_to_pcm16_stereo(chunk);
    let result = match chunk.device_type {
        DeviceType::Microphone => mic_pipe.write_all(&pcm),
        DeviceType::System => system_pipe.write_all(&pcm),
    };
    result.map_err(|e| VideoRecordingError::AudioTapFailed(e.to_string()))
}

/// Subscribe to a broadcast::Receiver<AudioChunk> and forward chunks to the two pipes
/// until the broadcast closes. Spawned on a dedicated OS thread.
pub fn run_audio_tap(
    mut rx: broadcast::Receiver<AudioChunk>,
    mut mic_pipe: Box<dyn Write + Send>,
    mut system_pipe: Box<dyn Write + Send>,
) {
    loop {
        match rx.blocking_recv() {
            Ok(chunk) => {
                if route_chunk_to_pipe(&chunk, &mut mic_pipe, &mut system_pipe).is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::recording_state::{AudioChunk, DeviceType};

    fn make_chunk(device: DeviceType, samples: Vec<f32>) -> AudioChunk {
        AudioChunk {
            data: samples,
            sample_rate: 48000,
            timestamp: 0.0,
            chunk_id: 0,
            device_type: device,
        }
    }

    #[test]
    fn f32_to_pcm16_silent() {
        let pcm = f32_to_pcm16(&[0.0_f32, 0.0, 0.0, 0.0]);
        assert_eq!(pcm, vec![0u8; 8]);
    }

    #[test]
    fn f32_to_pcm16_full_scale_positive() {
        let pcm = f32_to_pcm16(&[1.0_f32]);
        assert_eq!(pcm, vec![0xFF, 0x7F]);
    }

    #[test]
    fn f32_to_pcm16_full_scale_negative() {
        let pcm = f32_to_pcm16(&[-1.0_f32]);
        assert_eq!(pcm, vec![0x00, 0x80]);
    }

    #[test]
    fn f32_to_pcm16_clamps_overflow() {
        let pcm = f32_to_pcm16(&[2.0_f32, -2.0_f32]);
        assert_eq!(pcm, vec![0xFF, 0x7F, 0x00, 0x80]);
    }

    #[test]
    fn chunk_to_pcm16_writes_interleaved_stereo() {
        let chunk = make_chunk(DeviceType::Microphone, vec![0.5, -0.5, 0.5, -0.5]);
        let pcm = chunk_to_pcm16_stereo(&chunk);
        let expected_half_pos = (0.5_f32 * i16::MAX as f32) as i16;
        let expected_half_neg = (0.5_f32 * i16::MIN as f32) as i16;
        let mut expected = Vec::new();
        expected.extend_from_slice(&expected_half_pos.to_le_bytes());
        expected.extend_from_slice(&expected_half_neg.to_le_bytes());
        expected.extend_from_slice(&expected_half_pos.to_le_bytes());
        expected.extend_from_slice(&expected_half_neg.to_le_bytes());
        assert_eq!(pcm, expected);
    }
}
