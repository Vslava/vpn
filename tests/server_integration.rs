use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

use tokio_util::sync::CancellationToken;
use traffic_sentinel::crypto::Crypto;
use traffic_sentinel::protocol::{self, Frame};
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

    let server_socket = Arc::new(
        traffic_sentinel::transport::udp_bind(
            "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        )
        .await
        .expect("bind failed"),
    );
    let addr = server_socket.local_addr().unwrap();

    // Run server handshake
    let (session_key, client_addr) =
        traffic_sentinel::handshake::server_handshake(&server_socket, &psk)
            .await
            .expect("server handshake failed");
    eprintln!("server: handshake complete with {}", client_addr);

    let crypto = Arc::new(Crypto::new(&session_key));
    let seq = Arc::new(AtomicU32::new(0));
    let cancel = CancellationToken::new();

    // Spawn handle_client in background
    let cancel_clone = cancel.clone();
    let server_handle = tokio::spawn(async move {
        traffic_sentinel::server::handle_client(
            server_socket,
            tun,
            client_addr,
            crypto,
            seq,
            cancel_clone,
        )
        .await
    });

    // Client connects via UDP and does handshake
    let client = traffic_sentinel::transport::udp_connect(addr)
        .await
        .expect("client connect failed");
    let client_key =
        traffic_sentinel::handshake::client_handshake(&client, &psk)
            .await
            .expect("client handshake failed");
    assert_eq!(client_key, session_key);
    eprintln!("client: handshake complete");

    let client_crypto = Crypto::new(&client_key);
    let nonce = Crypto::generate_nonce();
    let ciphertext = client_crypto
        .encrypt(&nonce, payload)
        .expect("encrypt failed");
    let frame = Frame {
        nonce,
        seq: 0,
        flags: 0x00,
        payload: ciphertext,
    };
    let encoded = protocol::encode(&frame);
    client.send(&encoded).await.expect("client send failed");
    eprintln!(
        "client: sent encrypted frame ({} bytes)",
        encoded.len()
    );

    // Wait for echo response (server reads from TUN, re-encrypts, sends back)
    let mut recv_buf = vec![0u8; 2000];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), client.recv(&mut recv_buf))
        .await
        .expect("timeout waiting for response")
        .expect("client recv failed");
    eprintln!("client: received response ({} bytes)", n);

    let resp_frame =
        protocol::decode(&recv_buf[..n]).expect("decode response failed");
    let decrypted = client_crypto
        .decrypt(&resp_frame.nonce, &resp_frame.payload)
        .expect("decrypt response failed");

    assert_eq!(
        decrypted, payload,
        "decrypted response does not match original payload"
    );
    eprintln!("client: decrypted response matches original payload");

    cancel.cancel();
    let _ = server_handle.await;
}
