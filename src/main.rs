use std::net::SocketAddr;
use clap::Parser;
use traffic_sentinel::{client, config, server};

#[derive(Parser)]
#[command(name = "traffic-sentinel", about = "Encrypted VPN tunnel")]
struct Cli {
    #[arg(long)]
    mode: Option<String>,

    #[arg(long)]
    config: Option<String>,
}

fn setup_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

fn parse_psk(hex: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex).map_err(|e| format!("invalid PSK hex: {}", e))?;
    if bytes.len() != 32 {
        return Err(format!("PSK must be 32 bytes (64 hex chars), got {}", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[tokio::main]
async fn main() {
    setup_logging();
    let cli = Cli::parse();

    let mode_str = cli.mode.as_deref().unwrap_or_else(|| {
        eprintln!("error: --mode is required (client|server)");
        std::process::exit(1);
    });

    let mode: config::Mode = mode_str.parse().unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        std::process::exit(1);
    });

    let config_path = cli.config.as_deref().unwrap_or_else(|| {
        eprintln!("error: --config is required");
        std::process::exit(1);
    });

    let cfg = config::Config::from_file(config_path).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        std::process::exit(1)
    });

    cfg.validate_for_mode(&mode).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        std::process::exit(1);
    });

    let psk = parse_psk(&cfg.tunnel.psk).unwrap_or_else(|e| {
        eprintln!("error: {}", e);
        std::process::exit(1);
    });

    match mode {
        config::Mode::Client => run_client_mode(&cfg, psk).await,
        config::Mode::Server => run_server_mode(&cfg, psk).await,
    }
}

async fn run_client_mode(cfg: &config::Config, psk: [u8; 32]) {
    let client_cfg = cfg.client.as_ref().unwrap();

    let remote: SocketAddr = client_cfg.remote.parse().unwrap_or_else(|e| {
        eprintln!("error: invalid remote address '{}': {}", client_cfg.remote, e);
        std::process::exit(1);
    });

    let tun_ip_str = client_cfg.tun_ip.as_deref().unwrap_or("10.0.0.2");
    let tun_ip: std::net::Ipv4Addr = tun_ip_str.parse().unwrap_or_else(|e| {
        eprintln!("error: invalid TUN IP '{}': {}", tun_ip_str, e);
        std::process::exit(1);
    });
    let netmask = client_cfg.tun_netmask.unwrap_or(30);

    let gw_str = client_cfg.gateway.as_deref().unwrap_or("10.0.0.1");
    let gateway: std::net::Ipv4Addr = gw_str.parse().unwrap_or_else(|e| {
        eprintln!("error: invalid gateway '{}': {}", gw_str, e);
        std::process::exit(1);
    });

    let mtu = cfg.tunnel.mtu.unwrap_or(1400);

    if let Err(e) = client::run_client_full(remote, &psk, tun_ip, netmask, gateway, mtu).await {
        tracing::error!("client error: {}", e);
    }
}

async fn run_server_mode(cfg: &config::Config, psk: [u8; 32]) {
    let server_cfg = cfg.server.as_ref().unwrap();

    let listen: SocketAddr = server_cfg.listen.parse().unwrap_or_else(|e| {
        eprintln!("error: invalid listen address '{}': {}", server_cfg.listen, e);
        std::process::exit(1);
    });

    let tun_ip_str = server_cfg.tun_ip.as_deref().unwrap_or("10.0.0.1");
    let tun_ip: std::net::Ipv4Addr = tun_ip_str.parse().unwrap_or_else(|e| {
        eprintln!("error: invalid TUN IP '{}': {}", tun_ip_str, e);
        std::process::exit(1);
    });
    let netmask = server_cfg.tun_netmask.unwrap_or(30);

    let mtu = cfg.tunnel.mtu.unwrap_or(1400);

    if let Err(e) = server::run_server(listen, &psk, tun_ip, mtu, netmask).await {
        tracing::error!("server error: {}", e);
    }
}
