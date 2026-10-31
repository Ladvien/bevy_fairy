# 2026-10-31 hypothesis and experiment: 14 honesty bugs fixed in the two tools no pass had read end to end

Focus: none given · Commits: 577d6b0..611f2cc (clippy cleanup 794c301, loop state after) · Outcome: fixes only, no release (6 h rule)

## Summary
`hypothesis` and `experiment` were fixed for specific lies in B-001..B-068, but nobody had traced either tool's whole reply against what the game did. Two read-only audits found more than the five-bug budget allows, each reproduced against the fixture before it was fixed: a claim whose actions the game refused was REJECTED, conditions that could not be read were CONFIRMED at 100%, a typo'd component made a claim false and its negation true, one iteration reported "100.0% confidence", a refused action left an atomic batch's transaction open, and `undo` got stuck behind any batch. Per the bug budget (Phase 1), Phase 2 is skipped. An independent review of the fixes found four more gaps; all four were reproduced and fixed.

## Bugs fixed
Every bug below is must-fix (misleads, except B-105 which blocks) and has a ground-truth test in `tests/fixture_ground_truth.rs` that failed on the commit before its fix (checked with `git stash push -- src/`, run, `git stash pop`).

### B-095 (577d6b0): an action the game refused counted as a violation
Misleads y, blocks n, common y. `hypothesis test` with a modify of an unregistered component answered `REJECTED`, `success_rate: 0.0`, error "Action execution failed" (a constant). Zero iterations (`iterations: 0`) answered REJECTED, CONFIRMED with `threshold: 0`.
Fix: every iteration is `held`, `violated` or `not_evaluated`; the game's own message is in `examples`; the verdict bounds the held share by what is known (an unevaluated iteration counts against CONFIRMED and for REJECTED); `NOT_EVALUATED` when none read the condition; rate and interval are `null` then. Test: `hypothesis_whose_actions_the_game_refused_is_not_evaluated_not_rejected`.

### B-096, B-097 (7325dbf): conditions that could not be read were CONFIRMED
`component_types` as a string became `[]`, a string inside `all` was skipped, a non-integer `count` or `entity_id` became "any"; `action_succeeds` as a string invented a spawn of an empty entity and ran it every iteration. Pre-fix reply: `verdict: CONFIRMED, confidence_level: "100.0%"`. Fix: serde with `deny_unknown_fields` replaces 140 lines of hand parsing, `Assertion::check` refuses empty lists, `component_matches` and unmeasurable metrics before any action runs. Tests: `hypothesis_refuses_a_condition_it_cannot_read_instead_of_confirming_it`, `hypothesis_action_succeeds_without_an_action_is_refused_and_spawns_nothing`.

### B-098, B-099 (0507dfa): `3` is not `3.0`, and a typo is not a measurement
`component_equals` compared `serde_json::Value`; the fixture's crate weighs `3.0`, an agent writes `3`, a true claim was REJECTED. A component name the game never registered was judged false and its negation true. Fix: numbers compare as numbers; `world.list_components` is read once before any action and an unregistered name is refused with the close registered paths (`fixture_game::Crat` offers `fixture_game::Crate`). Tests: `hypothesis_component_equals_does_not_tell_3_from_3_point_0`, `hypothesis_refuses_a_component_the_game_never_registered_instead_of_judging_the_claim` (it also checks that a registered component no entity holds is still an honest REJECTED).

### B-100 (cdcac6d): one iteration reported 100% confidence
A Wald interval has zero width at a rate of 1; `confidence_level` was a linear remap of its bound. Now the 95% Wilson interval (1 of 1 is [0.207, 1.0]), labelled with level and method; `confidence_level` is gone. Test: `hypothesis_read_once_does_not_claim_certainty`.

### B-101, B-102 (3506ce6): experiment lost results and stuck `undo`
A refused action (empty component type) was an `Err` that escaped the loop: the spawns that had happened vanished from the reply, and in an atomic batch the `?` skipped rollback and the transaction pop, so the first spawn stayed in the game and no later action was recorded for undo. A batch was its own history entry with no reverse, so `undo` answered "`batch` cannot be undone" forever. Fix: refused actions are failed results; an atomic batch stops at its first failure, reverses what ran and marks `rolled_back`; a batch lists each action's result (entity ids) and is not a history entry, its actions are. Tests: `experiment_reports_a_refused_action_beside_the_ones_that_ran`, `experiment_batch_names_what_it_spawned_and_can_be_undone`.

### B-103, B-104 (160c564): archetypes no game could spawn, fields silently dropped
`experiment archetypes` listed player/enemy/projectile/static_mesh: hard-coded short names (`Transform`, `Health`) and `{x,y,z}` objects that BRP refuses. `Action` and `ComponentSpec` dropped unknown fields (a `path` on a component overwrote the whole component). Assumed decision (owner may override): delete the archetypes (options: read them from the game, which has no such concept; keep the constants, which lie; delete), and `deny_unknown_fields`. Test: `experiment_offers_no_archetypes_and_refuses_fields_it_does_not_read`.

### B-105 (b065da2): the variation generator panicked
`numeric_min >= numeric_max` or `max_mutations: 0` panicked ("cannot sample empty range", `hypothesis_system.rs:757`), so the call never answered. Refused, naming the argument, before any action runs. Test: `hypothesis_refuses_variation_settings_that_would_panic_the_generator` (panicked on the previous commit).

### B-106, B-107, B-108 (611f2cc): the independent review's findings
1. A batch inside an atomic batch that half-succeeded kept its spawn under a reply saying "rolled back" (rollback never looked inside a failed result). Now a batch cannot contain a batch.
2. `action_succeeds`' action was outside the registry check, so `not(action_succeeds spawn Crat)` was CONFIRMED. Its components now feed the check; an empty `type_id` is refused.
3. `threshold: 0` confirmed a claim that failed every time. It must be above 0.
4. `success_rate` is over evaluated iterations while the verdict counts all; the description says so.
Skeptic pass: each is reproducible by reading and by a test red on 794c301; none had a benign reading. Round 2 not run.

## Change
None: five or more must-fix bugs, so Phase 2 is skipped. Candidates logged: B-021's frame ring and the state ring armed by default (last pass's recommendation) stay open.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals (success / mean calls / invalid) | not run at the start (load 43-59); last record ff29519: 6/6, 5.5, 0 | 611f2cc, two runs: 6/6, 6.3 then 5.2, 0 invalid. `spinner_stops` took 16 then 10 calls (known spread 8-16); the other five tasks 2/5/4/5/6 then 2/5/3/5/6 |
| held-out evals | ff29519: 3/3, 4.7 | not run: no release this pass |
| mutation testing | | not run (host load); each fix was instead shown red on the previous commit by stashing `src/` |
| ratchet on touched files | | OK at 794c301 (before the review fixes) and again at the final code, see LOG |
| fixture ground truth | 30 tests | 44 tests, all pass |

Transcript excerpt, live through the real binary over stdio (`/tmp/fairy_live_smoke.py`, removed), fixture game on a free port:

```
hypothesis not(entity_exists Crat)      before: verdict CONFIRMED          after: isError, "component fixture_game::Crat is not registered in the game ... close registered names: fixture_game::Crate"
hypothesis component_types: "...Crate"  before: CONFIRMED, "100.0%"        after: isError, 'invalid type: string "fixture_game::Crate", expected a sequence. A condition is ...'
hypothesis spawn of fixture_game::NoSuch before: REJECTED, "Action execution failed"   after: NOT_EVALUATED, success_rate null, "Unknown component type: `fixture_game::NoSuch`"
hypothesis weight 3 (game has 3.0)      before: REJECTED                   after: CONFIRMED, interval [0.207, 1.0] (one iteration)
experiment archetypes                   before: four archetypes            after: "Unknown action", available_actions [execute, undo, redo, history, clear_history]
experiment atomic [spawn, refused]      before: tool error, spawn left in the game   after: batch failed "Transaction rolled back (action 1 failed ...)", spawn marked rolled_back
```

## Metrics
Server LOC 20524 -> 20503 (`git diff --shortstat edce726..HEAD -- src`: 607 added, 628 removed, the removals being the archetypes and the hand-written condition parser); plugin 3561 unchanged; direct dependencies 52 unchanged (serde was already there); public modules 26, MCP tools 14 unchanged; clippy warnings 74 -> 69 (`field_reassign_with_default` 20 -> 15, `src/server/tools/experiment.rs` 5 -> 0, baseline lowered in this pass's loop commit); tests 427 -> 440 (+14 ground truth, +4 unit, -5 removed); release binary size not re-measured (no release).

## Review
Independent reviewer (fresh context, read-only, diff only): four gaps, listed under B-106..B-108, all survived the skeptic pass and are fixed.

## Test and eval diffs
- `tests/tool_contract.rs`: `hypothesis_runs_the_requested_action` and `every_advertised_variation_type_runs` sent `{"type": "entity_exists"}`, the vacuous condition B-096 refuses; with it the second stopped reaching the variation parser and passed for the wrong reason. Both send `{"type": "performance_within", "metric": "entity_count", "max_value": 1000}` now. Assertions unchanged.
- `src/server/hypothesis_system.rs`: `test_test_result_confirmation` (tested `is_confirmed`, replaced by `verdict`) became two verdict tests; `src/server/experiment_system.rs` and `tools/experiment.rs`: `test_entity_factory`, `test_apply_archetype`, `test_action_enum_serialization`, `test_handle_archetypes` deleted with the code they tested.
- `tests/fixture_ground_truth.rs`: 14 tests added, none edited. No eval task or held-out file changed.

## Release and install
No release: v0.10.0 was tagged 2026-10-30 21:22 (-0500) and the 6 h rule makes 03:22 the earliest; the changes ride in the next release, which is breaking (0.11.0: `hypothesis` reply shape, `experiment` lost `archetypes`, `Action` refuses unknown fields). v0.10.0 checked at the start: crates.io has it, tag `v0.10.0` is on the remote, docs.rs `doc_status: true`, installed binary 0.10.0.
Installed binary: rebuilt from the pushed commit (see LOG for the version string and tools/list check).

## Deferred / open
- B-109 history goes stale after a game restart (unconfirmed), B-110 experiment replies are not read back and undo/redo name the reverse op, B-111 hypothesis iterations are never restored and variations leave object/array values untouched, B-112 `timeout` does not bound the run and the client lock is held throughout.
- B-074, B-075, B-080, B-082 (whole-module deletions) still need the owner's OK naming each path; B-049; B-092; B-093; B-021; B-017.
- Issues #1 and #2 (stale) were not worked.

## Next focus
Finish `hypothesis`: B-111 (restore what an iteration changed, or refuse a variation over a delete) and B-112 (one deadline per run, take the client per iteration). Then the default-armed state ring from the last pass, which needs an eval whose answer is a frame the fixture has already passed.

## Cleanup
Removed what this pass created: `/tmp/fairy_live_smoke.py`, `/tmp/fairy_clippy.json`, `/tmp/fairy_by_lint.tsv`, `/tmp/fairy_by_file.tsv`, `/tmp/fairy_metrics.json`, `/tmp/fairy_metrics2.json`, `/tmp/m3.json`; no worktree was made. Left for a person (not mine): `/tmp/fairy-gate-baseline.log` and `/tmp/fairy-gate-targeted.log`, and `/home/ladvien/target/fairy-loop` at 478 GB.
