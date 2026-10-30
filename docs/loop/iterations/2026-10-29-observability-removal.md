# 2026-10-29 dead-code: `observability` feature deleted, `--all-features` builds (B-019)

Focus: none · Outcome: shipped (v0.5.0)

## Summary
The `observability` feature never compiled (42 errors), poisoned `full-debugging` and CI's `--all-features`, and was reached by none of the 14 tools. Deleted it with its config, env vars and orphan deps. Candidates considered: this (priority 2, pure deletion); B-044/B-047 description fixes (need eval measurement); B-021 burst capture (large).

## Bugs fixed
### B-019: observability does not compile
Must-fix: blocks (red `--all-features`). Failing-first check: the ratchet's per-feature sweep recorded `observability`=42 and `full-debugging`=42 errors; now both 0 (`scripts/ratchet.sh --record`), and `cargo check --all-features --all-targets` completes with no errors.
Also honesty: `Config::observability` and 9 env vars (`METRICS_PORT`, ...) were parsed, validated and ignored in default builds; removed.

## Evidence
`scripts/ratchet.sh` → OK before baseline refresh. `cargo test --workspace --no-fail-fast -j 3`: every `test result` line ok, 0 failed. Evals not run: no tool schema or handler changed.
known_red: clippy 1462 → 1428; features with errors 2 → 0; baseline sha e092e01.

## Metrics
~2.8k Rust lines removed, 11 optional deps removed (opentelemetry*, prometheus, metrics*). Nothing added.

## Test and eval diffs
Deleted `tests/observability_integration_test.rs`: it was gated on the deleted feature and so never compiled or ran. No assertion in a running test changed.

## Release and install
0.4.0 → 0.5.0 (breaking: removed pub `Config::observability`, `ObservabilityConfig`, feature, env vars).

## Deferred / open
B-013/B-030 unreachable modules, B-021 burst capture, B-044/B-047 description gaps.

## Next focus
Description fixes for `observe`/`brp` (B-044, B-047), measured by dev-eval invalid-call count.
