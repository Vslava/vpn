use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

use crate::config;
use crate::crypto::Crypto;
use crate::error::Error;
use crate::ip_pool::IpPool;
use crate::protocol::{self, Frame, FLAG_PONG};
use crate::tun::TunDevice;

const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(180);

struct ClientSession {
    crypto: Arc<Crypto>,
    seq: Arc<AtomicU32>,
    client_addr: std::net::SocketAddr,
}

type SessionMap = Arc<Mutex<HashMap<Ipv4Addr, ClientSession>>>;

pub async fn run_server(
    addr: std::net::SocketAddr,
    psk: &[u8; 32],
    mtu: u16,
    tun_subnet: Option<&str>,
) -> Result<(), Error> {
    let subnet_cidr = tun_subnet.unwrap_or(config::DEFAULT_TUN_SUBNET);
    let (tun_ip, netmask) = config::parse_tun_subnet(Some(subnet_cidr))?;
    let tun = Arc::new(crate::tun::create_tun("ts0", mtu, tun_ip, netmask).await?);
    tracing::info!(iface = "ts0", ip = %tun_ip, netmask = netmask, "Created TUN interface");

    let ip_pool = Arc::new(Mutex::new(IpPool::new(subnet_cidr)?));

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

    let sessions: SessionMap = Arc::new(Mutex::new(HashMap::new()));

    let shared_tun = tun.clone();
    let shared_socket = socket.clone();
    let shared_sessions = sessions.clone();
    let shared_cancel = cancel.clone();
    let tun_reader = tokio::spawn(tun_to_clients_loop(
        shared_tun,
        shared_socket,
        shared_sessions,
        shared_cancel,
    ));

    loop {
        if cancel.is_cancelled() {
            break;
        }

        let hs_socket = socket.clone();
        let (session_key, client_addr, client_ip, _netmask) = match crate::handshake::server_handshake(
            &hs_socket, psk, &ip_pool,
        )
        .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(error = %e, "Handshake failed, retrying");
                continue;
            }
        };

        tracing::info!(peer = %client_addr, tun_ip = %client_ip, "Handshake complete");

        let crypto = Arc::new(Crypto::new(&session_key));
        let seq = Arc::new(AtomicU32::new(0));

        {
            let mut map = sessions.lock().unwrap();
            map.insert(
                client_ip,
                ClientSession {
                    crypto: crypto.clone(),
                    seq: seq.clone(),
                    client_addr,
                },
            );
        }

        let client_cancel = cancel.clone();
        let client_tun = tun.clone();
        let client_sessions = sessions.clone();
        let client_ip_pool = ip_pool.clone();
        let spawn_socket = socket.clone();

        tokio::spawn(async move {
            let result = handle_client(
                spawn_socket,
                client_tun,
                client_addr,
                crypto,
                seq,
                client_ip,
                client_cancel.clone(),
            )
            .await;

            {
                let mut map = client_sessions.lock().unwrap();
                map.remove(&client_ip);
            }
            {
                let mut pool = client_ip_pool.lock().unwrap();
                pool.release(client_ip);
            }

            if let Err(ref e) = result {
                tracing::error!(client = %client_ip, error = %e, "Client error");
            }

            tracing::info!(client = %client_ip, "Client disconnected");
        });
    }

    tun_reader.abort();
    let _ = tun_reader.await;

    if let Some(ref iface) = ext_iface {
        let _ = crate::nat::cleanup_nat(iface).await;
    }
    tracing::info!("Deleting TUN");
    drop(tun);
    tracing::info!("Shutdown complete");
    Ok(())
}

async fn tun_to_clients_loop(
    tun: Arc<TunDevice>,
    socket: Arc<UdpSocket>,
    sessions: SessionMap,
    cancel: CancellationToken,
) {
    loop {
        if cancel.is_cancelled() {
            break;
        }

        let mut buf = vec![0u8; tun.mtu() as usize];
        let n = match tun.recv(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("TUN read error: {e}");
                break;
            }
        };
        buf.truncate(n);

        let dest_ip = extract_dest_ipv4(&buf);
        if dest_ip.is_none() {
            continue;
        }
        let dest_ip = dest_ip.unwrap();

        let (crypto, seq, client_addr) = {
            let map = sessions.lock().unwrap();
            match map.get(&dest_ip) {
                Some(session) => (
                    session.crypto.clone(),
                    session.seq.clone(),
                    session.client_addr,
                ),
                None => {
                    tracing::debug!(dest = %dest_ip, "No session for destination IP, dropping");
                    continue;
                }
            }
        };

        match encrypt_and_send(&buf, &crypto, &seq, &socket, client_addr).await {
            Ok(()) => {}
            Err(e) => {
                tracing::error!("Failed to send to client {}: {e}", client_addr);
            }
        }
    }
}

fn extract_dest_ipv4(buf: &[u8]) -> Option<Ipv4Addr> {
    if buf.len() < 20 {
        return None;
    }
    let version_ihl = buf[0];
    if (version_ihl >> 4) != 4 {
        return None;
    }
    let ihl = (version_ihl & 0x0F) as usize;
    if buf.len() < ihl * 4 {
        return None;
    }
    Some(Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]))
}

async fn encrypt_and_send(
    plaintext: &[u8],
    crypto: &Crypto,
    seq: &Arc<AtomicU32>,
    socket: &UdpSocket,
    client_addr: std::net::SocketAddr,
) -> Result<(), Error> {
    let nonce = Crypto::generate_nonce();
    let ciphertext = crypto.encrypt(&nonce, plaintext)?;
    let s = seq.fetch_add(1, Ordering::Relaxed);
    tracing::debug!(seq = s, len = plaintext.len(), "Packet sent");
    let frame = Frame {
        nonce,
        seq: s,
        flags: 0x00,
        payload: ciphertext,
    };
    let encoded = protocol::encode(&frame);
    socket.send_to(&encoded, client_addr).await.map_err(Error::Io)?;
    Ok(())
}

pub async fn handle_client(
    socket: Arc<UdpSocket>,
    tun: Arc<TunDevice>,
    client_addr: std::net::SocketAddr,
    crypto: Arc<Crypto>,
    seq: Arc<AtomicU32>,
    _client_ip: Ipv4Addr,
    cancel: CancellationToken,
) -> Result<(), Error> {
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }

        let mut buf = vec![0u8; 65535];

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
            let s = seq.fetch_add(1, Ordering::Relaxed);
            let pong = Frame {
                nonce,
                seq: s,
                flags: FLAG_PONG,
                payload: vec![],
            };
            socket
                .send_to(&protocol::encode(&pong), client_addr)
                .await
                .map_err(Error::Io)?;
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
}
