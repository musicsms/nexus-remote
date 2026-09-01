use nexus_capture::{CaptureSource, CapturedFrame, LatestFrameQueue};
use nexus_codec::{CodecKind, EncoderConfig, SoftwareFallbackEncoder, VideoEncoder};
use nexus_crypto::NonceSequence;
use nexus_protocol::video_packet::flags;
use nexus_protocol::VideoPacketHeader;
use nexus_transport::video::{encode_video_datagram, packetize_video_frame, seal_video_frame};
use thiserror::Error;

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
        let encoded_frames = self.encoder.encode(latest)?;

        let mut datagrams = Vec::new();
        for encoded in encoded_frames {
            // 4. Build a header from the capture metadata matched to this
            // access unit, which can belong to an earlier pipelined input.
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

            // 6. Packetize encrypted payload into datagram chunks.
            let packets = packetize_video_frame(&base_header, &encrypted_frame.ciphertext, 1200)?;

            // 7. Encode each packet into wire format (VideoPacketHeader + payload).
            for (header, payload) in packets {
                datagrams.push(encode_video_datagram(&header, &payload)?);
            }
        }

        Ok(datagrams)
    }

    /// Request an immediate keyframe generation.
    pub fn request_keyframe(&mut self) -> Result<(), StreamerError> {
        self.encoder.request_keyframe()?;
        Ok(())
    }
}
