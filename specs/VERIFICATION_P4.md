# Verification Plan: Phase P4 — Multi-client

## Общая информация

- **Цель**: Сервер обслуживает несколько клиентов одновременно. Каждый получает уникальный IP из пула при handshake. Сервер демультиплексирует обратный трафик.
- **Среда**: Linux x86_64, требуется root (sudo) для Docker-тестов.
- **Предусловия**: P3 полностью пройден, UDP транспорт работает, tun_* параметры автоматизированы.

---

## P4.1: IP Pool

### Проверка: создание пула

```rust
let pool = IpPool::new("10.0.0.0/24").unwrap();
// pool резервирует 10.0.0.1 для сервера, свободны 10.0.0.2 – 10.0.0.254
```

### Проверка: allocate / release

```rust
let ip = pool.allocate().unwrap();
assert_eq!(ip, Ipv4Addr::new(10, 0, 0, 2));
let ip2 = pool.allocate().unwrap();
assert_eq!(ip2, Ipv4Addr::new(10, 0, 0, 3));
pool.release(ip);
let ip3 = pool.allocate().unwrap();
assert_eq!(ip3, Ipv4Addr::new(10, 0, 0, 2)); // reused
```

### Проверка: исчерпание пула

```rust
// allocate все IP → следующий allocate возвращает Err
```

### Unit test

```bash
cargo test --lib ip_pool
```

---

## P4.2: Extended handshake

### Проверка: server_hello содержит client_ip + netmask

```rust
let server = udp_bind(addr).await?;
let client = udp_connect(server_addr).await?;
let pool = IpPool::new("10.0.0.0/24").unwrap();

let (ck, (sk, _, client_ip, netmask)) = tokio::join!(
    client_handshake(&client, &psk),
    server_handshake(&server, &psk, &pool),
);

assert_eq!(ck.unwrap().0, sk.unwrap().0); // matching session keys
assert_eq!(client_ip, Ipv4Addr::new(10, 0, 0, 2));
assert_eq!(netmask, 24);
```

### Проверка: два клиента получают разные IP

```rust
let (ip1, ip2) = run_two_handshakes(...);
assert_ne!(ip1, ip2);
```

### Проверка: server_hello — 69 байт

Поймать server_hello через proxy, проверить `n == 69`.

### Unit test

```bash
cargo test --lib handshake
```

---

## P4.3: Server — multi-client loop

### Проверка: сервер принимает второго клиента

- Клиент A подключается, получает IP, трафик идёт
- Клиент B подключается **не отключая A**, получает другой IP, трафик идёт
- Сервер логирует два handshake, два IP

### Проверка: демультиплексирование

- Клиент A пингует `8.8.8.8`, клиент B пингует `1.1.1.1`
- Ответы приходят правильным клиентам (не перепутаны)

### Проверка: дисконнект — освобождение IP

- Клиент A отключается (SIGTERM)
- Клиент B продолжает работать
- Сервер логирует disconnect, IP клиента A возвращается в пул
- Клиент C подключается, получает IP клиента A (reuse)

### Проверка: idle timeout клиента

- Клиент замолкает на 180s
- Сервер закрывает сессию, освобождает IP
- Другие клиенты не затронуты

---

## P4.4: Client — receive IP from handshake

### Проверка: клиент создаёт TUN с назначенным IP

- Первый клиент: TUN `10.0.0.2/24`, gateway `10.0.0.1`
- Второй клиент: TUN `10.0.0.3/24`, gateway `10.0.0.1`

```bash
# На клиенте после handshake
ip addr show ts0
# → inet 10.0.0.2/24 (для первого клиента)
```

### Проверка: клиентский конфиг без tun_*

```toml
[client]
remote = "SERVER_IP:8443"
# IP назначается сервером при handshake
```

---

## P4.5: Docker multi-client

### Проверка: два клиента одновременно

```bash
bash tests/docker_multiclient.sh
```

**Ожидаемый результат:**
- Сервер запущен
- Клиент A подключён, пингует сервер
- Клиент B подключён, пингует сервер (другой IP)
- Оба клиента работают одновременно
- При отключении A — B продолжает работать

### Проверка: три клиента

- Все три получают уникальные IP
- Трафик всех трёх проходит

---

## P4.6: Regression

### Проверка: single-client

```bash
bash tests/docker_e2e.sh
bash tests/docker_heartbeat.sh  
bash tests/docker_reconnect.sh
```

**Ожидаемый результат**: все три скрипта проходят (single-client = частный случай multi-client).

### Проверка: unit tests + clippy

```bash
cargo test --lib
cargo clippy -- -D warnings
```

---

## Итоговый чеклист P4

- [ ] P4.1: IP Pool — allocate/release/reuse
- [ ] P4.2: Handshake — server_hello 69 байт, client_ip + netmask
- [ ] P4.3: Server — multi-client loop, демультиплексирование
- [ ] P4.4: Client — получает IP из handshake, создаёт TUN
- [ ] P4.5: Docker — 2+ клиента одновременно
- [ ] P4.6: Regression — single-client, unit tests, clippy
