# 2026-10-30 observe: single-entity reads no longer reported as entity 0

Focus: honesty audit · Outcome: fixes only

## Bugs fixed
### B-062: observe `show entity N` keyed reflection and diff snapshots on entity 0
Must-fix: misleads yes — `world.get_components` replies carry no entity; the handler fell back to `unwrap_or(0)`, so `reflection_data[].entity_id` was 0 and every diff snapshot of any single entity shared one id.
Fix: take the id from the request's own `entity` param; with none, decode nothing. Failing-first check: `tests/agent_tools.rs::observe_reflection_names_the_entity_that_was_asked_about` (failed with `entity_id: 0`, passes with 4242).

## Evidence
`cargo test --workspace -j 3` all green; `scripts/ratchet.sh` on touched files OK. Evals and live MCP run not performed this pass (host load ~15); no tool schema changed.

## Release and install
No release: v0.7.2 tag ~3 h old (6 h rule). Fix rides the next release (0.8.0, breaking per earlier log). Installed binary not rebuilt this pass.

## Skipped issues
#1 (stale report against the old bevy-debugger-mcp crate), #2 (question, not a request).

## Next focus
B-021 ring buffer; B-024 wrapper structs.
