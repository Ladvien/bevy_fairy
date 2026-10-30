# 2026-10-30 dead-modules: 38 unreachable server modules deleted (B-030, B-013)

Focus: none given · Outcome: shipped to main, no release (last tag v0.7.2 was 50 min old; 6 h rule)

## Summary
No bug, `consumer` or enhancement issue was workable (#1, #2 are stale reports, answered). Chose a
delete/consolidate change: a reachability pass from `src/main.rs` over `src/server/` found 38 of 66 modules
(legacy `mcp_server`, session manager, profilers, issue detector, performance budget, query builder, visual
overlays, pools/caches, ...) that no registered tool or the binary can reach. Deleted them and the tests/benches
that only exercised them. Server 45,890 -> 24,076 lines.

Candidates: B-052 (box Error, public-signature churn, minor release), B-021 (burst capture, >400 lines),
this deletion (priority 2, zero tool-surface change). Picked the deletion.

## Proof
- `cargo check --tests --benches --examples --keep-going`: clean after the deletions.
- `cargo test` (default features): all green, including `tool_list`, `tool_contract`, `docs_drift`, `instructions`, `fixture_ground_truth`.
- `scripts/ratchet.sh`: OK against the re-recorded baseline (counts only went down; clippy was 1451).
- Dev evals via `evals/run.py`: 6/6, 0 invalid calls, mean 6.3 calls (before: 5/6 with racy `spinner_stops`).
- Tool list unchanged (tool_list test passes without regenerating docs/mcp-tools.md).

## Test and eval diffs (justified)
Deleted, because they only imported deleted modules: tests/integration_test.rs, memory_optimization_tests.rs,
session_management_integration_tests.rs, system_profiler_integration_tests.rs, visual_overlay_performance_test.rs;
benches optimization/performance/regression; never-compiled tests/integration/ and tests/mocks/.
Edited tests/brp_refactor_integration_tests.rs: removed `test_debug_handler_integration` (it tested the deleted
`DebugBrpHandler`) plus two touched-file clippy fixes. No assertion was loosened. Held-out suite untouched.

## Metrics
Server LOC 45890 -> 24076; public modules 68 -> 28; plugin LOC 3260. No increases. `known_red` re-recorded lower.

## Release and install
No release (6 h rule). Next release removes public library modules: semver-breaking, so v0.8.0.

## Deferred
B-052 (box Error), B-021 (burst capture), remaining dead items under src/server/tools (replay_v2, observe_optimized).

## Next focus
B-021 burst capture, or B-052 as the 0.8.0 breaking sweep.
