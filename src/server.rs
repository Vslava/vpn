use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;
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

    let ext_iface = crate::route::save_default_route().await.ok().flatten().map(|r| r.ifname);
    if let Some(ref iface) = ext_iface {
        crate::nat::setup_nat(iface).await?;
    } else {
        tracing::warn!("No default route found; skipping NAT setup.");
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
    let tx_map: Arc<Mutex<HashMap<std::net::SocketAddr, mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(HashMap::new()));

    let tun_reader = tokio::spawn(tun_to_clients_loop(
        tun.clone(), socket.clone(), sessions.clone(), cancel.clone(),
    ));

    loop {
        if cancel.is_cancelled() { break; }

        let mut buf = vec![0u8; 65535];
        let (n, peer) = tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = socket.recv_from(&mut buf) => result.map_err(Error::Io)?,
        };

        let known_ip = sessions.lock().unwrap()
            .iter().find(|(_, s)| s.client_addr == peer).map(|(ip, _)| *ip);

        if let Some(_ip) = known_ip {
            let sender = {
                tx_map.lock().unwrap().get(&peer).cloned()
            };
            if let Some(tx) = sender {
                let _ = tx.send(buf[..n].to_vec()).await;
            }
            continue;
        }

        if n == 64 {
            let mut hs_buf = [0u8; 64];
            hs_buf.copy_from_slice(&buf[..64]);

            let result = crate::handshake::server_handshake_dispatch(&socket, psk, &ip_pool, &hs_buf, peer).await;
            let (session_key, client_addr, client_ip, _netmask) = match result {
                Ok(r) => r,
                Err(e) => { tracing::debug!(error = %e, "Handshake failed"); continue; }
            };

            tracing::info!(peer = %client_addr, tun_ip = %client_ip, "Handshake complete");

            let crypto = Arc::new(Crypto::new(&session_key));
            let seq = Arc::new(AtomicU32::new(0));
            let (tx, rx) = mpsc::channel::<Vec<u8>>(256);

            sessions.lock().unwrap().insert(client_ip, ClientSession {
                crypto: crypto.clone(), seq: seq.clone(), client_addr,
            });
            tx_map.lock().unwrap().insert(client_addr, tx);

            let cl_tun = tun.clone();
            let cl_socket = socket.clone();
            let cl_crypto = crypto.clone();
            let cl_seq = seq.clone();
            let cl_cancel = cancel.clone();
            let cl_ip_pool = ip_pool.clone();
            let cl_sessions = sessions.clone();
            let cl_tx_map = tx_map.clone();
            let cl_ip = client_ip;
            let cl_peer = client_addr;

            tokio::spawn(async move {
                let r = handle_client(cl_tun, cl_socket, cl_peer, cl_crypto, cl_seq, rx, cl_cancel.clone()).await;
                cl_sessions.lock().unwrap().remove(&cl_ip);
                cl_tx_map.lock().unwrap().remove(&cl_peer);
                cl_ip_pool.lock().unwrap().release(cl_ip);
                if let Err(ref e) = r { tracing::error!(client = %cl_ip, error = %e, "Client error"); }
                tracing::info!(client = %cl_ip, "Client disconnected");
            });
        }
    }

    tun_reader.abort();
    let _ = tun_reader.await;

    if let Some(ref iface) = ext_iface {
        let _ = crate::nat::cleanup_nat(iface).await;
    }
    drop(tun);
    tracing::info!("Shutdown complete");
    Ok(())
}

async fn handle_client(
    tun: Arc<TunDevice>,
    socket: Arc<UdpSocket>,
    client_addr: std::net::SocketAddr,
    crypto: Arc<Crypto>,
    seq: Arc<AtomicU32>,
    mut rx: mpsc::Receiver<Vec<u8>>,
    cancel: CancellationToken,
) -> Result<(), Error> {
    loop {
        if cancel.is_cancelled() { return Ok(()); }

        let buf = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            result = tokio::time::timeout(CLIENT_IDLE_TIMEOUT, rx.recv()) => match result {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(()),
                Err(_) => return Err(Error::Timeout("client idle timeout".into())),
            },
        };

        let frame = protocol::decode(&buf)?;

        if frame.is_ping() {
            let nonce = Crypto::generate_nonce();
            let s = seq.fetch_add(1, Ordering::Relaxed);
            let pong = Frame { nonce, seq: s, flags: FLAG_PONG, payload: vec![] };
            socket.send_to(&protocol::encode(&pong), client_addr).await.map_err(Error::Io)?;
            continue;
        }

        if frame.is_pong() { continue; }

        let plaintext = match crypto.decrypt(&frame.nonce, &frame.payload) {
            Ok(p) => p,
            Err(e) => {
                if buf.len() == 64 {
                    tracing::debug!("64-byte failed decrypt — possible new client, ending session");
                    return Ok(());
                }
                tracing::debug!("Decryption failed: {e}");
                continue;
            }
        };

        tun.send(&plaintext).await.map_err(Error::Io)?;
    }
}

async fn tun_to_clients_loop(
    tun: Arc<TunDevice>,
    socket: Arc<UdpSocket>,
    sessions: SessionMap,
    cancel: CancellationToken,
) {
    loop {
        if cancel.is_cancelled() { break; }

        let mut buf = vec![0u8; tun.mtu() as usize];
        let n = match tun.recv(&mut buf).await {
            Ok(n) => n,
            Err(e) => { tracing::error!("TUN read: {e}"); break; }
        };
        buf.truncate(n);

        let dest_ip = match extract_dest_ipv4(&buf) {
            Some(ip) => ip,
            None => continue,
        };

        let (crypto, seq, client_addr) = {
            let map = sessions.lock().unwrap();
            match map.get(&dest_ip) {
                Some(s) => (s.crypto.clone(), s.seq.clone(), s.client_addr),
                None => { continue; }
            }
        };

        let nonce = Crypto::generate_nonce();
        if let Ok(ciphertext) = crypto.encrypt(&nonce, &buf) {
            let s = seq.fetch_add(1, Ordering::Relaxed);
            let frame = Frame { nonce, seq: s, flags: 0x00, payload: ciphertext };
            let encoded = protocol::encode(&frame);
            let _ = socket.send_to(&encoded, client_addr).await;
        }
    }
}

fn extract_dest_ipv4(buf: &[u8]) -> Option<Ipv4Addr> {
    if buf.len() < 20 { return None; }
    if (buf[0] >> 4) != 4 { return None; }
    let ihl = (buf[0] & 0x0F) as usize;
    if buf.len() < ihl * 4 { return None; }
    Some(Ipv4Addr::new(buf[16], buf[17], buf[18], buf[19]))
}
