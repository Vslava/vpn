use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame, FLAG_PONG};
use crate::tun::TunDevice;

pub async fn run_server(
    addr: std::net::SocketAddr,
    psk: &[u8; 32],
    tun_ip: std::net::Ipv4Addr,
    mtu: u16,
    netmask: u8,
) -> Result<(), Error> {
    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, tun_ip, netmask).await?);
    tracing::info!(iface = "ts0", ip = %tun_ip, "Created TUN interface");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %addr, "Listening");

    let cancel = CancellationToken::new();
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        crate::wait_for_shutdown().await;
        tracing::info!("Shutdown signal received");
        sig_cancel.cancel();
    });

    loop {
        let stream = tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = listener.accept() => {
                let (stream, peer) = result?;
                tracing::info!(peer = %peer, "Client connected");
                stream
            }
        };

        let result = handle_client(stream, psk, tun.clone(), cancel.clone()).await;

        if let Err(ref e) = result {
            tracing::error!(error = %e, "Client error");
        }

        if cancel.is_cancelled() {
            break;
        }

        tracing::warn!("Client disconnected, waiting for new connection");
    }

    tracing::info!("Deleting TUN");
    drop(tun);
    tracing::info!("Shutdown complete");
    Ok(())
}

pub async fn handle_client(
    mut stream: tokio::net::TcpStream,
    psk: &[u8; 32],
    tun: Arc<TunDevice>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    let session_key = crate::handshake::server_handshake(&mut stream, psk).await?;
    tracing::info!("Handshake complete");

    crate::transport::set_keepalive(&stream)?;
    stream.set_nodelay(true)?;

    let (mut reader, mut writer) = tokio::io::split(stream);

    let crypto = Arc::new(Crypto::new(&session_key));
    let seq = Arc::new(AtomicU32::new(0));

    // Channel: h1 (PING handler) → h2 (TCP writer) for PONG frames
    let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

    // h1: TCP → TUN (handles PING → sends PONG via channel)
    let mut h1 = {
        let crypto = crypto.clone();
        let tun = tun.clone();
        let seq_h1 = seq.clone();
        tokio::spawn(async move {
            loop {
                let mut len_buf = [0u8; 2];
                reader.read_exact(&mut len_buf).await.map_err(Error::Io)?;
                let frame_len = u16::from_be_bytes(len_buf) as usize;
                let mut frame_data = vec![0u8; 2 + frame_len];
                frame_data[..2].copy_from_slice(&len_buf);
                reader.read_exact(&mut frame_data[2..]).await.map_err(Error::Io)?;

                let frame = protocol::decode(&frame_data)?;

                if frame.is_ping() {
                    let nonce = Crypto::generate_nonce();
                    let s = seq_h1.fetch_add(1, Ordering::Relaxed);
                    let pong = Frame {
                        nonce,
                        seq: s,
                        flags: FLAG_PONG,
                        payload: vec![],
                    };
                    pong_tx.send(protocol::encode(&pong)).await
                        .map_err(|_| Error::Io(std::io::Error::other("pong channel closed")))?;
                    continue;
                }

                if frame.is_pong() {
                    continue;
                }

                let plaintext = crypto.decrypt(&frame.nonce, &frame.payload)?;
                tracing::debug!(seq = frame.seq, len = plaintext.len(), "Packet received");
                tun.send(&plaintext).await.map_err(Error::Io)?;
            }
        })
    };

    // h2: TUN → TCP + channel (PONG) → TCP
    let mut h2 = {
        let crypto = crypto.clone();
        let tun = tun.clone();
        let seq_h2 = seq.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    result = async {
                        let mut buf = vec![0u8; tun.mtu() as usize];
                        let n = tun.recv(&mut buf).await.map_err(Error::Io)?;
                        buf.truncate(n);

                        let nonce = Crypto::generate_nonce();
                        let ciphertext = crypto.encrypt(&nonce, &buf)?;

                        let s = seq_h2.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(seq = s, len = buf.len(), "Packet sent");
                        let frame = Frame {
                            nonce,
                            seq: s,
                            flags: 0x00,
                            payload: ciphertext,
                        };

                        let encoded = protocol::encode(&frame);
                        writer.write_all(&encoded).await.map_err(Error::Io)?;
                        Ok::<_, Error>(())
                    } => { result?; }
                    Some(pong) = pong_rx.recv() => {
                        writer.write_all(&pong).await.map_err(Error::Io)?;
                    }
                }
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
            tracing::info!("Client handling cancelled");
            Ok(())
        }
        result = &mut h1 => {
            h2.abort();
            let _ = h2.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::other(e.to_string()))),
            }
        }
        result = &mut h2 => {
            h1.abort();
            let _ = h1.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::other(e.to_string()))),
            }
        }
    }
}


