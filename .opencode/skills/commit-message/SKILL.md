---
name: commit-message
description: Use when generating a commit message. Formats according to the project's commit convention defined in opencode-commit-rule.md: conventional commits with extended body.
---

# OpenCode Rule: Conventional Commits с расширенным телом

**Trigger:** При генерации commit message.

## Формат

```
<type>(<scope>): <subject>
```

Или, когда нужно больше контекста в заголовке:

```
<type>(<scope>): <subject> — <detail>
```

## Типы (`type`)

| Тип | Когда |
|------|-------|
| `feat` | Новая функциональность. |
| `fix` | Исправление бага. |
| `test` | Только тесты (новые, портирование, аудит покрытия). |
| `docs` | Только документация (внутренняя, комментарии, changelog, design notes). |
| `refactor` | Поведенчески-эквивалентное изменение — rename, extract, перестановка. |
| `chore` | Инфраструктура: удаление мёртвого кода, рефреш данных, смена конфигурации CI. |
| `docs+test` / `feat+refactor` | Сдвоенный тип при равном весе двух категорий. |

## Scope

Scope — область изменения: **существительное, нижний регистр, без пробелов**. Допустим множественный через запятую (`auth,webhooks`). Можно опускать, когда scope очевиден из контекста или изменение затрагивает несколько несвязанных областей.

## Subject

- **Imperative mood** (английский): «add», «port», «close», «consolidate», «strip», «rename», «drop», «fix», «bump», «migrate», «expose», «revert».
- Без точки в конце.
- Subject должен быть самодостаточным — читатель видит его в `git log --oneline` и понимает суть без контекста.

Когда subject + scope достаточно — подробности в заголовке не нужны. Когда subject умещается в одно слово — detail-часть раскрывает.

## Body

Body — опциональный, но **рекомендованный** для нетривиальных изменений. Два типа содержимого:

1. **Rationale (1-2 предложения):** почему изменение произошло, какая проблема решается. Язык — любой, на котором мыслит автор (допустим русский или английский).

2. **Technical details (bullet points):** что изменилось, ключевые технические детали. Имена функций/файлов/сущностей в backtick`ах.
   - Для тестов: количество тестов, результаты прогона (`N suites / M tests, 0 fail`).
   - Для рефактора: мотивация rename\`а, что было не так.
   - Для фикса: триггер и механизм исправления.

**Длина body — произвольная**, от одного предложения до нескольких абзацев. Главное правило: **body должен добавлять информацию, которую невозможно уместить в subject**. Не дублировать subject в body.

## Стилистические принципы

1. **Императив.** Subject всегда в повелительном наклонении: «add», «drop», «fix», не «added», «dropped», «fixes».
2. **Самодостаточность.** Commit message должен быть понятен без ссылок на PR, issue, внешние тикеты. Если ссылка на issue есть — она в body, не в subject.
3. **Эмодзи — нет.** Серьёзный tone, без эмодзи/смайликов.
4. **Тип + scope** — это первое, что видит читатель в `git log --oneline` / `git log --graph`. Выбирать осмысленно.
5. **Scope в lower-case.** Scope — всегда строчные буквы, точка/дефис как разделители (`cli`, `ci`, `core`, `v2.1`, `middleware`).
6. **Корректные пробелы.** После типа и scope — пробел. После `—` тире — пробелы с двух сторон (или em-dash без пробелов, как принято в проекте).
7. **Body без пустых секций.** Если body нечего сказать — не писать. Пустое «Why» хуже отсутствия body.

## Примеры

```
feat(cli): add retry/discard ops for dead-letter queue
```

```
test(auth): port integration suite to Jest harness (35 tests)
tsc clean, jest → 4 suites / 35 passed, 0 fail.
```

```
refactor(maintenance): rename formatByClass → formatAuditByClass
```

```
chore: delete obsolete shell-based test runner (scripts/tests/)
The old runner was fully superseded by the Jest harness.
scripts/seed.ts retains the production seeding utility.

Verified: no remaining imports reference the deleted directory;
full suite 900+ tests pass, tsc clean.
```

```
docs(plan): refine distribution scope — verdaccio-in-compose, dist scripts, release-runbook
User-approved clarification: Verdaccio runs as a compose service,
distribution scripts orchestrate clean-install validation,
release-runbook documents real-registry publishing steps.
```

