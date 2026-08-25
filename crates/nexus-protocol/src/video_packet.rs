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

/// Total header length in bytes: 1+1+2+4+2+2+8+2.
pub const HEADER_LEN: usize = 22;
pub const CURRENT_VERSION: u8 = 1;

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
    pub payload_len: u16,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoPacketError {
    #[error("buffer too short for header: need {HEADER_LEN} bytes, got {got}")]
    HeaderTooShort { got: usize },
    #[error("buffer too short for payload: header declares {declared} bytes, got {got}")]
    PayloadTooShort { declared: usize, got: usize },
}

impl VideoPacketHeader {
    /// Encodes the header followed by `payload` into `out`. Does not
    /// validate that `payload.len()` matches `self.payload_len` — callers
    /// are expected to set `payload_len` to `payload.len()` before calling.
    pub fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        out.reserve(HEADER_LEN + payload.len());
        out.push(self.version);
        out.push(self.flags);
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.packet_id.to_be_bytes());
        out.extend_from_slice(&self.packet_count.to_be_bytes());
        out.extend_from_slice(&self.timestamp_us.to_be_bytes());
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
        let flags = buf[1];
        let stream_id = u16::from_be_bytes([buf[2], buf[3]]);
        let frame_id = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let packet_id = u16::from_be_bytes([buf[8], buf[9]]);
        let packet_count = u16::from_be_bytes([buf[10], buf[11]]);
        let timestamp_us = u64::from_be_bytes([
            buf[12], buf[13], buf[14], buf[15], buf[16], buf[17], buf[18], buf[19],
        ]);
        let payload_len = u16::from_be_bytes([buf[20], buf[21]]);

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
    fn decode_rejects_truncated_payload() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
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
}
