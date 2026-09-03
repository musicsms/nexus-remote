//! Video packet header (Spec Section 21).
//!
//! Byte layout is fixed network order (big-endian) for every multi-byte
//! field — Section 21 requires "all fields and byte order must be
//! formally specified before compatibility is promised"; this module is
//! that specification for the Phase 0 PoC. It does not by itself satisfy
//! the full Definition of Done in Section 56 (fuzzing, security review,
//! backward-compat docs come later, once this header is used on a real
//! network path rather than a loopback PoC).

use thiserror::Error;

/// Total v2 header length in bytes: 1+1+2+4+2+2+8+8+2.
pub const HEADER_LEN: usize = 30;
/// Offset of the explicit sender frame-sequence number in a wire header.
pub const NONCE_SEQUENCE_OFFSET: usize = 20;
/// Version 2 adds the explicit nonce sequence required to decrypt reordered
/// frames independently; version 1 packets are deliberately rejected.
pub const CURRENT_VERSION: u8 = 2;

/// Conservative QUIC-datagram-safe payload budget (Spec Section 57 rule 6:
/// "every network message has an explicit maximum size limit"). Real-world
/// QUIC datagrams are bounded by path MTU; 1200 bytes is a safe floor that
/// avoids IP fragmentation on virtually all paths, including the common
/// IPv6 minimum-MTU case. Payloads larger than this need the fragmentation
/// `packet_id`/`packet_count` already anticipate — not yet implemented.
pub const MAX_PAYLOAD_LEN: usize = 1200;

pub mod flags {
    pub const KEYFRAME: u8 = 0b0000_0001;
    pub const FRAME_START: u8 = 0b0000_0010;
    pub const FRAME_END: u8 = 0b0000_0100;
    pub const FEC: u8 = 0b0000_1000;
    pub const CONFIG: u8 = 0b0001_0000;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPacketHeader {
    pub version: u8,
    pub flags: u8,
    pub stream_id: u16,
    pub frame_id: u32,
    pub packet_id: u16,
    pub packet_count: u16,
    pub timestamp_us: u64,
    /// Sender's monotonic per-direction sequence used to derive the AEAD nonce.
    pub nonce_sequence: u64,
    pub payload_len: u16,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoPacketError {
    #[error("unsupported video packet version: expected {expected}, got {actual}")]
    UnsupportedVersion { expected: u8, actual: u8 },
    #[error("buffer too short for header: need {HEADER_LEN} bytes, got {got}")]
    HeaderTooShort { got: usize },
    #[error("buffer too short for payload: header declares {declared} bytes, got {got}")]
    PayloadTooShort { declared: usize, got: usize },
    #[error("payload exceeds MAX_PAYLOAD_LEN: max {max}, declared {actual}")]
    PayloadTooLarge { max: usize, actual: usize },
}

impl VideoPacketHeader {
    /// Encodes the header followed by `payload` into `out`. Does not
    /// validate that `payload.len()` matches `self.payload_len` — callers
    /// are expected to set `payload_len` to `payload.len()` before calling.
    pub fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        debug_assert!(
            payload.len() <= MAX_PAYLOAD_LEN,
            "payload exceeds MAX_PAYLOAD_LEN ({} > {})",
            payload.len(),
            MAX_PAYLOAD_LEN
        );
        out.reserve(HEADER_LEN + payload.len());
        out.push(self.version);
        out.push(self.flags);
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.packet_id.to_be_bytes());
        out.extend_from_slice(&self.packet_count.to_be_bytes());
        out.extend_from_slice(&self.timestamp_us.to_be_bytes());
        out.extend_from_slice(&self.nonce_sequence.to_be_bytes());
        out.extend_from_slice(&self.payload_len.to_be_bytes());
        out.extend_from_slice(payload);
    }

    /// Decodes a header and its payload slice from `buf`. Rejects
    /// truncated input rather than panicking or reading out of bounds —
    /// Spec Section 57 rule 5 ("protocol parsers treat all remote input
    /// as hostile").
    pub fn decode(buf: &[u8]) -> Result<(VideoPacketHeader, &[u8]), VideoPacketError> {
        if buf.len() < HEADER_LEN {
            return Err(VideoPacketError::HeaderTooShort { got: buf.len() });
        }

        let version = buf[0];
        if version != CURRENT_VERSION {
            return Err(VideoPacketError::UnsupportedVersion {
                expected: CURRENT_VERSION,
                actual: version,
            });
        }
        let flags = buf[1];
        let stream_id = u16::from_be_bytes([buf[2], buf[3]]);
        let frame_id = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let packet_id = u16::from_be_bytes([buf[8], buf[9]]);
        let packet_count = u16::from_be_bytes([buf[10], buf[11]]);
        let timestamp_us = u64::from_be_bytes([
            buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
        ]);
        let nonce_sequence = u64::from_be_bytes([
            buf[20], buf[21], buf[22], buf[23], buf[24], buf[25], buf[26], buf[27],
        ]);
        let payload_len = u16::from_be_bytes([buf[28], buf[29]]);

        if payload_len as usize > MAX_PAYLOAD_LEN {
            return Err(VideoPacketError::PayloadTooLarge {
                max: MAX_PAYLOAD_LEN,
                actual: payload_len as usize,
            });
        }

        let payload_start = HEADER_LEN;
        let payload_end = payload_start + payload_len as usize;
        if buf.len() < payload_end {
            return Err(VideoPacketError::PayloadTooShort {
                declared: payload_len as usize,
                got: buf.len() - payload_start,
            });
        }

        Ok((
            VideoPacketHeader {
                version,
                flags,
                stream_id,
                frame_id,
                packet_id,
                packet_count,
                timestamp_us,
                nonce_sequence,
                payload_len,
            },
            &buf[payload_start..payload_end],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_encode_decode() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: flags::KEYFRAME | flags::FRAME_START | flags::FRAME_END,
            stream_id: 1,
            frame_id: 42,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 1_234_567,
            nonce_sequence: 0,
            payload_len: 4,
        };
        let payload = [0xDE, 0xAD, 0xBE, 0xEF];

        let mut buf = Vec::new();
        header.encode(&payload, &mut buf);

        let (decoded, decoded_payload) = VideoPacketHeader::decode(&buf).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded_payload, &payload);
    }

    #[test]
    fn decode_rejects_truncated_header() {
        let buf = [0u8; 10];
        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(err, VideoPacketError::HeaderTooShort { got: 10 });
    }

    #[test]
    fn decode_rejects_unsupported_version() {
        let mut buf = vec![0u8; HEADER_LEN];
        buf[0] = CURRENT_VERSION + 1;
        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(
            err,
            VideoPacketError::UnsupportedVersion {
                expected: CURRENT_VERSION,
                actual: CURRENT_VERSION + 1,
            }
        );
    }

    #[test]
    fn decode_rejects_truncated_payload() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            nonce_sequence: 0,
            payload_len: 100,
        };
        let mut buf = Vec::new();
        header.encode(&[], &mut buf); // declares 100 bytes but encodes 0

        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(
            err,
            VideoPacketError::PayloadTooShort {
                declared: 100,
                got: 0
            }
        );
    }

    #[test]
    fn decode_rejects_oversized_payload() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            nonce_sequence: 0,
            payload_len: (MAX_PAYLOAD_LEN + 1) as u16,
        };
        // Header-only buffer (no payload bytes appended) — PayloadTooLarge must
        // fire from the declared payload_len alone, before any check for
        // whether that many bytes are actually present.
        let mut buf = Vec::new();
        buf.push(header.version);
        buf.push(header.flags);
        buf.extend_from_slice(&header.stream_id.to_be_bytes());
        buf.extend_from_slice(&header.frame_id.to_be_bytes());
        buf.extend_from_slice(&header.packet_id.to_be_bytes());
        buf.extend_from_slice(&header.packet_count.to_be_bytes());
        buf.extend_from_slice(&header.timestamp_us.to_be_bytes());
        buf.extend_from_slice(&header.nonce_sequence.to_be_bytes());
        buf.extend_from_slice(&header.payload_len.to_be_bytes());

        let err = VideoPacketHeader::decode(&buf).unwrap_err();
        assert_eq!(
            err,
            VideoPacketError::PayloadTooLarge {
                max: MAX_PAYLOAD_LEN,
                actual: MAX_PAYLOAD_LEN + 1
            }
        );
    }

    #[test]
    fn golden_vector_byte_layout() {
        let header = VideoPacketHeader {
            version: 0x01,
            flags: 0x02,
            stream_id: 0x0304,
            frame_id: 0x0506_0708,
            packet_id: 0x090A,
            packet_count: 0x0B0C,
            timestamp_us: 0x0D0E_0F10_1112_1314,
            nonce_sequence: 0x1516_1718_191A_1B1C,
            payload_len: 0x0004,
        };
        let payload = [0xAAu8, 0xBB, 0xCC, 0xDD];

        let mut buf = Vec::new();
        header.encode(&payload, &mut buf);

        let expected: [u8; 34] = [
            0x01, 0x02, // version, flags
            0x03, 0x04, // stream_id (big-endian)
            0x05, 0x06, 0x07, 0x08, // frame_id (big-endian)
            0x09, 0x0A, // packet_id (big-endian)
            0x0B, 0x0C, // packet_count (big-endian)
            0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, // timestamp_us (big-endian)
            0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, // nonce_sequence (big-endian)
            0x00, 0x04, // payload_len (big-endian)
            0xAA, 0xBB, 0xCC, 0xDD, // payload
        ];

        assert_eq!(buf, expected);
    }
}
