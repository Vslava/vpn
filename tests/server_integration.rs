use std::net::Ipv4Addr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use traffic_sentinel::crypto::Crypto;
use traffic_sentinel::protocol::{self, Frame};
use traffic_sentinel::server;
use traffic_sentinel::tun;

/// Tests the server's full bidirectional pipeline over a real TUN interface:
/// client sends encrypted frame → server decrypts → writes to TUN →
/// server reads from TUN → re-encrypts → sends back to client.
///
/// Requires root (sudo).
#[tokio::test]
#[ignore = "requires root (sudo) for TUN creation"]
async fn test_server_forwarding_pipeline() {
    let psk = [0xABu8; 32];
    let payload = b"Hello, VPN forwarder!";

    let tun = tun::create_tun("ts_test", 1400, Ipv4Addr::new(10, 0, 0, 1), 30)
        .await
        .expect("create TUN failed (requires sudo)");
    let tun = Arc::new(tun);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failed");
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.expect("accept failed");
        eprintln!("server: accepted client from {}", peer);
        server::handle_client(stream, &psk, tun).await
    });

    let mut client = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect failed");
    client.set_nodelay(true).unwrap();
    eprintln!("client: connected to server");

    let crypto = Crypto::new(&psk);
    let nonce = Crypto::generate_nonce();
    let ciphertext = crypto
        .encrypt(&nonce, payload)
        .expect("encrypt failed");
    let frame = Frame {
        nonce,
        seq: 0,
        flags: 0x00,
        payload: ciphertext,
    };
    let encoded = protocol::encode(&frame);
    client.write_all(&encoded).await.expect("write frame failed");
    eprintln!("client: sent encrypted frame ({} bytes)", encoded.len());

    let mut len_buf = [0u8; 2];
    tokio::time::timeout(std::time::Duration::from_secs(5), client.read_exact(&mut len_buf))
        .await
        .expect("timeout waiting for response")
        .expect("read length failed");
    let frame_len = u16::from_be_bytes(len_buf) as usize;
    let mut resp = vec![0u8; 2 + frame_len];
    resp[..2].copy_from_slice(&len_buf);
    client
        .read_exact(&mut resp[2..])
        .await
        .expect("read frame body failed");
    eprintln!(
        "client: received response frame ({} bytes)",
        resp.len()
    );

    let resp_frame = protocol::decode(&resp).expect("decode response failed");
    let decrypted = crypto
        .decrypt(&resp_frame.nonce, &resp_frame.payload)
        .expect("decrypt response failed");

    assert_eq!(
        decrypted, payload,
        "decrypted response does not match original payload"
    );
    eprintln!("client: decrypted response matches original payload");

    server_handle.abort();
}
