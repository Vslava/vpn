use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame, FLAG_PONG};
use crate::tun::TunDevice;

const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

pub async fn run_server(
    addr: std::net::SocketAddr,
    psk: &[u8; 32],
    tun_ip: std::net::Ipv4Addr,
    mtu: u16,
    netmask: u8,
) -> Result<(), Error> {
    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, tun_ip, netmask).await?);
    tracing::info!(iface = "ts0", ip = %tun_ip, "Created TUN interface");

    let ext_iface = crate::route::save_default_route()
        .await
        .ok()
        .flatten()
        .map(|r| r.ifname);

    if let Some(ref iface) = ext_iface {
        crate::nat::setup_nat(iface).await?;
    } else {
        tracing::warn!("No default route found; skipping NAT setup. \
            Return traffic from the internet may not reach the tunnel.");
    }

    let socket = Arc::new(crate::transport::udp_bind(addr).await?);
    tracing::info!(addr = %addr, "Listening");

    let cancel = CancellationToken::new();
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        crate::wait_for_shutdown().await;
        tracing::info!("Shutdown signal received");
        sig_cancel.cancel();
    });

    loop {
        let (session_key, client_addr) = match crate::handshake::server_handshake(&socket, psk).await
        {
            Ok(result) => result,
            Err(e) => {
                if cancel.is_cancelled() {
                    break;
                }
                tracing::error!(error = %e, "Handshake failed, retrying");
                continue;
            }
        };
        tracing::info!(peer = %client_addr, "Handshake complete");

        let crypto = Arc::new(Crypto::new(&session_key));
        let seq = Arc::new(AtomicU32::new(0));

        let result = handle_client(
            socket.clone(),
            tun.clone(),
            client_addr,
            crypto,
            seq,
            cancel.clone(),
        )
        .await;

        if let Err(ref e) = result {
            tracing::error!(error = %e, "Client error");
        }

        if cancel.is_cancelled() {
            break;
        }

        tracing::warn!("Client disconnected, waiting for new connection");
    }

    if let Some(ref iface) = ext_iface {
        let _ = crate::nat::cleanup_nat(iface).await;
    }
    tracing::info!("Deleting TUN");
    drop(tun);
    tracing::info!("Shutdown complete");
    Ok(())
}

pub async fn handle_client(
    socket: Arc<UdpSocket>,
    tun: Arc<TunDevice>,
    client_addr: std::net::SocketAddr,
    crypto: Arc<Crypto>,
    seq: Arc<AtomicU32>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

    // h1: UDP → TUN (handles PING → sends PONG via channel)
    let mut h1 = {
        let socket = socket.clone();
        let tun = tun.clone();
        let crypto = crypto.clone();
        let seq_h1 = seq.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];

            loop {
                let (n, peer) = tokio::time::timeout(
                    CLIENT_IDLE_TIMEOUT,
                    socket.recv_from(&mut buf),
                )
                .await
                .map_err(|_| Error::Timeout("client idle timeout".into()))??;

                if peer != client_addr {
                    tracing::debug!(peer = %peer, "Ignoring datagram from unknown source");
                    continue;
                }

                let frame = protocol::decode(&buf[..n])?;

                if frame.is_ping() {
                    let nonce = Crypto::generate_nonce();
                    let s = seq_h1.fetch_add(1, Ordering::Relaxed);
                    let pong = Frame {
                        nonce,
                        seq: s,
                        flags: FLAG_PONG,
                        payload: vec![],
                    };
                    pong_tx
                        .send(protocol::encode(&pong))
                        .await
                        .map_err(|_| Error::Io(std::io::Error::other("pong channel closed")))?;
                    continue;
                }

                if frame.is_pong() {
                    continue;
                }

                let plaintext = match crypto.decrypt(&frame.nonce, &frame.payload) {
                    Ok(p) => p,
                    Err(e) => {
                        if n == 64 {
                            tracing::debug!("64-byte failed decrypt — possible new client handshake, ending session");
                            return Ok(());
                        }
                        tracing::debug!("Decryption failed: {e}");
                        continue;
                    }
                };
                tracing::debug!(seq = frame.seq, len = plaintext.len(), "Packet received");
                tun.send(&plaintext).await.map_err(Error::Io)?;
            }
        })
    };

    // h2: TUN → UDP + channel (PONG) → UDP
    let mut h2 = {
        let socket = socket.clone();
        let tun = tun.clone();
        let crypto = crypto.clone();
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
                        socket.send_to(&encoded, client_addr).await.map_err(Error::Io)?;
                        Ok::<_, Error>(())
                    } => { result?; }
                    Some(pong) = pong_rx.recv() => {
                        socket.send_to(&pong, client_addr).await.map_err(Error::Io)?;
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
