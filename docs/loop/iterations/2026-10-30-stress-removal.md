# 2026-10-30 stress-removal: deleted unreachable stress modules (B-038)

Focus: priority order · Outcome: shipped (deletion)

## Summary
Issues #1 (stale, answered by owner) and #2 (a question) not actionable. No new bug found. Change: delete `tools/stress.rs`, `stress_test_system.rs` (1617 lines), their registrations in `mcp_server.rs`/`tools/orchestration.rs`, the `debug_performance` pipeline template, and the `stress-testing` feature.

## Change
Plan: pure deletion, B-038 · Goal: no code for a tool that is not offered · Proof: `cargo test --workspace` all green (includes `tool_list::stress_test_is_not_offered`, docs_drift, tool_contract); `scripts/ratchet.sh`: OK.
Candidates considered: B-044/B-047 description tweaks (need eval runs); B-052 Error boxing (public signatures, large).

## Evidence
Evals not run: no tool, schema or description changed.

## Metrics
server loc 47558 -> 45890; public modules 72 -> 68 (recount); tools 14; clippy not worse (ratchet OK).

## Release and install
0.6.1 -> 0.7.0 (minor, removed public modules and a feature).

## Deferred / open
B-013 remainder, B-044, B-047.

## Next focus
B-013: next dead-module deletion (`mcp_server` legacy, replay duplicates).
