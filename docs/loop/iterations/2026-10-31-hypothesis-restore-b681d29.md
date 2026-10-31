# 2026-10-31 hypothesis-restore: hypothesis varied nothing, piled up changes in the live game, and timed one action; experiment never read a write back

Focus: none given (priority order) · Commits: 0829adc..b681d29 · Outcome: fixes only (three open must-fix bugs from the last pass, B-110, B-111, B-112; no Phase 2 change; no release)

## Summary
The last pass left three bugs it had read but not run. All three reproduced against the fixture, and a skeptic-pass review of the first fix found six more in the same code. `hypothesis` now varies the fields inside a component (it varied nothing for any struct), reverses each iteration's changes so iterations are independent trials, and bounds the run with one deadline while freeing the BRP client between iterations; `experiment` reads a `modify` back, answers `undo`/`redo` as what they are, and keeps redo entries pointed at a respawned entity. No release: v0.10.0 was tagged 3 h ago and the 6 h rule holds the changes for the next one.

## Bugs fixed

### B-111: hypothesis variations changed nothing, and iterations accumulated in the live game
Must-fix: misleads yes, blocks no, common/no workaround yes. `grid`, `boundary` and `fuzz` tested `is_number()` on a component's value; a Bevy component is an object, so every iteration ran the base action and was counted as a separate trial (the interval narrowed, nothing was tested). Separately, spawns and modifies were never reversed, so "one spawn makes exactly one more crate" held once and was violated by the leftovers of the iterations before it.
Fix: leaf-level generators (`for_each_written_leaf`: objects, arrays, batches), integer literals stay integers under `random`/`fuzz`, `grid`/`boundary` write what they were given, no infinity (it serialised to `null`), `include_negative` honoured (`-1` was always tried), `fuzz` no longer rewrites a delete's id to a random number below 1000; a variation that changes nothing, and `iterations: 0`, are refused before anything runs; each iteration is reversed after its condition is read (`ActionExecutor::restore_all`, newest first, `action_succeeds` probes included) and `game_state` counts `actions_reversed` and `left_applied`.
Failing-first (all red on the unchanged source for the stated reason): `fixture_ground_truth::hypothesis_variation_reaches_the_fields_inside_a_component` (held 0 of 2, expected 1), `::hypothesis_refuses_a_variation_that_would_change_nothing`, `::hypothesis_puts_the_game_back_after_every_iteration` (held 1 of 5, expected 5), `::hypothesis_says_what_it_could_not_put_back`.

### B-112: `timeout` bounded one action and the client lock was held for the whole run
Must-fix: blocks yes (a long run made every other tool wait). Fix: one deadline for the run, checked between iterations once the client is held; the client is taken per iteration; `iterations_planned`, `varied_iterations` and `stopped_early` in the reply; iterations that never ran count against CONFIRMED like not-evaluated ones.
Failing-first: `::hypothesis_timeout_bounds_the_whole_run_and_leaves_the_client_free` (a concurrent `status` waited the whole run; 400 iterations ran to the end).
The first draft cut an action off with `timeout_at`. The full suite, under load, caught the consequence: an action cut off mid-flight had reached the game and the executor had never recorded it, so a spawned crate leaked into the next check. The deadline is now checked only between iterations (CLAUDE.md says why).

### B-110: `experiment` replies were not read back, and undo/redo answered as something else
Must-fix: misleads yes. A `modify` the game accepted and a system overwrote the next frame read `success`; `undo` of a spawn answered `action_type: "delete"`; redo of a spawn made a new entity while the redo entries behind it kept the old id.
Fix: `read_back {matches, components}` taken after the write, with a message that says both halves when it differs; `action_type` `undo`/`redo` with `of_action`; `retarget` on the waiting redo entries.
Failing-first: `::experiment_modify_reads_the_entity_back_and_says_when_it_does_not_hold_the_value` (the spinner's `frames` counts up every frame, so a write of 1000000 cannot still be there), `::experiment_undo_and_redo_say_what_they_did_and_redo_follows_a_respawned_entity`.

### Found by the independent review of 0829adc (fixed in f5d36ec)
Four were reproduced as failing fixture tests on the committed source before they were fixed (src stashed, tests kept); the other two have unit tests written with the fix (the unreadable condition cannot return a result on the old code; the string default is covered by `grid_only_multiplies_the_kinds_of_value_the_actions_write_and_leaves_unlisted_kinds_alone`, which was not run against the old code). An atomic batch whose delete succeeded and whose next action was refused left the crate deleted while `game_state` said everything was reversed (`::hypothesis_counts_what_an_atomic_batch_could_not_roll_back`); the iteration executor kept 100 actions and forgot the 101st spawn unreversed and uncounted (`::hypothesis_reverses_every_action_of_an_iteration_however_many_there_are`: 100 reversed, 101 spawned); grid values were rounded over an integer literal, so `grid [0.5, 1.5]` over `{"weight": 5}` wrote 1 and 2 (`::hypothesis_grid_writes_the_listed_values_as_given_over_an_integer_literal`); `iterations: 0` was NOT_EVALUATED with no reason (`::hypothesis_refuses_an_empty_plan_and_names_the_argument`); an unreadable condition aborted the run and dropped the counts (unit test `a_condition_that_cannot_be_read_ends_the_run_with_what_was_counted`, which cannot return a result on the old code). Also changed: `grid`'s `string_values` has no default (it was `"default"`, which overwrote every string once variation reached nested values), the deadline is checked after the client wait, and the `game_state` note lists a modify of a component the entity lacked.

## Change
None. The three bugs above were the pass; Phase 2 was skipped (three must-fix bugs plus six review gaps in one code path is the "fix the most misleading and stop" case).

## Evidence
| Suite | Before (v0.10.0 binary, same session) | After |
|---|---|---|
| dev evals (success / mean calls / invalid) | 6/6, 5.0, 0 | 0829adc: 6/6, 6.2, 0 · b681d29: 5/6, 4.0 on the 5 solved, 0 |
| spinner_stops reruns | 3/3 (16, 13, 10 calls) | 3/3 (9, 8, 10 calls) |
| held-out evals | not run (no release this pass) | not run |
| manual mutants on touched handlers | n/a | 13 of 13 killed (11 in round A, 2 in round B; the `restore_all` ordering mutant is caught only because a test for it was added first) |
| ground-truth tests (fixture_ground_truth) | 45 | 57 |
| workspace tests | 441 | 460 (1 ignored, as before) |

Transcript excerpt, `spinner_stops` on b681d29, the one failed run: the agent read the frozen angle once and answered "I can't give the exact frame it stopped on, only that it was on or before frame 510" after 4 calls (fact `stop_frame_in_110_130` missed; honest, not wrong). It touched neither `hypothesis` nor `experiment`; the task's pass rate over the log was 25 of 31 before this pass, and the three reruns on each binary passed. Logged as B-117, not as a regression of this change.

Live proof through a real MCP client over stdio (`tools/list` 14 tools, then calls against the fixture), b681d29:
```
experiment modify crate weight 99      -> read_back {"matches": true,  weight 99.0}
experiment modify spinner frames 1e6   -> "The game accepted the write ..., but a read back after it shows the entity does not hold what was written for fixture_game::Spinner"; read_back {"matches": false, frames 1000001}
experiment undo x2                     -> action_type undo, of_action modify, "Undid `modify`: ..."; crates back to [-7.5,1,2,3,4,5]
hypothesis one spawn = one more crate (random x5) -> CONFIRMED held 5/5, actions_reversed 5, left_applied 0; crates after: 6
hypothesis grid [10,20] over crate weight, claim 20 -> INCONCLUSIVE held 1 violated 1, weights restored: True
hypothesis variation grid, no actions  -> isError: variation_type `grid` ... there are no `actions` to vary ... Nothing was run
hypothesis 400 iterations, timeout 1   -> 1.1 s, planned 400 run 21, "the 1 s timeout passed after 21 of 400 iterations", crates after: 6
```

## Metrics
Server LOC 20503 -> 20936 (+433): the variation rewrite nets about +60, `run` +90 (restore, deadline, plan checks), `experiment_system.rs` +190 (read-back, `Restoration`, redo retargeting), about 120 are in-file unit tests. Clippy 69 -> 69, every touched file clean; tools 14; direct dependencies 52; plugin LOC unchanged.

## Review
An independent `reviewer` agent with only commit 0829adc, the plan and the gate output reported 3 material and 4 minor gaps (listed above). Skeptic pass: gap 3 (integer read off the literal) was half benign, since the old code wrote a float over an integer literal and `u32` fields refused it, but rounding a *listed* value was a silent distortion and was fixed; the part that remains (an `f32` field written as `5` is varied in whole numbers by `random` and `fuzz`) is documented and logged as B-115. Gap 7a (`quick_test` overriding `iterations` and `timeout`) was already the case before this pass; the argument text now says so and B-116 records it. The reviewer's "read-back timing" note (one read a frame or so after the write) is now in the tool description. A second round was not needed.

## Test and eval diffs
- `tests/fixture_ground_truth.rs`: 12 tests added, none changed or removed.
- `tests/hypothesis_assertions.rs`: one import and one line, `a.evaluate(&mut c, &mut Restoration::default())`; `Assertion::evaluate` gained the `&mut Restoration` parameter (an `action_succeeds` condition's own action is reversed and counted). No assertion changed.
- `src/server/hypothesis_system.rs` unit tests: `counted()` gained the new `TestResult` fields; seven tests added; none removed.
- No eval task, held-out task or protected check changed.

## Release and install
No release: v0.10.0 is 3 h old (the 6 h rule, measured from the tag's commit time), so these changes ride in the next release; CHANGELOG `Unreleased` carries them, with the breaking part named (reply shape, a variation that changes nothing is refused, `grid` `string_values` default). `cargo semver-checks` was not run; it belongs to the release commit.
Installed binary: `cargo install --path . --locked --force` from the pushed commit a04a2b5 (`bevy-fairy --version` says 0.10.0, because nothing was bumped; it carries the unreleased changes). The same live script run against the installed binary showed 14 tools and the same `read_back`, `game_state`, refusal and `stopped_early` replies. Downstream copies of this crate (bevy_emerge pins a git revision) pick the change up when they move their pin to `main`.

## Cleanup
Removed what this pass created: `/tmp/fairy_*.log`, `/tmp/fairy_live_smoke.py`, `/tmp/fairy-eval-new`, `/tmp/fairy-eval-fixture`, and the nine `evals/transcripts/2026-10-31-*` directories (git-ignored); no worktree was made and the stash is empty. Left for a person (not mine): `/tmp/fairy-gate-baseline.log` and `/tmp/fairy-gate-targeted.log` from earlier passes (7 KB), and `/home` is at 96% (148 GB free) with `/home/ladvien/target/fairy-loop` the shared build cache.

## Deferred / open
- B-109 (experiment history is stale after a game restart: `undo` can delete the entity that now has the recorded id) is still unconfirmed; the fix needs a game-instance token in `bevy_fairy/status`, a plugin change.
- New: B-114 (entity ids stored inside component values are not retargeted on redo), B-115 (integer-ness is read off the literal, not the field type), B-116 (`quick_test` overrides `iterations`/`timeout`), B-117 (`spinner_stops` fails about one run in five).
- B-074, B-075, B-080, B-082 (whole-module deletions) still need the owner's OK naming each path; B-020, B-021, B-042, B-043, B-049, B-092, B-093, B-017.
- Issues #1 and #2 (stale, no `consumer` label, no enhancement) were not worked; no consumer issue was open.

## Next focus
Either B-109 (a game-instance token in `bevy_fairy/status`, so `experiment` can refuse a stale history; plugin and server, needs a ground-truth test that restarts a fixture) or B-117 (make a frozen value's stop frame answerable from one `watch` call, tuned against the dev task only). B-109 is the one that can delete the wrong entity.
