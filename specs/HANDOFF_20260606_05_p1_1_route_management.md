# Handoff: P1.1 Route Management

## Summary

Реализован и верифицирован модуль управления маршрутами (`src/route.rs`) для фазы P1 проекта `traffic-sentinel`. Написаны 4 публичные функции (save/set/restore/add_exclude), интеграционные тесты (2 теста с sudo), 20 unit-тестов продолжают проходить. Решение по выбору `rtnetlink` (вместо `ip route` команд) зафиксировано в THOUGHTS.md (Q15).

## What Was Done

### P1.1 Implementation

- `src/route.rs` (184 строки) — модуль управления маршрутами на `rtnetlink`:
  - `save_default_route()` — запрос всех IPv4 маршрутов через netlink, фильтр по `destination_prefix_length == 0`, парсинг gateway/OIF/priority
  - `set_tun_route(tun_ifname, tun_gw)` — del всех default маршрутов, add via TUN
  - `add_exclude_route(server_ip, orig)` — add /32 маршрут к серверу через оригинальный gateway
  - `restore_route(orig)` — del default, add оригинального маршрута
  - `eexist_ok()` — все add операции игнорируют EEXIST (возникает когда DHCP client пере-добавляет маршрут между del и add)
  - `get_ifname()`/`get_ifindex()` — lookup имени/индекса интерфейса через `LinkAttribute::IfName`

### Dependencies

- Добавлены в `Cargo.toml`: `rtnetlink = "0.21"`, `netlink-packet-route = "0.30"`, `tokio-stream = "0.1"`

### Refactoring

- Создан `src/lib.rs` — все публичные модули переехали в библиотеку, чтобы интеграционные тесты (`tests/`) могли их импортировать
- `src/main.rs` — удалены `mod` declarations, вместо них `use traffic_sentinel::{client, config, ...}`

### Tests

- `tests/route_integration.rs` — 2 интеграционных теста:
  - `test_save_default_route` — read-only, сравнивает с `ip route show default`
  - `test_tun_route_roundtrip` — create TUN → set_tun_route → add_exclude_route → restore_route → drop TUN
- `tests/run_route_tests.sh` — bash-скрипт для ручных тестов с sudo (вкл. edge cases)

### Verification against VERIFICATION_P1.md

- `save_default_route`: `[x]` — gateway `192.168.1.1`, iface `wlp0s20f3`, metric 600
- `set_default_tun_route`: `[x]` — маршрут переключён на ts0
- `add_exclude_route`: `[x]` — /32 маршрут через оригинальный gateway
- `restore_route`: `[x]` — маршрут восстановлен, TUN ts0 удалён
- `add_exclude_route` localhost: `[x]` — EEXIST не ломает
- Edge cases: double set_tun_route, restore без save — `[x]` (EEXIST tolerant)
- Остальное: `[ ]` — требует `ip route del default` или двух машин

## Decisions Made

- **Q15 (THOUGHTS.md)**: Route management через `rtnetlink` вместо `ip route` команд, чтобы не зависеть от формата вывода Linux-команд
- **EEXIST handling**: Все add операции tolerant к EEXIST, т.к. DHCP/NetworkManager может пере-добавить маршрут между del и add в restore_route

## Issues Found

- `restore_route` падал с `EEXIST` (-17) на первом прогоне — DHCP клиент пере-добавлял оригинальный default route между `del` и `add` в restore_route. Исправлено: `eexist_ok()` игнорирует EEXIST
- `set_tun_route` и `add_exclude_route` аналогично могли упасть с EEXIST. Исправлено тем же `eexist_ok()`

## Current State

- `cargo test --lib` — 20/20 unit-тестов проходят
- `cargo test --test route_integration` — 2/2 с sudo проходят
- `cargo build --release` — собирается без ошибок и warnings
- Release-бинарник: `target/release/traffic-sentinel`

## Next Steps

1. **P1.2**: ECDH handshake (`src/handshake.rs`) — X25519 + PSK hybrid key exchange
2. **P1.3**: Server forwarder — bidirectional трафик через server-side TUN
3. **P1.4**: Full config loading — validation rules
4. **P1.5**: Wire everything into main.rs — handshake, routes, full bootstrap

## Relevant Files

- `src/route.rs` — route management implementation (rtnetlink)
- `src/lib.rs` — library crate for integration tests
- `tests/route_integration.rs` — автоматические интеграционные тесты
- `tests/run_route_tests.sh` — bash-скрипт ручных тестов
- `specs/VERIFICATION_P1.md` — верификация P1 (P1.1 помечен `[x]`)
- `specs/THOUGHTS.md` — Q15 (rtnetlink vs ip route)
- `Cargo.toml` — добавлены rtnetlink, netlink-packet-route, tokio-stream
