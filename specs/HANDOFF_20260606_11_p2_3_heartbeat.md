# Handoff: P2.3 — Heartbeat / Keepalive

## Summary

Реализован P2.3 Heartbeat/keepalive: application-level PING/PONG фреймы, TCP_KEEPALIVE, heartbeat timeout → reconnect. Написаны Docker-тесты (14/14 checks passed). Все unit-тесты (60/60) проходят, clippy чист.

## What Was Done

### Protocol (`src/protocol.rs`)
- Добавлены `FLAG_DATA = 0x00`, `FLAG_PING = 0x01`, `FLAG_PONG = 0x02`
- Методы `Frame::is_ping()`, `Frame::is_pong()`

### Error types (`src/error.rs`)
- Добавлен `Error::Timeout(String)` — для heartbeat timeout и handshake timeout

### TCP_KEEPALIVE (`src/transport.rs`, `Cargo.toml`)
- Добавлена зависимость `socket2 = "0.5"`
- Функция `set_keepalive(stream)` — включает TCP_KEEPALIVE с 60s

### Client heartbeat (`src/client.rs`)
- **run_client**:
  - `set_keepalive(&stream)` при старте сессии
  - **PING sender** (spawned task): отправляет PING через `mpsc`-канал в h1 (writer task), когда `last_rx.elapsed() >= hb_interval` (только в тишине)
  - **h2 (TCP reader)**: перехватывает PONG фреймы — не форвардит в TUN, обновляет `last_rx`
  - **Timeout watchdog** (spawned task): проверяет `last_rx.elapsed()` каждую секунду, при превышении `hb_timeout` отправляет `Error::Timeout` через `mpsc`-канал в main select!
  - **handshake timeout**: `tokio::time::timeout(15s, client_handshake)` — чтобы reconnect не зависал на мёртвом сервере

### Server PING handler (`src/server.rs`)
- `set_keepalive(&stream)` при подключении клиента
- **h1 (TCP reader)**: при получении PING → отправляет PONG через `mpsc`-канал в h2 (writer task)
- **h2 (writer)**: `select!` между TUN→TCP и каналом PONG

### Config (`src/config.rs`)
- `ClientConfig`: `heartbeat_interval: Option<u64>`, `heartbeat_timeout: Option<u64>`
- Default: interval=30s, timeout=60s

### Docker tests (`tests/docker_heartbeat.sh`)
- **Test 1 (PING/PONG)**: idle 25s (2.5× interval=10) — PONG responses keep connection alive, 6 checks
- **Test 2 (Timeout → reconnect)**: `docker pause` server → heartbeat timeout → reconnect после `docker unpause`, 5 checks
- **Test 3 (Active traffic)**: continuous ping 20s — активный трафик подавляет PING, 3 checks

## What Didn't Work / Issues Found

- **Bug: watchdog cancel caused clean shutdown instead of reconnect**: Timeout watchdog вызывал `cancel.cancel()`, что приводило к срабатыванию biased `cancel.cancelled()` ветки в select! → `Ok(())` вместо `Err(Timeout)`. **Fix**: убрать `cancel_watch.cancel()`, watchdog только шлёт ошибку через `mpsc`.
- **Client handshake без таймаута**: При reconnect к замороженному серверу TCP connect успевал (kernel-level accept), но handshake read блокировался навсегда. **Fix**: `tokio::time::timeout(15s, client_handshake)`.
- **Server writer — PONG доступ**: h1 (TCP reader) не мог писать PONG напрямую, т.к. `writer` был moved в h2. **Fix**: `mpsc`-канал от h1 к h2 для PONG фреймов.

## Current State

- **P2.3: Heartbeat — полностью завершён**
- ✅ 60/60 unit tests pass
- ✅ clippy: 0 warnings (`-D warnings`)
- ✅ 14/14 Docker verification tests pass
- ✅ `docs/VERIFICATION_P2.md`: P2.3 marked `[x]`

## Next Steps

- P2.4: Logging audit — structured fields, levels, all required events
- P2.5: Error handling audit — unwrap/expect, единый тип ошибки

## Relevant Files

- `src/client.rs` — PING sender, PONG handler, timeout watchdog, handshake timeout
- `src/server.rs` — PING→PONG через mpsc-канал
- `src/protocol.rs` — FLAG_PING/FLAG_PONG, is_ping/is_pong
- `src/transport.rs` — set_keepalive()
- `src/config.rs` — heartbeat_interval, heartbeat_timeout
- `src/error.rs` — Error::Timeout
- `tests/docker_heartbeat.sh` — 3 теста, 14 checks
- `docs/VERIFICATION_P2.md` — P2.3 checklist marked passed
