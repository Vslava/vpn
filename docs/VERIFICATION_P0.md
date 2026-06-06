# Verification Plan: Phase P0 — Proof of Concept

## Общая информация

- **Цель**: Проверить, что TUN → чтение → шифрование → TCP → дешифровка → лог работает.
- **Среда**: Linux x86_64, требуется root (sudo).
- **Команда сборки**: `cargo build --release`
- **Команда запуска сервера**: `sudo ./target/release/traffic-sentinel --mode server --config server.toml`
- **Команда запуска клиента**: `sudo ./target/release/traffic-sentinel --mode client --config client.toml`

---

## P0.1: Project init + scaffolding

### Проверка: компиляция с минимальной структурой

```bash
cargo check
cargo build --release
```

**Ожидаемый результат**: бинарник `target/release/traffic-sentinel` создан.

### Проверка: парсинг аргументов CLI

```bash
./target/release/traffic-sentinel --help
```

**Ожидаемый результат**: вывод help с `--mode`, `--config`.

```bash
./target/release/traffic-sentinel --mode client
./target/release/traffic-sentinel --mode server
```

**Ожидаемый результат**: процесс запускается (может упасть с ошибкой конфига — это нормально на P0.1).

### Проверка: config loading

Создать файл `test.toml`:
```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400
```

Создать раннер, который выводит распаршенный `Config` (или добавить `--dump-config`):
```bash
./target/release/traffic-sentinel --config test.toml --dump-config
```

**Ожидаемый результат**: структура Config выведена в stdout.

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Нет флага `--mode` | `./traffic-sentinel` | Ошибка: required argument |
| Невалидный `--mode` | `./traffic-sentinel --mode wat` | Ошибка: invalid value |
| Конфиг не существует | `./traffic-sentinel --mode client --config /nonexistent.toml` | Ошибка: file not found |
| Пустой конфиг | `cat /dev/null > empty.toml; ./traffic-sentinel --mode client --config empty.toml` | Ошибка: missing fields |

---

## P0.2: TUN interface

### Проверка: создание TUN-интерфейса

Написать тестовый раннер на базе `src/tun.rs`, который создаёт TUN и 5 секунд держит.

```bash
sudo ./target/release/test_tun_create
# в другом окне:
ip addr show ts0
ip link show ts0
```

**Ожидаемый результат**:
```
5: ts0: <NO-CARRIER,POINTOPOINT,MULTICAST,NO-ARP,UP> mtu 1400 qdisc fq_codel state DOWN mode DEFAULT group default qlen 500
    inet 10.0.0.2/30 scope global ts0
```

### Проверка: чтение из TUN (ping сам себе)

- TUN создан с IP 10.0.0.2/30
- TUN reader читает пакеты из интерфейса
- В другом окне: `ping 10.0.0.1 -c 3`

**Ожидаемый результат**: reader получает 3 ICMP Echo Request пакета (первые 20 байт — IP header: protocol=1).

*Этот тест перенесён в P1.3 (Server forwarder) — требует bidirectional трафика.*

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Двойной create | Создать TUN с тем же именем | Ошибка: `io::Error` (EADDRINUSE-like) |
| PID file cleanup | Создать TUN, убить процесс (SIGKILL), пересоздать | TUN создаётся заново (старый удаляется ядром) |
| MTU границы | MTU = 576 | TUN создаётся, `ip link show` показывает MTU 576 |
| TUN без IP | Вызвать `create_tun` без назначения IP | TUN существует, но без `inet` — пакеты не маршрутизируются |
| Не-root запуск | `./traffic-sentinel` без sudo | Ошибка: `PermissionDenied` (Cannot create TUN) |

### Проверка: удаление TUN (cleanup)

- Написать раннер, который создаёт TUN, удаляет, проверяет `ip link show ts0`

**Ожидаемый результат**: интерфейс исчезает.

---

## P0.3: Crypto engine

### Проверка: encrypt → decrypt roundtrip (известный ключ)

- Фиксированный ключ: `[0u8; 32]`
- Фиксированный nonce: `[0u8; 24]`
- Plaintext: произвольный массив байт (1, 16, 64, 1400, 1500, 65535 байт)
- `ciphertext = encrypt(nonce, plaintext)` → `decrypted = decrypt(nonce, ciphertext)`

**Ожидаемый результат**: `decrypted == plaintext` для всех размеров.

### Проверка: decrypt с неправильным nonce

- Ключ K, nonce N1, plaintext P → ciphertext C
- `decrypt(K, N2, C)` где N2 ≠ N1

**Ожидаемый результат**: ошибка аутентификации (AEAD tag mismatch).

### Проверка: decrypt с неправильным ключом

- Ключ K1, nonce N, plaintext P → ciphertext C
- `decrypt(K2, N, C)` где K2 ≠ K1

**Ожидаемый результат**: ошибка аутентификации.

### Проверка: повреждённый ciphertext

- Флипнуть 1 бит в ciphertext → decrypt

**Ожидаемый результат**: ошибка аутентификации.

### Проверка: пустой plaintext

- `encrypt(nonce, &[])` → ciphertext (только AEAD tag)

**Ожидаемый результат**: `decrypt(nonce, ciphertext)` → пустой `Vec<u8>`.

### Проверка: nonce генерация уникальна

- Вызвать `generate_nonce()` 10^5 раз
- Проверить, что все nonce уникальны

**Ожидаемый результат**: 0 коллизий.

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Nonce размер | Передать `&[0u8; 12]` вместо 24 | Compile error или panic в рантайме |
| Ключ все нули | `key = [0u8; 32]` | Шифрование работает (ключ валидный) |
| Plaintext > MTU*100 | 100000 байт | encrypt/decrypt работают (на уровне crypto нет лимита) |
| Nonce все нули | Несколько раз encrypt с одним nonce | ОК — nonce может повторяться между вызовами (но не рекомендуется) |

---

## P0.4: Packet protocol framing

### Проверка: encode → decode roundtrip

```rust
let frame = Frame { nonce: [1u8; 24], seq: 42, flags: 0x01, payload: vec![0xAA; 100] };
let encoded = encode(&frame);
let decoded = decode(&encoded).unwrap();
assert_eq!(frame.nonce, decoded.nonce);
assert_eq!(frame.seq, decoded.seq);
assert_eq!(frame.flags, decoded.flags);
assert_eq!(frame.payload, decoded.payload);
```

### Проверка: формат заголовка

- `encode` с nonce = `[0x01; 24]`, seq = `0xDEADBEEF`, flags = `0xAB`, payload = `[0x42; 10]`
- Проверить первые 2 байта: `(24 + 4 + 1 + 10) = 39 = 0x0027` (BE)
- Проверить байты 2-25: `[0x01; 24]`
- Проверить байты 26-29: `0xDE, 0xAD, 0xBE, 0xEF`
- Проверить байт 30: `0xAB`
- Проверить байты 31+: `[0x42; 10]`

### Проверка: decode невалидных данных

| Сценарий | Вход | Ожидаемый результат |
|----------|------|---------------------|
| Слишком короткий | `vec![0x00, 0x01]` | Ошибка: `Error::UnexpectedEof` |
| Length = 0 | `vec![0x00, 0x00]` | Ошибка: `Error::ZeroLength` |
| Length > MTU*2 | `vec![0xFF, 0xFF]` | Ошибка: `Error::FrameTooLarge` |
| Гигантский length | `vec![0x7F, 0xFF, ...]` (32767) | Ошибка: `Error::FrameTooLarge` если > лимита |
| Мусор | `vec![0xDE, 0xAD, 0xBE, 0xEF, ...]` | decode length, затем пытается читать — ошибка |

### Проверка: граничные размеры payload

| Размер payload | Ожидаемый encoded размер | Комментарий |
|----------------|--------------------------|-------------|
| 0 | 31 (header только) | flags без payload |
| 1 | 32 | Минимальный пакет |
| 1400 | 1431 | Максимальный MTU-пакет |
| 65535 | 65566 | Максимальный IPv4 пакет |

---

## P0.5: TCP transport

### Проверка: connect + listen

- Запустить listener на порту 0 (random port от ОС)
- Получить `local_addr()`
- Запустить connect к этому адресу

**Ожидаемый результат**: соединение установлено.

### Проверка: write_frame + read_frame

- Listener: `write_frame(data)` → Client: `read_frame()` → сравнить

```rust
let data = vec![0xAB; 1000];
listener.write_frame(&data).await?;
let received = client.read_frame().await?;
assert_eq!(data, received);
```

### Проверка: фрейминг (границы сообщений)

- Listener: `write_frame(msg1)`, `write_frame(msg2)` (без задержки)
- Client: `read_frame()`, `read_frame()`

**Ожидаемый результат**: msg1 и msg2 получены раздельно (не склеены).

### Проверка: TCP_NODELAY

- Прочитать `socket2::SockRef::from(&stream).nodelay()?`

**Ожидаемый результат**: `true`.

### Проверка: большие пакеты

- Отправить 100 фреймов по 65535 байт payload подряд
- Проверить, что все получены без потерь и в правильном порядке

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Подключение к несуществующему порту | `connect("127.0.0.1:1")` | `io::Error::ConnectionRefused` |
| Подключение с таймаутом | `connect("10.255.255.1:8443")` с 3s timeout | `io::Error::TimedOut` |
| Разрыв соединения (write) | Client отключается, Server пишет | `io::Error::BrokenPipe` / `ConnectionReset` |
| Разрыв соединения (read) | Client отключается, Server читает | `Ok(0)` если EOF или `ConnectionReset` |
| Частичная запись | Протестировать малый буфер записи | Все данные доставлены (write_all семантика) |
| Конкурентный read/write | Писать и читать из разных задач | Данные приходят в правильном порядке |

---

## P0.6: Client pipeline

### Проверка: TUN → encrypt → TCP (однонаправленно)

- Запустить server listener (отдельный процесс или тестовый хендлер)
- Запустить client pipeline
- В TUN интерфейс клиента: `ping 10.0.0.1 -c 1`

**Ожидаемый результат**:
1. TUN читает ICMP-пакет
2. Crypto.encrypt с random nonce
3. Protocol.encode → TCP write
4. На стороне сервера (тест): raw TCP data прочитана
5. Первые 2 байта raw TCP данных = длина (не 0)
6. Проверить, что raw TCP stream НЕ содержит байта `0x08` (ICMP type) — пакет зашифрован

*Этот тест перенесён в P1.3 (Server forwarder) — требует bidirectional трафика.*

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Остановка TCP (сервер умер) | kill сервер | Ошибка в pipeline, процесс завершается |
| TUN buffer полон | Писать быстрее, чем читать TUN | Block (sequential pipeline) — естественная backpressure |
| Пустой TUN (idle) | Нет трафика | Обе задачи висят на `await` — CPU 0% |
| TCP write датаграмма > MTU | IPv4 пакет 65535 байт | encrypt(1400 байт) = 24+16+1400 = 1440 в одном frame |

---

## P0.7: Server pipeline

### Проверка: приём TCP → decrypt → лог

- Тестовый отправитель посылает зашифрованный пакет
- Server pipeline принимает, дешифрует
- Логирование: `DEBUG Decrypted packet: IP protocol={}, len={}`

**Ожидаемый результат**: в логе виден ICMP (protocol=1), UDP (17) или TCP (6).

### Проверка: корректный decrypt нескольких пакетов

- Отправить 1000 зашифрованных пакетов с разными nonce

**Ожидаемый результат**: все дешифрованы, ни одного AEAD error.

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| Битый пакет в середине | Вставить повреждённый frame | Error в decode/decrypt, соединение разрывается (P0 behaviour) |
| Пустой frame | frame с flags=0x00, payload=0 | decode OK, decrypt OK → 0 байт → не пишем в TUN |
| TCP connect без данных | Открыть TCP, не слать данные | Server висит на read_frame — timeout? |
| Два клиента | Два TCP connect | В P0 только одно соединение — второй rejected или ignored |

---

## P0.8: main.rs dispatch

### Проверка: `--mode client` запускает client pipeline

```bash
sudo ./target/release/traffic-sentinel --mode client --config client.toml &
```

- `ip link show ts0` — интерфейс существует
- `ss -tpn | grep 8443` — TCP соединение есть

### Проверка: `--mode server` запускает server pipeline

```bash
sudo ./target/release/traffic-sentinel --mode server --config server.toml &
```

- `ss -tlnp | grep 8443` — слушает порт

### Проверка: оба процесса работают вместе

- Сервер запущен
- Клиент запущен
- `ping 10.0.0.1 -c 3` с клиентской машины

**Ожидаемый результат**: пинг не отвечает (нет обратного трафика в P0), но сервер логирует ICMP.

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| --mode без --config | `--mode client` | Ошибка: missing --config |
| --config server.toml + --mode client | Разные секции | Ошибка: missing [client] section |
| --config client.toml + --mode server | Разные секции | Ошибка: missing [server] section |
| Запуск без прав | `./traffic-sentinel` (без sudo) | Ошибка: permission denied |

---

## P0.9: Loopback test (интеграционный)

### Топология

```
[Client Machine]
  TUN ts0: 10.0.0.2/30
        |
  traffic-sentinel --mode client
        |
  TCP :8443 (encrypted)
        |
  traffic-sentinel --mode server  (localhost)
  (decrypt & log)
```

### Сценарий 1: Ping через loopback

```bash
# Terminal 1: server
sudo ./target/release/traffic-sentinel --mode server --config server.toml

# Terminal 2: client
sudo ./target/release/traffic-sentinel --mode client --config client.toml

# Terminal 3: send traffic
ping 10.0.0.1 -c 5
```

**Критерии успеха**:
1. Server stdout/log показывает: `DEBUG Decrypted packet: protocol=1, len=84` (ICMP Echo Request)
2. `tcpdump -i lo -X port 8443` показывает зашифрованные данные:
   - Нет открытого текста (не видно `ICMP`, `echo request` в ASCII)
   - Данные выглядят как случайные байты
3. Все 5 ICMP-пакетов приняты и дешифрованы

### Сценарий 2: Множественные пакеты

```bash
ping -f 10.0.0.1 -c 100    # flood ping
```

**Ожидаемый результат**: все 100 пакетов дешифрованы, ни одной ошибки AEAD. Задержка между пакетами не растёт (sequential pipeline не забивается).

### Сценарий 3: UDP через TUN

```bash
# send UDP packet to TUN network
echo "hello" | nc -u -w1 10.0.0.1 9999
```

**Ожидаемый результат**: сервер логирует `protocol=17, len=<size>`.

### Сценарий 4: Разные размеры пакетов

```bash
ping -s 100 -c 3 10.0.0.1   # 100 bytes payload → ~128 total
ping -s 1000 -c 3 10.0.0.1  # 1000 → ~1028
ping -s 1372 -c 3 10.0.0.1  # 1372 → 1400 (MTU)
ping -s 1373 -c 3 10.0.0.1  # 1373 → 1401 (fragmented by kernel)
```

**Ожидаемый результат**:
- Пакеты до MTU приходят как один фрейм
- Пакеты больше MTU фрагментируются ядром до попадания в TUN
- Каждый фрагмент отдельно шифруется и отправляется
- На сервере все фрагменты дешифрованы

### Сценарий 5: Длительный прогон (soak test)

```bash
# background ping for 60 seconds
ping 10.0.0.1 > /dev/null &
PING_PID=$!
sleep 60
kill $PING_PID
```

**Ожидаемый результат**:
- За 60 секунд ~60 пакетов (или ~600 с -i 0.1)
- Все дешифрованы
- CPU < 5% на idle (когда пинг не идёт)
- Memory стабильна (нет утечки)

### Сценарий 6: Остановка сервера

- Клиент работает, трафик идёт
- Убить сервер (Ctrl+C или SIGTERM)
- Клиент обнаруживает разрыв TCP (ожидаемое поведение в P0 — exit)

**Ожидаемый результат**: клиент завершается с ошибкой "Connection lost".

### Сценарий 7: Проверка на отсутствие утечки TUN

- Запустить клиент
- Убить клиента SIGKILL (kill -9)
- `ip link show ts0`

**Ожидаемый результат**: TUN интерфейс не существует (ядро очищает при закрытии fd).

---

## Итоговый чеклист P0

- [x] P0.1: `cargo build --release` успешен, CLI парсит аргументы
- [x] P0.2: TUN интерфейс создаётся, пакеты читаются (запись → ответ → перенесено в P1.3)
- [x] P0.3: Crypto: encrypt→decrypt roundtrip, неверный ключ → ошибка
- [x] P0.4: Protocol: encode→decode roundtrip, невалидные данные → ошибка
- [x] P0.5: TCP transport: connect/listen, write/read frame, фрейминг
- [x] P0.6: Client pipeline: TUN→encrypt→TCP (TCP→decrypt→TUN → перенесено в P1.3)
- [x] P0.7: Server pipeline: TCP→decrypt→log
- [x] P0.8: main dispatch: `--mode client` и `--mode server` работают
- [x] P0.9: Loopback: ping проходит через encrypt/decrypt, TCP не содержит plaintext
