//! Low-latency Opus settings shared with RTABC.

#![allow(dead_code)] // Each endpoint uses one half of the shared codec surface.

use ruopus::{Application, EncodeError, OpusDecoder, OpusEncoder, PacketError, Signal};

pub const SAMPLE_RATE: usize = 48_000;
pub const CHANNELS: usize = 2;
pub const FRAME_DURATION_MS: usize = 10;
pub const FRAME_SAMPLES_PER_CHANNEL: usize = SAMPLE_RATE * FRAME_DURATION_MS / 1_000;
pub const FRAME_SAMPLES: usize = FRAME_SAMPLES_PER_CHANNEL * CHANNELS;
pub const TARGET_BITRATE: u32 = 128_000;
pub const MAX_PACKET_BYTES: usize = 400;

pub struct AudioEncoder {
    inner: OpusEncoder,
}

impl AudioEncoder {
    pub fn new() -> Self {
        let mut inner = OpusEncoder::new(CHANNELS);
        inner.set_application(Application::Audio);
        inner.set_signal(Signal::Auto);
        inner.set_bitrate(Some(TARGET_BITRATE));
        inner.set_complexity(5);
        inner.set_vbr(true);
        inner.set_inband_fec(true);
        inner.set_packet_loss_perc(10);
        Self { inner }
    }

    pub fn encode(&mut self, pcm: &[f32]) -> Result<Vec<u8>, EncodeError> {
        self.inner.encode_auto(pcm, MAX_PACKET_BYTES)
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }
}

pub struct AudioDecoder {
    inner: OpusDecoder,
}

impl AudioDecoder {
    pub fn new() -> Self {
        Self {
            inner: OpusDecoder::new(CHANNELS),
        }
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<f32>, PacketError> {
        self.inner.decode_packet(packet)
    }

    pub fn conceal(&mut self) -> Vec<f32> {
        self.inner.decode_lost(FRAME_SAMPLES_PER_CHANNEL)
    }

    pub fn recover_fec(&mut self, next_packet: &[u8]) -> Result<Vec<f32>, PacketError> {
        self.inner
            .decode_fec(next_packet, FRAME_SAMPLES_PER_CHANNEL)
    }

    pub fn reset(&mut self) {
        self.inner = OpusDecoder::new(CHANNELS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_frame() -> Vec<f32> {
        (0..FRAME_SAMPLES_PER_CHANNEL)
            .flat_map(|sample| {
                let phase = sample as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
                let value = phase.sin() * 0.25;
                [value, value]
            })
            .collect()
    }

    #[test]
    fn opus_round_trip_has_expected_frame_size() {
        let mut encoder = AudioEncoder::new();
        let mut decoder = AudioDecoder::new();
        let packet = encoder.encode(&tone_frame()).unwrap();
        let decoded = decoder.decode(&packet).unwrap();

        assert!(packet.len() <= MAX_PACKET_BYTES);
        assert_eq!(decoded.len(), FRAME_SAMPLES);
        assert!(decoded.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn plc_produces_one_complete_frame() {
        let mut encoder = AudioEncoder::new();
        let mut decoder = AudioDecoder::new();
        let packet = encoder.encode(&tone_frame()).unwrap();
        decoder.decode(&packet).unwrap();

        let concealed = decoder.conceal();
        assert_eq!(concealed.len(), FRAME_SAMPLES);
        assert!(concealed.iter().all(|sample| sample.is_finite()));
    }
}
