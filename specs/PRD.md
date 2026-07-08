# PRD: Локальный форвардер трафика (Traffic Sentinel)

## 1. Overview

**Traffic Sentinel** — Rust-приложение, которое перехватывает весь исходящий сетевой трафик с локальной машины и перенаправляет его на внешний сервер (шлюз/прокси). Режим работы — всегда-on форвардер, цель — обеспечить полную маршрутизацию трафика через внешнюю точку без участия пользователя.

## 2. Problem Statement

Стандартные VPN-решения (OpenVPN, WireGuard, Tailscale) требуют установки клиента, конфигурации интерфейсов, часто работают на L3 и не дают гибкости в перехвате на уровне пакетов. Нужно лёгкое (resource-light), однобинарное средство на Rust, которое:

- перехватывает **весь** трафик (TCP, UDP, ICMP) на локальной машине;
- направляет его на внешний сервер-шлюз;
- работает на Linux (и, в перспективе, macOS/Windows).

## 3. Goals

- G1. Перехват всех исходящих пакетов L3 (IPv4) на локальной машине.
- G2. Режим **клиент**: инкапсуляция и отправка трафика на сервер-шлюз.
- G3. Режим **сервер**: приём трафика от клиента, дешифровка, форвард в интернет и обратная отправка ответов клиенту.
- G4. Минимальные задержки и накладные расходы (< 5% CPU на idle).
- G5. Единый статический бинарник без внешних зависимостей (musl target).
- G6. Сквозное шифрование (end-to-end) всего туннельного трафика (ChaCha20-Poly1305).
- G7. Поддержка Linux и Windows.

## 4. Non-Goals

- NG1. GUI / TUI (только CLI с минимальным конфигом).
- NG2. Split-tunneling / exclusion rules в v1.
- NG3. Встроенный DNS-сервер или блокировка рекламы.
- NG4. Работа как system-level service (systemd unit — в будущем, не в v1).
- NG5. Поддержка IPv6 в v1.

## 5. Functional Requirements

| ID | Requirement | Priority |
|---|---|---|
| FR1 | При старте приложение создаёт виртуальный интерфейс (TUN) и назначает ему IP. | P0 |
| FR2 | Весь исходящий трафик маршрутизируется через TUN-интерфейс (via default route). | P0 |
| FR3 | Пакеты из TUN-интерфейса читаются, инкапсулируются в пользовательский протокол и отправляются на удалённый сервер. | P0 |
| FR4 | Ответные пакеты от сервера деинкапсулируются и записываются обратно в TUN. | P0 |
| FR5 | Приложение корректно обрабатывает сигналы SIGTERM/SIGINT: восстанавливает оригинальную таблицу маршрутизации, удаляет TUN-интерфейс. | P0 |
| FR6 | Единый бинарник с двумя режимами: `--mode client` (форвард трафика на сервер) и `--mode server` (приём трафика, форвард в интернет). | P0 |
| FR7 | Конфигурация через TOML-файл: секции `[tunnel]`, `[server]` (listen, опциональный tun_subnet), `[client]` (remote, reconnect/heartbeat). Параметры TUN-подсети (tun_ip, tun_netmask, gateway) автоматизированы — сервер по умолчанию использует `10.0.0.0/24`, клиент получает IP автоматически. | P1 |
| FR8 | Логирование уровня INFO/DEBUG в stdout/stderr с метками времени. | P1 |
| FR9 | Транспорт туннеля — UDP (датаграммы с encrypted фреймами). Надёжность возлагается на внутренние протоколы (inner TCP), туннель не добавляет второй уровень retransmission. | P3 |
| FR10 | Keepalive / heartbeat между клиентом и сервером для детекта разрыва TCP-соединения. | P1 |
| FR11 | Все инкапсулированные пакеты шифруются потоковым шифром XChaCha20-Poly1305 (каждый пакет — отдельный AEAD nonce). | P0 |
| FR12 | Ключ шифрования — Pre-Shared Key (PSK) из конфига (hex/base64), 256 бит. | P0 |
| FR13 | На старте клиент и сервер обмениваются эфемерными ключами через X25519 ECDH, подписанными PSK (hybrid key exchange). Для multi-client сервер назначает клиенту уникальный IP в handshake. | P1 |

## 6. Non-Functional Requirements

| ID | Requirement | Target |
|---|---|---|
| NFR1 | Производительность: пропускная способность | > 80% от raw-пропускной способности сети |
| NFR2 | Потребление памяти | < 50 MB RSS в idle |
| NFR3 | CPU utilization | < 5% на idle, < 30% при насыщении |
| NFR4 | Время восстановления после разрыва соединения | < 5 секунд |
| NFR5 | Single binary | статическая сборка (x86_64-unknown-linux-musl) |
| NFR6 | Safety: отсутствие unsafe кода (кроме FFI к libc/ioctl для TUN) | минимизировать unsafe |
| NFR7 | Криптография: только audited крейты (libsodium-binding или чистая реализация на Rust) | запрещены self-implemented примитивы |

## 7. High-Level Architecture

```
+------------------+       +------------------+       +------------------+
|   Local Machine  |       |    Internet      |       |    Gateway       |
|                  |       |                  |       |    Server        |
|  App (браузер и  |       |                  |       |   (шлюз/receiver)|
|  т.д.)           |       |                  |       |                  |
|    |             |       |  [encrypted]     |       |                  |
|    v             |       |  (XChaCha20-     |       |                  |
| [TUN interface]  |       |   Poly1305)      |       |                  |
|    | (raw IP)    |       |  over UDP        |       |                  |
|    v             |       |                  |       |                  |
| [Traffic         | ====> | ==============> | ====> | [Decrypt → Decap |
|  Sentinel]       |       |                  |       |  → Forward]     |
|    ^             |       |                  |       |    |             |
|    |<============ | <=== | =============== | <=== | [Encrypt → Send] |
|    |             |       |                  |       |                  |
+------------------+       +------------------+       +------------------+
```

Единый бинарник, режим выбирается аргументом `--mode client|server`.

**Компоненты (общие для обоих режимов):**

1. **CLI Parser** — `--mode`, `--config`, `--help`.
2. **Config Loader** — чтение TOML, валидация.
3. **Key Exchange** — X25519 ECDH поверх PSK, handshake при старте сессии.
4. **Crypto Engine** — XChaCha20-Poly1305 Encrypt/Decrypt, nonce-счётчик.
5. **Transport** — UDP-сокет (tokio UdpSocket), каждый encrypted фрейм — отдельная датаграмма. Handshake поверх UDP с retransmission.
6. **Packet Protocol** — Encapsulator / Decapsulator (заголовок: длина, seq, flags).
7. **Signal Handler** — graceful shutdown.
8. **Logger** — `tracing`, structured.

**Компоненты режима `client`:**

| Компонент | Назначение |
|---|---|
| TUN Manager | Создание/удаление TUN-интерфейса `ts0`, IP, маршруты |
| Packet Reader | Асинхронное чтение сырых IP-пакетов из TUN |
| Route Manager | Сохранение оригинальных маршрутов, установка default route на TUN, восстановление при остановке |
| Writer | Запись деинкапсулированных/расшифрованных пакетов в TUN |

**Компоненты режима `server`:**

| Компонент | Назначение |
|---|---|
| UDP Listener | Приём UDP-датаграмм от клиента, деинкапсуляция фреймов |
| Forwarder | Отправка расшифрованных IP-пакетов в реальный интернет через сырой сокет (IP_HDRINCL) или через TUN-интерфейс сервера |
| Reverse Writer | Отправка зашифрованных ответных пакетов обратно клиенту |

## 8. Platform Support

| Platform | v1 | Примечание |
|---|---|---|---|
| Linux (x86_64) | ✅ | TUN через tun-rs, маршруты через rtnetlink |
| Windows (x86_64) | ✅ | TUN через wintun (tun-rs), маршруты через WinAPI (iphlpapi) |
| Linux (aarch64) | ⬜ | После v1 |
| macOS | ⬜ | TUN через utun, маршруты через route(8) |

## 9. Technology Stack

- **Язык**: Rust (edition 2024)
- **Runtime**: tokio (async I/O)
- **TUN**: `tun-rs` (v2, feature `async`)
- **Транспорт**: tokio UdpSocket (UDP)
- **Маршруты**: парсинг `/proc/net/route` + `netlink` (rtnetlink) или вызов `ip route`
- **Логирование**: `tracing` (structured, async-friendly)
- **Конфиг**: `toml` + `serde`, секции `[client]` / `[server]` под общим `[tunnel]`
- **Криптография**: `sodiumoxide` (libsodium bindings) или `xchacha20poly1305` + `x25519-dalek` (pure Rust)
- **Сборка**: `cargo-zigbuild` для musl

## 10. Scenarios

### 10.1 Успешный запуск (клиент)

1. Пользователь запускает `traffic-sentinel --mode client --config client.toml`.
2. Приложение проверяет права (CAP_NET_ADMIN / root).
3. Создаётся TUN-интерфейс `ts0` с IP `10.0.0.2/24` (default, сервер может переопределить через handshake).
4. Сохраняется текущий default route.
5. Устанавливается default route через `10.0.0.1` (TUN-пир — адрес сервера в туннеле).
6. Клиент открывает UDP-сокет к `server-ip:8443`, выполняет handshake (ECDH + PSK) поверх UDP с retransmission.
7. После handshake — поток датаграмм: TUN → Reader → Encrypt → Encapsulate → UDP → Server.
8. При получении сигнала SIGTERM — восстановление маршрутов, удаление TUN, exit 0.

### 10.2 Успешный запуск (сервер)

1. Пользователь запускает `traffic-sentinel --mode server --config server.toml`.
2. Приложение проверяет права (CAP_NET_ADMIN / root).
3. Сервер создаёт TUN-интерфейс `ts0` с IP `10.0.0.1/24` (первый IP из tun_subnet) и открывает UDP-сокет на `0.0.0.0:8443`, ожидает handshake от клиента.
4. После handshake — сервер принимает зашифрованные датаграммы, дешифрует, форвардит в интернет.
5. Ответные пакеты из интернета возвращаются через TUN сервера, шифруются, отправляются клиенту.
6. При SIGTERM — закрытие сокетов, удаление TUN, восстановление маршрутов, exit 0.

### 10.3 Разрыв соединения

1. Связь между клиентом и сервером теряется (heartbeat timeout / ICMP unreachable).
2. Клиент детектит разрыв (отсутствие PONG в течение heartbeat_timeout).
3. Пытается переподключиться (backoff: 1s, 2s, 4s, ... max 30s) с повторным handshake по UDP.
4. При успешном переподключении — поток возобновляется.
5. Если reconnect исчерпан — graceful shutdown (восстановление маршрутов, удаление TUN).

## 11. Security Considerations

- Приложение требует CAP_NET_ADMIN (или root) для создания TUN и изменения маршрутов.
- **Шифрование**: XChaCha20-Poly1305 — потоковый шифр с аутентификацией (AEAD). Nonce — монотонный счётчик на поток, начальное значение — X25519 ECDH.
- **Key Exchange**: hybrid — PSK (256-bit из конфига) + X25519 эфемерные ключи. Защита от MitM даже при скомпрометированном PSK (PFS).
- **PSK хранение**: только в config-файле (0400 perms). В будущем — env var или secrets manager.
- **Replay protection**: монотонный nonce + таймстемп в handshake отбрасывает старые сессии. Для пакетов — unique nonce в рамках сессии.
- **Параметры для будущего**: пост-квантовая криптография (Kyber + X25519 hybrid).
- **Server-side**: обязательная аутентификация через PSK + ECDH. Без неё — сервер отклоняет handshake.

## 12. Open Questions / Risks

| # | Вопрос | Статус |
|---|---|---|
| Q1 | ~~**Какой протокол инкапсуляции использовать?**~~ → **Решено**: UDP-транспорт (датаграммы). Фрейм (nonce+seq+flags+encrypted payload) отправляется как UDP-датаграмма. TCP-over-TCP meltdown не возникает — inner TCP сам обрабатывает потери. Надёжность для данных внутри туннеля обеспечивается inner TCP. Для handshake поверх UDP — retransmission на стороне клиента (timeout + retry). | **Решено** |
| Q2 | ~~**Как обрабатывать ICMP?**~~ → **Решено**: пропускаем "as-is" в туннеле (шифруется и передаётся как любой другой IP-пакет). | **Решено** |
| Q3 | ~~**MTU — как выставлять?**~~ → **Решено**: TUN MTU = 1400 (default, конфигурируется). MSS clamp = 1360 опционально. Overhead: Poly1305 (16) + nonce (24) + seq+flags (5) + запас ≈ 45. PMTUD через ICMP от TUN. | **Решено** |
| Q4 | **IPv6 поддержка?** Нужна ли в v1? Значительно усложняет Route Manager (дефолтные маршруты IPv4 + IPv6). | **Предложено отложить** |
| Q5 | ~~**Шифрование?**~~ → **Решено**: XChaCha20-Poly1305 + X25519 ECDH + PSK. | **Решено** |
| Q6 | ~~**Multi-client / server-side design?**~~ → **Решено**: единый бинарник, `--mode server` и `--mode client`. | **Решено** |
| Q7 | **Права доступа — решение**: все платформы — запуск от администратора/root. Linux — `sudo` (uid != 0 → exit). Windows — UAC манифест, wintun драйвер требует админ. | **Решено** |
| Q8 | **Как детектить, что трафик на gateway не должен идти через TUN (чтобы избежать loop)?** Нужно добавить исключающий маршрут (route via current default gateway) для IP-адреса gateway-сервера. | **OK — стандартная практика** |
| Q9 | ~~**Крейт для TUN?**~~ → **Решено**: `tun-rs` (v2). | **Решено** |
| Q10 | ~~**Многопоточность / очереди.**~~ → **Решено**: вариант A — две tokio-задачи (`tun_to_tcp`, `tcp_to_tun`), sequential pipeline, без каналов, tokio multi_thread. | **Решено** |
| Q11 | ~~**Поведение при переполнении буфера.**~~ → **Решено**: Block. В sequential pipeline (Q10) это поведение по умолчанию — await на write тормозит read из TUN. | **Решено** |
| Q12 | ~~**Nonce — структура.**~~ → **Решено**: вариант A — полностью случайный 24-байтовый nonce на каждый пакет (CSPRNG, `getrandom`). Nonce пишется в заголовок пакета. Overhead: 24 байта на пакет. | **Решено** |
| Q13 | ~~**Key rotation.**~~ → **Не делаем.** Сессионный ключ живёт до перезапуска. | **Отклонено** |
| Q14 | **Cipher — XChaCha20-Poly1305 vs AES-256-GCM.** XChaCha20 — software-friendly, const-time, нет аппаратной зависимости. AES-GCM требует AES-NI и сложнее в const-time реализации. XChaCha20 — предпочтительный выбор для Rust. | **Предложено XChaCha20-Poly1305** |
| Q15 | ~~**Автоматизация TUN-параметров (tun_ip, tun_netmask, gateway).**~~ → **Решено**: параметры убраны из конфига. Сервер по умолчанию использует подсеть `10.0.0.0/24`, свой IP — `10.0.0.1`. Клиент получает `10.0.0.2`. Серверу можно указать `tun_subnet` для переопределения. Для multi-client сервер будет выдавать IP через handshake. | **Решено** |

## 13. Milestones / Phases

| Phase | Scope | Estimate |
|---|---|---|
| **P0 — Proof of Concept** | `--mode client` + `--mode server`. TUN interface + чтение пакетов + XChaCha20 шифрование + TCP. Loopback test: пишем в TUN клиента → читаем на сервере. | 1 неделя |
| **P1 — Minimal Viable** | Route management (сохранение/восстановление), ECDH handshake, полный шифрованный цикл: клиент↔сервер, TCP через туннель работает. | 2 недели |
| **P2 — Production Ready** | Graceful shutdown, reconnection, heartbeat, конфиг, логирование, тесты, error handling. | 1 неделя |
| **P3 — Hardening** | UDP transport (fix TCP-over-TCP meltdown), автоматизация TUN-параметров (убрать из конфига), performance tuning, security review, IPv6 (если решено), CI/CD, пост-квантовая гибридная KEX. | 2 недели |
| **P4 — Multi-client** | Сервер обслуживает несколько клиентов одновременно. IP-пул с аллокацией при handshake. Расширенный handshake (server_hello → 69 байт: pubkey+hmac+client_ip+netmask). Демультиплексирование обратного трафика через shared TUN reader. | 1 неделя |
