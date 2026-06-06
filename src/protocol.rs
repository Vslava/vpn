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
    let mut buf = Vec::with_capacity(2 + total_len);

    buf.extend_from_slice(&(total_len as u16).to_be_bytes());
    buf.extend_from_slice(&frame.nonce);
    buf.extend_from_slice(&frame.seq.to_be_bytes());
    buf.push(frame.flags);
    buf.extend_from_slice(&frame.payload);

    buf
}

pub fn decode(data: &[u8]) -> Result<Frame, Error> {
    if data.len() < 2 {
        return Err(Error::Protocol("frame too short for length header".into()));
    }

    let frame_len_no_header = u16::from_be_bytes([data[0], data[1]]) as usize;

    if frame_len_no_header < HEADER_LEN {
        return Err(Error::Protocol(format!(
            "frame length {frame_len_no_header} too small, minimum {HEADER_LEN}"
        )));
    }

    let payload_len = frame_len_no_header - HEADER_LEN;
    if payload_len > MAX_PAYLOAD {
        return Err(Error::Protocol(format!(
            "payload length {payload_len} exceeds maximum {MAX_PAYLOAD}"
        )));
    }

    if data.len() < 2 + frame_len_no_header {
        return Err(Error::Protocol(format!(
            "frame data too short: need {need} bytes, got {got}",
            need = 2 + frame_len_no_header,
            got = data.len()
        )));
    }

    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&data[2..26]);

    let seq = u32::from_be_bytes([data[26], data[27], data[28], data[29]]);
    let flags = data[30];
    let payload = data[31..(2 + frame_len_no_header)].to_vec();

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
        let total_len = HEADER_LEN + 10;

        assert_eq!(&encoded[0..2], &(total_len as u16).to_be_bytes());
        assert_eq!(&encoded[2..26], &[0x01u8; 24]);
        assert_eq!(&encoded[26..30], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(encoded[30], 0xAB);
        assert_eq!(&encoded[31..], &[0x42; 10]);
    }

    #[test]
    fn test_decode_too_short() {
        assert!(decode(&[0x00, 0x01]).is_err());
    }

    #[test]
    fn test_decode_zero_length() {
        assert!(decode(&[0x00, 0x00]).is_err());
    }

    #[test]
    fn test_decode_frame_too_large() {
        let mut data = vec![0u8; 2 + HEADER_LEN + MAX_PAYLOAD + 1];
        let large_len = (HEADER_LEN + MAX_PAYLOAD + 1) as u16;
        data[..2].copy_from_slice(&large_len.to_be_bytes());
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
}
