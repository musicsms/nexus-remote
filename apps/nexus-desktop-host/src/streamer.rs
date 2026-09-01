use nexus_capture::{CaptureSource, CapturedFrame, LatestFrameQueue};
use nexus_codec::{CodecKind, EncoderConfig, SoftwareFallbackEncoder, VideoEncoder};
use nexus_crypto::NonceSequence;
use nexus_protocol::video_packet::flags;
use nexus_protocol::VideoPacketHeader;
use nexus_transport::video::{encode_video_datagram, packetize_video_frame, seal_video_frame};
use thiserror::Error;

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use nexus_capture::SyntheticCaptureSource;
    use nexus_codec::{CodecError, EncodedFrame};
    use nexus_transport::video::decode_video_datagram;

    #[test]
    fn packet_headers_use_encoded_frame_metadata() {
        let mut streamer =
            HostVideoStreamer::new(SyntheticCaptureSource::new(2, 2, 30), [9; 32], 2, 2).unwrap();
        let encoded = EncodedFrame {
            frame_id: 77,
            timestamp_us: 123_456,
            keyframe: false,
            data: Bytes::from_static(b"encoded"),
        };

        let datagrams = streamer.packetize_encoded_frame(encoded).unwrap();
        let (header, _) = decode_video_datagram(&datagrams[0]).unwrap();
        assert_eq!(header.frame_id, 77);
        assert_eq!(header.timestamp_us, 123_456);
    }

    #[test]
    fn output_pending_is_non_fatal_and_emits_no_packets() {
        let mut streamer =
            HostVideoStreamer::new(SyntheticCaptureSource::new(2, 2, 30), [9; 32], 2, 2).unwrap();

        let packets = streamer
            .packetize_encode_result(Err(CodecError::OutputPending))
            .unwrap();
        assert!(packets.is_empty());
    }
}

/// Errors arising during host video streaming pipeline.
#[derive(Debug, Error)]
pub enum StreamerError {
    #[error("Capture error: {0}")]
    Capture(String),

    #[error("Codec error: {0}")]
    Codec(#[from] nexus_codec::CodecError),

    #[error("Crypto seal error: {0}")]
    Seal(#[from] nexus_crypto::FrameSealError),

    #[error("Packetize error: {0}")]
    Packetize(#[from] nexus_transport::video::PacketizeError),

    #[error("Datagram encode error: {0}")]
    Datagram(#[from] nexus_transport::video::VideoDatagramError),

    #[error("Channel closed")]
    ChannelClosed,
}

/// Coordinates capture -> encode -> AEAD seal -> packetize -> datagram transmission.
pub struct HostVideoStreamer<C: CaptureSource> {
    capture: C,
    encoder: SoftwareFallbackEncoder,
    queue: LatestFrameQueue<CapturedFrame>,
    aead_key: [u8; 32],
    nonce_seq: NonceSequence,
    codec_config_id: u32,
    stream_id: u8,
}

impl<C: CaptureSource> HostVideoStreamer<C> {
    /// Creates a new `HostVideoStreamer` with the given capture source and session AEAD key.
    pub fn new(
        capture: C,
        aead_key: [u8; 32],
        width: u32,
        height: u32,
    ) -> Result<Self, StreamerError> {
        let encoder_config = EncoderConfig {
            codec: CodecKind::H264,
            width,
            height,
            max_fps: 30,
            bitrate_bps: 4_000_000,
        };

        let mut encoder = SoftwareFallbackEncoder::new();
        encoder.configure(encoder_config)?;

        Ok(Self {
            capture,
            encoder,
            queue: LatestFrameQueue::new(),
            aead_key,
            nonce_seq: NonceSequence::new(1),
            codec_config_id: 1,
            stream_id: 1,
        })
    }

    /// Captures the latest frame, encodes, seals with AEAD, and produces datagram packets.
    pub fn process_next_frame(&mut self) -> Result<Vec<Vec<u8>>, StreamerError> {
        // 1. Capture frame
        let frame = self
            .capture
            .next_frame()
            .map_err(|e| StreamerError::Capture(e.to_string()))?;
        self.queue.replace(frame);

        // 2. Pop newest frame (ADR-022 depth-1 queue)
        let Some(latest) = self.queue.take() else {
            return Ok(Vec::new());
        };

        // 3. Encode video frame
        let encoded = self.encoder.encode(latest);
        self.packetize_encode_result(encoded)
    }

    fn packetize_encode_result(
        &mut self,
        encoded: Result<nexus_codec::EncodedFrame, nexus_codec::CodecError>,
    ) -> Result<Vec<Vec<u8>>, StreamerError> {
        let encoded = match encoded {
            Ok(encoded) => encoded,
            // An asynchronous hardware encoder may accept an input before its
            // output is available. Keep the worker alive and wait for the next
            // pump/encode call to retrieve it.
            Err(nexus_codec::CodecError::OutputPending) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        self.packetize_encoded_frame(encoded)
    }

    fn packetize_encoded_frame(
        &mut self,
        encoded: nexus_codec::EncodedFrame,
    ) -> Result<Vec<Vec<u8>>, StreamerError> {
        // 4. Build base header
        let mut flags_val = 0u8;
        if encoded.keyframe {
            flags_val |= flags::KEYFRAME;
        }

        let base_header = VideoPacketHeader {
            version: 1,
            flags: flags_val,
            stream_id: self.stream_id as u16,
            frame_id: encoded.frame_id as u32,
            packet_id: 0,
            packet_count: 1,
            payload_len: 0,
            timestamp_us: encoded.timestamp_us,
        };

        // 5. Seal encoded frame with ChaCha20-Poly1305 AEAD (ADR-025)
        let encrypted_frame = seal_video_frame(
            &self.aead_key,
            &mut self.nonce_seq,
            &base_header,
            self.codec_config_id,
            &encoded.data,
        )?;

        // 6. Packetize encrypted payload into datagram chunks
        let chunk_size = 1200; // Safe MTU for QUIC datagrams
        let packets = packetize_video_frame(&base_header, &encrypted_frame.ciphertext, chunk_size)?;

        // 7. Encode each packet into wire format (VideoPacketHeader + payload)
        let mut datagrams = Vec::with_capacity(packets.len());
        for (header, payload) in packets {
            let encoded_dg = encode_video_datagram(&header, &payload)?;
            datagrams.push(encoded_dg);
        }

        Ok(datagrams)
    }

    /// Request an immediate keyframe generation.
    pub fn request_keyframe(&mut self) -> Result<(), StreamerError> {
        self.encoder.request_keyframe()?;
        Ok(())
    }
}
