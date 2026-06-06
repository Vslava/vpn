# Verification Plan: Phase P2 — Production Ready

## Общая информация

- **Цель**: Проверить graceful shutdown, reconnection, heartbeat, logging, error handling, тесты.
- **Среда**: Linux x86_64, требуется root (sudo) для интеграционных тестов.
- **Предусловия**: P1 полностью пройден, полный bidirectional цикл работает.

---

## P2.1: Graceful shutdown

### Проверка: SIGTERM на клиенте

```bash
# Запустить клиент
sudo ./target/release/traffic-sentinel --mode client --config client.toml &
PID=$!

# Подождать запуска
sleep 2

# Отправить SIGTERM
kill -TERM $PID
wait $PID

# Проверить
ip route show default
ip link show ts0 2>&1
```

**Ожидаемый результат**:
- Exit code 0
- `ip route show default` → оригинальный gateway (не ts0)
- `ip link show ts0` → `does not exist`
- В логах последовательность: `Restoring routes` → `Deleting TUN` → `Closing TCP` → `Shutdown complete`

### Проверка: SIGTERM на сервере

```bash
# Запустить сервер
sudo ./target/release/traffic-sentinel --mode server --config server.toml &
PID=$!
sleep 2

kill -TERM $PID
wait $PID
```

**Ожидаемый результат**:
- Exit code 0
- TUN интерфейс сервера удалён
- `ss -tlnp | grep 8443` → ничего (порт не слушается)

### Проверка: SIGINT (Ctrl+C) на клиенте

```bash
sudo ./target/release/traffic-sentinel --mode client --config client.toml
# Нажать Ctrl+C
```

**Ожидаемый результат**: аналог SIGTERM — маршруты восстановлены, TUN удалён.

### Проверка: остановка во время передачи трафика

```bash
# Непрерывный ping
ping 8.8.8.8 > /dev/null &
PING_PID=$!

# Запустить клиент, подождать
sudo ./traffic-sentinel --mode client --config client.toml &
TS_PID=$!
sleep 5

# Убить клиент во время активного трафика
kill -TERM $TS_PID
wait $TS_PID
```

**Ожидаемый результат**:
- Пинг потеряет 2-3 пакета (пока маршруты переключаются)
- После восстановления маршрута — пинг продолжается (через оригинальный gateway)
- TUN удалён

### Проверка: kill -9 (SIGKILL) — зомби TUN

```bash
sudo ./traffic-sentinel --mode client --config client.toml &
PID=$!
sleep 2
kill -9 $PID
sleep 1

# Проверить
ip link show ts0 2>&1
ip route show default
```

**Ожидаемый результат**:
- `ip link show ts0` → `does not exist` (ядро очищает fd при закрытии процесса)
- `ip route show default` → **всё ещё маршрут через ts0** (ядро НЕ восстанавливает маршруты)
- **Это ожидаемый failure scenario** — при SIGKILL маршруты не восстанавливаются

### Проверка: двойной сигнал

- Отправить SIGTERM дважды

**Ожидаемый результат**: первый обрабатывается, второй игнорируется (shutdown уже запущен).

### Проверка: остановка при ошибке (pipeline failure)

- Запустить клиент с подключением
- Убить сервер
- Клиент обнаруживает ошибку → начинает shutdown (reconnect если настроен, или full shutdown)

**Ожидаемый результат**: shutdown последовательность выполнена корректно (маршруты, TUN).

### Проверка: время shutdown

```bash
# Замерить время от SIGTERM до полного выхода
timeout 5 bash -c '
  sudo ./traffic-sentinel --mode client --config client.toml &
  PID=$!
  sleep 2
  START=$(date +%s%N)
  kill -TERM $PID
  wait $PID
  END=$(date +%s%N)
  echo "Shutdown took $(( (END - START) / 1000000 )) ms"
'
```

**Ожидаемый результат**: < 1000 ms.

---

## P2.2: Reconnection (client)

### Проверка: reconnect при разрыве TCP

```bash
# Терминал 1: запустить сервер
sudo ./traffic-sentinel --mode server --config server.toml

# Терминал 2: запустить клиент
sudo ./traffic-sentinel --mode client --config client.toml

# Убить сервер
# Подождать и запустить снова
```

**Ожидаемый результат**:
1. Сервер убит → клиент логирует `ERROR TCP connection lost`
2. Клиент логирует `WARN Reconnecting in 1s...`
3. Клиент пытается переподключиться: `WARN Reconnecting in 2s...`
4. Сервер запущен снова
5. Клиент логирует `INFO Reconnected, starting handshake...`
6. Handshake проходит → `INFO Handshake complete, resuming`
7. Трафик продолжает идти

### Проверка: exponential backoff

```bash
# Запустить клиент, сервер не запущен
sudo ./traffic-sentinel --mode client --config client.toml 2>&1 | grep "Reconnecting"
```

**Ожидаемый результат**: интервалы: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s...

```bash
# Измерить интервалы
sudo ./traffic-sentinel --mode client --config client.toml 2>&1 | ts '%H:%M:%S' | grep -oP '\d+s?'
# ts — moreutils пакет, добавляет таймстемпы
```

### Проверка: max retries и graceful shutdown

- Не запускать сервер
- Ждать, пока клиент исчерпает попытки (или проверить с `max_retries = 3` в конфиге)

**Ожидаемый результат**: после исчерпания попыток → graceful shutdown (routes restored, TUN deleted).

### Проверка: reconnect с восстановлением трафика

```bash
# Непрерывный ping в фоне
ping 8.8.8.8 -i 0.5 > /tmp/ping.log &
PING_PID=$!

# Запустить клиент + сервер
# Подождать
# Убить сервер на 10 секунд
# Запустить сервер снова
# Подождать 15 секунд
# Остановить ping
kill $PING_PID

# Посмотреть потери
grep "timeout\|100% packet loss" /tmp/ping.log
```

**Ожидаемый результат**: потери только за время, когда сервер был мёртв (10-20 пакетов). После переподключения — пинг возобновляется без потерь.

### Проверка: reconnect при перезапуске сервера (graceful)

```bash
# Терминал 1: сервер
sudo ./traffic-sentinel --mode server --config server.toml

# Терминал 2: клиент с пингом
ping 8.8.8.8 -i 1 > /dev/null &

# Терминал 3:
# Перезапустить сервер:
sudo killall traffic-sentinel
sleep 1
sudo ./traffic-sentinel --mode server --config server.toml
```

**Ожидаемый результат**: клиент переподключается, пинг возобновляется. Потери < 5 пакетов.

### Проверка: новый session key после reconnect

- Перехватить TCP трафик после reconnect
- Сравнить encrypted payload до и после

**Ожидаемый результат**: session key изменился (новый ECDH handshake).

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Сервер падает сразу после handshake | Kill -9 сервера | Клиент начинает reconnect |
| Клиент reconnect, но сейчас подключён другой клиент | Один клиент уже подключён | Новый handshake заменяет (или rejected — если multi-client не поддерживается) |
| Сетевой таймаут при reconnect | `iptables -A OUTPUT -p tcp --dport 8443 -j DROP` | Backoff, retry, graceful shutdown |
| reconnect и shutdown одновременно | Kill клиент во время retry | Shutdown отменяет retry |
| Очень быстрые reconnect | Сервер флуктуирует (up/down каждые 500ms) | Клиент успевает reconnect, backoff сбрасывается при успехе |

### Проверка: server-side reconnect (ожидание нового клиента)

- Клиент отключается
- Сервер логирует `WARN Client disconnected, waiting for new connection...`
- Сервер продолжает слушать
- Новый клиент подключается → handshake → трафик

---

## P2.3: Heartbeat / keepalive

### Проверка: TCP_KEEPALIVE включён

- Прочитать `socket2::SockRef::from(&stream).keepalive()?`

**Ожидаемый результат**: `true`.

### Проверка: application-level ping (PING frame)

- Запустить клиент + сервер
- Через tcpdump проверить, что каждые ~30s появляется frame с `flags = PING`

```bash
tcpdump -i any -X port 8443 2>/dev/null | grep -c "PING"
```

(Конкретная проверка зависит от wire format)

**Ожидаемый результат**: PING фреймы видны в трафике.

### Проверка: PONG response

- Сервер получает PING → отвечает PONG
- Клиент получает PONG → сбрасывает таймер

**Тест**: на стороне клиента искусственно задержать PONG (на сервере не отправлять ответ).
- Клиент должен через 60s таймаута начать reconnect

### Проверка: heartbeat timeout → reconnect

- Заморозить сервер (SIGSTOP)
- Клиент не получает PONG в течение 60s → reconnect

```bash
# Запустить всё
# Заморозить сервер
kill -STOP $SERVER_PID
# Ждать ~70 секунд
# Разморозить
kill -CONT $SERVER_PID
```

**Ожидаемый результат**: клиент переподключился, трафик возобновлён.

### Проверка: heartbeat при активном трафике

- При активном трафике каждые 30s PING не нужен (любой frame сбрасывает таймер)
- Проверить, что при непрерывном трафике PING не отправляется

**Ожидаемый результат**: PING отправляется только в тишине.

---

**Docker verification**: `tests/docker_heartbeat.sh` — 14/14 checks passed
- Test 1: idle 25s (2.5× heartbeat_interval=10) — PONG responses keep connection alive, no reconnect
- Test 2: `docker pause` server → heartbeat timeout after heartbeat_timeout=15 → reconnect after `docker unpause`
- Test 3: continuous ping 20s — active traffic suppresses PING, no reconnect

---

## P2.4: Logging

### Проверка: уровни логирования

```
RUST_LOG=info   → только INFO и выше
RUST_LOG=debug  → DEBUG + INFO + WARN + ERROR
RUST_LOG=warn   → только WARN + ERROR
RUST_LOG=error  → только ERROR
```

```bash
RUST_LOG=info sudo ./traffic-sentinel --mode client --config client.toml
RUST_LOG=debug sudo ./traffic-sentinel --mode client --config client.toml
```

### Проверка: структура логов

- Каждое сообщение содержит:
  - timestamp (ISO 8601)
  - level
  - target (модуль)
  - message

```
2026-06-06T12:00:00.123Z  INFO traffic_sentinel::client: Starting in client mode
2026-06-06T12:00:00.456Z  INFO traffic_sentinel::tun: Created TUN interface ts0 (mtu=1400, ip=10.0.0.2/30)
2026-06-06T12:00:01.000Z  INFO traffic_sentinel::transport: Connected to 1.2.3.4:8443
2026-06-06T12:00:01.200Z  INFO traffic_sentinel::handshake: Handshake complete
2026-06-06T12:00:01.300Z  DEBUG traffic_sentinel::pipeline: Packet sent: seq=1, len=120, proto=ICMP
```

### Проверка: все обязательные log events

| Event | Level | Когда |
|-------|-------|-------|
| Starting in client/server mode | INFO | startup |
| Loaded config from <path> | INFO | config loaded |
| Created TUN interface | INFO | TUN created |
| Restored routes | INFO | shutdown |
| Deleted TUN interface | INFO | shutdown |
| Shutdown complete | INFO | exit |
| Connected to <addr> | INFO | TCP connect |
| Client connected from <addr> | INFO | server accept |
| Handshake complete | INFO | handshake done |
| Reconnecting in <n> seconds | WARN | reconnect |
| Reconnected, resuming | INFO | reconnect success |
| Packet sent/received | DEBUG | per packet |
| TCP connection lost | ERROR | disconnect |
| Crypto error | ERROR | AEAD failure |
| Handshake failed | ERROR | HMAC mismatch |
| Max retries exceeded | ERROR | reconnect exhausted |

### Проверка: нет чувствительных данных в логах

- PSK не логируется
- Session key не логируется
- Nonce не логируется в открытом виде (можно логировать первые 4 байта для отладки)

### Проверка: structured fields

- `tracing` events должны содержать context поля:

```rust
info!(mode = "client", "Starting");
debug!(seq = packet.seq, len = packet.len, "Packet sent");
warn!(retry = attempt, max = MAX_RETRIES, "Reconnecting");
error!(error = %e, "Connection lost");
```

### Проверка: производительность логирования

- Замерить overhead при `RUST_LOG=debug` и при `RUST_LOG=info` при активном трафике
- Overhead < 2%

---

## P2.5: Error handling audit

### Проверка: нет unwrap/expect в production paths

```bash
# Поиск unwrap в исходниках (исключая тесты)
rg '\bunwrap\b' src/ --no-filename -n
rg '\bexpect\b' src/ --no-filename -n
```

**Ожидаемый результат**: все вхождения только в тестовых модулях (`#[cfg(test)]`).

### Проверка: все ошибки имеют понятное сообщение

- `io::Error` — обёрнуты в контекст: `"Failed to create TUN interface: {io_error}"`
- Крипто-ошибки — `"AEAD decryption failed: data may be corrupted"`
- Сетевые ошибки — `"Connection to {addr} failed: {io_error}"`

### Проверка: error chain

- Все публичные функции возвращают `Result<T, Error>` где `Error` — единый тип с `Display` и `source()`
- `Error` можно сматчить на варианты:
  - `Error::Tun` / `Error::Crypto` / `Error::Transport` / `Error::Config`
  - `Error::Handshake` / `Error::Protocol` / `Error::Route`

### Проверка: обработка конкретных ошибок

| Сценарий | Ошибка | Реакция |
|----------|--------|---------|
| TUN create failed (permissions) | `io::Error::PermissionDenied` | Exit с сообщением "Run with sudo / root" |
| TUN create failed (already exists) | `io::Error::AddrInUse` | Попробовать другое имя? Или exit |
| TCP connect ECONNREFUSED | `io::Error::ConnectionRefused` | Reconnect (client) |
| TCP write EPIPE/BrokenPipe | `io::Error::BrokenPipe` | Reconnect (client) |
| TCP read ConnectionReset | `io::Error::ConnectionReset` | Reconnect (client) |
| AEAD decrypt failed | `Error::Crypto("decryption failed")` | Disconnect (cannot trust packet) |
| Handshake HMAC mismatch | `Error::Handshake("PSK mismatch")` | Disconnect, не reconnect (PSK не изменится) |
| Config parse error | `toml::de::Error` | Exit с указанием строки |
| Route restore failed | `io::Error` | WARN и continue (не блокирует shutdown) |

### Проверка: resilience

- Если TUN write возвращает ошибку (EAGAIN не должно быть, но на всякий случай):
  - Sequential pipeline блокируется, ошибка всплывает → reconnect
- Если crypto возвращает ошибку:
  - Пакет дропается? Или весь reconnect? **Решение**: reconnect при crypto error (безопаснее).

---

## P2.6: Tests (unit + integration)

### Unit test: crypto roundtrip

```bash
cargo test --lib crypto
```

**Тестовые случаи**:
- `test_encrypt_decrypt_roundtrip` — малые/большие/пустые данные
- `test_decrypt_wrong_nonce_fails` — AEAD error
- `test_decrypt_wrong_key_fails` — AEAD error
- `test_decrypt_corrupted_ciphertext_fails` — 1 бит flip → AEAD error
- `test_nonce_uniqueness` — 100k nonce, 0 коллизий
- `test_encrypt_deterministic` — одинаковые (key, nonce, plain) → одинаковый cipher

### Unit test: protocol encode/decode

```bash
cargo test --lib protocol
```

**Тестовые случаи**:
- `test_encode_decode_roundtrip` — различные размеры payload (0, 1, 100, 1400, 65535)
- `test_encode_decode_fields` — nonce, seq, flags сохраняются
- `test_decode_truncated` — EOF
- `test_decode_zero_length` — Error::ZeroLength
- `test_decode_too_large` — Error::FrameTooLarge
- `test_decode_garbage` — ошибка

### Unit test: handshake

```bash
cargo test --lib handshake
```

**Тестовые случаи**:
- `test_handshake_matching_keys` — in-memory duplex, ключи совпадают
- `test_handshake_wrong_psk_fails` — PSK mismatch → Error
- `test_handshake_key_uniqueness` — 100 handshake, все ключи разные (PFS)
- `test_handshake_timeout` — no data → timeout error

### Unit test: config validation

```bash
cargo test --lib config
```

**Тестовые случаи**:
- `test_valid_client_config`
- `test_valid_server_config`
- `test_invalid_psk_length`
- `test_invalid_psk_chars`
- `test_invalid_mtu`
- `test_invalid_port`
- `test_missing_section_for_mode`

### Unit test: route (mocked)

```bash
cargo test --lib route
```

**Тестовые случаи** (используя `Command::new("ip")` — требуется, но тест проверяет, что команды возвращают 0):
- `test_save_restore_route` — сохранить → изменить → восстановить → проверить
- `test_exclude_route` — добавить → проверить `ip route` → удалить
- `test_no_default_route` — временно удалить default → save → Ok(None)

### Integration test: full loop (требует root)

```bash
sudo cargo test --test integration -- --test-threads=1
```

**Тестовые случаи**:

```rust
#[test]
fn test_ping_through_tunnel() {
    // 1. Setup server TUN, start server
    // 2. Setup client TUN, start client
    // 3. Wait for handshake
    // 4. ping 8.8.8.8 -c 3
    // 5. Assert 3 replies
    // 6. Assert routes restored after shutdown
}

#[test]
fn test_reconnect() {
    // 1. Start server + client
    // 2. Verify traffic
    // 3. Kill server
    // 4. Wait for reconnect attempts
    // 5. Restart server
    // 6. Verify traffic resumes
}

#[test]
fn test_graceful_shutdown_restores_routes() {
    // 1. Start client
    // 2. Send SIGTERM
    // 3. Check ip route show default != tun gw
    // 4. Check ip link show ts0 fails
}

#[test]
fn test_config_error_handling() {
    // 1. Provide invalid config
    // 2. Verify appropriate error message
    // 3. Exit code != 0
}
```

### Проверка: coverage

```bash
# Install cargo-tarpaulin
cargo tarpaulin --ignore-tests --out Html
```

**Ожидаемый результат**: > 80% coverage для core модулей (crypto, protocol, handshake, config).

### Проверка: нет dead code

```bash
cargo build --release 2>&1 | grep -i "warning"
```

**Ожидаемый результат**: 0 warnings (или только expected, например unused mut в некоторых флагах).

### Проверка: clippy

```bash
cargo clippy -- -D warnings
```

**Ожидаемый результат**: 0 clippy warnings.

---

## Финальный smoke test (end-to-end)

После прохождения всех этапов P2 — полный smoke test:

```bash
# 1. Сборка
cargo build --release
cargo test --lib

# 2. Подготовка конфигов
cat > server.toml << 'EOF'
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
EOF

cat > client.toml << 'EOF'
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[client]
remote = "127.0.0.1:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
EOF

# 3. IP forwarding
echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward

# 4. Запуск сервера
sudo ./target/release/traffic-sentinel --mode server --config server.toml &
SERVER_PID=$!
sleep 2

# 5. Запуск клиента
sudo RUST_LOG=info ./target/release/traffic-sentinel --mode client --config client.toml &
CLIENT_PID=$!
sleep 5

# 6. Проверка трафика
ping 8.8.8.8 -c 5 || echo "FAIL: ping"
curl -s https://example.com > /dev/null || echo "FAIL: http"
dig +short google.com @8.8.8.8 || echo "FAIL: dns"

# 7. Проверка reconnect
kill -TERM $SERVER_PID
sleep 15
sudo ./target/release/traffic-sentinel --mode server --config server.toml &
SERVER_PID=$!
sleep 10
ping 8.8.8.8 -c 3 || echo "FAIL: reconnect ping"

# 8. Graceful shutdown
kill -TERM $CLIENT_PID
wait $CLIENT_PID 2>/dev/null
sleep 1
ip route show default | grep -q "ts0" && echo "FAIL: route not restored"
ip link show ts0 2>&1 | grep -q "does not exist" || echo "FAIL: TUN not deleted"

# 9. Остановка сервера
kill -TERM $SERVER_PID
wait $SERVER_PID 2>/dev/null

echo "=== SMOKE TEST COMPLETE ==="
```

---

## Итоговый чеклист P2

- [x] P2.1: Graceful shutdown — SIGTERM/SIGINT восстанавливает routes + TUN + TCP
- [x] P2.2: Reconnection — exponential backoff, retry, reconnect with new handshake (Docker verification passed)
- [x] P2.3: Heartbeat/keepalive — TCP_KEEPALIVE, PING/PONG, timeout→reconnect
- [x] P2.4: Logging — уровни, структура, обязательные events, безопасность данных
- [x] P2.5: Error handling — Result-only, контекстные ошибки, правильная реакция на каждый тип
- [x] P2.6: Tests — unit (crypto, protocol, handshake, config), integration (ping, reconnect, shutdown)
