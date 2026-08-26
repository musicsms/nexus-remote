use nexus_crypto::{
    open_encoded_frame, seal_encoded_frame, EncodedFrameMetadata, EncryptedFrame, FrameSealError,
    NonceSequence,
};
use nexus_protocol::{VideoPacketError, VideoPacketHeader};
use thiserror::Error;

pub const MAX_VIDEO_DATAGRAM_SIZE: usize = 64 * 1024;

pub fn seal_video_frame(
    key: &[u8; 32],
    sequence: &mut NonceSequence,
    header: &VideoPacketHeader,
    codec_config_id: u32,
    encoded_frame: &[u8],
) -> Result<EncryptedFrame, FrameSealError> {
    seal_encoded_frame(
        key,
        sequence,
        EncodedFrameMetadata {
            protocol_version: header.version as u32,
            channel: header.stream_id as u32,
            frame_id: header.frame_id,
            codec_config_id,
        },
        encoded_frame,
    )
}

pub fn open_video_frame(
    key: &[u8; 32],
    header: &VideoPacketHeader,
    codec_config_id: u32,
    encrypted: &EncryptedFrame,
) -> Result<Vec<u8>, nexus_crypto::AeadError> {
    open_encoded_frame(
        key,
        EncodedFrameMetadata {
            protocol_version: header.version as u32,
            channel: header.stream_id as u32,
            frame_id: header.frame_id,
            codec_config_id,
        },
        encrypted,
    )
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoDatagramError {
    #[error("video datagram is too large: {actual} bytes (limit {limit})")]
    TooLarge { actual: usize, limit: usize },
    #[error("invalid video packet: {0}")]
    Packet(#[from] VideoPacketError),
    #[error("video header payload length does not match payload")]
    PayloadLengthMismatch,
}

pub fn encode_video_datagram(
    header: &VideoPacketHeader,
    payload: &[u8],
) -> Result<Vec<u8>, VideoDatagramError> {
    if payload.len() > nexus_protocol::video_packet::MAX_PAYLOAD_LEN {
        return Err(VideoDatagramError::TooLarge {
            actual: payload.len(),
            limit: nexus_protocol::video_packet::MAX_PAYLOAD_LEN,
        });
    }
    if header.payload_len as usize != payload.len() {
        return Err(VideoDatagramError::PayloadLengthMismatch);
    }
    let total = nexus_protocol::video_packet::HEADER_LEN + payload.len();
    if total > MAX_VIDEO_DATAGRAM_SIZE {
        return Err(VideoDatagramError::TooLarge {
            actual: total,
            limit: MAX_VIDEO_DATAGRAM_SIZE,
        });
    }
    let mut encoded = Vec::with_capacity(total);
    header.encode(payload, &mut encoded);
    Ok(encoded)
}

pub fn decode_video_datagram(
    bytes: &[u8],
) -> Result<(VideoPacketHeader, &[u8]), VideoDatagramError> {
    if bytes.len() > MAX_VIDEO_DATAGRAM_SIZE {
        return Err(VideoDatagramError::TooLarge {
            actual: bytes.len(),
            limit: MAX_VIDEO_DATAGRAM_SIZE,
        });
    }
    Ok(VideoPacketHeader::decode(bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_protocol::video_packet::CURRENT_VERSION;

    #[test]
    fn round_trip_video_datagram() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 1,
            frame_id: 2,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 3,
            payload_len: 3,
        };
        let bytes = encode_video_datagram(&header, &[1, 2, 3]).unwrap();
        let (decoded, payload) = decode_video_datagram(&bytes).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn rejects_oversized_datagram() {
        let bytes = vec![0; MAX_VIDEO_DATAGRAM_SIZE + 1];
        assert!(matches!(
            decode_video_datagram(&bytes),
            Err(VideoDatagramError::TooLarge { .. })
        ));
    }

    #[test]
    fn rejects_oversized_payload() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: (nexus_protocol::video_packet::MAX_PAYLOAD_LEN + 1) as u16,
        };
        let err = encode_video_datagram(
            &header,
            &vec![0; nexus_protocol::video_packet::MAX_PAYLOAD_LEN + 1],
        )
        .unwrap_err();
        assert!(matches!(err, VideoDatagramError::TooLarge { .. }));
    }

    #[test]
    fn rejects_payload_length_mismatch() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 0,
            frame_id: 0,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: 2,
        };
        assert_eq!(
            encode_video_datagram(&header, &[1]),
            Err(VideoDatagramError::PayloadLengthMismatch)
        );
    }

    #[test]
    fn seals_video_frame_before_packetization_and_authenticates_header_metadata() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 7,
            frame_id: 9,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: 0,
        };
        let mut sequence = NonceSequence::new(0x0102_0304);
        let encrypted = seal_video_frame(&[4; 32], &mut sequence, &header, 12, b"encoded").unwrap();
        assert_eq!(
            open_video_frame(&[4; 32], &header, 12, &encrypted).unwrap(),
            b"encoded"
        );
        let mut changed = header.clone();
        changed.frame_id += 1;
        assert_eq!(
            open_video_frame(&[4; 32], &changed, 12, &encrypted),
            Err(nexus_crypto::AeadError::AuthenticationFailed)
        );
    }
}
