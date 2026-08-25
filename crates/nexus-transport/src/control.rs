use prost::Message;
use thiserror::Error;

pub const MAX_CONTROL_MESSAGE_SIZE: usize = 64 * 1024;
pub const CONTROL_FRAME_PREFIX_SIZE: usize = 4;

#[derive(Debug, Error)]
pub enum ControlMessageError {
    #[error("control message is too large: {actual} bytes (limit {limit})")]
    TooLarge { actual: usize, limit: usize },
    #[error("control message failed to decode: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("truncated or incorrectly framed control message")]
    TruncatedFrame,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_protocol::TextInput;

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
}
