# Traffic Sentinel

Зашифрованный VPN-туннель на Rust. Перехватывает весь L3-трафик через TUN-интерфейс, шифрует (XChaCha20-Poly1305 + X25519 ECDH + PSK) и отправляет на сервер-шлюз через TCP.

## Архитектура

```
  App (браузер и т.д.)          Internet           Gateway Server
       |                         [encrypted]             |
  [TUN interface]                TCP tunnel              |
       | (raw IP)                                        |
  [Traffic Sentinel] —————————————— TCP ———————————→ [Decrypt → Forward]
       ↑                                                      |
       ←———————————————————— TCP —————————————————————— [Encrypt → Send]
```

Единый бинарник, режим выбирается флагом `--mode client|server`.

## Быстрый старт

```bash
# Сборка
cargo build --release

# Запуск сервера
sudo ./target/release/traffic-sentinel --mode server --config server.toml

# Запуск клиента (на другой машине)
sudo ./target/release/traffic-sentinel --mode client --config client.toml
```

### Минимальный server.toml

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"

[server]
listen = "0.0.0.0:8443"
```

### Минимальный client.toml

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"

[client]
remote = "SERVER_IP:8443"
```

## Возможности

- L3-туннель (TCP, UDP, ICMP) через TUN-интерфейс
- Сквозное шифрование: XChaCha20-Poly1305 + X25519 ECDH + PSK (perfect forward secrecy)
- Автоматическое управление маршрутами (сохранение/восстановление default route)
- Graceful shutdown (SIGTERM/SIGINT → восстановление маршрутов, удаление TUN)
- Reconnect с exponential backoff (клиент)
- Heartbeat/PING-PONG для детекта разрыва TCP
- Структурированное логирование (tracing, уровни через `RUST_LOG`)
- Единый статический бинарник

## Платформы

| Платформа | Статус |
|---|---|
| Linux (x86_64) | ✅ Полностью работает |
| Windows | ⚠️ TUN создаётся, маршруты не управляются (нет WinAPI iphlpapi) |
| Linux (aarch64) | ⬜ Запланировано |
| macOS | ⬜ Запланировано |

## Документация

- [SETUP_LINUX.md](docs/SETUP_LINUX.md)
- [SETUP_WINDOWS.md](docs/SETUP_WINDOWS.md)
- [DEVELOPMENT.md](DEVELOPMENT.md) — тестирование, статус, документы разработки

## Лицензия

MIT
