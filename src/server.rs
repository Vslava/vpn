use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame};
use crate::tun::TunDevice;

pub async fn run_server(
    addr: std::net::SocketAddr,
    psk: &[u8; 32],
    tun_ip: std::net::Ipv4Addr,
    mtu: u16,
    netmask: u8,
) -> Result<(), Error> {
    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, tun_ip, netmask).await?);
    tracing::info!("server: TUN ts0 created, IP {}", tun_ip);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("server: listening on {}", addr);

    let cancel = CancellationToken::new();
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        crate::wait_for_shutdown().await;
        tracing::info!("server: shutdown signal received");
        sig_cancel.cancel();
    });

    let accept = tokio::select! {
        result = listener.accept() => result,
        _ = cancel.cancelled() => {
            tracing::info!("Deleting TUN");
            drop(tun);
            tracing::info!("Shutdown complete");
            return Ok(());
        }
    };

    let (stream, peer) = accept?;
    tracing::info!("server: client connected from {}", peer);

    let result = handle_client(stream, psk, tun.clone(), cancel).await;

    if let Err(ref e) = result {
        tracing::error!("server: client error: {}", e);
    }

    tracing::info!("Deleting TUN");
    drop(tun);
    tracing::info!("Shutdown complete");
    result
}

pub async fn handle_client(
    mut stream: tokio::net::TcpStream,
    psk: &[u8; 32],
    tun: Arc<TunDevice>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    let session_key = crate::handshake::server_handshake(&mut stream, psk).await?;
    tracing::info!("server: handshake complete");

    stream.set_nodelay(true)?;

    let (mut reader, mut writer) = tokio::io::split(stream);

    let crypto = Arc::new(Crypto::new(&session_key));
    let seq = AtomicU32::new(0);

    let mut h1 = {
        let crypto = crypto.clone();
        let tun = tun.clone();
        tokio::spawn(async move {
            loop {
                let mut len_buf = [0u8; 2];
                reader.read_exact(&mut len_buf).await.map_err(Error::Io)?;
                let frame_len = u16::from_be_bytes(len_buf) as usize;
                let mut frame_data = vec![0u8; 2 + frame_len];
                frame_data[..2].copy_from_slice(&len_buf);
                reader.read_exact(&mut frame_data[2..]).await.map_err(Error::Io)?;

                let frame = protocol::decode(&frame_data)?;
                let plaintext = crypto.decrypt(&frame.nonce, &frame.payload)?;
                tun.send(&plaintext).await.map_err(Error::Io)?;
            }
        })
    };

    let mut h2 = {
        let crypto = crypto.clone();
        let tun = tun.clone();
        tokio::spawn(async move {
            loop {
                let mut buf = vec![0u8; tun.mtu() as usize];
                let n = tun.recv(&mut buf).await.map_err(Error::Io)?;
                buf.truncate(n);

                let nonce = Crypto::generate_nonce();
                let ciphertext = crypto.encrypt(&nonce, &buf)?;

                let s = seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let frame = Frame {
                    nonce,
                    seq: s,
                    flags: 0x00,
                    payload: ciphertext,
                };

                let encoded = protocol::encode(&frame);
                writer.write_all(&encoded).await.map_err(Error::Io)?;
            }
        })
    };

    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            h1.abort();
            h2.abort();
            let _ = h1.await;
            let _ = h2.await;
            tracing::info!("server: client handling cancelled");
            Ok(())
        }
        result = &mut h1 => {
            h2.abort();
            let _ = h2.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            }
        }
        result = &mut h2 => {
            h1.abort();
            let _ = h1.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            }
        }
    }
}


