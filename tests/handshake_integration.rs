use std::net::SocketAddr;
use rand::RngCore;
use tokio::net::TcpListener;

fn random_psk() -> [u8; 32] {
    let mut psk = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut psk);
    psk
}

#[tokio::test]
async fn test_handshake_over_real_tcp() {
    let psk = random_psk();
    let listener = TcpListener::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        traffic_sentinel::handshake::server_handshake(
            &mut tokio::io::BufStream::new(stream),
            &psk,
        )
        .await
        .unwrap()
    });

    let client_stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    client_stream.set_nodelay(true).unwrap();
    let client_key = traffic_sentinel::handshake::client_handshake(
        &mut tokio::io::BufStream::new(client_stream),
        &psk,
    )
    .await
    .unwrap();

    let server_key = server_handle.await.unwrap();
    assert_eq!(client_key, server_key);
}

#[tokio::test]
async fn test_handshake_tcp_wrong_psk() {
    let client_psk = [0xABu8; 32];
    let server_psk = [0xBAu8; 32];
    let listener = TcpListener::bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        traffic_sentinel::handshake::server_handshake(
            &mut tokio::io::BufStream::new(stream),
            &server_psk,
        )
        .await
    });

    let client_stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    client_stream.set_nodelay(true).unwrap();
    let client_result = traffic_sentinel::handshake::client_handshake(
        &mut tokio::io::BufStream::new(client_stream),
        &client_psk,
    )
    .await;

    let server_result = server_handle.await.unwrap();
    assert!(client_result.is_err());
    assert!(server_result.is_err());
}
