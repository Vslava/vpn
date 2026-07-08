# Traffic Sentinel — Development

## Статус

P2 (Production Ready) завершён. Текущий этап — P4 (Multi-client): сервер обслуживает несколько клиентов одновременно.

## Тестирование

```bash
cargo test --lib          # unit-тесты (71 тест)
cargo test --tests        # интеграционные тесты (без sudo некоторые ignored)
cargo clippy --all        # lint
cargo llvm-cov --all      # покрытие (требуется cargo-llvm-cov)

# Docker-тесты (требуют --cap-add NET_ADMIN --device /dev/net/tun)
bash tests/docker_e2e.sh
bash tests/docker_reconnect.sh
bash tests/docker_heartbeat.sh
bash tests/docker_multiclient.sh
```

## Документы разработки

- [PRD](specs/PRD.md) — требования и архитектура
- [IMPLEMENTATION_PLAN](specs/IMPLEMENTATION_PLAN.md) — план реализации по фазам
- [THOUGHTS](specs/THOUGHTS.md) — история решений
- [VERIFICATION_P0](specs/VERIFICATION_P0.md)
- [VERIFICATION_P1](specs/VERIFICATION_P1.md)
- [VERIFICATION_P2](specs/VERIFICATION_P2.md)
- [VERIFICATION_P3](specs/VERIFICATION_P3.md)
- [VERIFICATION_P4](specs/VERIFICATION_P4.md)
