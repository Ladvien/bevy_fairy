# 2026-10-30 observe-semantic-names: B-065 fixed, semantic queries resolve short component names

Focus: none · Outcome: fixes only

## Summary
Semantic queries ("find memory leaks") are built from short names (`Transform`). Since B-064 they were refused because the game registers full paths. A short name that ends exactly one registered path is now rewritten for semantic queries only, and reported in `metadata.resolved_component_names`. Unknown or ambiguous names are still refused with the candidates. Caller-typed short names are still refused (existing test `observe_refuses_a_component_name_the_game_does_not_register` pins that; unchanged).

## Bugs fixed
### B-065
Must-fix: blocks. Fix in `src/server/tools/observe.rs` (`resolve_unique_short_names`). Failing-first: `fixture_ground_truth::observe_resolves_the_short_names_a_semantic_query_is_built_from` (ground-truth; verified FAILED on HEAD, ok after) plus a unit test for the ambiguous case.

## Evidence
Ratchet ok (observe.rs clean); `cargo test --workspace -j 3` all green. Evals not run this pass (host load ~27; no tool schema change). Metrics: no new deps/tools.

## Release and install
No release: last tag 4 h old (< 6 h rule); change rides the next release.

## Deferred
B-021 ring buffer, B-024, B-042. Issues #1 (stale, answered) and #2 (question) are not work items.
