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
    pub tun_netmask: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub listen: String,
    pub tun_ip: Option<String>,
    pub tun_netmask: Option<u8>,
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
        let cfg: Self = toml::from_str(&content)
            .map_err(|e| crate::error::Error::Config(format!("failed to parse config: {}", e)))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), crate::error::Error> {
        validate_psk(&self.tunnel.psk)?;
        if let Some(mtu) = self.tunnel.mtu {
            validate_mtu(mtu)?;
        }
        Ok(())
    }

    pub fn validate_for_mode(&self, mode: &Mode) -> Result<(), crate::error::Error> {
        match mode {
            Mode::Client => {
                let client = self.client.as_ref().ok_or_else(|| {
                    crate::error::Error::Config(
                        "client mode requires [client] section in config".to_string(),
                    )
                })?;
                validate_socket_addr(&client.remote)?;
                if let Some(ref ip) = client.tun_ip {
                    validate_ip(ip)?;
                }
                if let Some(mask) = client.tun_netmask {
                    validate_netmask(mask)?;
                }
            }
            Mode::Server => {
                let server = self.server.as_ref().ok_or_else(|| {
                    crate::error::Error::Config(
                        "server mode requires [server] section in config".to_string(),
                    )
                })?;
                validate_socket_addr(&server.listen)?;
                if let Some(ref ip) = server.tun_ip {
                    validate_ip(ip)?;
                }
                if let Some(mask) = server.tun_netmask {
                    validate_netmask(mask)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_psk(psk: &str) -> Result<(), crate::error::Error> {
    if psk.len() != 64 {
        return Err(crate::error::Error::Config(format!(
            "PSK must be 64 hex chars (32 bytes), got {}",
            psk.len()
        )));
    }
    hex::decode(psk).map_err(|e| {
        crate::error::Error::Config(format!("invalid PSK hex: {}", e))
    })?;
    Ok(())
}

fn validate_mtu(mtu: u16) -> Result<(), crate::error::Error> {
    if mtu < 576 {
        return Err(crate::error::Error::Config(format!(
            "MTU must be >= 576, got {}",
            mtu
        )));
    }
    Ok(())
}

fn validate_socket_addr(addr: &str) -> Result<(), crate::error::Error> {
    addr.parse::<std::net::SocketAddr>().map_err(|e| {
        crate::error::Error::Config(format!("invalid socket address '{}': {}", addr, e))
    })?;
    Ok(())
}

fn validate_ip(ip: &str) -> Result<(), crate::error::Error> {
    ip.parse::<std::net::Ipv4Addr>().map_err(|e| {
        crate::error::Error::Config(format!("invalid IP address '{}': {}", ip, e))
    })?;
    Ok(())
}

fn validate_netmask(mask: u8) -> Result<(), crate::error::Error> {
    if mask == 0 || mask > 32 {
        return Err(crate::error::Error::Config(format!(
            "netmask must be 1-32, got {}",
            mask
        )));
    }
    Ok(())
}

pub fn netmask_from_prefix(bits: u8) -> std::net::Ipv4Addr {
    let mask = if bits == 0 { 0u32 } else { !0u32 << (32 - bits as u32) };
    std::net::Ipv4Addr::from(mask.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_psk_ok() {
        assert!(validate_psk(
            "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
        )
        .is_ok());
    }

    #[test]
    fn test_validate_psk_short() {
        let err = validate_psk("abcd").unwrap_err();
        assert!(err.to_string().contains("64 hex chars"));
    }

    #[test]
    fn test_validate_psk_invalid_hex() {
        let err = validate_psk(
            "zzzzbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe",
        )
        .unwrap_err();
        assert!(err.to_string().contains("invalid PSK hex"));
    }

    #[test]
    fn test_validate_psk_uppercase() {
        assert!(validate_psk(
            "DEADBEEFCAFEBABEDEADBEEFCAFEBABEDEADBEEFCAFEBABEDEADBEEFCAFEBABE"
        )
        .is_ok());
    }

    #[test]
    fn test_validate_psk_with_0x_prefix() {
        let err = validate_psk(
            "0xdeadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe",
        )
        .unwrap_err();
        assert!(err.to_string().contains("64 hex chars"));
    }

    #[test]
    fn test_validate_mtu_min() {
        assert!(validate_mtu(576).is_ok());
    }

    #[test]
    fn test_validate_mtu_below_min() {
        let err = validate_mtu(0).unwrap_err();
        assert!(err.to_string().contains(">= 576"));
    }

    #[test]
    fn test_validate_mtu_ok() {
        assert!(validate_mtu(1400).is_ok());
        assert!(validate_mtu(1500).is_ok());
        assert!(validate_mtu(9000).is_ok());
    }

    #[test]
    fn test_validate_netmask_ok() {
        assert!(validate_netmask(30).is_ok());
        assert!(validate_netmask(32).is_ok());
        assert!(validate_netmask(1).is_ok());
    }

    #[test]
    fn test_validate_netmask_zero() {
        let err = validate_netmask(0).unwrap_err();
        assert!(err.to_string().contains("1-32"));
    }

    #[test]
    fn test_validate_netmask_too_high() {
        let err = validate_netmask(33).unwrap_err();
        assert!(err.to_string().contains("1-32"));
    }

    #[test]
    fn test_validate_socket_addr_ok() {
        assert!(validate_socket_addr("0.0.0.0:8443").is_ok());
    }

    #[test]
    fn test_validate_socket_addr_no_port() {
        let err = validate_socket_addr("1.2.3.4").unwrap_err();
        assert!(err.to_string().contains("invalid socket address"));
    }

    #[test]
    fn test_validate_socket_addr_bad_port() {
        let err = validate_socket_addr("1.2.3.4:99999").unwrap_err();
        assert!(err.to_string().contains("invalid socket address"));
    }

    #[test]
    fn test_validate_socket_addr_bad_ip() {
        let err = validate_socket_addr("not-an-ip:8443").unwrap_err();
        assert!(err.to_string().contains("invalid socket address"));
    }

    #[test]
    fn test_validate_ip_ok() {
        assert!(validate_ip("10.0.0.1").is_ok());
    }

    #[test]
    fn test_validate_ip_bad() {
        let err = validate_ip("not-an-ip").unwrap_err();
        assert!(err.to_string().contains("invalid IP address"));
    }

    #[test]
    fn test_validate_for_mode_client_requires_client_section() {
        let cfg = Config {
            tunnel: TunnelConfig {
                psk: "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
                    .to_string(),
                mtu: None,
            },
            client: None,
            server: None,
        };
        let err = cfg.validate_for_mode(&Mode::Client).unwrap_err();
        assert!(err.to_string().contains("[client] section"));
    }

    #[test]
    fn test_validate_for_mode_server_requires_server_section() {
        let cfg = Config {
            tunnel: TunnelConfig {
                psk: "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
                    .to_string(),
                mtu: None,
            },
            client: None,
            server: None,
        };
        let err = cfg.validate_for_mode(&Mode::Server).unwrap_err();
        assert!(err.to_string().contains("[server] section"));
    }

    #[test]
    fn test_validate_for_mode_client_ok() {
        let cfg = Config {
            tunnel: TunnelConfig {
                psk: "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
                    .to_string(),
                mtu: Some(1400),
            },
            client: Some(ClientConfig {
                remote: "1.2.3.4:8443".to_string(),
                tun_ip: Some("10.0.0.2".to_string()),
                tun_netmask: Some(30),
            }),
            server: None,
        };
        assert!(cfg.validate_for_mode(&Mode::Client).is_ok());
    }

    #[test]
    fn test_validate_for_mode_server_ok() {
        let cfg = Config {
            tunnel: TunnelConfig {
                psk: "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
                    .to_string(),
                mtu: Some(1400),
            },
            client: None,
            server: Some(ServerConfig {
                listen: "0.0.0.0:8443".to_string(),
                tun_ip: Some("10.0.0.1".to_string()),
                tun_netmask: Some(30),
            }),
        };
        assert!(cfg.validate_for_mode(&Mode::Server).is_ok());
    }

    #[test]
    fn test_validate_for_mode_both_sections_ok() {
        let cfg = Config {
            tunnel: TunnelConfig {
                psk: "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
                    .to_string(),
                mtu: None,
            },
            client: Some(ClientConfig {
                remote: "1.2.3.4:8443".to_string(),
                tun_ip: None,
                tun_netmask: None,
            }),
            server: Some(ServerConfig {
                listen: "0.0.0.0:8443".to_string(),
                tun_ip: None,
                tun_netmask: None,
            }),
        };
        assert!(cfg.validate_for_mode(&Mode::Client).is_ok());
        assert!(cfg.validate_for_mode(&Mode::Server).is_ok());
    }

    #[test]
    fn test_netmask_from_prefix_30() {
        assert_eq!(
            netmask_from_prefix(30),
            std::net::Ipv4Addr::new(255, 255, 255, 252)
        );
    }

    #[test]
    fn test_netmask_from_prefix_24() {
        assert_eq!(
            netmask_from_prefix(24),
            std::net::Ipv4Addr::new(255, 255, 255, 0)
        );
    }

    #[test]
    fn test_netmask_from_prefix_32() {
        assert_eq!(
            netmask_from_prefix(32),
            std::net::Ipv4Addr::new(255, 255, 255, 255)
        );
    }

    #[test]
    fn test_netmask_from_prefix_0() {
        assert_eq!(
            netmask_from_prefix(0),
            std::net::Ipv4Addr::new(0, 0, 0, 0)
        );
    }
}
