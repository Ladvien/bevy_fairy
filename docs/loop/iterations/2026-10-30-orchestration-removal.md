# 2026-10-30 orchestration-removal: dead tool orchestrator deleted

Focus: none · Outcome: shipped (deletion), no release (6 h rule)

## Summary
No bug or workable issue was open (#1 and #2 are stale reports about the pre-0.3 WebSocket/rmcp server, both answered). Priority 2: `server::tool_orchestration` (1065 lines) and `server::tools::orchestration` (104) were not constructed by the binary or any test, backed no tool, cached tool results with no expiry, and its auto-record/auto-experiment hooks only logged "placeholder". Deleted.

## Change
Files: `src/server/tool_orchestration.rs`, `src/server/tools/orchestration.rs` (removed), two `pub mod` lines. Proof: `grep -rn create_orchestrator|ToolOrchestrator src tests` had no users outside the two files; `cargo test --workspace` green; `scripts/ratchet.sh` OK; tool list unchanged.
Candidates considered: ring buffer (B-021, large, host load ~30); B-024 (already carried by handler types); this.

## Metrics
Server loc down by ~1170; clippy `tool_orchestration.rs` 4 warnings gone (known_red re-recorded, only down). Public module count down by 2.

## Test and eval diffs
none. Evals not rerun: no tool behaviour changed.

## Release and install
No release: last tag v0.7.2 was 5.5 h old (<6 h rule). Breaking for library users of the removed pub module; next release is a minor bump (0.8.0) per CHANGELOG. Installed binary unchanged in behaviour (module unreachable from binary).

## Deferred
- empty `orchestration = []` feature in Cargo.toml left (ratchet feature list); remove with a ratchet re-record.
- B-021 ring buffer.

## Next focus
B-021 ring buffer with trigger dump, on a quiet host.
