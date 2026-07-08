use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::Duration;

use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use tokio::net::UdpSocket;
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::error::Error;
use crate::ip_pool::IpPool;

type HmacSha256 = Hmac<Sha256>;

const HANDSHAKE_TIMEOUT_SECS: u64 = 10;
const HANDSHAKE_RETRY_INTERVAL: Duration = Duration::from_secs(3);
const HANDSHAKE_MAX_RETRIES: usize = 5;
const HANDSHAKE_MSG_SIZE: usize = 64;
const SERVER_HELLO_SIZE: usize = 69;

fn hmac_sha256(key: &[u8; 32], data: &[u8; 32]) -> Result<[u8; 32], Error> {
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|_| Error::Crypto("HMAC key init failed".into()))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().into())
}

fn derive_session_key(
    psk: &[u8; 32],
    shared_secret: &[u8; 32],
    client_pub: &[u8; 32],
    server_pub: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(psk);
    hasher.update(shared_secret);
    hasher.update(client_pub);
    hasher.update(server_pub);
    hasher.finalize().into()
}

fn build_handshake_msg(psk: &[u8; 32], secret: &EphemeralSecret) -> Result<([u8; 64], [u8; 32]), Error> {
    let public = PublicKey::from(secret);
    let pub_bytes = public.to_bytes();
    let hmac = hmac_sha256(psk, &pub_bytes)?;

    let mut msg = [0u8; 64];
    msg[..32].copy_from_slice(&pub_bytes);
    msg[32..].copy_from_slice(&hmac);

    Ok((msg, pub_bytes))
}

fn parse_and_verify_handshake_msg(msg: &[u8; 64], psk: &[u8; 32]) -> Result<[u8; 32], Error> {
    let mut pub_bytes = [0u8; 32];
    pub_bytes.copy_from_slice(&msg[..32]);

    let mut received_hmac = [0u8; 32];
    received_hmac.copy_from_slice(&msg[32..]);

    let expected_hmac = hmac_sha256(psk, &pub_bytes)?;
    if received_hmac != expected_hmac {
        return Err(Error::Handshake("HMAC mismatch".into()));
    }

    Ok(pub_bytes)
}

pub async fn client_handshake(socket: &UdpSocket, psk: &[u8; 32]) -> Result<([u8; 32], Ipv4Addr, u8), Error> {
    let client_secret = EphemeralSecret::random_from_rng(OsRng);
    let (client_hello, client_pub_bytes) = build_handshake_msg(psk, &client_secret)?;

    let mut recv_buf = [0u8; SERVER_HELLO_SIZE];

    for attempt in 0..HANDSHAKE_MAX_RETRIES {
        socket.send(&client_hello).await?;

        match tokio::time::timeout(HANDSHAKE_RETRY_INTERVAL, socket.recv(&mut recv_buf)).await {
            Ok(Ok(n)) if n == SERVER_HELLO_SIZE => {
                let mut hmac_msg = [0u8; 64];
                hmac_msg.copy_from_slice(&recv_buf[..64]);
                let server_pub_bytes = parse_and_verify_handshake_msg(&hmac_msg, psk)?;

                let server_pub = PublicKey::from(server_pub_bytes);
                let shared_secret = client_secret.diffie_hellman(&server_pub);
                let shared_bytes = shared_secret.to_bytes();

                let session_key = derive_session_key(
                    psk,
                    &shared_bytes,
                    &client_pub_bytes,
                    &server_pub_bytes,
                );

                let client_ip = Ipv4Addr::new(
                    recv_buf[64], recv_buf[65], recv_buf[66], recv_buf[67],
                );
                let netmask = recv_buf[68];

                return Ok((session_key, client_ip, netmask));
            }
            Ok(Ok(_)) => {
                continue;
            }
            Ok(Err(_)) => {
                continue;
            }
            Err(_) => {
                if attempt == HANDSHAKE_MAX_RETRIES - 1 {
                    return Err(Error::Handshake("no response from server after retries".into()));
                }
                continue;
            }
        }
    }

    Err(Error::Handshake("no response from server".into()))
}

pub async fn server_handshake(
    socket: &UdpSocket,
    psk: &[u8; 32],
    ip_pool: &Mutex<IpPool>,
) -> Result<([u8; 32], SocketAddr, Ipv4Addr, u8), Error> {
    loop {
        let mut buf = [0u8; HANDSHAKE_MSG_SIZE];

        let (n, client_addr) = tokio::time::timeout(
            Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
            socket.recv_from(&mut buf),
        )
        .await
        .map_err(|_| Error::Handshake("handshake timeout: no client".into()))??;

        if n != HANDSHAKE_MSG_SIZE {
            continue;
        }

        let client_pub_bytes = match parse_and_verify_handshake_msg(&buf, psk) {
            Ok(pubkey) => pubkey,
            Err(_) => continue,
        };

        let client_ip = {
            let mut pool = ip_pool
                .lock()
                .map_err(|_| Error::Handshake("ip pool lock poisoned".into()))?;
            pool.allocate()
                .ok_or_else(|| Error::Handshake("no available client IPs in pool".into()))?
        };

        let netmask: u8 = {
            let pool = ip_pool
                .lock()
                .map_err(|_| Error::Handshake("ip pool lock poisoned".into()))?;
            pool.netmask()
        };

        let server_secret = EphemeralSecret::random_from_rng(OsRng);
        let server_pub = PublicKey::from(&server_secret);
        let server_pub_bytes = server_pub.to_bytes();

        let server_hmac = hmac_sha256(psk, &server_pub_bytes)?;
        let mut response = [0u8; SERVER_HELLO_SIZE];
        response[..32].copy_from_slice(&server_pub_bytes);
        response[32..64].copy_from_slice(&server_hmac);
        response[64..68].copy_from_slice(&client_ip.octets());
        response[68] = netmask;

        socket.send_to(&response, client_addr).await?;

        let client_pub = PublicKey::from(client_pub_bytes);
        let shared_secret = server_secret.diffie_hellman(&client_pub);
        let shared_bytes = shared_secret.to_bytes();

        let session_key = derive_session_key(
            psk,
            &shared_bytes,
            &client_pub_bytes,
            &server_pub_bytes,
        );

        return Ok((session_key, client_addr, client_ip, netmask));
    }
}

pub async fn server_handshake_dispatch(
    socket: &UdpSocket,
    psk: &[u8; 32],
    ip_pool: &Mutex<IpPool>,
    client_hello: &[u8; 64],
    client_addr: SocketAddr,
) -> Result<([u8; 32], SocketAddr, Ipv4Addr, u8), Error> {
    let client_pub_bytes = parse_and_verify_handshake_msg(client_hello, psk)?;

    let client_ip = {
        let mut pool = ip_pool.lock().map_err(|_| Error::Handshake("ip pool lock poisoned".into()))?;
        pool.allocate().ok_or_else(|| Error::Handshake("no available client IPs in pool".into()))?
    };

    let netmask = {
        let pool = ip_pool.lock().map_err(|_| Error::Handshake("ip pool lock poisoned".into()))?;
        pool.netmask()
    };

    let server_secret = EphemeralSecret::random_from_rng(OsRng);
    let server_pub = PublicKey::from(&server_secret);
    let server_pub_bytes = server_pub.to_bytes();

    let server_hmac = hmac_sha256(psk, &server_pub_bytes)?;
    let mut response = [0u8; SERVER_HELLO_SIZE];
    response[..32].copy_from_slice(&server_pub_bytes);
    response[32..64].copy_from_slice(&server_hmac);
    response[64..68].copy_from_slice(&client_ip.octets());
    response[68] = netmask;

    socket.send_to(&response, client_addr).await?;

    let client_pub = PublicKey::from(client_pub_bytes);
    let shared_secret = server_secret.diffie_hellman(&client_pub);
    let shared_bytes = shared_secret.to_bytes();

    let session_key = derive_session_key(psk, &shared_bytes, &client_pub_bytes, &server_pub_bytes);

    Ok((session_key, client_addr, client_ip, netmask))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{udp_bind, udp_connect};
    use rand::RngCore;

    fn random_psk() -> [u8; 32] {
        let mut psk = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut psk);
        psk
    }

    fn setup_pool() -> Mutex<IpPool> {
        Mutex::new(IpPool::new("10.0.0.0/24").unwrap())
    }

    async fn setup_udp_pair() -> (UdpSocket, UdpSocket, SocketAddr) {
        let server = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = udp_connect(server_addr).await.unwrap();
        (client, server, server_addr)
    }

    #[tokio::test]
    async fn test_client_server_handshake_matching_keys() {
        let psk = random_psk();
        let (client, server, _) = setup_udp_pair().await;
        let pool = setup_pool();

        let (client_result, server_result) = tokio::join!(
            client_handshake(&client, &psk),
            server_handshake(&server, &psk, &pool),
        );

        let (client_key, client_ip, _nm) = client_result.unwrap();
        let (server_key, _client_addr, server_ip, _nm) = server_result.unwrap();
        assert_eq!(client_key, server_key);
        assert_eq!(client_ip, server_ip);
        assert_eq!(client_ip, Ipv4Addr::new(10, 0, 0, 2));
    }

    #[test]
    fn test_hmac_sha256_deterministic() {
        let key = [0xABu8; 32];
        let data = [0xBAu8; 32];
        let h1 = hmac_sha256(&key, &data).unwrap();
        let h2 = hmac_sha256(&key, &data).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hmac_sha256_different_key() {
        let key1 = [0xABu8; 32];
        let key2 = [0xBAu8; 32];
        let data = [0u8; 32];
        let h1 = hmac_sha256(&key1, &data).unwrap();
        let h2 = hmac_sha256(&key2, &data).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hmac_sha256_different_data() {
        let key = [0xABu8; 32];
        let data1 = [0u8; 32];
        let data2 = [1u8; 32];
        let h1 = hmac_sha256(&key, &data1).unwrap();
        let h2 = hmac_sha256(&key, &data2).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_derive_session_key_deterministic() {
        let psk = [0xABu8; 32];
        let shared = [0x01u8; 32];
        let client_pub = [0x02u8; 32];
        let server_pub = [0x03u8; 32];
        let k1 = derive_session_key(&psk, &shared, &client_pub, &server_pub);
        let k2 = derive_session_key(&psk, &shared, &client_pub, &server_pub);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_session_key_different_inputs() {
        let psk = random_psk();
        let shared = [0x01u8; 32];
        let client_pub = [0x02u8; 32];
        let server_pub = [0x03u8; 32];

        let base = derive_session_key(&psk, &shared, &client_pub, &server_pub);

        let mut diff_client = client_pub;
        diff_client[0] ^= 1;
        let key2 = derive_session_key(&psk, &shared, &diff_client, &server_pub);
        assert_ne!(base, key2);

        let mut diff_server = server_pub;
        diff_server[0] ^= 1;
        let key3 = derive_session_key(&psk, &shared, &client_pub, &diff_server);
        assert_ne!(base, key3);

        let mut diff_shared = shared;
        diff_shared[0] ^= 1;
        let key4 = derive_session_key(&psk, &diff_shared, &client_pub, &server_pub);
        assert_ne!(base, key4);

        let mut diff_psk = psk;
        diff_psk[0] ^= 1;
        let key5 = derive_session_key(&diff_psk, &shared, &client_pub, &server_pub);
        assert_ne!(base, key5);
    }

    #[tokio::test]
    async fn test_client_server_wrong_psk() {
        let client_psk = [0xABu8; 32];
        let server_psk = [0xBAu8; 32];
        let (client, server, _) = setup_udp_pair().await;
        let pool = setup_pool();

        let (client_result, server_result) = tokio::join!(
            client_handshake(&client, &client_psk),
            server_handshake(&server, &server_psk, &pool),
        );

        assert!(client_result.is_err());
        assert!(server_result.is_err());
    }

    #[tokio::test]
    async fn test_perfect_forward_secrecy() {
        let psk = random_psk();
        let mut keys = std::collections::HashSet::new();

        for _ in 0..20 {
            let (client, server, _) = setup_udp_pair().await;
            let pool = setup_pool();

            let (client_result, server_result) = tokio::join!(
                client_handshake(&client, &psk),
                server_handshake(&server, &psk, &pool),
            );

            let (key, _, _) = client_result.unwrap();
            let (server_key, _, _, _) = server_result.unwrap();
            assert_eq!(key, server_key);
            assert!(keys.insert(key), "duplicate session key (no PFS)");
        }
    }

    #[tokio::test]
    async fn test_psk_all_zeros() {
        let psk = [0u8; 32];
        let (client, server, _) = setup_udp_pair().await;
        let pool = setup_pool();

        let (client_result, server_result) = tokio::join!(
            client_handshake(&client, &psk),
            server_handshake(&server, &psk, &pool),
        );

        let (client_key, _, _) = client_result.unwrap();
        let (server_key, _, _, _) = server_result.unwrap();
        assert_eq!(client_key, server_key);
    }

    #[tokio::test]
    async fn test_psk_all_ff() {
        let psk = [0xFFu8; 32];
        let (client, server, _) = setup_udp_pair().await;
        let pool = setup_pool();

        let (client_result, server_result) = tokio::join!(
            client_handshake(&client, &psk),
            server_handshake(&server, &psk, &pool),
        );

        let (client_key, _, _) = client_result.unwrap();
        let (server_key, _, _, _) = server_result.unwrap();
        assert_eq!(client_key, server_key);
    }

    /// Helper: set up proxy between client and server for tampered tests.
    /// Returns (client_socket, proxy_to_client, proxy_to_server, server_socket)
    async fn setup_proxy_pair() -> (UdpSocket, UdpSocket, UdpSocket, UdpSocket) {
        let server = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server.local_addr().unwrap();

        let proxy_to_client = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let proxy_client_addr = proxy_to_client.local_addr().unwrap();

        let proxy_to_server = udp_connect(server_addr).await.unwrap();

        let client = udp_connect(proxy_client_addr).await.unwrap();

        (client, proxy_to_client, proxy_to_server, server)
    }

    #[tokio::test]
    async fn test_tampered_server_hello() {
        let psk = random_psk();
        let (client, proxy_to_client, proxy_to_server, server) = setup_proxy_pair().await;
        let pool = setup_pool();

        let client_handle = tokio::spawn(async move {
            client_handshake(&client, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, client_addr) = proxy_to_client.recv_from(&mut buf).await.unwrap();
            assert_eq!(n, 64);

            proxy_to_server.send(&buf[..n]).await.unwrap();

            let mut srv_buf = [0u8; SERVER_HELLO_SIZE];
            let n = proxy_to_server.recv(&mut srv_buf).await.unwrap();
            assert_eq!(n, SERVER_HELLO_SIZE);
            srv_buf[63] ^= 0xFF;
            proxy_to_client.send_to(&srv_buf[..n], client_addr).await.unwrap();
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&server, &psk, &pool).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(server_result.unwrap().is_ok());
        assert!(client_result.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_tampered_client_hello() {
        let psk = random_psk();
        let (client, proxy_to_client, proxy_to_server, server) = setup_proxy_pair().await;
        let pool = setup_pool();

        let client_handle = tokio::spawn(async move {
            client_handshake(&client, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, _client_addr) = proxy_to_client.recv_from(&mut buf).await.unwrap();
            assert_eq!(n, 64);
            buf[63] ^= 0xFF;

            proxy_to_server.send(&buf[..n]).await.unwrap();

            let mut srv_buf = [0u8; SERVER_HELLO_SIZE];
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                proxy_to_server.recv(&mut srv_buf),
            )
            .await;
            assert!(result.is_err(), "server should not respond to tampered hello");
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&server, &psk, &pool).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(client_result.unwrap().is_err());
        assert!(server_result.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_tampered_public_key() {
        let psk = random_psk();
        let (client, proxy_to_client, proxy_to_server, server) = setup_proxy_pair().await;
        let pool = setup_pool();

        let client_handle = tokio::spawn(async move {
            client_handshake(&client, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let (n, _client_addr) = proxy_to_client.recv_from(&mut buf).await.unwrap();
            assert_eq!(n, 64);

            proxy_to_server.send(&buf[..n]).await.unwrap();

            let mut srv_buf = [0u8; SERVER_HELLO_SIZE];
            let n = proxy_to_server.recv(&mut srv_buf).await.unwrap();
            assert_eq!(n, SERVER_HELLO_SIZE);
            srv_buf[0] ^= 1;
            proxy_to_client.send_to(&srv_buf[..n], _client_addr).await.unwrap();
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&server, &psk, &pool).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(server_result.unwrap().is_ok());
        assert!(client_result.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_server_handshake_timeout() {
        let server = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let psk = random_psk();
        let pool = setup_pool();
        let result = server_handshake(&server, &psk, &pool).await;
        assert!(result.is_err());
    }
}
