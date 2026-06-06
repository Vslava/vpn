# Verification Plan: Phase P1 — Minimal Viable

## Общая информация

- **Цель**: Проверить полный шифрованный bidirectional forwarding с route management и ECDH handshake.
- **Среда**: Linux x86_64, требуется root (sudo).
- **Предусловия**: P0 полностью пройден, все модули существуют и протестированы.

---

## P1.1: Route management

### Проверка: save_default_route

- Записать текущий default route
- Вызвать `save_default_route()`
- Сравнить с `ip route show default`

```bash
# Текущий маршрут:
default via 192.168.1.1 dev wlp0s20f3 proto dhcp metric 600
```

**Ожидаемый результат**: структура `DefaultRoute` содержит gateway `192.168.1.1`, interface `wlp0s20f3`, metric `600`.

Результат: `[x]` — пройдено (интеграционный тест `test_save_default_route`).

### Проверка: save_default_route когда нет default route

- Временно удалить default route: `ip route del default`
- Вызвать `save_default_route()`

**Ожидаемый результат**: `Ok(None)` или специальный вариант `NoDefaultRoute` (не ошибка).

Результат: `[ ]` — не тестировалось (изменяет сеть во время теста).

### Проверка: set_default_tun_route

```bash
# Предусловие: TUN ts0 существует с IP 10.0.0.2/30
ip route show default
# → default via 192.168.1.1 dev wlp0s20f3

# Вызвать set_tun_route("10.0.0.1")
ip route show default
```

**Ожидаемый результат**:
```
default via 10.0.0.1 dev ts0
```

Результат: `[x]` — пройдено (через `test_tun_route_roundtrip`, маршрут переключен на ts0).

### Проверка: add_exclude_route

- Сервер расположен по адресу `1.2.3.4:8443`
- Вызвать `add_exclude_route("1.2.3.4")`

```bash
ip route show | grep 1.2.3.4
```

**Ожидаемый результат**:
```
1.2.3.4 via 192.168.1.1 dev wlp0s20f3
```
(маршрут к серверу идёт через оригинальный gateway, минуя TUN)

Результат: `[x]` — пройдено (для `8.8.8.8` в составе `test_tun_route_roundtrip`).

### Проверка: add_exclude_route для localhost

- Сервер на `127.0.0.1`
- Вызвать `add_exclude_route("127.0.0.1")`

**Ожидаемый результат**: маршрут `127.0.0.0/8 dev lo` уже существует — функция ничего не меняет (или добавляет /32, что избыточно, но не ломает).

Результат: `[x]` — функция не ломает локальную маршрутизацию (EEXIST игнорируется).

### Проверка: restore_route

```bash
# Установлен TUN route
ip route show default
# → default via 10.0.0.1 dev ts0

# Вызвать restore_route(saved_route)
ip route show default
```

**Ожидаемый результат**: маршрут восстановлен:
```
default via 192.168.1.1 dev wlp0s20f3 proto dhcp metric 600
```

Результат: `[x]` — пройдено (проверено `ip route show default` после теста).

### Проверка: restore_route при изначально отсутствующем default

- Удалить default, сохранить (NoDefaultRoute), вызвать restore

**Ожидаемый результат**: default route не появляется (no-op).

Результат: `[ ]` — не тестировалось.

### Проверка: race condition — трафик во время смены маршрута

- Непрерывный ping на внешний IP (8.8.8.8)
- Быстро переключить default на TUN и обратно

**Ожидаемый результат**: не более 1-2 потерянных пакетов (атомарность `ip route replace`).

Результат: `[ ]` — не тестировалось.

### Edge cases

| Сценарий | Действие | Ожидаемый результат | Статус |
|----------|----------|---------------------|--------|
| Двойной вызов set_tun_route | Вызвать дважды | Второй раз — no-op (EEXIST) | `[x]` |
| restore_route без save | Вызвать restore с заведомо неверными данными | Маршрут не добавляется / ошибка | `[x]` (EEXIST) |
| Несколько default routes | `ip route add default via 10.0.0.1 metric 200` | save_default_route сохраняет primary (с меньшей metric) | `[ ]` |
| TUN интерфейс удалён | set_tun_route после удаления TUN | `Error: "interface 'ts0' not found"` | `[ ]` |
| Маршрут к серверу через TUN (loop) | Не вызвать add_exclude_route | Пакеты к серверу попадают в TUN → шифрование → TCP → сервер — **образуется loop**, трафик не доходит | `[ ]` |

### Специальный тест: loop prevention

```bash
# 1. Сохранить маршруты
# 2. Установить TUN default
# 3. НЕ вызывать add_exclude_route для IP сервера
# 4. Попытаться подключиться к серверу (curl http://server-ip:8443)
```

**Ожидаемый результат**: соединение не устанавливается (таймаут), трафик зациклен.

```bash
# 5. Теперь с add_exclude_route
# 6. Попытаться подключиться
```

**Ожидаемый результат**: соединение устанавливается.

Результат: `[ ]` — не тестировалось (требует двух машин).

---

## P1.2: ECDH handshake

### Проверка: клиент и сервер генерируют одинаковый ключ

- Тест: два процесса в памяти (без сети):
  - Создать виртуальный TCP-дуплекс (в памяти loopback)
  - Запустить `client_handshake(stream_a, psk)` и `server_handshake(stream_b, psk)` в двух задачах
  - Сравнить session_key

```rust
let psk = [0xABu8; 32];
let (client_key, server_key) = tokio::join!(
    client_handshake(&mut stream_a, &psk),
    server_handshake(&mut stream_b, &psk),
);
assert_eq!(client_key, server_key);
```

### Проверка: handshake с неправильным PSK

- Client: PSK_A
- Server: PSK_B (PSK_A ≠ PSK_B)

**Ожидаемый результат**: Server возвращает `Error::HandshakeFailed` (HMAC mismatch). Соединение закрывается.

### Проверка: handshake с разными эфемерными ключами

- Повторить handshake 100 раз с разными эфемерными парами
- Каждый раз session key уникален (PFS — Perfect Forward Secrecy)

**Ожидаемый результат**: все 100 ключей различны.

### Проверка: replay handshake

- Записать client_hello (public_key + HMAC) от предыдущей сессии
- Воспроизвести его в новой сессии

**Ожидаемый результат**: handshake отклоняется (эфемерный ключ сервера изменился → итоговый shared secret другой). Если нет явного anti-replay — хотя бы уникальность ключа гарантирует безопасность.

### Проверка: длина PSK не 32 байта

- PSK = `[0xAB; 16]` (128 бит)

**Ожидаемый результат**: ошибка валидации на старте (PSK должен быть 256 бит).

### Проверка: таймаут handshake

- Открыть TCP-соединение, не слать handshake данные
- Ждать 10 секунд

**Ожидаемый результат**: сервер закрывает соединение по таймауту.

### Проверка: handshake поверх реального TCP

- Сервер слушает порт, клиент коннектится
- Handshake проходит, обе стороны получают ключ
- После handshake: если клиент шлёт данные, сервер decrypt с полученным ключом

**Ожидаемый результат**: данные расшифрованы.

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| PSK все нули | `[0u8; 32]` | Handshake работает (ключ валидный) |
| PSK все 0xFF | `[0xFF; 32]` | Handshake работает |
| Client отправляет > 32 байт public_key | Отправить 64 байта | Сервер читает только 32, остальное — следующий фрейм (no-op) |
| Server отправляет > 32 байт | Аналогично | Клиент читает 32, остальное игнорируется |
| HMAC подмена в пути | Изменить 1 байт HMAC | HMAC mismatch → Error::HandshakeFailed |
| Public key подмена в пути | Изменить 1 байт public_key | Сервер принимает, но итоговый shared secret у клиента и сервера разный — дальнейшие пакеты не расшифруются |

---

## P1.3: Server forwarder

> **Перенесено из P0**: 
> - P0.2 "Проверка: запись в TUN (ping reply)" — теперь возможна благодаря двустороннему трафику.
> - P0.6 "Проверка: TCP → decrypt → TUN (обратно)" — клиентский pipeline декодирует и пишет в TUN.

### Топология P1.3

```
[Client Machine]
  TUN ts0: 10.0.0.2/30
  default via 10.0.0.1 dev ts0
        |
  traffic-sentinel --mode client --remote server-ip:8443
        |
  TCP (encrypted) === Internet / LAN ===>
        |
  traffic-sentinel --mode server --listen 0.0.0.0:8443
  TUN ts0: 10.0.0.1/30 (server-side TUN)
  default via <server_gateway> (forwarding via kernel IP stack от server TUN)
        |
  [Server's network namespace]
        |
  [Internet] --- 8.8.8.8, example.com
```

### Проверка: server TUN создаётся

```bash
# Запустить сервер
sudo ./target/release/traffic-sentinel --mode server --config server.toml

# В другом окне:
ip addr show ts0   # на сервере
```

**Ожидаемый результат**: интерфейс `ts0` существует, IP 10.0.0.1/30.

### Проверка: сервер форвардит пакет (без ответа)

- На клиенте сгенерировать UDP пакет на внешний адрес
- Проверить, что сервер пишет его в свой TUN

**Тест**:
```bash
# На клиенте (трафик пойдёт через TUN из-за default route)
ping 8.8.8.8 -c 1 &

# На сервере, проверить что ICMP arrived:
tcpdump -i ts0 -c 1 icmp
```

**Ожидаемый результат**: ICMP Echo Request виден на server TUN ts0.

### Проверка: двусторонний трафик (ответ возвращается)

Для этого теста сервер должен:
1. Читать decrypted пакет
2. Писать его в server TUN ts0 (10.0.0.1)
3. Ядро сервера форвардит его через eth0 в интернет
4. Ответ приходит на сервер
5. Сервер читает ответ из TUN ts0
6. Шифрует и отправляет клиенту

**Включить IP forwarding на сервере**:
```bash
echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward
```

**Проверить**:
```bash
# Клиент
ping 8.8.8.8 -c 3
```

**Ожидаемый результат**: ping успешен (3 reply).

### Проверка: HTTP(S) трафик

```bash
# Клиент
curl -v https://example.com
```

**Ожидаемый результат**: страница загружается. Время загрузки — не более 2x от прямого соединения.

### Проверка: DNS (UDP :53)

```bash
# Клиент
nslookup google.com 8.8.8.8
# или
dig +short example.com @8.8.8.8
```

**Ожидаемый результат**: DNS-ответ получен.

### Проверка: ICMP типы

| Тип | Команда | Ожидаемый результат |
|-----|---------|---------------------|
| Echo Request/Reply | `ping 8.8.8.8` | Ответ получен |
| TTL exceeded | `ping -t 1 8.8.8.8` | `Time to live exceeded` от первого роутера |
| Fragmentation needed | `ping -M do -s 2000 8.8.8.8` | `Frag needed and DF set` (MTU 1400, так что ping > 1372 должен вызвать) |
| Destination unreachable | `ping 192.0.2.1` | `Destination Host Unreachable` |

### Edge cases

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| IP forwarding выключен | `sysctl net.ipv4.ip_forward=0` | Пакеты доходят до TUN, но не форвардятся в интернет (таймаут на клиенте) |
| Сервер не может создать TUN | TUN уже используется | Ошибка, процесс завершается |
| Пакет с source IP не из нашей подсети | Сфальсифицированный пакет | Сервер форвардит (на его стороне нет фильтрации source) |
| Ответный пакет > MTU | TCP MSS clamping | Пакет фрагментируется, каждый фрагмент приходит в TUN сервера |
| Server TUN buffer overflow | Писать быстрее, чем читать | Backpressure — sequential pipeline |
| Несколько клиентов | Второй клиент коннектится | Пока не поддерживается — rejected |
| NAT на клиенте | Клиент за NAT | TCP к серверу проходит (NAT-friendly) |
| ICMP redirect | Маршрутизация на клиенте | ICMP Redirect не доходит до клиента (шифрован) |

---

## P1.4: Full config loading

### Проверка: полный конфиг клиента

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[client]
remote = "1.2.3.4:8443"
tun_ip = "10.0.0.2"
tun_netmask = 30
```

- Загрузить, проверить поля:
  - `config.tunnel.psk` = 32 байта hex
  - `config.tunnel.mtu` = 1400
  - `config.client.remote` = `1.2.3.4:8443`
  - `config.client.tun_ip` = `10.0.0.2`

### Проверка: полный конфиг сервера

```toml
[tunnel]
psk = "deadbeefcafebabedeadbeefcafebabedeadbeefcafebabedeadbeefcafebabe"
mtu = 1400

[server]
listen = "0.0.0.0:8443"
tun_ip = "10.0.0.1"
tun_netmask = 30
```

### Проверка: валидация PSK

| PSK | Ожидаемый результат |
|-----|---------------------|
| `"abcd"` (короткий) | Ошибка: PSK must be 64 hex chars (32 bytes) |
| `"zzzz...zzzz"` (не hex) | Ошибка: invalid hex character |
| `"DEADBEEF...` (UPPER) | OK (case-insensitive hex parsing) |
| `""` (пустой) | Ошибка: missing field |
| `0xdeadbeef...` | Ошибка: invalid hex character ('x') |

### Проверка: валидация remote/listen

| Адрес | Ожидаемый результат |
|-------|---------------------|
| `"1.2.3.4"` (без порта) | Ошибка: missing port |
| `"1.2.3.4:99999"` (порт вне диапазона) | Ошибка: port out of range |
| `"not-an-ip:8443"` | Ошибка: invalid IP address |
| `"0.0.0.0:8443"` | OK |
| `"[::1]:8443"` | OK или ошибка (IPv6 не поддерживается) |

### Проверка: валидация MTU

| MTU | Ожидаемый результат |
|-----|---------------------|
| 0 | Ошибка: MTU must be >= 576 |
| 576 | OK (минимальный IPv4) |
| 1400 | OK |
| 1500 | OK |
| 9000 | OK |
| 65536 | Ошибка: MTU must be <= 65535 |

### Проверка: валидация tun_netmask

| mask | Ожидаемый результат |
|------|---------------------|
| 0 | Ошибка: mask must be 1-31 |
| 30 | OK |
| 32 | OK (point-to-point) |

### Проверка: взаимная валидация mode/config

- `--mode client` + config без `[client]` секции → ошибка
- `--mode server` + config без `[server]` секции → ошибка
- Config с обеими секциями + любой `--mode` → OK (используется соответствующая)

### Проверка: missing config file

```bash
./traffic-sentinel --mode client --config /nonexistent/path.toml
```

**Ожидаемый результат**: `Error: No such file or directory` или `ConfigNotFound`.

---

## P1.5: Wire into main.rs (интеграционный)

### Проверка: полный bootstrap sequence

```bash
# На сервере (предварительно создать server.toml)
sudo ./target/release/traffic-sentinel --mode server --config server.toml

# На клиенте (client.toml)
sudo ./target/release/traffic-sentinel --mode client --config client.toml
```

**Проверить последовательно**:
1. Лог сервера: `INFO Starting in server mode`
2. Лог сервера: `INFO Loading config from server.toml`
3. Лог сервера: `INFO Creating TUN interface ts0`
4. `ip addr show ts0` на сервере → 10.0.0.1/30
5. Лог сервера: `INFO Listening on 0.0.0.0:8443`
6. Лог клиента: `INFO Starting in client mode`
7. Лог клиента: `INFO Creating TUN interface ts0`
8. `ip addr show ts0` на клиенте → 10.0.0.2/30
9. Лог клиента: `INFO Connecting to 1.2.3.4:8443`
10. Лог клиента: `INFO TCP connected` → `Starting handshake...`
11. Лог клиента: `INFO Handshake complete, session key established`
12. Лог сервера: `INFO Client connected, handshake complete`
13. Проверить, что default route на клиенте → via 10.0.0.1 dev ts0
14. Лог клиента: `INFO Starting packet pipeline`

### Проверка: ping до интернета (полный цикл)

```bash
# На клиенте (когда оба процесса запущены)
ping 8.8.8.8 -c 5
```

**Ожидаемый результат**: 5 успешных ответов.

```bash
ping 8.8.8.8 -c 5 -i 0.1   # 50ms interval
```

**Ожидаемый результат**: без потерь, RTT незначительно выше прямого.

### Проверка: TCP трафик (пример)

```bash
# Несколько сессий
curl -s https://example.com | head -5
curl -s https://google.com | head -5
curl -s https://github.com | head -5
```

**Ожидаемый результат**:
- Все три сайта открываются
- Сертификаты валидны (нет MITM — трафик end-to-end encrypted, не расшифровывается на сервере)

### Проверка: UDP (DNS)

```bash
dig +short google.com @8.8.8.8
```

**Ожидаемый результат**: IP-адрес google.com.

### Проверка: скорость

```bash
# Прямое соединение (baseline)
iperf3 -c iperf.he.net -t 10 -J > /tmp/baseline.json

# Через туннель (на клиенте)
iperf3 -c iperf.he.net -t 10 -J > /tmp/tunnel.json
```

**Ожидаемый результат**: throughput > 80% от baseline.

### Проверка: остановка (Ctrl+C на клиенте)

```bash
# На клиенте Ctrl+C
```

**Ожидаемый результат**:
1. `INFO Restoring routes...`
2. `ip route show default` → оригинальный gateway (192.168.1.1)
3. `INFO Deleting TUN interface ts0`
4. `ip link show ts0` → не существует
5. `INFO Closing TCP connection`
6. `INFO Shutdown complete`

### Проверка: остановка (Ctrl+C на сервере)

```bash
# На сервере Ctrl+C
```

**Ожидаемый результат**:
1. Сервер: TUN удалён, TCP listener закрыт
2. Клиент: обнаруживает разрыв TCP → начинает reconnection (или exit в P1)

### Проверка: запуск когда уже есть default route через TUN

- Запустить клиент → он установит default через ts0
- Остановить клиент (Ctrl+C)
- Запустить снова

**Ожидаемый результат**: второй запуск работает корректно (save_default_route сохраняет оригинальный gateway, не ts0).

### Проверка: клиент без доступа к серверу

```bash
# client.toml: remote = "10.255.255.1:8443" (недоступный адрес)
sudo ./traffic-sentinel --mode client --config client.toml
```

**Ожидаемый результат**:
- TUN создан (так как это первый шаг)
- Route не изменён (так как мы не подключились к серверу — но тогда трафик не пойдёт в TUN)
- **Edge case**: лучше не менять маршруты до успешного handshake

### Проверка: откат при ошибке инициализации

| Сценарий | Действие | Ожидаемый результат |
|----------|----------|---------------------|
| TUN создан, но не могу подключиться | Недоступный сервер | TUN удалён, routes не изменялись → чистое состояние |
| TUN создан, route изменён, но handshake не прошёл | Неверный PSK | TUN удалён, routes восстановлены |
| TUN не создан | Нет прав | routes не изменялись → просто exit |

---

## Итоговый чеклист P1

- [x] P1.1: Route management — сохранение/восстановление default route, exclude route
- [ ] P1.2: ECDH handshake — matching session key, неверный PSK → error, PFS
- [ ] P1.3: Server forwarder — bidirectional трафик, ping/HTTP/UDP/DNS работают
- [ ] P1.4: Config loading — все поля валидируются, краевые случаи
- [ ] P1.5: Full integration — полный bootstrap, трафик, остановка, recovery
