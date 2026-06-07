# Traffic Sentinel — Development

## Статус

P2 (Production Ready) завершён. Текущий этап — multi-client (сервер обслуживает несколько клиентов одновременно).

## Тестирование

```bash
cargo test --all          # unit + integration тесты
cargo clippy --all        # lint
cargo llvm-cov --all      # покрытие (требуется cargo-llvm-cov)

# Docker-тесты (требуют --cap-add NET_ADMIN --device /dev/net/tun)
bash tests/docker_e2e.sh
bash tests/docker_reconnect.sh
bash tests/docker_heartbeat.sh
```

## Документы разработки

- [PRD](docs/PRD.md) — требования и архитектура
- [IMPLEMENTATION_PLAN](docs/IMPLEMENTATION_PLAN.md) — план реализации по фазам
- [THOUGHTS](docs/THOUGHTS.md) — история решений
- [VERIFICATION_P0](docs/VERIFICATION_P0.md)
- [VERIFICATION_P1](docs/VERIFICATION_P1.md)
- [VERIFICATION_P2](docs/VERIFICATION_P2.md)
