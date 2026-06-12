use std::io;
use std::net::SocketAddr;

use tokio::net::UdpSocket;

pub async fn udp_bind(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(addr).await?;
    Ok(socket)
}

pub async fn udp_connect(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(addr).await?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pair() -> (UdpSocket, UdpSocket, SocketAddr) {
        let server = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = udp_connect(server_addr).await.unwrap();
        (client, server, server_addr)
    }

    #[tokio::test]
    async fn test_udp_bind() {
        let socket = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        assert!(socket.local_addr().is_ok());
    }

    #[tokio::test]
    async fn test_udp_connect() {
        let server = udp_bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let server_addr = server.local_addr().unwrap();
        let client = udp_connect(server_addr).await.unwrap();
        assert!(client.local_addr().is_ok());
    }

    #[tokio::test]
    async fn test_send_recv() {
        let (client, server, _server_addr) = setup_pair().await;

        client.send(b"hello").await.unwrap();
        let mut buf = [0u8; 16];
        let (n, _) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[tokio::test]
    async fn test_recv_from_returns_peer() {
        let (client, server, _server_addr) = setup_pair().await;

        client.send(b"ping").await.unwrap();
        let mut buf = [0u8; 16];
        let (n, peer) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"ping");
        assert!(peer.port() > 0);
    }

    #[tokio::test]
    async fn test_bidirectional() {
        let (client, server, _server_addr) = setup_pair().await;

        client.send(b"from client").await.unwrap();
        let mut buf = [0u8; 16];
        let (n, client_addr) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"from client");

        server.send_to(b"from server", client_addr).await.unwrap();
        let mut rbuf = [0u8; 16];
        let n = client.recv(&mut rbuf).await.unwrap();
        assert_eq!(&rbuf[..n], b"from server");
    }

    #[tokio::test]
    async fn test_large_datagram() {
        let (client, server, _server_addr) = setup_pair().await;

        let payload = vec![0xABu8; 1400];
        client.send(&payload).await.unwrap();
        let mut buf = vec![0u8; 2000];
        let (n, _) = server.recv_from(&mut buf).await.unwrap();
        assert_eq!(n, 1400);
        assert_eq!(&buf[..n], &payload[..]);
    }

    #[tokio::test]
    async fn test_connection_refused() {
        let result = udp_connect("127.0.0.1:1".parse().unwrap()).await;
        assert!(result.is_ok());
    }


}
