use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame};
use crate::tun::TunDevice;

pub async fn run_client(
    tun: Arc<TunDevice>,
    stream: TcpStream,
    crypto: Arc<Crypto>,
) -> Result<(), Error> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let seq = std::sync::atomic::AtomicU32::new(0);

    let tun_tx = tun.clone();
    let crypto_tx = crypto.clone();
    let tun_to_tcp = tokio::spawn(async move {
        loop {
            let mut buf = vec![0u8; tun_tx.mtu() as usize];
            let n = tun_tx.recv(&mut buf).await.map_err(Error::Io)?;
            buf.truncate(n);

            let nonce = Crypto::generate_nonce();
            let ciphertext = crypto_tx.encrypt(&nonce, &buf)?;

            let s = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let frame = Frame {
                nonce,
                seq: s,
                flags: 0x00,
                payload: ciphertext,
            };

            let encoded = protocol::encode(&frame);
            writer.write_all(&encoded).await
                .map_err(|e| Error::Io(e))?;
        }
    });

    let crypto_rx = crypto.clone();
    let tcp_to_tun = tokio::spawn(async move {
        loop {
            let mut len_buf = [0u8; 2];
            reader.read_exact(&mut len_buf).await.map_err(Error::Io)?;
            let frame_len = u16::from_be_bytes(len_buf) as usize;

            let mut frame_data = vec![0u8; 2 + frame_len];
            frame_data[..2].copy_from_slice(&len_buf);
            reader.read_exact(&mut frame_data[2..]).await.map_err(Error::Io)?;

            let frame = protocol::decode(&frame_data)?;

            let plaintext = crypto_rx.decrypt(&frame.nonce, &frame.payload)?;

            tun.send(&plaintext).await.map_err(Error::Io)?;
        }
    });

    tokio::select! {
        result = tun_to_tcp => {
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            }
        }
        result = tcp_to_tun => {
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            }
        }
    }
}
