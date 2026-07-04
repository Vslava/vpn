# Handoff: PRD + Архитектурные решения

## Summary

Написан PRD (Product Requirements Document) для `traffic-sentinel` — Rust-приложения, перехватывающего весь локальный трафик и направляющего его на внешний сервер через шифрованный TCP-туннель. Приняты все ключевые архитектурные решения, зафиксированы в THOUGHTS.md.

## What Was Done

### Документы

- **`specs/PRD.md`** — полный PRD: overview, goals/non-goals, functional и non-functional requirements, архитектура, компоненты, платформы, сценарии, security, open questions, milestones.
- **`specs/THOUGHTS.md`** — зафиксированы все принятые решения с форматом: вопрос, варианты, решение, формулировка человека.
- **`AGENTS.md`** — правило для opencode: при принятии решения записывать в THOUGHTS.md в формате вопрос/варианты/решение/формулировка человека.

### Принятые решения

| № | Вопрос | Решение |
|---|---|---|
| Q1 | Шифрование | XChaCha20-Poly1305 + X25519 ECDH + PSK |
| Q2 | Режимы | Единый бинарник, `--mode server` и `--mode client` |
| Q3 | IP/port | TOML-конфиг: `[server].listen`, `[client].remote` |
| Q4 | Протокол | TCP-туннель (IP-in-TCP) |
| Q5 | ICMP | Пропускать как есть |
| Q6 | MTU | 1400, конфигурируется |
| Q7 | TUN crate | `tun-rs` v2 |
| Q8 | Права | sudo (Linux), UAC (Windows), sudo (macOS) |
| Q9 | Многопоточность | Две tokio-задачи, sequential pipeline, без каналов |
| Q10 | Буфер | Block (естественная backpressure) |
| Q11 | Nonce | Случайный 24 байта на пакет |
| Q12 | Key rotation | Не делать |
| Q13 | Платформы | v1: Linux + Windows |

## Current State

- PRD завершён и согласован.
- Все открытые вопросы (Q1–Q14) закрыты или отклонены.
- THOUGHTS.md содержит полную историю решений с формулировками человека.
- Код ещё не написан.

## Next Steps

1. Инициализация Rust-проекта: `cargo init`, зависимости (`tun-rs`, `tokio`, `xchacha20poly1305`, `x25519-dalek`, `serde`, `toml`, `tracing`, `clap`).
2. Реализация P0: TUN interface + чтение пакетов + XChaCha20 шифрование + TCP-отправка (loopback test).
3. Реализация P1: Route management, ECDH handshake, полный цикл клиент↔сервер.

## Relevant Files

- `specs/PRD.md` — Product Requirements Document
- `specs/THOUGHTS.md` — Архитектурные решения с обоснованием
- `AGENTS.md` — Правила для opencode
