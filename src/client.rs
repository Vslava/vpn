use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use crate::config;
use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol::{self, Frame, FLAG_DATA, FLAG_PING};
use crate::tun::TunDevice;

pub async fn run_client(
    tun: Arc<TunDevice>,
    socket: UdpSocket,
    crypto: Arc<Crypto>,
    cancel: CancellationToken,
    heartbeat_interval: Option<u64>,
    heartbeat_timeout: Option<u64>,
) -> Result<(), Error> {
    let hb_interval = std::time::Duration::from_secs(heartbeat_interval.unwrap_or(30));
    let hb_timeout = std::time::Duration::from_secs(heartbeat_timeout.unwrap_or(60));

    let socket = Arc::new(socket);
    let seq = Arc::new(AtomicU32::new(0));

    // Channel: heartbeat → h1 (PING frames to send)
    let (ping_tx, mut ping_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

    // Channel: heartbeat → main (error signal)
    let (err_tx, mut err_rx) = tokio::sync::mpsc::channel::<Error>(1);

    // Shared: last time a frame was received from server
    let last_rx = Arc::new(std::sync::Mutex::new(Instant::now()));

    // ── h1: TUN → UDP (data frames) + channel → UDP (PING frames) ──
    let tun_tx = tun.clone();
    let crypto_tx = crypto.clone();
    let seq_tx = seq.clone();
    let socket_h1 = socket.clone();
    let mut h1 = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                result = async {
                    let mut buf = vec![0u8; tun_tx.mtu() as usize];
                    let n = tun_tx.recv(&mut buf).await.map_err(Error::Io)?;
                    buf.truncate(n);

                    let nonce = Crypto::generate_nonce();
                    let ciphertext = crypto_tx.encrypt(&nonce, &buf)?;

                    let s = seq_tx.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(seq = s, len = buf.len(), "Packet sent");
                    let frame = Frame {
                        nonce,
                        seq: s,
                        flags: FLAG_DATA,
                        payload: ciphertext,
                    };

                    let encoded = protocol::encode(&frame);
                    socket_h1.send(&encoded).await.map_err(Error::Io)?;
                    Ok::<_, Error>(())
                } => {
                    result?;
                }
                Some(ping) = ping_rx.recv() => {
                    socket_h1.send(&ping).await.map_err(Error::Io)?;
                }
            }
        }
    });

    // ── h2: UDP → TUN (handle PONG) ──
    let crypto_rx = crypto.clone();
    let last_rx_h2 = last_rx.clone();
    let socket_h2 = socket.clone();
    let mut h2 = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];

        loop {
            let n = socket_h2.recv(&mut buf).await.map_err(Error::Io)?;

            let frame = protocol::decode(&buf[..n])?;

            if frame.is_pong() {
                *last_rx_h2.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                continue;
            }

            let plaintext = match crypto_rx.decrypt(&frame.nonce, &frame.payload) {
                Ok(p) => p,
                Err(e) => {
                    tracing::debug!("Decryption failed (stale frame?): {e}");
                    continue;
                }
            };
            tracing::debug!(seq = frame.seq, len = plaintext.len(), "Packet received");
            tun.send(&plaintext).await.map_err(Error::Io)?;
            *last_rx_h2.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        }
    });

    // ── Heartbeat: PING sender + timeout watchdog ──
    let hb_cancel = cancel.clone();
    let last_rx_hb = last_rx.clone();
    let seq_hb = seq.clone();
    let ping_tx_hb = ping_tx.clone();
    let mut hb = tokio::spawn(async move {
        let mut interval = tokio::time::interval(hb_interval);
        interval.tick().await;

        loop {
            tokio::select! {
                biased;
                _ = hb_cancel.cancelled() => break,
                _ = interval.tick() => {
                    let since_rx = {
                        let last = last_rx_hb.lock().unwrap_or_else(|e| e.into_inner());
                        last.elapsed()
                    };
                    if since_rx >= hb_interval {
                        let nonce = Crypto::generate_nonce();
                        let s = seq_hb.fetch_add(1, Ordering::Relaxed);
                        let frame = Frame {
                            nonce,
                            seq: s,
                            flags: FLAG_PING,
                            payload: vec![],
                        };
                        let encoded = protocol::encode(&frame);
                        if ping_tx_hb.send(encoded).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    // ── Timeout watchdog: fire if no rx for hb_timeout ──
    let cancel_watch = cancel.clone();
    let last_rx_watch = last_rx.clone();
    let err_tx_watch = err_tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if cancel_watch.is_cancelled() {
                break;
            }
            let elapsed = {
                let last = last_rx_watch.lock().unwrap_or_else(|e| e.into_inner());
                last.elapsed()
            };
            if elapsed >= hb_timeout {
                err_tx_watch
                    .send(Error::Timeout("heartbeat timeout".into()))
                    .await
                    .ok();
                break;
            }
        }
    });

    // ── Main select ──
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            h1.abort();
            h2.abort();
            hb.abort();
            let _ = h1.await;
            let _ = h2.await;
            let _ = hb.await;
            tracing::info!("Forwarding cancelled");
            Ok(())
        }
        result = &mut h1 => {
            h2.abort();
            hb.abort();
            let _ = h2.await;
            let _ = hb.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::other(e.to_string()))),
            }
        }
        result = &mut h2 => {
            h1.abort();
            hb.abort();
            let _ = h1.await;
            let _ = hb.await;
            match result {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::Io(std::io::Error::other(e.to_string()))),
            }
        }
        _ = &mut hb => {
            h1.abort();
            h2.abort();
            let _ = h1.await;
            let _ = h2.await;
            tracing::error!(component = "heartbeat", "Heartbeat sender exited unexpectedly");
            Err(Error::Timeout("heartbeat failure".into()))
        }
        Some(e) = err_rx.recv() => {
            h1.abort();
            h2.abort();
            hb.abort();
            let _ = h1.await;
            let _ = h2.await;
            let _ = hb.await;
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_client_session(
    remote: std::net::SocketAddr,
    psk: &[u8; 32],
    mtu: u16,
    server_ip: Ipv4Addr,
    saved_route: &Option<crate::route::DefaultRoute>,
    cancel: CancellationToken,
    heartbeat_interval: Option<u64>,
    heartbeat_timeout: Option<u64>,
) -> Result<(), Error> {
    let socket = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(()),
        result = crate::transport::udp_connect(remote) => result?,
    };
    tracing::info!(remote = %remote, "Connected");

    let (session_key, client_ip, netmask) = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Ok(()),
        result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            crate::handshake::client_handshake(&socket, psk),
        ) => match result {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(Error::Timeout("handshake timeout".into())),
        },
    };
    tracing::info!(assigned_ip = %client_ip, netmask = netmask, "Handshake complete");

    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, client_ip, netmask).await?);
    tracing::info!(iface = "ts0", ip = %client_ip, netmask = netmask, "Created TUN interface");

    if let Some(ref route) = saved_route {
        crate::route::add_exclude_route(server_ip, route).await?;
        let gateway = {
        let netmask_addr = config::netmask_from_prefix(netmask);
        let client_u32 = u32::from(client_ip);
        let netmask_u32 = u32::from(netmask_addr);
        let network_u32 = client_u32 & netmask_u32;
        Ipv4Addr::from((network_u32 + 1).to_be_bytes())
    };
    crate::route::set_tun_route("ts0", gateway).await?;
        tracing::info!(gateway = %client_ip, "Routes configured");
    }

    let crypto = Arc::new(Crypto::new(&session_key));
    run_client(tun, socket, crypto, cancel, heartbeat_interval, heartbeat_timeout).await
}

fn reconnect_backoff(attempt: u32, max_delay_secs: u64) -> u64 {
    let delay = 2u64.saturating_pow(attempt.min(31));
    delay.min(max_delay_secs)
}

pub async fn run_client_full(
    remote: std::net::SocketAddr,
    psk: &[u8; 32],
    mtu: u16,
    max_retries: Option<u32>,
    reconnect_max_delay: u64,
    heartbeat_interval: Option<u64>,
    heartbeat_timeout: Option<u64>,
) -> Result<(), Error> {
    let saved_route = crate::route::save_default_route().await?;
    tracing::info!(?saved_route, "Saved default route");

    let server_ip = match remote.ip() {
        std::net::IpAddr::V4(ip) => ip,
        std::net::IpAddr::V6(_) => return Err(Error::Config("IPv6 not supported".into())),
    };

    let cancel = CancellationToken::new();
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        crate::wait_for_shutdown().await;
        tracing::info!("Shutdown signal received");
        sig_cancel.cancel();
    });

    let mut attempt = 0u32;
    loop {
        let result = run_client_session(
            remote,
            psk,
            mtu,
            server_ip,
            &saved_route,
            cancel.clone(),
            heartbeat_interval,
            heartbeat_timeout,
        )
        .await;

        match result {
            Ok(()) => break,
            Err(e) => {
                if matches!(&e, Error::Handshake(_)) {
                    tracing::error!(error = %e, "Fatal handshake error");
                    break;
                }

                if let Some(max) = max_retries {
                    if attempt >= max {
                        tracing::error!(max, "Max retries exceeded");
                        break;
                    }
                }

                let delay = reconnect_backoff(attempt, reconnect_max_delay);
                match &e {
                    Error::Crypto(_) => tracing::error!(error = %e, "Crypto error"),
                    Error::Timeout(_) => tracing::error!(error = %e, "Connection timeout"),
                    _ => tracing::error!(error = %e, "Connection lost"),
                }
                tracing::warn!(retry = attempt, delay = delay, "Reconnecting");

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
            tracing::error!(error = %e, "Failed to restore route");
        }
    }

    tracing::info!("Shutdown complete");

    Ok(())
}