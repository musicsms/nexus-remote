//! Portable authenticated video receive and semantic-input encoding boundary.

use nexus_crypto::{
    open_encoded_frame, AeadError, EncodedFrameMetadata, EncryptedFrame, NonceSequence,
    NonceSequenceError,
};
use nexus_input::{InputError, InputEvent, KeyAction, MouseButton};
use nexus_protocol::{
    video_packet, CursorPosition, KeyEvent, MouseButton as ProtoMouseButton, MouseMove, MouseWheel,
    TextInput,
};
use nexus_transport::{
    control::{decode_framed_control, encode_framed_control, ControlMessageError},
    video::{decode_video_datagram, ReassemblyError, VideoDatagramError, VideoFrameReassembler},
};
use thiserror::Error;

/// The MVP has one negotiated H.264 codec configuration. Future configuration
/// changes must carry an authenticated configuration identifier before this is
/// made dynamic.
const CODEC_CONFIG_ID: u32 = 1;

/// An authenticated encoded access unit ready for the decoder boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrameJob {
    pub frame_id: u32,
    pub timestamp_us: u64,
    pub keyframe: bool,
    pub access_unit: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ClientReceiverError {
    #[error("invalid video datagram: {0}")]
    Datagram(#[from] VideoDatagramError),
    #[error("video datagram contains trailing bytes")]
    TrailingDatagramBytes,
    #[error("invalid video fragment: {0}")]
    Reassembly(#[from] ReassemblyError),
    #[error("frame nonce allocation failed: {0}")]
    Nonce(#[from] NonceSequenceError),
    #[error("frame authentication failed: {0}")]
    Authentication(#[from] AeadError),
    #[error("invalid control message: {0}")]
    Control(#[from] ControlMessageError),
}

/// Validates and decrypts host-to-client video before making frames available.
///
/// `nonce_domain` must be the host-to-client direction/channel domain negotiated
/// for this session; it must not be reused for client-to-host traffic.
pub struct ClientReceiver {
    frame_key: [u8; 32],
    receive_nonces: NonceSequence,
    reassembler: VideoFrameReassembler,
    latest_frame: Option<DecodedFrameJob>,
}

impl ClientReceiver {
    pub fn new(frame_key: [u8; 32], nonce_domain: u32) -> Self {
        Self {
            frame_key,
            receive_nonces: NonceSequence::new(nonce_domain),
            reassembler: VideoFrameReassembler::default(),
            latest_frame: None,
        }
    }

    /// Accepts one hostile-network datagram and publishes a job only after full
    /// reassembly and AEAD authentication succeed.
    pub fn accept_datagram(&mut self, bytes: &[u8]) -> Result<(), ClientReceiverError> {
        let (header, payload) = decode_video_datagram(bytes)?;
        if bytes.len() != nexus_protocol::video_packet::HEADER_LEN + payload.len() {
            return Err(ClientReceiverError::TrailingDatagramBytes);
        }

        let Some(assembled) = self.reassembler.process_packet(&header, payload)? else {
            return Ok(());
        };

        let encrypted = EncryptedFrame {
            nonce: self.receive_nonces.next_nonce()?,
            ciphertext: assembled.payload,
        };
        let access_unit = open_encoded_frame(
            &self.frame_key,
            EncodedFrameMetadata {
                protocol_version: assembled.header.version as u32,
                channel: assembled.header.stream_id as u32,
                frame_id: assembled.header.frame_id,
                codec_config_id: CODEC_CONFIG_ID,
            },
            &encrypted,
        )?;

        self.latest_frame = Some(DecodedFrameJob {
            frame_id: assembled.header.frame_id,
            timestamp_us: assembled.header.timestamp_us,
            keyframe: assembled.header.flags & video_packet::flags::KEYFRAME != 0,
            access_unit,
        });
        Ok(())
    }

    /// Validates an inbound cursor-position control frame before a future UI
    /// boundary consumes it. It deliberately retains no remote text or media.
    pub fn accept_control(&mut self, bytes: &[u8]) -> Result<(), ClientReceiverError> {
        let _: CursorPosition = decode_framed_control(bytes)?;
        Ok(())
    }

    /// Returns the newest authenticated frame, dropping it from the depth-one
    /// handoff slot.
    pub fn drain_latest_frame(&mut self) -> Option<DecodedFrameJob> {
        self.latest_frame.take()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClientInputError {
    #[error("invalid semantic input: {0}")]
    Input(#[from] InputError),
    #[error("control message is too large: {actual} bytes (limit {limit})")]
    ControlTooLarge { actual: usize, limit: usize },
}

/// Encodes validated, OS-independent input as a bounded framed control message.
pub struct ClientInputSender;

impl ClientInputSender {
    pub fn encode(event: InputEvent) -> Result<Vec<u8>, ClientInputError> {
        event.validate()?;
        match event {
            InputEvent::Key {
                physical_code,
                logical_code,
                action,
                modifiers,
            } => Self::frame(&KeyEvent {
                physical_code,
                logical_code,
                pressed: action == KeyAction::Down,
                modifiers: modifiers.bits(),
            }),
            InputEvent::Text(text) => Self::frame(&TextInput { text }),
            InputEvent::MouseMove { x, y } => Self::frame(&MouseMove { x, y }),
            InputEvent::MouseButton { button, pressed } => Self::frame(&ProtoMouseButton {
                button: match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                },
                pressed,
            }),
            InputEvent::MouseWheel { delta_x, delta_y } => {
                Self::frame(&MouseWheel { delta_x, delta_y })
            }
        }
    }

    fn frame<M: prost::Message>(message: &M) -> Result<Vec<u8>, ClientInputError> {
        encode_framed_control(message).map_err(|error| match error {
            ControlMessageError::TooLarge { actual, limit } => {
                ClientInputError::ControlTooLarge { actual, limit }
            }
            ControlMessageError::Decode(_) | ControlMessageError::TruncatedFrame => {
                unreachable!("encoding a protobuf message cannot decode or truncate")
            }
        })
    }
}
