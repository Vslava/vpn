use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub tunnel: TunnelConfig,
    pub client: Option<ClientConfig>,
    pub server: Option<ServerConfig>,
}

#[derive(Debug, Deserialize)]
pub struct TunnelConfig {
    pub psk: String,
    pub mtu: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct ClientConfig {
    pub remote: String,
    pub tun_ip: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    pub tun_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Client,
    Server,
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "client" => Ok(Mode::Client),
            "server" => Ok(Mode::Server),
            _ => Err(format!("invalid mode '{}' — use 'client' or 'server'", s)),
        }
    }
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self, crate::error::Error> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::error::Error::Config(format!("failed to read config: {}", e)))?;
        toml::from_str(&content)
            .map_err(|e| crate::error::Error::Config(format!("failed to parse config: {}", e)))
    }
}
