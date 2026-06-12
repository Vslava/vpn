use crate::error::Error;

const HEADER_LEN: usize = 24 + 4 + 1;
const MAX_PAYLOAD: usize = 65506;

pub const FLAG_DATA: u8 = 0x00;
pub const FLAG_PING: u8 = 0x01;
pub const FLAG_PONG: u8 = 0x02;

impl Frame {
    pub fn is_ping(&self) -> bool {
        self.flags == FLAG_PING
    }

    pub fn is_pong(&self) -> bool {
        self.flags == FLAG_PONG
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub nonce: [u8; 24],
    pub seq: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn encoded_len(&self) -> u16 {
        (HEADER_LEN + self.payload.len()) as u16
    }
}

pub fn encode(frame: &Frame) -> Vec<u8> {
    let total_len = HEADER_LEN + frame.payload.len();
    let mut buf = Vec::with_capacity(total_len);

    buf.extend_from_slice(&frame.nonce);
    buf.extend_from_slice(&frame.seq.to_be_bytes());
    buf.push(frame.flags);
    buf.extend_from_slice(&frame.payload);

    buf
}

pub fn decode(data: &[u8]) -> Result<Frame, Error> {
    if data.len() < HEADER_LEN {
        return Err(Error::Protocol(format!(
            "frame too short: got {got} bytes, need at least {HEADER_LEN}",
            got = data.len()
        )));
    }

    let payload_len = data.len() - HEADER_LEN;
    if payload_len > MAX_PAYLOAD {
        return Err(Error::Protocol(format!(
            "payload length {payload_len} exceeds maximum {MAX_PAYLOAD}"
        )));
    }

    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&data[0..24]);

    let seq = u32::from_be_bytes([data[24], data[25], data[26], data[27]]);
    let flags = data[28];
    let payload = data[29..].to_vec();

    Ok(Frame {
        nonce,
        seq,
        flags,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let frame = Frame {
            nonce: [1u8; 24],
            seq: 42,
            flags: 0x01,
            payload: vec![0xAA; 100],
        };

        let encoded = encode(&frame);
        let decoded = decode(&encoded).unwrap();

        assert_eq!(frame.nonce, decoded.nonce);
        assert_eq!(frame.seq, decoded.seq);
        assert_eq!(frame.flags, decoded.flags);
        assert_eq!(frame.payload, decoded.payload);
    }

    #[test]
    fn test_header_format() {
        let frame = Frame {
            nonce: [0x01u8; 24],
            seq: 0xDEADBEEF,
            flags: 0xAB,
            payload: vec![0x42; 10],
        };

        let encoded = encode(&frame);

        assert_eq!(&encoded[0..24], &[0x01u8; 24]);
        assert_eq!(&encoded[24..28], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(encoded[28], 0xAB);
        assert_eq!(&encoded[29..], &[0x42; 10]);
        // No length prefix: total = 24 + 4 + 1 + 10 = 39
        assert_eq!(encoded.len(), 39);
    }

    #[test]
    fn test_decode_too_short() {
        assert!(decode(&[0x00; 10]).is_err());
    }

    #[test]
    fn test_decode_empty() {
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn test_decode_frame_too_large() {
        let data = vec![0u8; HEADER_LEN + MAX_PAYLOAD + 1];
        assert!(decode(&data).is_err());
    }

    #[test]
    fn test_decode_garbage() {
        let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03];
        assert!(decode(&garbage).is_err());
    }

    #[test]
    fn test_boundary_sizes() {
        for payload_len in &[0, 1, 1400, 65506] {
            let frame = Frame {
                nonce: [0u8; 24],
                seq: 0,
                flags: 0,
                payload: vec![0x42; *payload_len],
            };
            let encoded = encode(&frame);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded.payload.len(), *payload_len);
        }
    }

    #[test]
    fn test_encoded_len() {
        let frame = Frame {
            nonce: [0u8; 24],
            seq: 0,
            flags: 0,
            payload: vec![0x42; 100],
        };
        assert_eq!(frame.encoded_len(), 24 + 4 + 1 + 100);
    }

    #[test]
    fn test_flags_ping_pong() {
        let ping = Frame {
            nonce: [0u8; 24],
            seq: 0,
            flags: FLAG_PING,
            payload: vec![],
        };
        assert!(ping.is_ping());
        assert!(!ping.is_pong());

        let pong = Frame {
            nonce: [0u8; 24],
            seq: 0,
            flags: FLAG_PONG,
            payload: vec![],
        };
        assert!(!pong.is_ping());
        assert!(pong.is_pong());

        let data = Frame {
            nonce: [0u8; 24],
            seq: 0,
            flags: FLAG_DATA,
            payload: vec![],
        };
        assert!(!data.is_ping());
        assert!(!data.is_pong());
    }
}
