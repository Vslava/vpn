use std::io;
use std::net::SocketAddr;

use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    let stream = TcpStream::connect(addr).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

pub async fn listen(addr: SocketAddr) -> io::Result<TcpListener> {
    TcpListener::bind(addr).await
}

pub async fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let frame_len = u16::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; 2 + frame_len];
    buf[..2].copy_from_slice(&len_buf);
    stream.read_exact(&mut buf[2..]).await?;

    Ok(buf)
}

pub async fn write_frame(stream: &mut TcpStream, data: &[u8]) -> io::Result<()> {
    stream.write_all(data).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener as TokioListener;

    async fn setup_pair() -> (TcpStream, TcpStream) {
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();

        (client, server)
    }

    #[tokio::test]
    async fn test_connect_listen() {
        let (_client, _server) = setup_pair().await;
    }

    #[tokio::test]
    async fn test_write_read_frame() {
        let (mut client, mut server) = setup_pair().await;
        let payload_len = 1000u16;
        let mut frame = Vec::with_capacity(2 + payload_len as usize);
        frame.extend_from_slice(&payload_len.to_be_bytes());
        frame.resize(2 + payload_len as usize, 0xAB);

        write_frame(&mut server, &frame).await.unwrap();
        let received = read_frame(&mut client).await.unwrap();

        assert_eq!(frame, received);
    }

    #[tokio::test]
    async fn test_frame_boundaries() {
        let (mut client, mut server) = setup_pair().await;

        let len1 = 24 + 4 + 1 + 100;
        let len2 = 24 + 4 + 1 + 200;

        let mut frame1 = vec![0u8; 2 + len1];
        frame1[..2].copy_from_slice(&(len1 as u16).to_be_bytes());
        frame1[2..].fill(0x01);

        let mut frame2 = vec![0u8; 2 + len2];
        frame2[..2].copy_from_slice(&(len2 as u16).to_be_bytes());
        frame2[2..].fill(0x02);

        write_frame(&mut server, &frame1).await.unwrap();
        write_frame(&mut server, &frame2).await.unwrap();

        let received1 = read_frame(&mut client).await.unwrap();
        let received2 = read_frame(&mut client).await.unwrap();

        assert_eq!(frame1, received1);
        assert_eq!(frame2, received2);
    }

    #[tokio::test]
    async fn test_nodelay() {
        let (client, _server) = setup_pair().await;
        assert!(client.nodelay().unwrap());
    }

    #[tokio::test]
    async fn test_connection_refused() {
        let result = connect("127.0.0.1:1".parse().unwrap()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_large_burst() {
        let (mut client, mut server) = setup_pair().await;

        for i in 0..100 {
            let len = 24 + 4 + 1 + 65506;
            let mut frame = vec![0u8; 2 + len];
            frame[..2].copy_from_slice(&(len as u16).to_be_bytes());
            frame[2] = i;

            write_frame(&mut server, &frame).await.unwrap();
            let received = read_frame(&mut client).await.unwrap();
            assert_eq!(received[..2], (len as u16).to_be_bytes());
            assert_eq!(received[2], i);
        }
    }
}
