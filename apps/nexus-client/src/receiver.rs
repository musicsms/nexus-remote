//! Portable authenticated video receive and semantic-input encoding boundary.

use nexus_crypto::{
    nonce_from_sequence, open_encoded_frame, AeadError, EncodedFrameMetadata, EncryptedFrame,
};
use nexus_input::{InputError, InputEvent, KeyAction, MouseButton};
use nexus_protocol::{
    video_packet, CursorPosition, CursorShape, CursorShapeError, KeyEvent, MonitorInfo,
    MonitorInfoError, MouseButton as ProtoMouseButton, MouseMove, MouseWheel, TextInput,
};
use nexus_transport::{
    control::{decode_framed_control, encode_framed_control, ControlMessageError},
    video::{decode_video_datagram, ReassemblyError, VideoDatagramError, VideoFrameReassembler},
};
use thiserror::Error;

const NONCE_REPLAY_WINDOW: u64 = 4096;

#[derive(Debug, Default)]
struct NonceReplayWindow {
    highest_sequence: Option<u64>,
    accepted_sequences: std::collections::BTreeSet<u64>,
}

impl NonceReplayWindow {
    /// Records one sequence after its frame has passed AEAD. Values inside the
    /// bounded window may arrive out of order, but each is accepted once.
    fn commit_authenticated(&mut self, sequence: u64) -> bool {
        if self.accepted_sequences.contains(&sequence) {
            return false;
        }
        if self
            .highest_sequence
            .is_some_and(|highest| sequence <= highest && highest - sequence >= NONCE_REPLAY_WINDOW)
        {
            return false;
        }

        let highest = self
            .highest_sequence
            .map_or(sequence, |known| known.max(sequence));
        self.accepted_sequences.insert(sequence);
        self.highest_sequence = Some(highest);
        self.accepted_sequences
            .retain(|accepted| highest - *accepted < NONCE_REPLAY_WINDOW);
        true
    }
}

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

/// Bounded control state sent by the host on the datagram path.  These
/// messages are deliberately separate from video packets: a cursor update
/// must never be handed to the video reassembler as if it were a fragment.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientControlEvent {
    CursorPosition(CursorPosition),
    CursorShape(CursorShape),
}

#[derive(Debug, Error)]
pub enum ClientReceiverError {
    #[error("invalid video datagram: {0}")]
    Datagram(#[from] VideoDatagramError),
    #[error("video datagram contains trailing bytes")]
    TrailingDatagramBytes,
    #[error("invalid video fragment: {0}")]
    Reassembly(#[from] ReassemblyError),
    #[error("frame authentication failed: {0}")]
    Authentication(#[from] AeadError),
    #[error("frame nonce sequence was replayed or is outside the replay window")]
    NonceReplay,
    #[error("invalid control message: {0}")]
    Control(#[from] ControlMessageError),
    #[error("invalid cursor monitor: {0}")]
    CursorMonitor(#[from] MonitorInfoError),
    #[error("cursor control received before monitor bounds are configured")]
    CursorMonitorUnavailable,
    #[error("cursor position is outside the configured monitor")]
    CursorOutOfBounds,
    #[error("invalid cursor shape: {0}")]
    CursorShape(#[from] CursorShapeError),
}

/// Validates and decrypts host-to-client video before making frames available.
///
/// `nonce_domain` must be the host-to-client direction/channel domain negotiated
/// for this session; it must not be reused for client-to-host traffic.
pub struct ClientReceiver {
    frame_key: [u8; 32],
    nonce_domain: u32,
    reassembler: VideoFrameReassembler,
    nonce_replay_window: NonceReplayWindow,
    latest_frame: Option<DecodedFrameJob>,
    cursor_monitor: Option<MonitorInfo>,
}

impl ClientReceiver {
    pub fn new(frame_key: [u8; 32], nonce_domain: u32) -> Self {
        Self {
            frame_key,
            nonce_domain,
            reassembler: VideoFrameReassembler::default(),
            nonce_replay_window: NonceReplayWindow::default(),
            latest_frame: None,
            cursor_monitor: None,
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
            nonce: nonce_from_sequence(self.nonce_domain, assembled.header.nonce_sequence),
            ciphertext: assembled.payload,
        };
        let access_unit = open_encoded_frame(
            &self.frame_key,
            EncodedFrameMetadata {
                protocol_version: assembled.header.version as u32,
                channel: assembled.header.stream_id as u32,
                frame_id: assembled.header.frame_id,
                codec_config_id: CODEC_CONFIG_ID,
                timestamp_us: assembled.header.timestamp_us,
                keyframe: assembled.header.flags & video_packet::flags::KEYFRAME != 0,
            },
            &encrypted,
        )?;

        if !self
            .nonce_replay_window
            .commit_authenticated(assembled.header.nonce_sequence)
        {
            return Err(ClientReceiverError::NonceReplay);
        }

        if !self
            .reassembler
            .commit_authenticated_frame(assembled.header.frame_id)
        {
            return Ok(());
        }

        self.latest_frame = Some(DecodedFrameJob {
            frame_id: assembled.header.frame_id,
            timestamp_us: assembled.header.timestamp_us,
            keyframe: assembled.header.flags & video_packet::flags::KEYFRAME != 0,
            access_unit,
        });
        Ok(())
    }

    /// Configures the monitor coordinate space used to validate inbound cursor
    /// positions. Call this after authenticated monitor configuration arrives.
    pub fn set_cursor_monitor(&mut self, monitor: MonitorInfo) -> Result<(), ClientReceiverError> {
        monitor.validate()?;
        self.cursor_monitor = Some(monitor);
        Ok(())
    }

    /// Decodes and validates an inbound cursor position against the configured
    /// monitor before returning it to the UI boundary.
    pub fn accept_control(&self, bytes: &[u8]) -> Result<CursorPosition, ClientReceiverError> {
        let cursor: CursorPosition = decode_framed_control(bytes)?;
        let monitor = self
            .cursor_monitor
            .as_ref()
            .ok_or(ClientReceiverError::CursorMonitorUnavailable)?;
        let x_end = i64::from(monitor.origin_x) + i64::from(monitor.width);
        let y_end = i64::from(monitor.origin_y) + i64::from(monitor.height);
        if i64::from(cursor.x) < i64::from(monitor.origin_x)
            || i64::from(cursor.x) >= x_end
            || i64::from(cursor.y) < i64::from(monitor.origin_y)
            || i64::from(cursor.y) >= y_end
        {
            return Err(ClientReceiverError::CursorOutOfBounds);
        }
        Ok(cursor)
    }

    /// Decodes either supported host cursor control message.  Cursor shapes
    /// are validated before they cross the UI boundary, including a strict
    /// payload bound from the protocol crate.  The position fallback keeps
    /// the existing wire format compatible with hosts that send raw framed
    /// protobuf messages (there is no unbounded or dynamically typed queue).
    pub fn accept_control_datagram(
        &self,
        bytes: &[u8],
    ) -> Result<ClientControlEvent, ClientReceiverError> {
        // The legacy framing has no message-kind byte. CursorPosition and a
        // minimal CursorShape can therefore be wire-ambiguous; shape-only
        // fields (hotspot/pixel format/data, tags 5-7) are the safe marker.
        // Hosts sending a shape include at least one of these fields, while a
        // position remains compatible with the existing four-field format.
        if cursor_shape_fields_present(bytes) {
            let shape: CursorShape = decode_framed_control(bytes)?;
            shape.validate()?;
            return Ok(ClientControlEvent::CursorShape(shape));
        }
        self.accept_control(bytes)
            .map(ClientControlEvent::CursorPosition)
    }

    /// Returns the newest authenticated frame, dropping it from the depth-one
    /// handoff slot.
    pub fn drain_latest_frame(&mut self) -> Option<DecodedFrameJob> {
        self.latest_frame.take()
    }
}

/// CursorShape has fields 5-7 that CursorPosition cannot carry.  Looking for
/// those tags only disambiguates malformed shape payloads; valid shapes with
/// default optional fields still pass the strict `CursorShape::validate`
/// path above.
fn cursor_shape_fields_present(bytes: &[u8]) -> bool {
    let Some(declared) = bytes
        .get(..4)
        .and_then(|prefix| prefix.try_into().ok())
        .map(u32::from_be_bytes)
    else {
        return false;
    };
    let payload = bytes
        .get(4..)
        .filter(|payload| payload.len() == declared as usize);
    let Some(payload) = payload else { return false };
    let mut index = 0;
    let mut shape_field_seen = false;
    while index < payload.len() {
        let Some((tag, consumed)) = read_varint(payload, index) else {
            return false;
        };
        index += consumed;
        let field = tag >> 3;
        if field >= 5 {
            shape_field_seen = true;
        }
        let wire_type = tag & 7;
        let Some(skip) = (match wire_type {
            0 => read_varint(payload, index).map(|(_, size)| size),
            1 => Some(8),
            2 => read_varint(payload, index).and_then(|(len, size)| {
                usize::try_from(len)
                    .ok()
                    .map(|len| size.saturating_add(len))
            }),
            5 => Some(4),
            _ => None,
        }) else {
            return false;
        };
        let Some(next) = index.checked_add(skip) else {
            return false;
        };
        index = next;
    }
    index == payload.len() && shape_field_seen
}

fn read_varint(bytes: &[u8], mut index: usize) -> Option<(u64, usize)> {
    let start = index;
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(index)?;
        index += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, index - start));
        }
    }
    None
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
