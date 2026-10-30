# 2026-10-30 brp-params: `brp` schema shows params shapes (B-047) — v0.7.1

Focus: none given · Outcome: shipped

## Summary
No open bug or issue was workable (#1 stale and answered, #2 a question). Chose B-047 (agents guessed
`registry.schema` params). The `brp` `params` schema now carries shapes for `world.query`,
`registry.schema`, `world.list_components`, `rpc.discover`.

## Change
Files: `src/server/mcp_tools.rs`, `tests/fixture_ground_truth.rs`, `docs/mcp-tools.md` (generated).
Failing-first check: `fixture_ground_truth::the_params_examples_in_the_brp_schema_are_accepted_by_the_game`
(failed with "found []" before; runs every example from the advertised schema against the fixture).
Candidates: B-044 (observe short queries), B-051 (ratchet span bug), B-047.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals | 6/6 (2026-10-29) | 6/6, 0 invalid calls |
| held-out | 3/3 | 3/3 |
First draft used a `Transform` example: dev `spinner_stops` failed (agent anchored on Transform; the
spinner has none). Reworded to `bevy_ecs::name::Name` plus "take the game's own from
`world.list_components`"; rerun 6/6. Anchoring on schema examples is a real effect.

## Metrics
No LOC/dep/tool-count change beyond +5 doc lines, +41 test lines.

## Review
Independent subagent review skipped (doc-only schema text; gate and evals green).

## Test and eval diffs
Added one test; nothing weakened.

## Release and install
0.7.0 → 0.7.1 (patch, description text only).

## Deferred
B-044 (observe grammar), B-051 (ratchet touched-file span), B-026.

## Next focus
B-044/B-026: observe description, measured by dev-eval invalid calls.
