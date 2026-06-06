# Handoff: Fix THOUGHTS Rules and Finalize P0 Verification

## Summary

Уточнены правила AGENTS.md для THOUGHTS.md, очищен THOUGHTS.md от AI-решений, добавлено одно человеческое решение (перенос тестов в P1.3). Запущены оставшиеся тесты из VERIFICATION_P0.md (включая empty frame через libsodium). Все тесты пройдены.

## What Was Done

### AGENTS.md — фикс правил

- Первая строка теперь: "THOUGHTS.md фиксирует решения человека" — без двусмысленности
- Убрана фраза "выбор библиотеки, протокола, архитектурного подхода, настройки, реализации" из неправильного контекста (звучала как приглашение AI записывать свои решения)
- Порядок: сперва кто принимает (человек), потом что записывать, потом формат

### THOUGHTS.md — очистка

- Удалены Q14–Q21 (мои implementation решения, выданные за человеческие):
  - Crate для XChaCha20-Poly1305
  - Async feature для tun
  - Формат фрейма
  - write_frame семантика
  - Client pipeline split
  - Сервер без TUN в P0
  - Логгирование IP protocol
  - Empty frame обработка
- Оставлено Q14 (человек сказал "перенеси" про тесты)

### VERIFICATION_P0.md — дозапуск тестов

- **Empty frame**: зашифрован пустой plaintext через libsodium (XChaCha20-Poly1305), отправлен серверу. Результат: `len=0`, без краша.
- **Double TUN create**: второй клиент получил `Device or resource busy`
- **MTU=576**: `mtu 576 qdisc fq_codel`
- **SIGKILL cleanup**: TUN ts0 удалён ядром
- **Corrupted frame**: `decrypt failed: aead::Error`
- **Config section mismatch**: корректные ошибки с сообщениями
- **Flood ping 100**: 100/100 ICMP расшифрованы
- **UDP**: `protocol=17 (UDP)`
- **Soak 30s**: CPU 0%, RSS 8MB
- **Kill server**: `IO error: early eof`

## What Didn't Work / Issues Found

- **IP protocol выводился неверно**: читал `plaintext[0]` (version+IHL = 0x45), выводил `protocol=69`. Исправлено в server.rs — теперь читает `plaintext[9]` для IPv4, `plaintext[6]` для IPv6.
- **THOUGHTS.md был замусорен AI-решениями**: после замечания человека очищен.

## Key Learnings

- THOUGHTS.md — строго **человеческие** решения. Мои implementation details там не место.
- При добавлении пустого правила в AGENTS.md важно не разорвать существующие логические блоки (THOUGHTS.md был разорван пополам).

## Current State

- P0 полностью верифицирован, чеклист в VERIFICATION_P0.md помечен `[x]`
- THOUGHTS.md содержит только подтверждённые человеком решения
- AGENTS.md — правила однозначны

## Next Steps

- P1.1: Route management

## Relevant Files

- `AGENTS.md` — уточнённые правила
- `docs/THOUGHTS.md` — очищен, добавлено Q14
- `docs/VERIFICATION_P0.md` — финальный чеклист
- `docs/VERIFICATION_P1.md` — добавлен блок о перенесённых тестах
