/// Integration tests for preflight checks.
///
/// These tests verify runtime behavior.
/// Check composition is tested via unit tests in `src/checks.rs`.

use traffic_sentinel::checks;

/// Without root, at last the "root" check should fail.
#[test]
fn test_root_fails_without_sudo() {
    let failures = checks::run_preflight_checks(checks::Mode::Client);

    assert!(!failures.is_empty());
    let names: Vec<&str> = failures.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"root"), "root should be in failures: {names:?}");

    for (name, msg) in &failures {
        assert!(!msg.is_empty(), "check '{name}' has empty error message");
    }
}

/// All checks should pass when running as root with a properly configured system.
#[test]
#[ignore = "requires root (sudo) and full system config"]
fn test_all_checks_pass_with_sudo() {
    let failures = checks::run_preflight_checks(checks::Mode::Server);
    assert!(failures.is_empty(), "all checks should pass with sudo: {failures:?}");
}
