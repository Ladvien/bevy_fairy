# 2026-10-30 dead-clients: deleted never-compiled cli.rs and two duplicate BRP clients

Focus: priority order · Commits: 3bef565..HEAD · Outcome: shipped (deletion)

## Summary
Open issues: #1 (stale, already answered by the owner), #2 (a question). No new must-fix bug found; baseline suite green. Change: delete `cli.rs` (B-011, never compiled) and `brp_client_v2.rs`/`brp_client_refactored.rs` (unused duplicates of `BrpClient`, part of B-013).

## Change: delete dead BRP clients and cli.rs
Plan: remove 3 files + 2 `pub mod` lines · Goal: one BRP client, no uncompiled files · Proof: `grep` shows no other reference; workspace tests all pass (lib 304 -> 298, the six removed are the deleted files' own tests); `scripts/ratchet.sh`: OK.
Candidates considered: B-018 ignored test (needs feature decision); B-044/B-047 agent-description tweaks (need eval runs); chosen deletion is cheapest and risk-free to tool behaviour.

## Evidence
Evals not run: no tool, schema or description changed (same reasoning as 0.4.0/0.5.0).

## Metrics
server loc 51207 -> 47558; tools 14 unchanged; clippy unchanged (ratchet OK).

## Release and install
0.5.0 -> 0.6.0 (minor, removed public modules).

## Deferred / open
Issues #1 (stale, answered), #2 (question) skipped: not actionable. B-013 remainder, B-018, B-044, B-047.

## Next focus
B-013: next coherent dead-module deletion (`brp_client` siblings, `mcp_server_v2`, replay).
