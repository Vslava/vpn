# Handoff: P1.2 ECDH Handshake

## Summary

Реализован и верифицирован модуль ECDH handshake (`src/handshake.rs`) для фазы P1. X25519 эфемерные ключи + PSK (256-bit) с HMAC-SHA256 аутентификацией. Написаны 13 unit-тестов и 2 integration-теста (с реальным TCP), все проходят.

## What Was Done

### P1.2 Implementation

- `src/handshake.rs` (~280 строк) — ECDH + PSK hybrid handshake:
  - `client_handshake(stream, psk)` — генерирует X25519 эфемерный ключ, отправляет ClientHello (pub + HMAC), верифицирует ServerHello, вычисляет session key
  - `server_handshake(stream, psk)` — верифицирует ClientHello, генерирует свой эфемерный ключ, отправляет ServerHello, вычисляет session key
  - Протокол: фиксированные 64-байтные сообщения (32 pub + 32 HMAC-SHA256)
  - Key derivation: SHA-256(psk || ecdh_shared || client_pub || server_pub)
  - `hmac_sha256()` — HMAC-SHA256 для PSK proof
  - `derive_session_key()` — key derivation function
  - `send_handshake_msg()` / `recv_and_verify_msg()` — helpers с 10s timeout на read
  - `flush()` после write_all для совместимости с `BufStream`

### Dependencies

- Добавлены в `Cargo.toml`: `hmac = "0.12"`, `sha2 = "0.10"`

### Error Handling

- Добавлен `Error::Handshake(String)` variant в `src/error.rs`

### Tests

- **13 unit-тестов** (`src/handshake.rs`):
  - `test_hmac_sha256_deterministic` / `test_hmac_sha256_different_key` / `test_hmac_sha256_different_data` — HMAC корректность
  - `test_derive_session_key_deterministic` / `test_derive_session_key_different_inputs` — key derivation
  - `test_client_server_handshake_matching_keys` — matching session key через in-memory duplex
  - `test_client_server_wrong_psk` — разные PSK → обе стороны возвращают `Error::Handshake`
  - `test_perfect_forward_secrecy` — 100 уникальных session key (PFS)
  - `test_psk_all_zeros` / `test_psk_all_ff` — граничные значения PSK
  - `test_tampered_hmac_server_hello` / `test_tampered_hmac_client_hello` — HMAC подмена
  - `test_tampered_public_key` — public key подмена

- **2 integration-теста** (`tests/handshake_integration.rs`):
  - `test_handshake_over_real_tcp` — TcpListener + TcpStream, ключи совпадают
  - `test_handshake_tcp_wrong_psk` — разные PSK через реальный TCP → ошибка

### Verification against VERIFICATION_P1.md

- Matching keys: `[x]` — `test_client_server_handshake_matching_keys`
- Wrong PSK: `[x]` — `test_client_server_wrong_psk`
- PFS (100 keys): `[x]` — `test_perfect_forward_secrecy`
- Replay: `[x]` — PFS гарантирует уникальность
- PSK length: `[x]` — тип `&[u8; 32]` + `parse_psk()` в main.rs
- Timeout (10s): `[x]` — `tokio::time::timeout` в `recv_and_verify_msg`
- Real TCP: `[x]` — `tests/handshake_integration.rs`
- Edge cases (PSK 0/FF, HMAC tamper, pubkey tamper): `[x]`

## Issues Found

- Тест `test_client_server_wrong_psk` зависал — сервер падал с HMAC mismatch, клиент вечно ждал ответа. Исправлено: добавлен 10s timeout на все read операции в `recv_and_verify_msg`.
- `BufStream` буферизирует запись — handshake сообщение не доходило до сервера в TCP тесте. Исправлено: добавлен `flush()` после `write_all` в `send_handshake_msg`.

## Current State

- `cargo test --lib` — 33/33 unit-тестов проходят (20 старых + 13 новых)
- `cargo test --test handshake_integration` — 2/2 integration-теста проходят
- `cargo test --test route_integration` — 2/2 с sudo проходят (как и раньше)
- `cargo build --release` — собирается без ошибок и warnings
- VERIFICATION_P1.md: P1.2 помечен `[x]`

## Next Steps

1. **P1.3**: Server forwarder — bidirectional трафик через server-side TUN
2. **P1.4**: Full config loading — validation rules
3. **P1.5**: Wire everything into main.rs — handshake, routes, full bootstrap

## Relevant Files

- `src/handshake.rs` — ECDH handshake implementation (X25519 + PSK hybrid)
- `src/error.rs` — добавлен `Error::Handshake` variant
- `tests/handshake_integration.rs` — integration-тесты с реальным TCP
- `docs/VERIFICATION_P1.md` — верификация P1 (P1.2 помечен `[x]`)
- `docs/THOUGHTS.md` — Q1 (X25519 ECDH + PSK), остальные решения
- `AGENTS.md` — добавлены правила работы с VERIFICATION файлами
- `Cargo.toml` — добавлены hmac, sha2
