# Traffic Sentinel — Windows

**Статус**: Windows-поддержка в разработке. Маршрутизация (rtnetlink) требует Windows-аналога (WinAPI iphlpapi). На данный момент сборка под Windows не тестировалась.

## Сборка

### На Windows (native)

Требования: Rust (MSVC toolchain), [Wintun](https://www.wintun.net/) драйвер (устанавливается автоматически через `tun-rs`).

```powershell
# Debug-сборка
cargo build

# Release-сборка
cargo build --release
```

Бинарник: `target\release\traffic-sentinel.exe`.

### Кросс-компиляция с Linux

Требуется `mingw-w64` и Rust target `x86_64-pc-windows-gnu`:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64
cargo build --release --target x86_64-pc-windows-gnu
```

Бинарник: `target/x86_64-pc-windows-gnu/release/traffic-sentinel.exe`.

**Ограничения**: на данный момент сборка под Windows может не проходить из-за Linux-зависимостей (`rtnetlink`, `netlink-packet-route`). Требуется разделение платформо-зависимого кода через `#[cfg(target_os = "linux")]`.

## Конфигурация

Формат конфига — TOML, полностью идентичен [Linux-версии](SETUP_LINUX.md).

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[client]
remote = "1.2.3.4:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
gateway = "10.0.0.1"
max_retries = 10
reconnect_max_delay = 30
heartbeat_interval = 30
heartbeat_timeout = 60
```

### Запуск

### Требования

- Windows 8/10/11 (x86_64)
- Права администратора (UAC) для создания TUN и изменения маршрутов
- Драйвер Wintun (устанавливается автоматически `tun-rs` при первом запуске)

### Сервер

```powershell
# Запуск от администратора
$env:RUST_LOG = "info"
.\traffic-sentinel.exe --mode server --config server.toml
```

Сервер создаёт TUN-интерфейс, слушает TCP-порт, форвардит трафик после handshake.

### Клиент

```powershell
# Запуск от администратора
$env:RUST_LOG = "info"
.\traffic-sentinel.exe --mode client --config client.toml
```

Клиент создаёт TUN-интерфейс, меняет default route, форвардит трафик на сервер.

### Graceful shutdown

Нажать `Ctrl+C` в консоли.

### Уровни логирования

```powershell
$env:RUST_LOG = "info"     # INFO и выше
$env:RUST_LOG = "debug"    # DEBUG + INFO + WARN + ERROR
$env:RUST_LOG = "warn"     # WARN + ERROR
$env:RUST_LOG = "error"    # только ERROR
```

## Текущие ограничения

| Компонент | Статус |
|-----------|--------|
| TUN (wintun) | Должен работать через `tun-rs` |
| TCP-транспорт | ✅ Работает (tokio кроссплатформенный) |
| Шифрование (XChaCha20) | ✅ Работает (pure Rust) |
| Handshake (X25519) | ✅ Работает (pure Rust) |
| Маршрутизация (save/restore) | ❌ Требуется WinAPI (iphlpapi) — не реализовано |
| Route exclude для сервера | ❌ Не реализовано |
| Heartbeat / reconnect | ✅ Должно работать (чистый tokio) |
| Логирование | ✅ tracing работает кроссплатформенно |

## Известные проблемы

- При сборке под Windows могут отсутствовать некоторые Linux-only крейты (`rtnetlink`). Требуется условная компиляция.
- Маршруты не восстанавливаются при остановке (нет реализации WinAPI `CreateIpForwardEntry` / `DeleteIpForwardEntry`).
