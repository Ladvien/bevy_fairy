# 2026-10-30 observe: stale cache and silent-empty answers removed

Focus: honesty audit via live MCP pass · Commits: see git log · Outcome: fixes only

## Summary
A live pass against the fixture found `observe` answering from a 5-minute cache under a fresh timestamp, and answering `[]` for any filter naming an unregistered component (including the example in its own description). Both are fixed with ground-truth tests that failed first. No separate Phase 2 change: two must-fix bugs consumed the pass.

## Bugs fixed
### B-063: observe served cached state as current
Must-fix: misleads y — live: second identical read 1.5 s later returned `cache_hit: true`, same rows, new timestamp.
Fix: cache deleted (root cause: a result cache on live game state has no valid TTL). Failing-first: `tests/fixture_ground_truth.rs::observe_reads_the_live_game_every_time_it_is_asked` (ground-truth; mover x equal before).
### B-064: unregistered component name answered `[]`; type count labelled entity_count
Must-fix: misleads y — `find entities with component Crate` -> `[]`, `entity_count 0`; fixture has 6 crates.
Fix: names in `with`/`without` are checked against `world.list_components`; error lists registered paths ending in the name. Failing-first: `observe_refuses_a_component_name_the_game_does_not_register`; unit tests `count_of`, `filter_names`, `registered_paths_ending_with`.

## Evidence
| Suite | Before | After |
|---|---|---|
| ground-truth tests | 2 new fail | 14/14 pass |
| workspace `--all-features` tests | green | green |
| ratchet (touched file observe.rs clean) | ok | ok |
| dev/held-out evals, mutation testing | not run this pass (host load ~22; omp sessions + cargo-mutants skipped) | — |

Live proof (stdio MCP client, fixture): `find entities with component Crate` -> `{"error":"component not registered in the game", ... fixture_game::Crate}`; `list components` -> `component_type_count: 323`; the full-path Name query -> `entity_count: 8`.

## Metrics
observe/query_parser shrink (QueryCache etc. deleted); no new dependencies.

## Review
No independent subagent review run this pass (stated gap).

## Test and eval diffs
Deleted `query_parser::tests::test_query_cache` and `observe::tests::test_cache_stats` (tests of the deleted cache). Added two ground-truth tests and three unit tests.

## Release and install
No release: last tag v0.7.2 is 3.5 h old (<6 h rule); changes ride in the next release.

## Deferred / open
- B-065: semantic queries hard-code short, game-specific component names; now refuse.
- B-021 ring buffer, B-024 request-struct duplication.

## Next focus
B-065: make semantic queries resolve full type paths (or drop the ones needing non-Bevy components).
