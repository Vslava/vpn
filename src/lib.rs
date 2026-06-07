pub mod checks;
pub mod client;
pub mod config;
pub mod crypto;
pub mod error;
pub mod handshake;
pub mod nat;
pub mod protocol;
pub mod route;
pub mod server;
pub mod transport;
pub mod tun;

pub async fn wait_for_shutdown() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = async {
            if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                sig.recv().await;
            }
        } => {}
    }
}
