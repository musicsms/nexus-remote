use prost::Message;
use thiserror::Error;

pub const MAX_CONTROL_MESSAGE_SIZE: usize = 64 * 1024;
pub const CONTROL_FRAME_PREFIX_SIZE: usize = 4;
pub const CONTROL_KIND_PREFIX_SIZE: usize = 1;

/// Explicit kinds used by the host-to-client cursor envelope.  The legacy
/// framed-control format remains available for semantic input, but cursor
/// messages need an unambiguous type because protobuf messages with default
/// fields can otherwise have identical wire shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ControlMessageKind {
    CursorPosition = 1,
    CursorShape = 2,
}

impl TryFrom<u8> for ControlMessageKind {
    type Error = ControlMessageError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::CursorPosition),
            2 => Ok(Self::CursorShape),
            other => Err(ControlMessageError::UnknownKind(other)),
        }
    }
}

#[derive(Debug, Error)]
pub enum ControlMessageError {
    #[error("control message is too large: {actual} bytes (limit {limit})")]
    TooLarge { actual: usize, limit: usize },
    #[error("control message failed to decode: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("truncated or incorrectly framed control message")]
    TruncatedFrame,
    #[error("unknown control message kind: {0}")]
    UnknownKind(u8),
}

pub fn encode_control<M: Message>(message: &M) -> Result<Vec<u8>, ControlMessageError> {
    let size = message.encoded_len();
    if size > MAX_CONTROL_MESSAGE_SIZE {
        return Err(ControlMessageError::TooLarge {
            actual: size,
            limit: MAX_CONTROL_MESSAGE_SIZE,
        });
    }
    let mut bytes = Vec::with_capacity(size);
    message
        .encode(&mut bytes)
        .expect("prost Vec encoding cannot fail");
    Ok(bytes)
}

pub fn decode_control<M: Message + Default>(bytes: &[u8]) -> Result<M, ControlMessageError> {
    if bytes.len() > MAX_CONTROL_MESSAGE_SIZE {
        return Err(ControlMessageError::TooLarge {
            actual: bytes.len(),
            limit: MAX_CONTROL_MESSAGE_SIZE,
        });
    }
    Ok(M::decode(bytes)?)
}

pub fn encode_framed_control<M: Message>(message: &M) -> Result<Vec<u8>, ControlMessageError> {
    let payload = encode_control(message)?;
    let len = u32::try_from(payload.len()).expect("control limit fits in u32");
    let mut framed = Vec::with_capacity(CONTROL_FRAME_PREFIX_SIZE + payload.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&payload);
    Ok(framed)
}

pub fn decode_framed_control<M: Message + Default>(bytes: &[u8]) -> Result<M, ControlMessageError> {
    if bytes.len() < CONTROL_FRAME_PREFIX_SIZE {
        return Err(ControlMessageError::TruncatedFrame);
    }
    let declared =
        u32::from_be_bytes(bytes[..CONTROL_FRAME_PREFIX_SIZE].try_into().unwrap()) as usize;
    if declared > MAX_CONTROL_MESSAGE_SIZE {
        return Err(ControlMessageError::TooLarge {
            actual: declared,
            limit: MAX_CONTROL_MESSAGE_SIZE,
        });
    }
    if bytes.len() != CONTROL_FRAME_PREFIX_SIZE + declared {
        return Err(ControlMessageError::TruncatedFrame);
    }
    decode_control(&bytes[CONTROL_FRAME_PREFIX_SIZE..])
}

pub fn encode_framed_control_envelope<M: Message>(
    kind: ControlMessageKind,
    message: &M,
) -> Result<Vec<u8>, ControlMessageError> {
    let payload = encode_control(message)?;
    let declared = CONTROL_KIND_PREFIX_SIZE.checked_add(payload.len()).ok_or(
        ControlMessageError::TooLarge {
            actual: usize::MAX,
            limit: MAX_CONTROL_MESSAGE_SIZE,
        },
    )?;
    if declared > MAX_CONTROL_MESSAGE_SIZE {
        return Err(ControlMessageError::TooLarge {
            actual: declared,
            limit: MAX_CONTROL_MESSAGE_SIZE,
        });
    }
    let mut framed = Vec::with_capacity(CONTROL_FRAME_PREFIX_SIZE + declared);
    framed.extend_from_slice(&(declared as u32).to_be_bytes());
    framed.push(kind as u8);
    framed.extend_from_slice(&payload);
    Ok(framed)
}

pub fn decode_framed_control_envelope(
    bytes: &[u8],
) -> Result<(ControlMessageKind, &[u8]), ControlMessageError> {
    if bytes.len() < CONTROL_FRAME_PREFIX_SIZE + CONTROL_KIND_PREFIX_SIZE {
        return Err(ControlMessageError::TruncatedFrame);
    }
    let declared = u32::from_be_bytes(
        bytes[..CONTROL_FRAME_PREFIX_SIZE]
            .try_into()
            .expect("checked control frame prefix length"),
    ) as usize;
    if declared > MAX_CONTROL_MESSAGE_SIZE {
        return Err(ControlMessageError::TooLarge {
            actual: declared,
            limit: MAX_CONTROL_MESSAGE_SIZE,
        });
    }
    if bytes.len() != CONTROL_FRAME_PREFIX_SIZE + declared || declared < CONTROL_KIND_PREFIX_SIZE {
        return Err(ControlMessageError::TruncatedFrame);
    }
    let payload = &bytes[CONTROL_FRAME_PREFIX_SIZE..];
    let kind = ControlMessageKind::try_from(payload[0])?;
    Ok((kind, &payload[CONTROL_KIND_PREFIX_SIZE..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_protocol::{CursorShape, TextInput};

    #[test]
    fn round_trip_control_message() {
        let message = TextInput {
            text: "hello".into(),
        };
        let encoded = encode_control(&message).unwrap();
        assert_eq!(decode_control::<TextInput>(&encoded).unwrap(), message);
    }

    #[test]
    fn rejects_oversized_control_payload() {
        let bytes = vec![0; MAX_CONTROL_MESSAGE_SIZE + 1];
        assert!(matches!(
            decode_control::<TextInput>(&bytes),
            Err(ControlMessageError::TooLarge { .. })
        ));
    }

    #[test]
    fn length_prefix_frames_control_message() {
        let message = TextInput {
            text: "hello".into(),
        };
        let encoded = encode_framed_control(&message).unwrap();
        assert_eq!(
            decode_framed_control::<TextInput>(&encoded).unwrap(),
            message
        );
        assert!(matches!(
            decode_framed_control::<TextInput>(&encoded[..encoded.len() - 1]),
            Err(ControlMessageError::TruncatedFrame)
        ));
    }

    #[test]
    fn typed_envelope_preserves_cursor_kind_with_default_fields() {
        let shape = CursorShape {
            id: 7,
            width: 1,
            height: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            pixel_format: 0,
            data: Vec::new(),
        };
        let encoded =
            encode_framed_control_envelope(ControlMessageKind::CursorShape, &shape).unwrap();
        let (kind, payload) = decode_framed_control_envelope(&encoded).unwrap();
        assert_eq!(kind, ControlMessageKind::CursorShape);
        assert_eq!(decode_control::<CursorShape>(payload).unwrap(), shape);
    }
}
