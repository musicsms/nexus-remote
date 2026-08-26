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

pub const DEFAULT_MAX_PACKET_PAYLOAD: usize = nexus_protocol::video_packet::MAX_PAYLOAD_LEN;
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_PACKETS_PER_FRAME: usize = 16384;
pub const MAX_IN_FLIGHT_FRAMES: usize = 8;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PacketizeError {
    #[error("frame payload exceeds maximum allowed size: {actual} bytes (limit {limit})")]
    FrameTooLarge { actual: usize, limit: usize },
    #[error("packet chunk size must be between 1 and {max} bytes")]
    InvalidChunkSize { max: usize },
    #[error("packet count {count} exceeds maximum {max}")]
    TooManyPackets { count: usize, max: usize },
}

/// Slices an encoded/encrypted video frame into bounded datagram fragments.
pub fn packetize_video_frame(
    base_header: &VideoPacketHeader,
    payload: &[u8],
    chunk_size: usize,
) -> Result<Vec<(VideoPacketHeader, Vec<u8>)>, PacketizeError> {
    if chunk_size == 0 || chunk_size > nexus_protocol::video_packet::MAX_PAYLOAD_LEN {
        return Err(PacketizeError::InvalidChunkSize {
            max: nexus_protocol::video_packet::MAX_PAYLOAD_LEN,
        });
    }
    if payload.len() > MAX_FRAME_PAYLOAD_SIZE {
        return Err(PacketizeError::FrameTooLarge {
            actual: payload.len(),
            limit: MAX_FRAME_PAYLOAD_SIZE,
        });
    }

    if payload.is_empty() {
        let mut header = base_header.clone();
        header.flags |= nexus_protocol::video_packet::flags::FRAME_START
            | nexus_protocol::video_packet::flags::FRAME_END;
        header.packet_id = 0;
        header.packet_count = 1;
        header.payload_len = 0;
        return Ok(vec![(header, Vec::new())]);
    }

    let chunks: Vec<&[u8]> = payload.chunks(chunk_size).collect();
    let packet_count = chunks.len();
    if packet_count > u16::MAX as usize || packet_count > MAX_PACKETS_PER_FRAME {
        return Err(PacketizeError::TooManyPackets {
            count: packet_count,
            max: MAX_PACKETS_PER_FRAME,
        });
    }

    let mut packets = Vec::with_capacity(packet_count);
    for (i, chunk) in chunks.into_iter().enumerate() {
        let mut header = base_header.clone();
        header.packet_id = i as u16;
        header.packet_count = packet_count as u16;
        header.payload_len = chunk.len() as u16;
        if i == 0 {
            header.flags |= nexus_protocol::video_packet::flags::FRAME_START;
        }
        if i == packet_count - 1 {
            header.flags |= nexus_protocol::video_packet::flags::FRAME_END;
        }
        packets.push((header, chunk.to_vec()));
    }

    Ok(packets)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledFrame {
    pub header: VideoPacketHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReassemblyError {
    #[error("packet id {packet_id} is out of bounds for packet count {packet_count}")]
    InvalidPacketId { packet_id: u16, packet_count: u16 },
    #[error("packet count {count} exceeds limit {max}")]
    TooManyPackets { count: u16, max: usize },
    #[error("payload length {actual} does not match header {declared}")]
    PayloadLengthMismatch { declared: u16, actual: usize },
    #[error("packet flags missing FRAME_START for packet 0 or FRAME_END for last packet")]
    InvalidBoundaryFlags,
    #[error("inconsistent packet count or stream metadata across fragments")]
    InconsistentFragmentMetadata,
}

#[derive(Debug)]
struct InFlightFrame {
    base_header: VideoPacketHeader,
    packets: Vec<Option<Vec<u8>>>,
    received_count: usize,
    total_bytes: usize,
}

/// Bounded in-flight frame assembler that reconstructs full frames from datagram fragments.
#[derive(Debug)]
pub struct VideoFrameReassembler {
    frames: std::collections::BTreeMap<u32, InFlightFrame>,
    max_in_flight: usize,
    max_frame_bytes: usize,
    last_delivered_frame_id: Option<u32>,
}

impl Default for VideoFrameReassembler {
    fn default() -> Self {
        Self::new(MAX_IN_FLIGHT_FRAMES, MAX_FRAME_PAYLOAD_SIZE)
    }
}

impl VideoFrameReassembler {
    pub fn new(max_in_flight: usize, max_frame_bytes: usize) -> Self {
        Self {
            frames: std::collections::BTreeMap::new(),
            max_in_flight: max_in_flight.max(1),
            max_frame_bytes,
            last_delivered_frame_id: None,
        }
    }

    /// Process a received video packet.
    /// If this packet completes a frame, returns `Ok(Some(AssembledFrame))`.
    pub fn process_packet(
        &mut self,
        header: &VideoPacketHeader,
        payload: &[u8],
    ) -> Result<Option<AssembledFrame>, ReassemblyError> {
        if header.packet_count == 0 || header.packet_count as usize > MAX_PACKETS_PER_FRAME {
            return Err(ReassemblyError::TooManyPackets {
                count: header.packet_count,
                max: MAX_PACKETS_PER_FRAME,
            });
        }
        if header.packet_id >= header.packet_count {
            return Err(ReassemblyError::InvalidPacketId {
                packet_id: header.packet_id,
                packet_count: header.packet_count,
            });
        }
        if header.payload_len as usize != payload.len() {
            return Err(ReassemblyError::PayloadLengthMismatch {
                declared: header.payload_len,
                actual: payload.len(),
            });
        }
        if header.packet_id == 0
            && (header.flags & nexus_protocol::video_packet::flags::FRAME_START == 0)
        {
            return Err(ReassemblyError::InvalidBoundaryFlags);
        }
        if header.packet_id == header.packet_count - 1
            && (header.flags & nexus_protocol::video_packet::flags::FRAME_END == 0)
        {
            return Err(ReassemblyError::InvalidBoundaryFlags);
        }

        // Drop if this frame has already been delivered or is older than the last delivered frame (ADR-022).
        // This check must precede the single-packet fast path so delayed packets
        // cannot regress the delivery watermark.
        if let Some(last_id) = self.last_delivered_frame_id {
            if header.frame_id <= last_id {
                return Ok(None);
            }
        }

        // Single-packet fast path
        if header.packet_count == 1 {
            if payload.len() > self.max_frame_bytes {
                return Ok(None);
            }
            self.last_delivered_frame_id = Some(header.frame_id);
            self.prune_stale_before(header.frame_id);
            return Ok(Some(AssembledFrame {
                header: header.clone(),
                payload: payload.to_vec(),
            }));
        }

        // Manage in-flight capacity: if full, evict oldest in-flight frame
        if self.frames.len() >= self.max_in_flight && !self.frames.contains_key(&header.frame_id) {
            if let Some(oldest_key) = self.frames.keys().next().copied() {
                self.frames.remove(&oldest_key);
            }
        }

        let in_flight = self
            .frames
            .entry(header.frame_id)
            .or_insert_with(|| InFlightFrame {
                base_header: header.clone(),
                packets: vec![None; header.packet_count as usize],
                received_count: 0,
                total_bytes: 0,
            });

        // Validate consistency with earlier fragments of this frame
        if in_flight.packets.len() != header.packet_count as usize
            || in_flight.base_header.stream_id != header.stream_id
            || in_flight.base_header.version != header.version
        {
            return Err(ReassemblyError::InconsistentFragmentMetadata);
        }

        let idx = header.packet_id as usize;
        if in_flight.packets[idx].is_none() {
            in_flight.total_bytes += payload.len();
            if in_flight.total_bytes > self.max_frame_bytes {
                self.frames.remove(&header.frame_id);
                return Ok(None);
            }
            in_flight.packets[idx] = Some(payload.to_vec());
            in_flight.received_count += 1;
        }

        if in_flight.received_count == in_flight.packets.len() {
            let in_flight = self.frames.remove(&header.frame_id).unwrap();
            let mut full_payload = Vec::with_capacity(in_flight.total_bytes);
            for bytes in in_flight.packets.into_iter().flatten() {
                full_payload.extend_from_slice(&bytes);
            }
            let mut final_header = in_flight.base_header;
            final_header.payload_len = (full_payload.len().min(u16::MAX as usize)) as u16;
            final_header.flags |= nexus_protocol::video_packet::flags::FRAME_START
                | nexus_protocol::video_packet::flags::FRAME_END;

            self.last_delivered_frame_id = Some(header.frame_id);
            self.prune_stale_before(header.frame_id);

            Ok(Some(AssembledFrame {
                header: final_header,
                payload: full_payload,
            }))
        } else {
            Ok(None)
        }
    }

    fn prune_stale_before(&mut self, frame_id: u32) {
        let stale_keys: Vec<u32> = self
            .frames
            .keys()
            .copied()
            .filter(|&id| id < frame_id)
            .collect();
        for key in stale_keys {
            self.frames.remove(&key);
        }
    }
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
    fn single_packet_respects_max_frame_bytes() {
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::FRAME_START
                | nexus_protocol::video_packet::flags::FRAME_END,
            stream_id: 1,
            frame_id: 1,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: 4,
        };
        let mut reassembler = VideoFrameReassembler::new(2, 3);
        assert_eq!(
            reassembler.process_packet(&header, &[1, 2, 3, 4]).unwrap(),
            None
        );
    }

    #[test]
    fn drops_delayed_single_packet_without_regressing_watermark() {
        let make_header = |frame_id| VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::FRAME_START
                | nexus_protocol::video_packet::flags::FRAME_END,
            stream_id: 1,
            frame_id,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 0,
            payload_len: 1,
        };
        let mut reassembler = VideoFrameReassembler::new(2, 8);
        assert!(reassembler
            .process_packet(&make_header(10), &[1])
            .unwrap()
            .is_some());
        assert!(reassembler
            .process_packet(&make_header(9), &[2])
            .unwrap()
            .is_none());
        assert!(reassembler
            .process_packet(&make_header(11), &[3])
            .unwrap()
            .is_some());
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

    #[test]
    fn packetize_and_reassemble_single_packet_frame() {
        let base_header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::KEYFRAME,
            stream_id: 1,
            frame_id: 10,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 1000,
            payload_len: 0,
        };
        let payload = b"small single packet payload";
        let packets = packetize_video_frame(&base_header, payload, 1200).unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].0.packet_id, 0);
        assert_eq!(packets[0].0.packet_count, 1);
        assert_ne!(
            packets[0].0.flags & nexus_protocol::video_packet::flags::FRAME_START,
            0
        );
        assert_ne!(
            packets[0].0.flags & nexus_protocol::video_packet::flags::FRAME_END,
            0
        );

        let mut reassembler = VideoFrameReassembler::default();
        let assembled = reassembler
            .process_packet(&packets[0].0, &packets[0].1)
            .unwrap()
            .expect("should assemble immediately");
        assert_eq!(assembled.payload, payload);
        assert_eq!(assembled.header.frame_id, 10);
    }

    #[test]
    fn packetize_and_reassemble_multi_packet_frame_out_of_order() {
        let base_header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 1,
            frame_id: 20,
            packet_id: 0,
            packet_count: 0,
            timestamp_us: 2000,
            payload_len: 0,
        };
        let mut large_payload = Vec::new();
        for i in 0..2500 {
            large_payload.push((i % 256) as u8);
        }

        let packets = packetize_video_frame(&base_header, &large_payload, 1000).unwrap();
        assert_eq!(packets.len(), 3); // 1000 + 1000 + 500

        let mut reassembler = VideoFrameReassembler::default();

        // Feed packet 1 (middle) first -> None
        assert!(reassembler
            .process_packet(&packets[1].0, &packets[1].1)
            .unwrap()
            .is_none());

        // Feed packet 1 again (duplicate) -> None
        assert!(reassembler
            .process_packet(&packets[1].0, &packets[1].1)
            .unwrap()
            .is_none());

        // Feed packet 2 (last) -> None
        assert!(reassembler
            .process_packet(&packets[2].0, &packets[2].1)
            .unwrap()
            .is_none());

        // Feed packet 0 (first) -> Complete!
        let assembled = reassembler
            .process_packet(&packets[0].0, &packets[0].1)
            .unwrap()
            .expect("frame should complete");

        assert_eq!(assembled.payload, large_payload);
        assert_eq!(assembled.header.frame_id, 20);
    }

    #[test]
    fn reassembler_prunes_stale_incomplete_frames() {
        let mut reassembler = VideoFrameReassembler::default();

        let header_f1_p0 = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::FRAME_START,
            stream_id: 1,
            frame_id: 1,
            packet_id: 0,
            packet_count: 2,
            timestamp_us: 100,
            payload_len: 4,
        };
        // Frame 1 part 0 arrives
        assert!(reassembler
            .process_packet(&header_f1_p0, &[1, 2, 3, 4])
            .unwrap()
            .is_none());

        // Frame 2 arrives completely (single packet)
        let header_f2 = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::FRAME_START
                | nexus_protocol::video_packet::flags::FRAME_END,
            stream_id: 1,
            frame_id: 2,
            packet_id: 0,
            packet_count: 1,
            timestamp_us: 200,
            payload_len: 4,
        };
        let assembled_f2 = reassembler
            .process_packet(&header_f2, &[5, 6, 7, 8])
            .unwrap()
            .expect("frame 2 delivered");
        assert_eq!(assembled_f2.header.frame_id, 2);

        // Now late packet for Frame 1 arrives -> should be ignored (stale)
        let header_f1_p1 = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: nexus_protocol::video_packet::flags::FRAME_END,
            stream_id: 1,
            frame_id: 1,
            packet_id: 1,
            packet_count: 2,
            timestamp_us: 100,
            payload_len: 4,
        };
        assert!(reassembler
            .process_packet(&header_f1_p1, &[9, 10, 11, 12])
            .unwrap()
            .is_none());
    }

    #[test]
    fn reassembler_rejects_malformed_packets() {
        let mut reassembler = VideoFrameReassembler::default();

        // packet_id >= packet_count
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 1,
            frame_id: 1,
            packet_id: 2,
            packet_count: 2,
            timestamp_us: 0,
            payload_len: 1,
        };
        assert!(matches!(
            reassembler.process_packet(&header, &[1]),
            Err(ReassemblyError::InvalidPacketId { .. })
        ));

        // packet 0 missing FRAME_START
        let header = VideoPacketHeader {
            version: CURRENT_VERSION,
            flags: 0,
            stream_id: 1,
            frame_id: 1,
            packet_id: 0,
            packet_count: 2,
            timestamp_us: 0,
            payload_len: 1,
        };
        assert_eq!(
            reassembler.process_packet(&header, &[1]),
            Err(ReassemblyError::InvalidBoundaryFlags)
        );
    }
}
