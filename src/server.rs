use std::sync::Arc;

use crate::crypto::Crypto;
use crate::error::Error;
use crate::protocol;
use crate::transport;

pub async fn run_server(
    addr: std::net::SocketAddr,
    crypto: Arc<Crypto>,
) -> Result<(), Error> {
    let listener = transport::listen(addr).await.map_err(Error::Io)?;
    tracing::info!("server: listening on {}", addr);

    let (stream, peer) = listener.accept().await.map_err(Error::Io)?;
    tracing::info!("server: client connected from {}", peer);

    handle_stream(stream, crypto).await
}

pub async fn handle_stream(
    mut stream: tokio::net::TcpStream,
    crypto: Arc<Crypto>,
) -> Result<(), Error> {
    loop {
        let frame_data = transport::read_frame(&mut stream).await.map_err(Error::Io)?;
        let frame = protocol::decode(&frame_data)?;

        let plaintext = crypto.decrypt(&frame.nonce, &frame.payload)?;

        let version = plaintext.first().map(|b| b >> 4).unwrap_or(0);
        let ip_protocol = if version == 4 && plaintext.len() > 9 {
            plaintext[9]
        } else if version == 6 && plaintext.len() > 6 {
            plaintext[6]
        } else {
            0
        };
        let proto_name = match ip_protocol {
            1 => "ICMP",
            6 => "TCP",
            17 => "UDP",
            _ => "unknown",
        };
        tracing::debug!(
            "Decrypted packet: IP version={}, protocol={} ({}), len={}",
            version,
            ip_protocol,
            proto_name,
            plaintext.len()
        );
    }
}
