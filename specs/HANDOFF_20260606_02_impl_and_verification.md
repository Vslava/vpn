# Handoff: Implementation Plan, Verification Plans, and Skill

## Summary

Создан детальный implementation plan для `traffic-sentinel` (3 фазы, 20 шагов), три исчерпывающих плана верификации (по одному на фазу, с exact командами и edge case таблицами), и skill для генерации verification plans в будущем.

## What Was Done

### Implementation Plan

- **`docs/IMPLEMENTATION_PLAN.md`** — полный план реализации в 3 фазы:
  - **P0** (Proof of Concept): 9 шагов — от `cargo init` до loopback-теста. TUN → encrypt → TCP → decrypt → log.
  - **P1** (Minimal Viable): 5 шагов — route management, ECDH handshake, server forwarder, full config, integration.
  - **P2** (Production Ready): 6 шагов — graceful shutdown, reconnection, heartbeat, logging, error handling, tests.
- Включает: структуру проекта (12 модулей), список зависимостей с версиями, граф execution order, key interfaces (Rust trait signatures), platform-specific notes (Linux/Windows), acceptance criteria на каждую фазу.

### Verification Plans

- **`docs/VERIFICATION_P0.md`** — 9 подэтапов, 35+ тестовых сценариев:
  - Для P0.3 (crypto): roundtrip, wrong key/nonce/ciphertext, empty plaintext, nonce uniqueness
  - Для P0.9 (loopback): 7 сценариев — ping, flood, UDP, размеры, soak 60s, kill, SIGKILL cleanup
- **`docs/VERIFICATION_P1.md`** — 5 подэтапов:
  - P1.1: save/restore/exclude routes, loop prevention (специальный тест с curl), race condition
  - P1.2: ECDH matching keys, wrong PSK, PFS (100 handshakes), replay, timeout, wire-level test
  - P1.3: ICMP types (echo, TTL exceeded, frag needed, unreachable), HTTP(S), DNS, IP forwarding off
  - P1.4: матрица валидации PSK (5 случаев), адресов (5), MTU (5), netmask (3)
  - P1.5: полный bootstrap sequence, откат при ошибках инициализации
- **`docs/VERIFICATION_P2.md`** — 6 подэтапов:
  - P2.1: SIGTERM/SIGINT/SIGKILL, shutdown во время трафика, двойной сигнал, замер времени
  - P2.2: reconnect, exponential backoff, max retries, session key rotation после reconnect
  - P2.3: PING/PONG frame, timeout→reconnect, heartbeat suppression при активном трафике
  - P2.4: все 4 уровня логирования, 15 обязательных log events, проверка sensitive data
  - P2.5: `rg '\bunwrap\b' src/`, error chain, реакция на каждый тип ошибки
  - P2.6: unit tests (crypto, protocol, handshake, config, route), integration tests, clippy, coverage
  - Полный smoke test script в конце (копируемый bash)

### Skill

- **`.opencode/skills/verification-plan/SKILL.md`** — reusable skill для генерации verification plans:
  - Структура файла (шаблон)
  - Типы проверок (unit, integration, system, negative, edge, soak, resilience)
  - Формат команд (точные bash-команды, конкретный expected output)
  - Чеклист обязательных сценариев (permissions, битые данные, гонки, очистка, восстановление)
  - Критерии готовности самого плана

## Current State

- Документация завершена: PRD → Implementation Plan → Verification Plans → Skill
- Код ещё не написан
- Git: 4 commits (initial + impl plan + verification + skill)

## Next Steps

1. Начать реализацию P0: `cargo init`, зависимости, модули
2. После каждого подэтапа — прогонять соответствующий раздел verification plan
3. Проверять чеклист в конце каждого VERIFICATION_*.md перед переходом к следующей фазе

## Relevant Files

- `docs/IMPLEMENTATION_PLAN.md` — implementation plan
- `docs/VERIFICATION_P0.md` — verification plan for P0
- `docs/VERIFICATION_P1.md` — verification plan for P1
- `docs/VERIFICATION_P2.md` — verification plan for P2
- `.opencode/skills/verification-plan/SKILL.md` — skill for generating verification plans
- `docs/PRD.md` — product requirements (from previous session)
- `docs/THOUGHTS.md` — architectural decisions (from previous session)
