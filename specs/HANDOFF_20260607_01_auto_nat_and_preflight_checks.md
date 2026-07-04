# Handoff: Auto NAT (MASQUERADE) + Preflight Checks

## Summary

Two TDD cycles completed. First: server now automatically configures iptables MASQUERADE on startup and cleans it up on shutdown (fixes missing return traffic). Second: both client and server run preflight OS checks before starting — if the environment is misconfigured, the app reports all issues and exits with code 1.

## What Was Done

### Auto NAT (MASQUERADE) — commit `9ee6f6b`

- **`src/nat.rs`** (new): `setup_nat(iface)` / `cleanup_nat(iface)` — adds/removes `iptables -t nat -A POSTROUTING -o <iface> -j MASQUERADE`
  - Idempotent: checks with `iptables -C` before adding
  - Graceful: if `iptables` not in PATH, logs warning and returns `Ok(())`
  - `find_iptables()` searches PATH + common `/sbin`, `/usr/sbin` locations
- **`src/server.rs`**: `run_server` detects external interface via `route::save_default_route()` and calls `setup_nat()` at startup, `cleanup_nat()` at shutdown
- **`tests/nat_integration.rs`** (new): 4 tests — full cycle, idempotent setup, idempotent cleanup, graceful without iptables
- **`tests/docker_e2e.sh`**, `docker_heartbeat.sh`, `docker_reconnect.sh`: removed manual `iptables MASQUERADE` (now automatic)
- **`docs/SETUP_LINUX.md`**: fixed false claim that "NAT настраивается автоматически" — now accurately describes the automatic behavior
- **`specs/THOUGHTS.md`**: recorded Q21 decision
- **`README.md`**: added "Автоматическая настройка NAT" to feature list

### Preflight Checks — commit `27268b3`

- **`src/checks.rs`** (new): `run_preflight_checks(mode)` runs all applicable checks, collects all failures (no fail-fast)

  | Check | Client | Server | Method |
  |-------|--------|--------|--------|
  | root (UID=0) | ✅ | ✅ | `/proc/self/status` → `Uid:` line |
  | `/dev/net/tun` exists+r/w | ✅ | ✅ | `path.exists()` + `OpenOptions` |
  | `iptables` in PATH | — | ✅ | `iptables --version` |
  | `ip_forward == 1` | — | ✅ | `/proc/sys/net/ipv4/ip_forward` |
  | default route exists | ✅ | — | `/proc/net/route` → `00000000` dest |

- **`src/main.rs`**: runs checks after config validation, before mode dispatch. On failure: prints all errors to stderr, exits with code 1
- **`tests/checks_integration.rs`** (new): integration tests — root fails without sudo, all-clear with sudo (ignored)
- **`src/checks.rs`** `#[cfg(test)]`: 7 unit tests — check composition, failure collection, no-fail-fast, root check UID-aware
- **`specs/THOUGHTS.md`**: recorded Q22 decision
- **`DEVELOPMENT.md`**: updated test commands

## What Worked

- TDD workflow smooth for both features — clear red→green transitions
- The `nat::setup_nat` uses `iptables -C` (check) before `-A` (add), making it safe to call on every restart without accumulating duplicate rules
- Checks module uses `/proc` filesystem for root/route/ip_forward checks — no extra dependencies, no subprocesses
- The `run_checks(&[Check])` public function accepts a slice of `Check` structs, making unit tests trivial (inject mock pass/fail functions)
- CI-relevant tests (require sudo) are `#[ignore]` — run manually when needed

## Current State

- **Tests**: 68 lib tests pass, all integration tests pass (ignored ones require sudo)
- **Clippy**: clean
- **Both commits** are on `master`, no uncommitted changes
- The app now correctly sets up MASQUERADE on the server so return traffic from the internet reaches the client

## Next Steps

- Run the full Docker e2e test to confirm the NAT change works end-to-end (especially after removing manual `iptables` from test scripts): `bash tests/docker_e2e.sh`
- Verify the preflight checks under sudo: `sudo cargo test --test checks_integration -- --ignored --nocapture`

## Relevant Files

| Area | Files |
|------|-------|
| NAT module | `src/nat.rs`, `tests/nat_integration.rs` |
| Preflight checks | `src/checks.rs`, `tests/checks_integration.rs` |
| Server wiring | `src/server.rs` (lines 22-33, 70-72) |
| CLI wiring | `src/main.rs` (lines 67-81) |
| Docs | `docs/SETUP_LINUX.md`, `specs/THOUGHTS.md` (Q21, Q22) |
| Docker tests (updated) | `tests/docker_e2e.sh`, `tests/docker_heartbeat.sh`, `tests/docker_reconnect.sh` |
