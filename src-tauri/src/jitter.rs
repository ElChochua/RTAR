use std::collections::BTreeMap;

const DEFAULT_PREFILL_PACKETS: usize = 2;
const DEFAULT_MAX_PACKETS: usize = 8;
const MAX_CONSECUTIVE_MISSING: u32 = 6;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BufferedAudioPacket {
    pub sequence: u32,
    pub timestamp_us: u64,
    pub flags: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PlayoutItem {
    Waiting,
    Packet(BufferedAudioPacket),
    Missing {
        sequence: u32,
        next_payload: Option<Vec<u8>>,
    },
    Reset,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JitterStats {
    pub accepted: u64,
    pub duplicates: u64,
    pub late: u64,
    pub overflow_drops: u64,
    pub missing: u64,
    pub resets: u64,
}

pub struct AudioJitterBuffer {
    packets: BTreeMap<u32, BufferedAudioPacket>,
    expected: Option<u32>,
    started: bool,
    prefill_packets: usize,
    max_packets: usize,
    consecutive_missing: u32,
    stats: JitterStats,
}

impl AudioJitterBuffer {
    pub fn new() -> Self {
        Self::with_limits(DEFAULT_PREFILL_PACKETS, DEFAULT_MAX_PACKETS)
    }

    fn with_limits(prefill_packets: usize, max_packets: usize) -> Self {
        assert!(prefill_packets > 0);
        assert!(max_packets >= prefill_packets);
        Self {
            packets: BTreeMap::new(),
            expected: None,
            started: false,
            prefill_packets,
            max_packets,
            consecutive_missing: 0,
            stats: JitterStats::default(),
        }
    }

    pub fn insert(&mut self, packet: BufferedAudioPacket) {
        if let Some(expected) = self.expected {
            if sequence_is_older(packet.sequence, expected) {
                self.stats.late += 1;
                return;
            }
        }

        if self.packets.contains_key(&packet.sequence) {
            self.stats.duplicates += 1;
            return;
        }

        if self.packets.len() >= self.max_packets {
            let latest = *self.packets.last_key_value().expect("non-empty").0;
            if sequence_is_older(packet.sequence, latest) {
                self.packets.remove(&latest);
            } else {
                self.stats.overflow_drops += 1;
                return;
            }
            self.stats.overflow_drops += 1;
        }

        self.packets.insert(packet.sequence, packet);
        self.stats.accepted += 1;
    }

    pub fn pop(&mut self) -> PlayoutItem {
        if !self.started {
            if self.packets.len() < self.prefill_packets {
                return PlayoutItem::Waiting;
            }
            self.expected = self
                .packets
                .first_key_value()
                .map(|(sequence, _)| *sequence);
            self.started = true;
        }

        let expected = self.expected.expect("started jitter buffer has sequence");
        self.expected = Some(expected.wrapping_add(1));

        if let Some(packet) = self.packets.remove(&expected) {
            self.consecutive_missing = 0;
            return PlayoutItem::Packet(packet);
        }

        self.consecutive_missing += 1;
        self.stats.missing += 1;
        if self.consecutive_missing > MAX_CONSECUTIVE_MISSING && self.packets.is_empty() {
            self.reset_state();
            self.stats.resets += 1;
            return PlayoutItem::Reset;
        }

        let next_payload = self
            .packets
            .get(&expected.wrapping_add(1))
            .map(|packet| packet.payload.clone());
        PlayoutItem::Missing {
            sequence: expected,
            next_payload,
        }
    }

    pub fn reset(&mut self) {
        self.reset_state();
        self.stats.resets += 1;
    }

    #[cfg(test)]
    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    fn reset_state(&mut self) {
        self.packets.clear();
        self.expected = None;
        self.started = false;
        self.consecutive_missing = 0;
    }
}

fn sequence_is_older(sequence: u32, reference: u32) -> bool {
    (sequence.wrapping_sub(reference) as i32) < 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_codec::{
        AudioDecoder, AudioEncoder, FRAME_SAMPLES, FRAME_SAMPLES_PER_CHANNEL, SAMPLE_RATE,
    };

    fn packet(sequence: u32) -> BufferedAudioPacket {
        BufferedAudioPacket {
            sequence,
            timestamp_us: u64::from(sequence) * 10_000,
            flags: 0,
            payload: vec![sequence as u8],
        }
    }

    #[test]
    fn reorders_packets_before_playout() {
        let mut jitter = AudioJitterBuffer::with_limits(3, 8);
        jitter.insert(packet(2));
        jitter.insert(packet(0));
        jitter.insert(packet(1));

        for expected in 0..3 {
            match jitter.pop() {
                PlayoutItem::Packet(packet) => assert_eq!(packet.sequence, expected),
                other => panic!("unexpected playout item: {other:?}"),
            }
        }
    }

    #[test]
    fn reports_loss_and_exposes_next_packet_for_fec() {
        let mut jitter = AudioJitterBuffer::with_limits(2, 8);
        jitter.insert(packet(0));
        jitter.insert(packet(2));

        assert!(matches!(jitter.pop(), PlayoutItem::Packet(_)));
        assert_eq!(
            jitter.pop(),
            PlayoutItem::Missing {
                sequence: 1,
                next_payload: Some(vec![2])
            }
        );
        assert!(matches!(jitter.pop(), PlayoutItem::Packet(_)));
    }

    #[test]
    fn rejects_duplicate_and_late_packets() {
        let mut jitter = AudioJitterBuffer::with_limits(2, 8);
        jitter.insert(packet(0));
        jitter.insert(packet(0));
        jitter.insert(packet(1));
        jitter.pop();
        jitter.insert(packet(0));

        assert_eq!(jitter.stats().duplicates, 1);
        assert_eq!(jitter.stats().late, 1);
    }

    #[test]
    fn keeps_earliest_packets_when_capacity_is_exceeded() {
        let mut jitter = AudioJitterBuffer::with_limits(2, 3);
        jitter.insert(packet(0));
        jitter.insert(packet(2));
        jitter.insert(packet(3));
        jitter.insert(packet(1));

        assert_eq!(jitter.stats().overflow_drops, 1);
        assert!(matches!(jitter.pop(), PlayoutItem::Packet(packet) if packet.sequence == 0));
        assert!(matches!(jitter.pop(), PlayoutItem::Packet(packet) if packet.sequence == 1));
        assert!(matches!(jitter.pop(), PlayoutItem::Packet(packet) if packet.sequence == 2));
    }

    #[test]
    fn resets_after_extended_outage() {
        let mut jitter = AudioJitterBuffer::with_limits(2, 8);
        jitter.insert(packet(0));
        jitter.insert(packet(1));
        jitter.pop();
        jitter.pop();

        for _ in 0..MAX_CONSECUTIVE_MISSING {
            assert!(matches!(jitter.pop(), PlayoutItem::Missing { .. }));
        }
        assert_eq!(jitter.pop(), PlayoutItem::Reset);
        assert_eq!(jitter.stats().resets, 1);
    }

    #[test]
    fn sequence_age_handles_wrap() {
        assert!(sequence_is_older(u32::MAX, 0));
        assert!(!sequence_is_older(0, u32::MAX));
    }

    #[test]
    fn opus_survives_loss_duplicate_and_reordering() {
        let frame = || {
            (0..FRAME_SAMPLES_PER_CHANNEL)
                .flat_map(|sample| {
                    let phase = sample as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
                    let value = phase.sin() * 0.2;
                    [value, value]
                })
                .collect::<Vec<_>>()
        };
        let mut encoder = AudioEncoder::new();
        let packets = (0..4)
            .map(|sequence| BufferedAudioPacket {
                sequence,
                timestamp_us: u64::from(sequence) * 10_000,
                flags: 0,
                payload: encoder.encode(&frame()).unwrap(),
            })
            .collect::<Vec<_>>();

        let mut jitter = AudioJitterBuffer::with_limits(2, 8);
        jitter.insert(packets[3].clone());
        jitter.insert(packets[0].clone());
        jitter.insert(packets[2].clone());
        jitter.insert(packets[2].clone());

        let mut decoder = AudioDecoder::new();
        let mut decoded_frames = Vec::new();
        for _ in 0..4 {
            let decoded = match jitter.pop() {
                PlayoutItem::Packet(packet) => decoder.decode(&packet.payload).unwrap(),
                PlayoutItem::Missing { next_payload, .. } => next_payload
                    .and_then(|packet| decoder.recover_fec(&packet).ok())
                    .unwrap_or_else(|| decoder.conceal()),
                other => panic!("unexpected playout item: {other:?}"),
            };
            assert_eq!(decoded.len(), FRAME_SAMPLES);
            assert!(decoded.iter().all(|sample| sample.is_finite()));
            decoded_frames.push(decoded);
        }

        assert_eq!(decoded_frames.len(), 4);
        assert_eq!(jitter.stats().duplicates, 1);
        assert_eq!(jitter.stats().missing, 1);
    }
}
