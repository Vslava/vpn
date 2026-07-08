use std::net::SocketAddr;
use std::sync::Mutex;

use rand::RngCore;
use traffic_sentinel::ip_pool::IpPool;
use traffic_sentinel::transport::{udp_bind, udp_connect};

fn random_psk() -> [u8; 32] {
    let mut psk = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut psk);
    psk
}

fn setup_pool() -> Mutex<IpPool> {
    Mutex::new(IpPool::new("10.0.0.0/24").unwrap())
}

#[tokio::test]
async fn test_handshake_over_real_udp() {
    let psk = random_psk();
    let server = udp_bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let pool = setup_pool();

    let server_handle = tokio::spawn(async move {
        let (key, _client_addr, _ip, _nm) =
            traffic_sentinel::handshake::server_handshake(&server, &psk, &pool)
                .await
                .unwrap();
        key
    });

    let client = udp_connect(addr).await.unwrap();
    let (client_key, _ip, _nm) = traffic_sentinel::handshake::client_handshake(&client, &psk)
        .await
        .unwrap();

    let server_key = server_handle.await.unwrap();
    assert_eq!(client_key, server_key);
}

#[tokio::test]
async fn test_handshake_udp_wrong_psk() {
    let client_psk = [0xABu8; 32];
    let server_psk = [0xBAu8; 32];
    let server = udp_bind("127.0.0.1:0".parse::<SocketAddr>().unwrap())
        .await
        .unwrap();
    let addr = server.local_addr().unwrap();
    let pool = setup_pool();

    let server_handle = tokio::spawn(async move {
        let result = traffic_sentinel::handshake::server_handshake(&server, &server_psk, &pool).await;
        result
    });

    let client = udp_connect(addr).await.unwrap();
    let client_result =
        traffic_sentinel::handshake::client_handshake(&client, &client_psk).await;

    let server_result = server_handle.await.unwrap();
    assert!(client_result.is_err());
    assert!(server_result.is_err());
}
