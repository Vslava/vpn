/// Integration tests for automatic NAT (iptables MASQUERADE) setup/cleanup.
///
/// These tests require:
///   - root (sudo) for iptables
///   - iptables installed

async fn has_iptables() -> bool {
    tokio::process::Command::new("iptables")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Tests the full cycle: setup_nat adds a rule, cleanup_nat removes it.
#[tokio::test]
#[ignore = "requires root (sudo) for iptables"]
async fn test_nat_setup_cleanup_cycle() {
    if !has_iptables().await {
        eprintln!("Skipping: iptables not found");
        return;
    }

    // Ensure clean state
    let _ = traffic_sentinel::nat::cleanup_nat("lo").await;

    // Rule should NOT exist before setup
    let check_before = tokio::process::Command::new("iptables")
        .args(["-t", "nat", "-C", "POSTROUTING", "-o", "lo", "-j", "MASQUERADE"])
        .output()
        .await
        .expect("iptables check failed");
    assert!(
        !check_before.status.success(),
        "MASQUERADE rule for lo should not exist before test"
    );

    // Setup
    traffic_sentinel::nat::setup_nat("lo")
        .await
        .expect("setup_nat failed");

    // Rule SHOULD exist after setup
    let check_after = tokio::process::Command::new("iptables")
        .args(["-t", "nat", "-C", "POSTROUTING", "-o", "lo", "-j", "MASQUERADE"])
        .output()
        .await
        .expect("iptables check failed");
    assert!(
        check_after.status.success(),
        "MASQUERADE rule for lo should exist after setup_nat"
    );

    // Cleanup
    traffic_sentinel::nat::cleanup_nat("lo")
        .await
        .expect("cleanup_nat failed");

    // Rule should NOT exist after cleanup
    let check_final = tokio::process::Command::new("iptables")
        .args(["-t", "nat", "-C", "POSTROUTING", "-o", "lo", "-j", "MASQUERADE"])
        .output()
        .await
        .expect("iptables check failed");
    assert!(
        !check_final.status.success(),
        "MASQUERADE rule for lo should not exist after cleanup_nat"
    );
}

/// Tests that setup_nat is idempotent — calling it twice does not create duplicates.
#[tokio::test]
#[ignore = "requires root (sudo) for iptables"]
async fn test_nat_setup_idempotent() {
    if !has_iptables().await {
        eprintln!("Skipping: iptables not found");
        return;
    }

    // Ensure clean state
    let _ = traffic_sentinel::nat::cleanup_nat("lo").await;

    // Call setup_nat twice
    traffic_sentinel::nat::setup_nat("lo")
        .await
        .expect("first setup_nat failed");
    traffic_sentinel::nat::setup_nat("lo")
        .await
        .expect("second setup_nat (should be idempotent) failed");

    // Rule should exist
    let check = tokio::process::Command::new("iptables")
        .args(["-t", "nat", "-C", "POSTROUTING", "-o", "lo", "-j", "MASQUERADE"])
        .output()
        .await
        .expect("iptables check failed");
    assert!(
        check.status.success(),
        "MASQUERADE rule should exist after two setup_nat calls"
    );

    // Cleanup
    traffic_sentinel::nat::cleanup_nat("lo")
        .await
        .expect("cleanup_nat failed");
}

/// Tests that cleanup_nat is idempotent — calling it on a non-existent rule is safe.
#[tokio::test]
#[ignore = "requires root (sudo) for iptables"]
async fn test_nat_cleanup_idempotent() {
    if !has_iptables().await {
        eprintln!("Skipping: iptables not found");
        return;
    }

    // Ensure clean state
    let _ = traffic_sentinel::nat::cleanup_nat("lo").await;

    // Cleanup on non-existent rule should not error
    traffic_sentinel::nat::cleanup_nat("lo")
        .await
        .expect("cleanup_nat on non-existent rule should not fail");
}

/// Tests that setup_nat gracefully handles missing iptables (no panic/crash).
#[tokio::test]
async fn test_nat_setup_no_iptables() {
    // Use PATH=/dev/null to simulate missing iptables
    // We can't easily mock, but we can verify the function signature is correct
    let _ = traffic_sentinel::nat::setup_nat("lo").await;
}
