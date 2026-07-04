# Handoff: P2.2 — Reconnection Docker Verification

## Summary

Сессия посвящена запуску и отладке Docker-тестов для P2.2 (reconnection). Код P2.2 был реализован ранее. В этой сессии: написаны 4 интеграционных теста, пройдены все основные баги тестового раннера, получен финальный результат 12/12 passed.

## What Was Done

### Docker verification script (`tests/docker_reconnect.sh`)

- **Test 1 — Basic reconnect**: kill server (`docker exec pkill -TERM`), restart container (`docker start`), verify client reconnects and traffic resumes
- **Test 2 — Exponential backoff + max retries**: kill server, block port with iptables REJECT, wait for all retries, verify backoff delays (1,2,4,8,16), verifymax retries exceeded triggers graceful shutdown
- **Test 3 — Server accept loop**: connect client 1, kill container (`docker kill`), verify server logs "waiting for new connection", connect client 2, verify traffic
- **Test 4 — SIGTERM during reconnect**: kill server while client connected, verify reconnect starts, kill client with SIGTERM, verify graceful shutdown

### Infrastructure

- `tests/Dockerfile.e2e` — added `procps` package (needed for `pkill`)
- Custom Docker bridge network with `--ip 172.30.0.10` for server — survives container stop/start
- Entrypoint shell script for containers (TUN creation, iptables MASQUERADE)

## Issues Found (and fixed)

### `set -e` abort on pkill exit code
`pkill -f traffic-sentinel` kills the calling shell (because the shell's cmdline contains "traffic-sentinel"). pkill returns 143 (128+15), which with `set -e` aborts the script. **Fix**: removed `set -e` from the test script; use `|| true` everywhere.

### `eval "$@"` loses quoting for multi-word arguments
`step()` function used `eval "$@" > /dev/null 2>&1 || rc=$?`. Arguments like `"waiting for new connection"` have their quotes stripped before `eval` receives them. The 4-word pattern becomes 4 separate arguments — `$3` gets `"on"` instead of the timeout value.  **Fix**: `eval "$(printf '%q ' "$@")"` — `printf '%q'` properly escapes each argument.

### `pkill -x` cannot match `traffic-sentinel` (16 chars)
Linux truncates `/proc/PID/comm` to 15 characters. `pkill -x traffic-sentinel` never matches because the kernel stores `traffic-sentine` (15 chars). **Workaround**: always use `pkill -f` (matches full command line).

### Server doesn't detect client death without active traffic
`handle_client` uses `select!` on two spawned I/O tasks. With no traffic, both tasks block on I/O (waiting for TCP or TUN data). No error propagates, so the server never leaves `handle_client`. **Fix**: start a background ping before killing the client container so the server gets a BrokenPipe error on the next write.

### `docker exec -d` vs `docker exec &` for background ping
Using `docker exec ts-client ping ... &` (background) sometimes keeps the PID alive on the host after the container dies. **Fix**: use `docker exec -d` (detached mode) — the process runs entirely in the container.

## Current State

- **12/12 Docker tests pass** (all 4 scenarios)
- `tests/docker_reconnect.sh` — ready to run
- `specs/VERIFICATION_P2.md` — P2.2 marked `[x]`
- P2.3+ not started

## Next Steps

- P2.3: Heartbeat/keepalive — TCP_KEEPALIVE, PING/PONG, timeout→reconnect
- P2.4: Logging audit — structured fields, levels, all required events

## Relevant Files

- `tests/docker_reconnect.sh` — 4 integration tests for reconnect (12 checks)
- `tests/Dockerfile.e2e` — Docker image with procps for pkill
- `specs/VERIFICATION_P2.md` — P2.2 checklist marked passed
- `src/client.rs` — `run_client_session()`, `reconnect_backoff()`
- `src/server.rs` — `run_server()` with accept loop
