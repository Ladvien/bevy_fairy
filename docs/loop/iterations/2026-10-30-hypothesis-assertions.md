# 2026-10-30 hypothesis-assertions: B-067 fixed, assertions answer from the game

Focus: honesty audit · Outcome: fixes only

## Summary
`Assertion::evaluate` logged "not fully implemented - returning true" for `entity_exists`, `component_equals` and `component_matches`, so the `hypothesis` tool confirmed any such hypothesis. Now the first two query `world.query` and compare; the third is refused.

## Bugs fixed
### B-067
Must-fix: misleads yes. Fix: `src/server/hypothesis_system.rs` (root cause: TODO stubs returning `Ok(true)`). Failing-first check: `tests/hypothesis_assertions.rs` (ground-truth against a fake BRP game; 4 of 5 failed before).

## Evidence
`cargo test --workspace` green; `scripts/ratchet.sh src/server/hypothesis_system.rs tests/hypothesis_assertions.rs`: OK. Evals and live MCP proof not run (host load ~25; fix is in a path the fake-BRP test covers end to end). Mutation testing not run.

## Release and install
No release: last tag v0.7.2 is under 6 h old. Installed server not rebuilt this pass (no release); recorded as deferred.

## Deferred
- `component_matches` evaluator; eval suite and live proof on a quiet host.

## Next focus
Honesty audit of remaining TODO stubs: `playback_system.rs` sync (returns Ok without syncing), `tool_orchestration.rs` auto-record.
