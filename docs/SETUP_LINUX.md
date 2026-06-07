# Traffic Sentinel — Linux

## Сборка

Требования: Rust (edition 2021), `cargo`.

```bash
# Debug-сборка
cargo build

# Release-сборка (оптимизированный бинарник)
cargo build --release

# Статическая сборка (musl) — для переноса на другие Linux-машины
# cargo-zigbuild не обязателен, но даёт статический бинарник
cargo build --release --target x86_64-unknown-linux-musl
```

Бинарник: `target/release/traffic-sentinel` (или `target/debug/traffic-sentinel`).

## Конфигурация

На вход — TOML-файл. Содержит общую секцию `[tunnel]` и одну из секций режима (`[client]` или `[server]`).

### Общая секция

```toml
[tunnel]
# Pre-Shared Key — 32 байта (64 hex-символа). Обязателен.
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"

# MTU для TUN-интерфейса. Опционально, по умолчанию 1400.
mtu = 1400
```

### Режим клиента

```toml
[client]
# Адрес удалённого сервера. Обязателен.
remote = "1.2.3.4:8443"

# IP адрес TUN-интерфейса на клиенте. Опционально, по умолчанию "10.0.0.2".
tun_ip = "10.0.0.2"

# Маска подсети TUN. Опционально, по умолчанию 30.
tun_netmask = 30

# Gateway — IP сервера в TUN-сети. Опционально, по умолчанию "10.0.0.1".
gateway = "10.0.0.1"

# Максимальное количество попыток переподключения. Опционально,
# по умолчанию безлимит (пока не нажат Ctrl+C).
max_retries = 10

# Максимальная задержка между reconnect (exponential backoff).
# Опционально, по умолчанию 30 секунд.
reconnect_max_delay = 30

# Интервал PING-фреймов (секунды). Если трафик идёт — PING не шлётся.
# Опционально, по умолчанию 30.
heartbeat_interval = 30

# Таймаут heartbeat (секунды). Если за это время нет ни PONG, ни данных —
# клиент начинает reconnect. Опционально, по умолчанию 60.
heartbeat_timeout = 60
```

### Режим сервера

```toml
[server]
# Адрес для входящих TCP-подключений. Обязателен.
listen = "0.0.0.0:8443"

# IP адрес TUN-интерфейса на сервере. Опционально, по умолчанию "10.0.0.1".
tun_ip = "10.0.0.1"

# Маска подсети TUN. Опционально, по умолчанию 30.
tun_netmask = 30
```

### Полный пример (клиент)

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

### Полный пример (сервер)

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
```

## Запуск

### Требования

- Linux с поддержкой TUN (`/dev/net/tun`)
- Права root (CAP_NET_ADMIN) для создания TUN и изменения маршрутов

### Сервер

```bash
# Включить IP forwarding
echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward

# Запустить
sudo RUST_LOG=info ./traffic-sentinel --mode server --config server.toml
```

Сервер создаёт TUN-интерфейс `ts0`, слушает TCP-порт, ждёт подключения клиента.
После подключения — форвардит расшифрованные пакеты в интернет через TUN.
При запуске сервер автоматически настраивает NAT (MASQUERADE) через `iptables`
для внешнего интерфейса (определяется по default route). Если `iptables` не найден —
выводится предупреждение, но сервер продолжает работу.

### Клиент

```bash
sudo RUST_LOG=info ./traffic-sentinel --mode client --config client.toml
```

Клиент:
1. Создаёт TUN-интерфейс `ts0`
2. Сохраняет текущий default route
3. Устанавливает default route через TUN
4. Подключается к серверу по TCP
5. Выполняет handshake (X25519 ECDH + PSK)
6. Форвардит весь трафик через зашифрованное соединение

При разрыве соединения — автоматический reconnect с exponential backoff.

### Graceful shutdown

Отправить `SIGTERM` или нажать `Ctrl+C`:

```bash
kill -TERM $PID
```

При остановке:
- Восстанавливается оригинальный default route
- Удаляется TUN-интерфейс `ts0`
- Закрывается TCP-соединение

### Уровни логирования

Через переменную окружения `RUST_LOG`:

```bash
sudo RUST_LOG=info ./traffic-sentinel --mode client --config client.toml    # INFO и выше
sudo RUST_LOG=debug ./traffic-sentinel --mode client --config client.toml   # DEBUG + INFO + WARN + ERROR
sudo RUST_LOG=warn ./traffic-sentinel --mode client --config client.toml    # WARN + ERROR
sudo RUST_LOG=error ./traffic-sentinel --mode client --config client.toml   # только ERROR
```

### Пример: сервер + клиент на одной машине (для теста)

```bash
# Терминал 1: сервер
echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward
sudo ./traffic-sentinel --mode server --config server.toml

# Терминал 2: клиент
sudo ./traffic-sentinel --mode client --config client.toml

# Терминал 3: проверка
ping 8.8.8.8 -c 5
```

## Интеграционные тесты (Docker)

```bash
# End-to-end тест (ICMP, TCP echo, HTTP, DNS)
bash tests/docker_e2e.sh

# Тест reconnect (4 сценария)
bash tests/docker_reconnect.sh

# Тест heartbeat (3 сценария)
bash tests/docker_heartbeat.sh
```

Требуется Docker с `--cap-add NET_ADMIN --device /dev/net/tun`.
