use crate::error::Error;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use x25519_dalek::{EphemeralSecret, PublicKey};

type HmacSha256 = Hmac<Sha256>;

const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

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

async fn send_handshake_msg(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    psk: &[u8; 32],
    secret: &EphemeralSecret,
) -> Result<[u8; 32], Error> {
    let public = PublicKey::from(secret);
    let pub_bytes = public.to_bytes();
    let hmac = hmac_sha256(psk, &pub_bytes)?;

    let mut msg = [0u8; 64];
    msg[..32].copy_from_slice(&pub_bytes);
    msg[32..].copy_from_slice(&hmac);

    stream.write_all(&msg).await?;
    stream.flush().await?;
    Ok(pub_bytes)
}

async fn recv_and_verify_msg(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    psk: &[u8; 32],
) -> Result<[u8; 32], Error> {
    let mut msg = [0u8; 64];
    tokio::time::timeout(
        std::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        stream.read_exact(&mut msg),
    )
    .await
    .map_err(|_| Error::Handshake("handshake timeout".into()))??;

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

pub async fn client_handshake(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    psk: &[u8; 32],
) -> Result<[u8; 32], Error> {
    let client_secret = EphemeralSecret::random_from_rng(OsRng);
    let client_pub_bytes = send_handshake_msg(stream, psk, &client_secret).await?;

    let server_pub_bytes = recv_and_verify_msg(stream, psk).await?;

    let server_pub = PublicKey::from(server_pub_bytes);
    let shared_secret = client_secret.diffie_hellman(&server_pub);
    let shared_bytes = shared_secret.to_bytes();

    Ok(derive_session_key(
        psk,
        &shared_bytes,
        &client_pub_bytes,
        &server_pub_bytes,
    ))
}

pub async fn server_handshake(
    stream: &mut (impl AsyncReadExt + AsyncWriteExt + Unpin),
    psk: &[u8; 32],
) -> Result<[u8; 32], Error> {
    let client_pub_bytes = recv_and_verify_msg(stream, psk).await?;

    let server_secret = EphemeralSecret::random_from_rng(OsRng);
    let server_pub_bytes = send_handshake_msg(stream, psk, &server_secret).await?;

    let client_pub = PublicKey::from(client_pub_bytes);
    let shared_secret = server_secret.diffie_hellman(&client_pub);
    let shared_bytes = shared_secret.to_bytes();

    Ok(derive_session_key(
        psk,
        &shared_bytes,
        &client_pub_bytes,
        &server_pub_bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;

    fn random_psk() -> [u8; 32] {
        let mut psk = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut psk);
        psk
    }

    #[tokio::test]
    async fn test_client_server_handshake_matching_keys() {
        let psk = random_psk();
        let (mut client, mut server) = tokio::io::duplex(4096);

        let (client_key, server_key) = tokio::join!(
            client_handshake(&mut client, &psk),
            server_handshake(&mut server, &psk),
        );

        let client_key = client_key.unwrap();
        let server_key = server_key.unwrap();
        assert_eq!(client_key, server_key);
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
        let (mut client, mut server) = tokio::io::duplex(4096);

        let (client_result, server_result) = tokio::join!(
            client_handshake(&mut client, &client_psk),
            server_handshake(&mut server, &server_psk),
        );

        assert!(client_result.is_err());
        assert!(server_result.is_err());
    }

    #[tokio::test]
    async fn test_perfect_forward_secrecy() {
        let psk = random_psk();
        let mut keys = std::collections::HashSet::new();

        for _ in 0..100 {
            let (mut client, mut server) = tokio::io::duplex(4096);

            let (client_key, server_key) = tokio::join!(
                client_handshake(&mut client, &psk),
                server_handshake(&mut server, &psk),
            );

            let key = client_key.unwrap();
            assert_eq!(key, server_key.unwrap());
            assert!(keys.insert(key), "duplicate session key (no PFS)");
        }
    }

    #[tokio::test]
    async fn test_psk_all_zeros() {
        let psk = [0u8; 32];
        let (mut client, mut server) = tokio::io::duplex(4096);

        let (client_key, server_key) = tokio::join!(
            client_handshake(&mut client, &psk),
            server_handshake(&mut server, &psk),
        );

        let client_key = client_key.unwrap();
        let server_key = server_key.unwrap();
        assert_eq!(client_key, server_key);
    }

    #[tokio::test]
    async fn test_psk_all_ff() {
        let psk = [0xFFu8; 32];
        let (mut client, mut server) = tokio::io::duplex(4096);

        let (client_key, server_key) = tokio::join!(
            client_handshake(&mut client, &psk),
            server_handshake(&mut server, &psk),
        );

        let client_key = client_key.unwrap();
        let server_key = server_key.unwrap();
        assert_eq!(client_key, server_key);
    }

    #[tokio::test]
    async fn test_tampered_hmac_server_hello() {
        let psk = random_psk();
        let (mut client_end, mut proxy_end_c) = tokio::io::duplex(4096);
        let (mut proxy_end_s, mut server_end) = tokio::io::duplex(4096);

        let client_handle = tokio::spawn(async move {
            client_handshake(&mut client_end, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf_c = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_c, &mut buf_c).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_s, &buf_c).await.unwrap();

            let mut buf_s = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_s, &mut buf_s).await.unwrap();
            buf_s[63] ^= 0xFF;
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_c, &buf_s).await.unwrap();
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&mut server_end, &psk).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(server_result.unwrap().is_ok());
        assert!(client_result.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_tampered_hmac_client_hello() {
        let psk = random_psk();
        let (mut client_end, mut proxy_end_c) = tokio::io::duplex(4096);
        let (mut proxy_end_s, mut server_end) = tokio::io::duplex(4096);

        let client_handle = tokio::spawn(async move {
            client_handshake(&mut client_end, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf_c = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_c, &mut buf_c).await.unwrap();
            buf_c[63] ^= 0xFF;
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_s, &buf_c).await.unwrap();

            let mut buf_s = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_s, &mut buf_s).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_c, &buf_s).await.unwrap();
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&mut server_end, &psk).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(client_result.unwrap().is_err());
        assert!(server_result.unwrap().is_err());
    }

    #[tokio::test]
    async fn test_tampered_public_key() {
        let psk = random_psk();
        let (mut client_end, mut proxy_end_c) = tokio::io::duplex(4096);
        let (mut proxy_end_s, mut server_end) = tokio::io::duplex(4096);

        let client_handle = tokio::spawn(async move {
            client_handshake(&mut client_end, &psk).await
        });

        let proxy_handle = tokio::spawn(async move {
            let mut buf_c = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_c, &mut buf_c).await.unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_s, &buf_c).await.unwrap();

            let mut buf_s = [0u8; 64];
            tokio::io::AsyncReadExt::read_exact(&mut proxy_end_s, &mut buf_s).await.unwrap();
            buf_s[0] ^= 1;
            tokio::io::AsyncWriteExt::write_all(&mut proxy_end_c, &buf_s).await.unwrap();
        });

        let server_handle = tokio::spawn(async move {
            server_handshake(&mut server_end, &psk).await
        });

        let (client_result, server_result, _) = tokio::join!(client_handle, server_handle, proxy_handle);

        assert!(server_result.unwrap().is_ok());
        assert!(client_result.unwrap().is_err());
    }
}
