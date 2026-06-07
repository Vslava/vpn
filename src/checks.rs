/// Startup preflight checks — verify OS environment before running.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Client,
    Server,
}

pub struct Check {
    pub name: &'static str,
    pub desc: &'static str,
    pub check_fn: fn() -> Result<(), String>,
}

impl Check {
    pub const fn new(name: &'static str, desc: &'static str, check_fn: fn() -> Result<(), String>) -> Self {
        Check { name, desc, check_fn }
    }
}

/// Run all checks applicable to the given mode.
/// Returns a list of failed checks: (check_name, error_message).
pub fn run_preflight_checks(mode: Mode) -> Vec<(String, String)> {
    let checks = match mode {
        Mode::Client => client_checks(),
        Mode::Server => server_checks(),
    };
    run_checks(&checks)
}

/// Run a custom list of checks (useful for testing with mocks).
pub fn run_checks(checks: &[Check]) -> Vec<(String, String)> {
    let mut failures = Vec::new();
    for check in checks {
        match (check.check_fn)() {
            Ok(()) => {}
            Err(err) => failures.push((check.name.to_string(), err)),
        }
    }
    failures
}

pub fn common_checks() -> Vec<Check> {
    vec![
        Check::new("root", "running as root", check_root),
        Check::new("tun", "/dev/net/tun exists", check_tun_device),
    ]
}

pub fn client_checks() -> Vec<Check> {
    let mut checks = common_checks();
    checks.push(Check::new("default_route", "default route exists", check_default_route));
    checks
}

pub fn server_checks() -> Vec<Check> {
    let mut checks = common_checks();
    checks.push(Check::new("iptables", "iptables is available", check_iptables));
    checks.push(Check::new("ip_forward", "IP forwarding is enabled", check_ip_forward));
    checks
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_root() -> Result<(), String> {
    let uid = std::fs::read_to_string("/proc/self/status")
        .map_err(|e| format!("cannot read /proc/self/status: {e}"))?
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();

    if uid == "0" {
        Ok(())
    } else {
        Err(format!(
            "application must be run as root (current UID: {uid}). Use: sudo traffic-sentinel"
        ))
    }
}

fn check_tun_device() -> Result<(), String> {
    let path = std::path::Path::new("/dev/net/tun");
    if !path.exists() {
        return Err(
            "/dev/net/tun does not exist.\n  → mkdir -p /dev/net && mknod /dev/net/tun c 10 200 && chmod 666 /dev/net/tun"
                .to_string(),
        );
    }
    // Try opening to verify permissions
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("cannot open /dev/net/tun: {e}\n  → chmod 666 /dev/net/tun"))?;
    Ok(())
}

fn check_iptables() -> Result<(), String> {
    let output = std::process::Command::new("iptables")
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to execute iptables: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "iptables returned an error: {stderr}\n  → Install iptables: apt-get install iptables (or equivalent)"
        ))
    }
}

fn check_ip_forward() -> Result<(), String> {
    let content = std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .map_err(|e| format!("cannot read ip_forward sysctl: {e}"))?;

    if content.trim() == "1" {
        Ok(())
    } else {
        Err(format!(
            "IP forwarding is disabled (={}). The server cannot route traffic.\n  → echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward",
            content.trim()
        ))
    }
}

fn check_default_route() -> Result<(), String> {
    let content = std::fs::read_to_string("/proc/net/route")
        .map_err(|e| format!("cannot read routing table: {e}"))?;

    for line in content.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() >= 2 && fields[1] == "00000000" {
            return Ok(());
        }
    }
    Err("no default route found.\n  → The client needs a default route to save and restore.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_check() -> Result<(), String> { Ok(()) }
    fn fail_check() -> Result<(), String> { Err("error".to_string()) }

    #[test]
    fn test_run_checks_empty() {
        let failures = run_checks(&[]);
        assert!(failures.is_empty());
    }

    #[test]
    fn test_run_checks_collects_all() {
        let checks = vec![
            Check::new("ok", "", ok_check),
            Check::new("fail1", "", fail_check),
            Check::new("fail2", "", fail_check),
        ];
        let failures = run_checks(&checks);
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].0, "fail1");
        assert_eq!(failures[1].0, "fail2");
    }

    #[test]
    fn test_run_checks_no_fail_fast() {
        // Even with failures, all checks must execute (no early return)
        let checks = vec![
            Check::new("a", "", fail_check),
            Check::new("b", "", fail_check),
            Check::new("c", "", ok_check),
        ];
        let failures = run_checks(&checks);
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].0, "a");
        assert_eq!(failures[1].0, "b");
    }

    #[test]
    fn test_client_checks_includes_default_route() {
        let checks = client_checks();
        let names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        assert!(names.contains(&"default_route"), "{names:?}");
        assert!(!names.contains(&"iptables"));
        assert!(!names.contains(&"ip_forward"));
    }

    #[test]
    fn test_server_checks_includes_iptables_and_ip_forward() {
        let checks = server_checks();
        let names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        assert!(names.contains(&"iptables"), "{names:?}");
        assert!(names.contains(&"ip_forward"), "{names:?}");
        assert!(!names.contains(&"default_route"));
    }

    #[test]
    fn test_common_checks_includes_root_and_tun() {
        let checks = common_checks();
        let names: Vec<&str> = checks.iter().map(|c| c.name).collect();
        assert!(names.contains(&"root"), "{names:?}");
        assert!(names.contains(&"tun"), "{names:?}");
    }

    #[test]
    fn test_root_check_fails_without_sudo() {
        let result = check_root();
        let uid = std::process::Command::new("id").arg("-u").output().ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "?".to_string());
        if uid == "0" {
            assert!(result.is_ok(), "root check should pass when UID=0");
        } else {
            assert!(result.is_err(), "root check should fail when UID={uid}");
            let err = result.unwrap_err();
            assert!(err.contains("root"), "error should mention 'root': {err}");
        }
    }
}
