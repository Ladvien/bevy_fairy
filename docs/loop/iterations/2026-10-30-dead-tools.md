# 2026-10-30 dead-tools: last unreachable tool modules deleted (B-013)

Focus: none given · Outcome: shipped to main, no release (last tag v0.7.2 was ~80 min old; 6 h rule)

## Summary
No bug, `consumer` or enhancement issue workable (#1 and #2 are stale, already answered). Deleted the remaining
unreachable items under `src/server/tools/` (`replay_v2`, `observe_optimized`) and the two modules only they
used (`query_optimization`, `parallel_query_executor`), plus the one bench and the `criterion` dev-dependency.
B-010 ("no input or watch tool") is stale: `watch` is registered and input is `bevy_fairy/input` through `run`; closed.

Candidates: B-021 burst capture (>400 lines), B-052 box Error (breaking), this deletion. Picked the deletion.
Host load average was ~24, so the larger candidates were not started.

## Proof
- `scripts/ratchet.sh`: OK (no new errors, warnings or failures; includes the default-feature test run, so `tool_list`, `tool_contract`, `docs_drift`, `fixture_ground_truth` passed).
- `cargo check --all-targets`: Finished. Tool list unchanged, `docs/mcp-tools.md` not regenerated.
- Dev/held-out evals not re-run: no registered tool, plugin file or tool description changed, and the deleted code was unreferenced (`grep` over src, tests, examples, docs).

## Test and eval diffs
None to tests or evals. Deleted `benches/query_optimization_benchmarks.rs` (it only imported deleted modules; not a gate).

## Metrics
Server LOC 24076 -> 21861; public modules 28 -> 26; direct dependencies 53 -> 52 (criterion). `known_red` not re-recorded (counts can only have fallen).

## Release and install
No release (6 h rule). Removes public library modules: next release is 0.8.0.

## Deferred
B-021, B-052, B-020, B-042.

## Next focus
B-021 burst capture, or B-052 as the 0.8.0 breaking sweep, then release.
