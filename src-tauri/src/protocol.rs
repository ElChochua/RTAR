//! Versioned media datagram framing shared with RTABC.
//!
//! Keep this module byte-for-byte compatible with RTABC's `protocol.rs` until
//! the protocol moves into a separately versioned crate.

#![allow(dead_code)] // v1 reserves codecs and flags used by later stages.

use std::fmt;

pub const MAGIC: [u8; 4] = *b"LAL1";
pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 32;
pub const MAX_DATAGRAM_LEN: usize = 1200;
pub const MAX_PAYLOAD_LEN: usize = MAX_DATAGRAM_LEN - HEADER_LEN;

pub const FLAG_KEYFRAME: u8 = 1 << 0;
pub const FLAG_CONFIG: u8 = 1 << 1;
pub const FLAG_DISCONTINUITY: u8 = 1 << 2;
pub const FLAG_END_OF_STREAM: u8 = 1 << 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MediaKind {
    Audio = 1,
    Video = 2,
    Control = 3,
    Heartbeat = 4,
}

impl TryFrom<u8> for MediaKind {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Audio),
            2 => Ok(Self::Video),
            3 => Ok(Self::Control),
            4 => Ok(Self::Heartbeat),
            _ => Err(ProtocolError::UnknownMediaKind(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Codec {
    None = 0,
    PcmS16Le = 1,
    Opus = 2,
    Hevc = 3,
    Json = 4,
}

impl TryFrom<u8> for Codec {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::PcmS16Le),
            2 => Ok(Self::Opus),
            3 => Ok(Self::Hevc),
            4 => Ok(Self::Json),
            _ => Err(ProtocolError::UnknownCodec(value)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketHeader {
    pub media_kind: MediaKind,
    pub codec: Codec,
    pub flags: u8,
    pub stream_id: u32,
    pub sequence: u32,
    pub timestamp_us: u64,
    pub fragment_index: u16,
    pub fragment_count: u16,
}

impl PacketHeader {
    pub fn encode(self, payload_len: usize) -> Result<[u8; HEADER_LEN], ProtocolError> {
        validate_fragment(self.fragment_index, self.fragment_count)?;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }

        let payload_len = payload_len as u16;
        let mut bytes = [0_u8; HEADER_LEN];
        bytes[0..4].copy_from_slice(&MAGIC);
        bytes[4] = PROTOCOL_VERSION;
        bytes[5] = self.media_kind as u8;
        bytes[6] = self.codec as u8;
        bytes[7] = self.flags;
        bytes[8..12].copy_from_slice(&self.stream_id.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[16..24].copy_from_slice(&self.timestamp_us.to_be_bytes());
        bytes[24..26].copy_from_slice(&payload_len.to_be_bytes());
        bytes[26..28].copy_from_slice(&self.fragment_index.to_be_bytes());
        bytes[28..30].copy_from_slice(&self.fragment_count.to_be_bytes());
        // Bytes 30..32 are reserved for a future compatible extension.
        Ok(bytes)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    DatagramTooShort(usize),
    InvalidMagic,
    UnsupportedVersion(u8),
    UnknownMediaKind(u8),
    UnknownCodec(u8),
    InvalidPayloadLength { declared: usize, actual: usize },
    PayloadTooLarge(usize),
    InvalidFragment { index: u16, count: u16 },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ProtocolError {}

pub fn write_datagram(
    header: PacketHeader,
    payload: &[u8],
    output: &mut Vec<u8>,
) -> Result<(), ProtocolError> {
    let encoded = header.encode(payload.len())?;
    output.clear();
    output.reserve(HEADER_LEN + payload.len());
    output.extend_from_slice(&encoded);
    output.extend_from_slice(payload);
    Ok(())
}

pub fn decode_datagram(datagram: &[u8]) -> Result<(PacketHeader, &[u8]), ProtocolError> {
    if datagram.len() < HEADER_LEN {
        return Err(ProtocolError::DatagramTooShort(datagram.len()));
    }
    if datagram[0..4] != MAGIC {
        return Err(ProtocolError::InvalidMagic);
    }
    if datagram[4] != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion(datagram[4]));
    }

    let media_kind = MediaKind::try_from(datagram[5])?;
    let codec = Codec::try_from(datagram[6])?;
    let payload_len = u16::from_be_bytes([datagram[24], datagram[25]]) as usize;
    let actual_len = datagram.len() - HEADER_LEN;
    if payload_len != actual_len {
        return Err(ProtocolError::InvalidPayloadLength {
            declared: payload_len,
            actual: actual_len,
        });
    }
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(ProtocolError::PayloadTooLarge(payload_len));
    }

    let fragment_index = u16::from_be_bytes([datagram[26], datagram[27]]);
    let fragment_count = u16::from_be_bytes([datagram[28], datagram[29]]);
    validate_fragment(fragment_index, fragment_count)?;

    let header = PacketHeader {
        media_kind,
        codec,
        flags: datagram[7],
        stream_id: u32::from_be_bytes(datagram[8..12].try_into().expect("fixed slice")),
        sequence: u32::from_be_bytes(datagram[12..16].try_into().expect("fixed slice")),
        timestamp_us: u64::from_be_bytes(datagram[16..24].try_into().expect("fixed slice")),
        fragment_index,
        fragment_count,
    };

    Ok((header, &datagram[HEADER_LEN..]))
}

fn validate_fragment(index: u16, count: u16) -> Result<(), ProtocolError> {
    if count == 0 || index >= count {
        return Err(ProtocolError::InvalidFragment { index, count });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN_PACKET: [u8; 34] = [
        0x4c, 0x41, 0x4c, 0x31, 0x01, 0x01, 0x01, 0x04, 0x01, 0x02, 0x03, 0x04, 0x11, 0x12, 0x13,
        0x14, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01,
        0x00, 0x00, 0x34, 0x12,
    ];

    fn golden_header() -> PacketHeader {
        PacketHeader {
            media_kind: MediaKind::Audio,
            codec: Codec::PcmS16Le,
            flags: FLAG_DISCONTINUITY,
            stream_id: 0x0102_0304,
            sequence: 0x1112_1314,
            timestamp_us: 0x0102_0304_0506_0708,
            fragment_index: 0,
            fragment_count: 1,
        }
    }

    #[test]
    fn writes_stable_golden_packet() {
        let mut packet = Vec::new();
        write_datagram(golden_header(), &[0x34, 0x12], &mut packet).unwrap();
        assert_eq!(packet, GOLDEN_PACKET);
    }

    #[test]
    fn reads_stable_golden_packet() {
        let (header, payload) = decode_datagram(&GOLDEN_PACKET).unwrap();
        assert_eq!(header, golden_header());
        assert_eq!(payload, [0x34, 0x12]);
    }

    #[test]
    fn rejects_truncated_payload() {
        let error = decode_datagram(&GOLDEN_PACKET[..33]).unwrap_err();
        assert_eq!(
            error,
            ProtocolError::InvalidPayloadLength {
                declared: 2,
                actual: 1
            }
        );
    }

    #[test]
    fn rejects_unknown_version() {
        let mut packet = GOLDEN_PACKET;
        packet[4] = 2;
        assert_eq!(
            decode_datagram(&packet).unwrap_err(),
            ProtocolError::UnsupportedVersion(2)
        );
    }

    #[test]
    fn rejects_invalid_fragment_count() {
        let mut packet = GOLDEN_PACKET;
        packet[28..30].copy_from_slice(&0_u16.to_be_bytes());
        assert_eq!(
            decode_datagram(&packet).unwrap_err(),
            ProtocolError::InvalidFragment { index: 0, count: 0 }
        );
    }

    #[test]
    fn rejects_payload_over_mtu_budget() {
        let error = golden_header().encode(MAX_PAYLOAD_LEN + 1).unwrap_err();
        assert_eq!(error, ProtocolError::PayloadTooLarge(MAX_PAYLOAD_LEN + 1));
    }
}
