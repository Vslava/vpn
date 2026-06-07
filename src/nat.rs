use crate::error::Error;

/// Sets up iptables MASQUERADE for the given interface.
///
/// Checks if the rule already exists (idempotent). If `iptables` is not found,
/// logs a warning and returns `Ok(())` — NAT is considered optional.
pub async fn setup_nat(iface: &str) -> Result<(), Error> {
    let iptables = match find_iptables().await {
        Some(path) => path,
        None => {
            tracing::warn!("iptables not found, skipping NAT setup. \
                Return traffic from the internet may not reach the tunnel.");
            return Ok(());
        }
    };

    let check = tokio::process::Command::new(&iptables)
        .args(["-t", "nat", "-C", "POSTROUTING", "-o", iface, "-j", "MASQUERADE"])
        .output()
        .await
        .map_err(Error::Io)?;

    if check.status.success() {
        tracing::info!(iface = %iface, "NAT MASQUERADE rule already exists");
        return Ok(());
    }

    let add = tokio::process::Command::new(&iptables)
        .args(["-t", "nat", "-A", "POSTROUTING", "-o", iface, "-j", "MASQUERADE"])
        .output()
        .await
        .map_err(Error::Io)?;

    if !add.status.success() {
        let stderr = String::from_utf8_lossy(&add.stderr);
        tracing::warn!(iface = %iface, error = %stderr, "Failed to add NAT MASQUERADE rule");
        return Ok(());
    }

    tracing::info!(iface = %iface, "NAT MASQUERADE rule added");
    Ok(())
}

/// Removes the iptables MASQUERADE rule for the given interface.
///
/// If the rule does not exist or `iptables` is not found, logs and returns `Ok(())`.
pub async fn cleanup_nat(iface: &str) -> Result<(), Error> {
    let iptables = match find_iptables().await {
        Some(path) => path,
        None => {
            tracing::debug!("iptables not found, skipping NAT cleanup");
            return Ok(());
        }
    };

    let del = tokio::process::Command::new(&iptables)
        .args(["-t", "nat", "-D", "POSTROUTING", "-o", iface, "-j", "MASQUERADE"])
        .output()
        .await
        .map_err(Error::Io)?;

    if !del.status.success() {
        let stderr = String::from_utf8_lossy(&del.stderr);
        if stderr.contains("does not match") || stderr.contains("No chain/target/match") {
            tracing::debug!(iface = %iface, "NAT MASQUERADE rule did not exist, nothing to remove");
        } else {
            tracing::warn!(iface = %iface, error = %stderr, "Failed to remove NAT MASQUERADE rule");
        }
        return Ok(());
    }

    tracing::info!(iface = %iface, "NAT MASQUERADE rule removed");
    Ok(())
}

async fn find_iptables() -> Option<String> {
    let candidates = [
        "iptables",
        "/sbin/iptables",
        "/usr/sbin/iptables",
        "/usr/local/sbin/iptables",
    ];
    for cmd in &candidates {
        let ok = tokio::process::Command::new(cmd)
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(cmd.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_iptables_runs_without_panic() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(find_iptables());
    }
}
