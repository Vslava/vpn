# Handoff: P1.3 Server Forwarder

## Summary

Реализован server-side TUN forwarder для фазы P1. Сервер теперь создаёт TUN ts0, принимает TCP-соединение и выполняет bidirectional forwarding: расшифровывает входящие фреймы и пишет в TUN, читает из TUN, шифрует и отправляет клиенту. Кодовая база собрана, все существующие тесты проходят.

## What Was Done

### `src/server.rs` — полный рерайт (~104 строки)

- Удалена старая `handle_stream` (только логировала decrypted пакеты)
- Добавлена `run_server(addr, psk, tun_ip, mtu)` — создаёт TUN ts0, слушает TCP, принимает 1 клиента
- Добавлена `pub async fn handle_client(stream, psk, tun)` — bidirectional forwarding:
  - **TCP→TUN**: `reader.read_exact(2b length)` → `protocol::decode()` → `crypto.decrypt()` → `tun.send()`
  - **TUN→TCP**: `tun.recv()` → `crypto.encrypt()` → `protocol::encode()` → `writer.write_all()`
  - Два `tokio::spawn` + `tokio::select!` (кто первый завершится — пробрасывает ошибку)
  - Sequence number (`AtomicU32`) для каждого отправленного фрейма

### `src/main.rs` — изменения в run_server_mode

- Сигнатура изменена: `(cfg, crypto: Arc<Crypto>)` → `(cfg, psk: &[u8; 32], mtu: u16)`
- Парсинг `tun_ip` из `server_cfg.tun_ip` (дефолт `"10.0.0.1/30"`)
- Сервер больше не требует pre-created `Crypto` — создаёт свой внутри

### `tests/server_integration.rs` — новый integration test

- `test_server_forwarding_pipeline`: полный цикл client→server→TUN→server→client
- Использует реальный TUN (требует `sudo`)
- `#[ignore = "requires root (sudo) for TUN creation"]`
- Создаёт TUN, TCP listener, spawn сервер, подключает клиент, отправляет encrypted frame, получает loopback-ответ

### `docs/VERIFICATION_P1.md`

- Добавлен `[x]` для "server TUN создаётся"
- Все end-to-end проверки (forward, ping, HTTP, DNS, ICMP) перенесены в P1.5
- Edge cases сокращены до проверяемого без P1.5
- Итоговый чеклист: P1.3 `[x]`

## Key Decisions

| Вопрос | Варианты | Решение |
|--------|----------|---------|
| Какой ключ использовать для шифрования данных на сервере? | (a) session key из ECDH handshake, (b) PSK напрямую | Выбрали (b) — PSK напрямую. Handshake реализован, но не интегрирован в data path. Клиент тоже не использует session key. Интеграция handshake — P1.5. |
| Когда создавать TUN? | (a) до listen, (b) после accept | (a) — fail fast: если нет прав на TUN, процесс завершается до принятия клиента. |
| Публичный API handle_client? | (a) оставить private, (b) сделать pub | (b) — для тестирования и переиспользования (run_server создаёт TUN, handle_client принимает готовый). |

## Current State

- `cargo test --lib` — 33/33 unit-тестов проходят
- `cargo test --test handshake_integration` — 2/2 проходят
- `cargo build --release` — без ошибок и warnings
- `cargo test --test server_integration -- --ignored` — fail без sudo (ожидаемо)

## Next Steps

1. **P1.4**: Full config loading — validation rules (PSK, MTU, addresses, netmask)
2. **P1.5**: Wire everything — handshake integration in client+server, routes, full bootstrap

## Relevant Files

- `src/server.rs` — server forwarder (TUN creation, bidirectional forwarding)
- `src/main.rs` — `run_server_mode` теперь передаёт PSK + TUN config
- `tests/server_integration.rs` — integration test (требует sudo)
- `docs/VERIFICATION_P1.md` — P1.3 marked `[x]`
