# Handoff: P2.1 — Graceful Shutdown

## Summary

Реализован graceful shutdown для клиента и сервера: SIGTERM и SIGINT обрабатываются через `CancellationToken`, spawned-таски форвардинга корректно abortятся, маршруты восстанавливаются, TUN-интерфейс удаляется. Все verification-тесты P2.1 пройдены в Docker.

## What Was Done

### CancellationToken-based остановка форвардинга (`src/client.rs`, `src/server.rs`)

- **`run_client()`** и **`handle_client()`** получили параметр `CancellationToken`
- Внутренний `tokio::select!` — `biased` с приоритетом cancellation → abort обоих spawned тасок (tun→tcp, tcp→tun) + `.await` их завершения
- Без CancellationToken spawned таски оставались висеть, удерживая `Arc<TunDevice>`, что мешало cleanup

### Сигнальный watcher (`src/lib.rs`)

- Новая публичная функция **`wait_for_shutdown()`**: `tokio::select!` между `ctrl_c()` и `unix signal(SIGTERM)`
- Запускается в отдельном `tokio::spawn` в `run_client_full()` и `run_server()` — при сигнале вызывает `cancel.cancel()`

### Graceful shutdown клиента (`src/client.rs::run_client_full`)

- `run_client_full` больше не использует `tokio::select!` с `ctrl_c()` напрямую
- Сигнальный watcher запущен в фоне, `run_client` просто `await`-ится — при сигнале CancellationToken прерывает форвардинг
- Логи приведены к последовательности из VERIFICATION (`Restoring routes` → `Deleting TUN` → `Closing TCP connection` → `Shutdown complete`)

### Graceful shutdown сервера (`src/server.rs::run_server`)

- Аналогичный watcher в фоне
- `tokio::select!` между `listener.accept()` и `cancel.cancelled()` — если сигнал пришёл до клиента, сервер завершается без создания TUN-клона
- `handle_client` вызывается напрямую (не spawn), cancellation прерывает форвардинг

### Прочее

- `Cargo.toml`: добавлен `"rt"` в features `tokio-util` (нужен для `CancellationToken`)
- `tests/server_integration.rs`: обновлён вызов `handle_client` (новый параметр `CancellationToken`)
- `docs/VERIFICATION_P2.md`: P2.1 отмечен `[x]`

## What Worked

- **CancellationToken** оказался надёжным механизмом: cancellation происходит на первом же `.await` после вызова `cancel()`, обе spawned таски останавливаются за один scheduler-цикл
- **Docker-based verification**: удалось протестировать SIGTERM, double signal, stop during ping без sudo на хосте — PID 1 в контейнере теперь `sh`, но `kill` через `docker exec ... sh -c 'kill -TERM $PID'` работает, т.к. таргетируем PID через обёртку

## Issues Found

- В `run_client` (и `handle_client` на сервере) использовался `tokio::select!` с JoinHandle по значению → при сигнале JoinHandle потреблялся, abort был невозможен. Решение: `&mut JoinHandle` + `biased` + `abort()` в каждой ветке
- `CancellationToken` из `tokio-util` 0.7.18 требует feature `"rt"`, а не `"sync"` (как в более новых версиях)

## Current State

- `cargo build` — 0 warnings
- `cargo test --lib` — 60/60 unit-тестов
- **P2.1 verification (Docker)**:
  - SIGTERM на клиенте: exit 0, route restored, TUN deleted, log sequence match
  - SIGTERM на сервере: exit 0, TUN deleted
  - Stop during ping (SIGTERM): 10/10 packets, 0% loss, routes restored
  - Double SIGTERM: второй игнорирован, exit 0

## Next Steps

1. **P2.2: Reconnection** — exponential backoff, reconnect при разрыве TCP, новый handshake
2. **Обновить THOUGHTS.md** — решения этой сессии (выбор CancellationToken, подход с watcher)

## Relevant Files

- `src/client.rs` — `run_client` (+cancel), `run_client_full` (+signal watcher)
- `src/server.rs` — `handle_client` (+cancel), `run_server` (+signal watcher + accept-with-cancel)
- `src/lib.rs` — `wait_for_shutdown()`
- `docs/VERIFICATION_P2.md` — P2.1 checklist
