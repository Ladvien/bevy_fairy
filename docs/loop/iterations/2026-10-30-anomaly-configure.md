# 2026-10-30 anomaly: configure kept nothing it was not given

Focus: none · Outcome: fixes only (no release: last tag v0.7.2 is under 6 h old)

## Bugs fixed
### B-070: detect_anomaly configure reset unspecified fields to defaults, ignored non-numeric values
Must-fix: misleads yes. `handle_configure` started from `AnomalyConfig::default()`, so a second call silently undid the first, and `"window_size":"50"` was ignored while reporting success.
Fix: start from the config in force (`AnomalyDetectionSystem::config()`); a present value that is not a positive number is refused with the field named and an example; `status` now returns the config in force.
Failing-first checks: `tools::anomaly::tests::configure_keeps_fields_it_was_not_given`, `configure_refuses_a_value_it_cannot_apply`.

## Evidence
`cargo test --workspace` green; `scripts/ratchet.sh <touched files>` OK. Evals not re-run (host load 33); no tool schema changed.

## Test and eval diffs
Two tests added in anomaly.rs; none changed.
