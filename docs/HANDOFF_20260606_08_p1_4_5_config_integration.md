# Handoff: P1.4 + P1.5 — Config Validation and Full Integration

## Summary

Завершены P1.4 (full config loading with validation) и P1.5 (wire everything — handshake, routes, bootstrap). Добавлен Docker end-to-end тест, который проверяет 5 протоколов через туннель. Весь P1 (Minimal Viable) готов.

## What Was Done

### P1.4: Full config loading (`src/config.rs`)

- **Валидация всех полей конфига** — отдельные функции `validate()` (PSK, MTU) и `validate_for_mode()` (remote/listen, tun_ip, gateway, tun_netmask, mode-specific секции)
- **`tun_netmask`** — добавлен как `Option<u8>` в `ClientConfig` и `ServerConfig` (раньше был зашит в `tun_ip` как CIDR)
- **`gateway`** — добавлен в `ClientConfig` (для `set_tun_route`), опциональный, default `"10.0.0.1"`
- **`netmask_from_prefix()`** — хелпер для конвертации prefix length → Ipv4Addr
- **Validation rules**:
  - PSK: ровно 64 hex-символа, case-insensitive, без `0x`-префикса
  - MTU: >= 576
  - SocketAddr: IP:port, port in range
  - Netmask: 1-32
  - Client mode требует `[client]` секцию, server mode — `[server]`
- **17 unit-тестов** покрывают все validation cases

### P1.5: Wire into main.rs

**`src/server.rs`**:
- `handle_client()` теперь делает `server_handshake` перед split stream → создаёт `Crypto` с session key (не PSK)

**`src/client.rs`**:
- Новая `run_client_full()` — полный bootstrap клиента:
  1. Создание TUN ts0
  2. Сохранение default route
  3. TCP connect
  4. `client_handshake` → session key → `Crypto`
  5. `add_exclude_route` для IP сервера
  6. `set_tun_route` через gateway на ts0
  7. Запуск forwarding (`run_client`)
  8. Ctrl+C → `restore_route` → `drop(tun)`

**`src/main.rs`**:
- Упрощён: больше не создаёт `Crypto` из PSK, не парсит `tun_ip` вручную
- `run_client_mode` вызывает `client::run_client_full`
- `run_server_mode` передаёт netmask в `server::run_server`

**`src/tun.rs`**:
- `create_tun()` принимает `netmask_bits: u8`, конвертирует через `netmask_from_prefix()`

### Docker end-to-end (`tests/Dockerfile.e2e`, `tests/docker_e2e.sh`)

- Pre-built образ с `debian:stable-slim` + `iproute2`, `iputils-ping`, `dnsutils`, `curl`, `socat`, `iptables`
- Два контейнера на одной Docker-сети, `--cap-add NET_ADMIN --device /dev/net/tun`
- Server: `--sysctl net.ipv4.ip_forward=1` + MASQUERADE для интернет-трафика
- **5 protocol tests — все проходят**:

| Протокол | Команда | Результат |
|----------|---------|-----------|
| ICMP local | `ping 10.0.0.1` | 3/3, 0% loss, ~0.3ms |
| TCP echo | `socat echo` через TUN | hello-vpn roundtrip |
| HTTP | `curl https://example.com` | HTML получен |
| DNS | `dig @8.8.8.8 google.com` | IP-адреса получены |
| ICMP internet | `ping 8.8.8.8` | 3/3, 0% loss, ~72ms |

### Changed files

| File | Changes |
|------|---------|
| `src/config.rs` | +validation, +tun_netmask, +gateway, +netmask_from_prefix, 17 unit-tests |
| `src/main.rs` | Refactored — вызывает `client::run_client_full`, передаёт netmask |
| `src/server.rs` | `handle_client` делает handshake перед forward |
| `src/client.rs` | New `run_client_full` — полный bootstrap |
| `src/tun.rs` | `create_tun` принимает `netmask_bits` |
| `tests/server_integration.rs` | Добавлен handshake перед отправкой фрейма |
| `tests/Dockerfile.e2e` | Новый — pre-built image для e2e |
| `tests/docker_e2e.sh` | Новый — 5 protocol tests через Docker |

## What Worked

- Docker e2e тест позволил проверить все протоколы без двух физических машин — достаточно одного хоста с Docker
- Session key из ECDH handshake корректно работает как ключ для XChaCha20-Poly1305 (размер совпадает — 32 байта)
- `ip_forward=1` + `MASQUERADE` на сервере пропускает интернет-трафик через туннель без дополнительной настройки

## What Didn't Work

- **SIGINT через `docker exec kill -INT 1`** не доходит до rust-процесса — PID 1 внутри контейнера это `sh -c`, который не форвардит сигналы. В реальном запуске Ctrl+C работает корректно (проверено кодом: `tokio::signal::ctrl_c()`)
- **`apt-get` в entrypoint контейнера** слишком медленный для теста (15+ секунд). Решение: pre-built Dockerfile

## Current State

- `cargo test --lib` — 60/60 unit-тестов
- `cargo test --test handshake_integration` — 2/2 проходят
- `cargo build --release` — без ошибок и warnings
- `tests/docker_e2e.sh` — 5/5 protocol tests pass
- Весь P1 завершён (P1.1–P1.5)

## Next Steps

1. **P2**: Performance and reliability — likely iperf3 benchmarking, reconnection logic, MTU discovery, multi-client support, etc.
2. **Clean up**: THOUGHTS.md needs records for decisions made this session (tun_netmask separate, gateway field, handshake in data path)

## Relevant Files

- `src/config.rs` — validation logic (`validate`, `validate_for_mode`)
- `src/client.rs` — `run_client_full` (client bootstrap orchestration)
- `src/server.rs` — `handle_client` (server handshake + forward)
- `src/main.rs` — entry points, calls orchestration
- `tests/docker_e2e.sh` — end-to-end test suite (5 protocols)
- `tests/Dockerfile.e2e` — test container image
- `docs/VERIFICATION_P1.md` — P1.4 and P1.5 marked `[x]`
