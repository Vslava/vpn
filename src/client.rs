use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame};
use crate::tun::TunDevice;

pub async fn run_client(
    tun: Arc<TunDevice>,
    stream: TcpStream,
    crypto: Arc<Crypto>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    let (mut reader, mut writer) = tokio::io::split(stream);
    let seq = std::sync::atomic::AtomicU32::new(0);

    let tun_tx = tun.clone();
    let crypto_tx = crypto.clone();
    let mut h1 = tokio::spawn(async move {
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
                .map_err(Error::Io)?;
        }
    });

    let crypto_rx = crypto.clone();
    let mut h2 = tokio::spawn(async move {
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
        biased;
        _ = cancel.cancelled() => {
            h1.abort();
            h2.abort();
            let _ = h1.await;
            let _ = h2.await;
            tracing::info!("forwarding cancelled");
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

async fn run_client_session(
    remote: std::net::SocketAddr,
    psk: &[u8; 32],
    tun: Arc<TunDevice>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    let mut stream = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(()),
        result = crate::transport::connect(remote) => result.map_err(Error::Io)?,
    };
    tracing::info!("Connected to {}", remote);

    let session_key = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(()),
        result = crate::handshake::client_handshake(&mut stream, psk) => result?,
    };
    tracing::info!("Handshake complete, resuming");

    let crypto = Arc::new(Crypto::new(&session_key));
    run_client(tun, stream, crypto, cancel).await
}

fn reconnect_backoff(attempt: u32, max_delay_secs: u64) -> u64 {
    let delay = 2u64.saturating_pow(attempt.min(31));
    delay.min(max_delay_secs)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_client_full(
    remote: std::net::SocketAddr,
    psk: &[u8; 32],
    tun_ip: std::net::Ipv4Addr,
    netmask: u8,
    gateway: std::net::Ipv4Addr,
    mtu: u16,
    max_retries: Option<u32>,
    reconnect_max_delay: u64,
) -> Result<(), Error> {
    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, tun_ip, netmask).await?);
    tracing::info!("client: TUN ts0 created, IP {}/{}", tun_ip, netmask);

    let saved_route = crate::route::save_default_route().await?;
    tracing::info!("client: saved default route: {:?}", saved_route);

    if let Some(ref route) = saved_route {
        let server_ip = match remote.ip() {
            std::net::IpAddr::V4(ip) => ip,
            std::net::IpAddr::V6(_) => return Err(Error::Config("IPv6 not supported".into())),
        };
        crate::route::add_exclude_route(server_ip, route).await?;
        crate::route::set_tun_route("ts0", gateway).await?;
        tracing::info!("client: routes configured");
    }

    let cancel = CancellationToken::new();
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        crate::wait_for_shutdown().await;
        tracing::info!("client: shutdown signal received");
        sig_cancel.cancel();
    });

    let mut attempt = 0u32;
    loop {
        let result = run_client_session(remote, psk, tun.clone(), cancel.clone()).await;

        match result {
            Ok(()) => break,
            Err(e) => {
                if matches!(&e, Error::Handshake(_)) {
                    tracing::error!("Fatal handshake error: {}", e);
                    break;
                }

                if let Some(max) = max_retries {
                    if attempt >= max {
                        tracing::error!("Max retries ({}) exceeded", max);
                        break;
                    }
                }

                let delay = reconnect_backoff(attempt, reconnect_max_delay);
                tracing::error!("TCP connection lost: {}", e);
                tracing::warn!("Reconnecting in {}s...", delay);

                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(delay)) => {
                        attempt += 1;
                        continue;
                    }
                }
            }
        }
    }

    tracing::info!("Restoring routes");
    if let Some(ref route) = saved_route {
        if let Err(e) = crate::route::restore_route(route).await {
            tracing::error!("client: failed to restore route: {}", e);
        }
    }

    tracing::info!("Deleting TUN");
    drop(tun);

    tracing::info!("Shutdown complete");

    Ok(())
}
