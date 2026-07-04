# Verification Plan: Phase P3 — UDP Transport (Hardening)

## Общая информация

- **Цель**: Перевести транспорт туннеля с TCP на UDP для устранения TCP-over-TCP meltdown. Сохранить все существующие сценарии (handshake, шифрование, heartbeat, reconnect, graceful shutdown).
- **Среда**: Linux x86_64, требуется root (sudo) для интеграционных тестов.
- **Предусловия**: P2 полностью пройден, полный bidirectional цикл на TCP работает.

---

## P3.1: Protocol — remove length prefix

### Проверка: encode без length prefix

```rust
// protocol.rs
let frame = Frame { nonce: [1u8; 24], seq: 42, flags: 0x00, payload: vec![0xAA; 100] };
let encoded = encode(&frame);
// encoded.len() == 24 + 4 + 1 + 100 == 129 (без 2-байтового length)
assert_eq!(encoded.len(), 129);
// Первые 24 байта — nonce, а не length prefix
assert_eq!(encoded[0..24], [1u8; 24]);
```

### Проверка: decode without length prefix

```rust
let frame = Frame { nonce: [1u8; 24], seq: 42, flags: 0x00, payload: vec![0xAA; 100] };
let encoded = encode(&frame);
let decoded = decode(&encoded).unwrap();
assert_eq!(frame, decoded);
```

### Unit test

```bash
cargo test --lib protocol
```

**Ожидаемый результат**: все protocol-тесты проходят (encode/decode roundtrip, граничные размеры, decode garbage).

---

## P3.2: Transport — UDP sockets

### Проверка: udp_bind

```rust
let socket = udp_bind("127.0.0.1:0".parse().unwrap()).await?;
assert!(socket.local_addr().is_ok());
```

### Проверка: udp_connect

```rust
let server = udp_bind("127.0.0.1:0".parse().unwrap()).await?;
let server_addr = server.local_addr()?;
let client = udp_connect(server_addr).await?;
// client connected to server
client.send(b"hello").await?;
let mut buf = [0u8; 5];
let (n, peer) = server.recv_from(&mut buf).await?;
assert_eq!(&buf[..n], b"hello");
```

### Unit test

```bash
cargo test --lib transport
```

**Ожидаемый результат**: bind, connect, send/recv работают корректно.

---

## P3.3: Handshake over UDP

### Проверка: handshake matching keys

```rust
// real UDP sockets, not duplex
let server = udp_bind("127.0.0.1:0".parse().unwrap()).await?;
let server_addr = server.local_addr()?;
let client = udp_connect(server_addr).await?;
let psk = random_psk();
let (client_key, (server_key, _client_addr)) = tokio::join!(
    client_handshake(&client, &psk),
    server_handshake(&server, &psk),
);
assert_eq!(client_key.unwrap(), server_key.unwrap());
```

### Проверка: wrong PSK → error

```rust
let (client_result, server_result) = tokio::join!(
    client_handshake(&client, &client_psk),
    server_handshake(&server, &server_psk), // different PSK
);
assert!(client_result.is_err());
assert!(server_result.is_err());
```

### Проверка: handshake with packet loss

Имитация потери первого client_hello: client отправляет два hello (первый дропается), server отвечает на второй.

- Client: send hello, 3s timeout → retry
- Server: первый hello дропнут, второй получен → ответ

```rust
// Используем прокси-сокет, который дропает первый пакет
```

**Ожидаемый результат**: handshake завершается успешно после retransmission.

### Unit test

```bash
cargo test --lib handshake
```

**Ожидаемый результат**: matching keys, wrong PSK, PFS, tampered HMAC — все тесты проходят с UDP-сокетами.

---

## P3.4: Server — UDP data loop

### Проверка: server запускается

```bash
# bind UDP порт
sudo ss -ulpn | grep 8443  # после запуска server — есть
```

### Проверка: server принимает handshake от клиента

- Client подключается к server по UDP
- Handshake выполняется
- Лог: `Handshake complete`

### Проверка: server фильтрует чужие датаграммы

- Клиент A подключается, проходит handshake
- Клиент B отправляет data-фрейм на сервер (без handshake)
- Сервер игнорирует датаграмму от B (логирует `WARN`)

### Проверка: server timeout при отсутствии трафика

- После handshake клиент замолкает
- Через ~180s сервер возвращается в режим ожидания handshake

```bash
# После 180s тишины — новый клиент может выполнить handshake
```

---

## P3.5: Client — UDP data loop

### Проверка: client подключается

```bash
sudo ./target/release/traffic-sentinel --mode client --config client.toml
# Лог: Connected, Handshake complete, resuming
```

### Проверка: TUN→UDP→server

- Ping `10.0.0.1` (TUN peer)
- Пакет идёт через TUN → encrypt → UDP → server

### Проверка: reconnect при heartbeat timeout

- Остановить server (SIGSTOP)
- Ждать heartbeat_timeout (60s default)
- Клиент логирует `heartbeat timeout` → reconnect

### Проверка: reconnect с новым handshake

- Server перезапущен
- Клиент делает новый handshake (новый session key)
- Трафик возобновляется

---

## P3.6: Performance — видео стриминг

### Проверка: TCP-over-TCP meltdown устранён

Имитация потери пакетов на транспортном уровне:

```bash
# На сервере: добавить 1% потерь на UDP трафик
sudo tc qdisc add dev eth0 root netem loss 1%
```

**Ожидаемый результат с TCP-over-TCP (старое поведение):**
- speedtest-cli показывает < 5 Mbps при 1% loss

**Ожидаемый результат с UDP (новое поведение):**
- speedtest-cli показывает > 80% от пропускной способности канала

### Проверка: сквозная пропускная способность

```bash
# iperf3 через туннель
# Server side: iperf3 -s
# Client TUN IP: 10.0.0.2
# На клиентской машине: iperf3 -c 10.0.0.1 -t 30
```

**Ожидаемый результат**: throughput > 80% от raw-пропускной способности.

### Проверка: latency

```bash
ping -c 100 10.0.0.1  # ping через туннель
```

**Ожидаемый результат**: latency < 5ms (loopback) или близко к ping до сервера.

---

## P3.7: Auto TUN — удалить tun_* из конфига

### Проверка: server.toml без tun_ip/tun_netmask

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"

[server]
listen = "0.0.0.0:8443"
# tun_ip и tun_netmask отсутствуют — должны быть defaults
```

**Ожидаемый результат**: сервер запускается, создаёт TUN `ts0` с IP `10.0.0.1/24`.

### Проверка: client.toml без tun_ip/netmask/gateway

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"

[client]
remote = "SERVER_IP:8443"
# tun_ip, tun_netmask, gateway отсутствуют — должны быть defaults
```

**Ожидаемый результат**: клиент запускается, создаёт TUN `ts0` с IP `10.0.0.2/24`, gateway `10.0.0.1`.

### Проверка: server.toml с tun_subnet

```toml
[server]
listen = "0.0.0.0:8443"
tun_subnet = "10.100.0.0/24"
```

**Ожидаемый результат**: сервер создаёт TUN с IP `10.100.0.1/24` (первый IP подсети).

### Проверка: валидация tun_subnet

- `tun_subnet = "not-a-cidr"` → ошибка конфигурации
- `tun_subnet = "10.0.0.0/33"` → ошибка (невалидная маска)
- `tun_subnet = "10.0.0.0/8"` → ок (валидная CIDR)

### Проверка: старый конфиг с tun_ip выдаёт предупреждение

**Ожидаемый результат**: неизвестные ключи в конфиге игнорируются с warning (поведение serde по умолчанию с `#[serde(deny_unknown_fields)]` не используется).

### Unit test

```bash
cargo test --lib config
```

### Проверка: Docker-скрипты

```bash
bash tests/docker_e2e.sh
bash tests/docker_heartbeat.sh
bash tests/docker_reconnect.sh
```

**Ожидаемый результат**: все Docker-тесты проходят с обновлёнными конфигами (без tun_ip/netmask/gateway).

---

## P3.8: Docker end-to-end

### Проверка: docker_e2e.sh

```bash
bash tests/docker_e2e.sh
```

**Ожидаемый результат**: все проверки проходят:
- ICMP ping через туннель
- TCP echo (socat)
- HTTP (curl example.com)
- DNS (dig google.com)
- ICMP ping внешнего хоста

### Проверка: docker_heartbeat.sh

```bash
bash tests/docker_heartbeat.sh
```

**Ожидаемый результат**: PING/PONG, heartbeat timeout → reconnect, active traffic suppresses PING.

### Проверка: docker_reconnect.sh

```bash
bash tests/docker_reconnect.sh
```

**Ожидаемый результат**: reconnect after server kill, exponential backoff, graceful shutdown.

---

## P3.9: Unit tests regression

### Проверка: все unit тесты

```bash
cargo test --lib
```

**Ожидаемый результат**: все тесты проходят (protocol, transport, handshake, crypto, config, route, checks, nat).

### Проверка: clippy

```bash
cargo clippy -- -D warnings
```

**Ожидаемый результат**: 0 warnings.

### Проверка: build

```bash
cargo build --release
```

**Ожидаемый результат**: успешная сборка.

---

## Итоговый чеклист P3

- [ ] P3.1: Protocol — encode/decode без length prefix
- [ ] P3.2: Transport — UDP bind/connect
- [ ] P3.3: Handshake — работает поверх UDP с retransmission
- [ ] P3.4: Server — UDP data loop с recv_from/send_to
- [ ] P3.5: Client — UDP data loop с recv/send
- [ ] P3.6: Performance — видео стриминг без деградации
- [ ] P3.7: Auto TUN — tun_ip/netmask/gateway убраны из конфига, сервер управляет подсетью
- [ ] P3.8: Docker e2e — все три скрипта проходят (с обновлёнными конфигами)
- [ ] P3.9: Tests — cargo test + clippy clean
