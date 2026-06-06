use std::io::Write;

/// Tests that invalid config files produce appropriate errors.
#[test]
fn test_config_error_handling() {
    // Missing file
    let result = traffic_sentinel::config::Config::from_file("/nonexistent/config.toml");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("failed to read config"), "got: {err}");

    // Invalid TOML content
    let mut f = tempfile::NamedTempFile::new().expect("create temp file");
    write!(f, "invalid toml content {{").ok();
    let result = traffic_sentinel::config::Config::from_file(f.path().to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("failed to parse config"), "got: {err}");

    // Invalid PSK (too short)
    let mut f = tempfile::NamedTempFile::new().expect("create temp file");
    write!(
        f,
        r#"
        [tunnel]
        psk = "deadbeef"

        [client]
        remote = "127.0.0.1:8443"
        "#
    )
    .ok();
    let result = traffic_sentinel::config::Config::from_file(f.path().to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("64 hex chars"), "got: {err}");

    // Invalid MTU (below minimum)
    let mut f = tempfile::NamedTempFile::new().expect("create temp file");
    write!(
        f,
        r#"
        [tunnel]
        psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
        mtu = 100

        [client]
        remote = "127.0.0.1:8443"
        "#
    )
    .ok();
    let result = traffic_sentinel::config::Config::from_file(f.path().to_str().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains(">= 576"), "got: {err}");

    // Missing section for mode
    let mut f = tempfile::NamedTempFile::new().expect("create temp file");
    write!(
        f,
        r#"
        [tunnel]
        psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
        "#
    )
    .ok();
    let cfg = traffic_sentinel::config::Config::from_file(f.path().to_str().unwrap()).unwrap();
    let result = cfg.validate_for_mode(&traffic_sentinel::config::Mode::Client);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("[client] section"), "got: {err}");

    let result = cfg.validate_for_mode(&traffic_sentinel::config::Mode::Server);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("[server] section"), "got: {err}");
}

/// Full end-to-end test: ping through the tunnel.
/// Requires root (sudo) for TUN and route management.
#[tokio::test]
#[ignore = "requires root (sudo) for TUN and route management"]
async fn test_ping_through_tunnel() {
    let psk = [0xABu8; 32];

    // We'll use the full client/server pipeline.
    // This is a placeholder for the actual integration test.

    let _ = psk;
    todo!("implement full e2e ping test");
}

/// Tests client reconnect after server goes down.
/// Requires root (sudo).
#[tokio::test]
#[ignore = "requires root (sudo) for TUN and route management"]
async fn test_reconnect() {
    todo!("implement reconnect test");
}

/// Tests graceful shutdown restores routes and deletes TUN.
/// Requires root (sudo).
#[tokio::test]
#[ignore = "requires root (sudo) for TUN and route management"]
async fn test_graceful_shutdown_restores_routes() {
    todo!("implement graceful shutdown test");
}
