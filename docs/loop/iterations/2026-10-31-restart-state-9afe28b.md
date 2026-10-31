# 2026-10-31 restart-state: server state outlived the game; three tools wrote or judged across a restart

Focus: (none given) · Commits: 94cfc20..c52e818, tag v0.11.0 on 3540366 · Outcome: fixes shipped, v0.11.0 released

## Summary
The MCP server outlives the game it talks to, and three tools kept state that names entities by id: `experiment`'s undo history, `observe`'s diff baseline, and `detect_anomaly`'s detector windows. A restarted game gives the same ids to other entities, so `undo` wrote the old game's original values into the new game (B-109, the one the last pass named "can delete the wrong entity"), a diff reported the difference between two games as a change (B-118), and an entity of the new game was flagged against the speeds of the old one (B-119). `bevy_fairy/status` now reports an `instance` (one value per game process) and each store binds its state to it. A fourth bug, `quick_test` silently dropping the caller's `iterations` and `timeout` (B-116), was fixed on the way. No Phase 2 change: four reproduced must-fix bugs of one class were the pass.

## Bugs fixed
### B-109: experiment's history outlived its game
Must-fix: misleads yes, blocks no, common/no workaround no (`clear_history` after every restart, which nothing told the agent to do). `undo` of a modify put game 1's original value on game 2's entity.
Fix: the history belongs to the game process that ran its first action. `undo` and `redo` refuse a stale history and discard it; `execute` discards it and says so (`history_discarded`); `history` says `stale`, `recorded_against`, `redoable`. A game that cannot name itself leaves the history unverified (`game_instance_unverified`); a history recorded against a game that did name itself refuses everything while that game cannot be asked. Root cause: `EXPERIMENT_STATE` is a process-wide `OnceLock` keyed by nothing but entity id.
Failing-first, all red on the b681d29 tool (`e35dd4b`): `status_names_the_game_instance_and_a_second_game_has_another`, `experiment_undo_refuses_a_history_recorded_against_another_game`, `experiment_execute_drops_the_history_of_a_game_that_is_gone_and_says_so` (ground truth: the second game is a second fixture app in the same process, its crate mutated to 42.0, and the assertion is that 42.0 survives the undo).

### B-118: observe's diff compared two games
Must-fix: misleads yes. `observe_diff_does_not_compare_a_read_with_one_of_another_game` failed with `changes: [{EntityAdded ...}]` and no `baseline`. `ObserveState` records the instance with the snapshot; another process is `Baseline::AnotherGame`, a new baseline that says why (`b3b693e`). Found while fixing B-109: the same class.

### B-119: detect_anomaly scored a restarted game against the old one
Must-fix: misleads yes. `tests/anomaly_restart.rs` (a fake BRP endpoint whose instance changes; the fixture has no `Velocity`) failed with "Entity 7 velocity magnitude 50.00 is 3.75 standard deviations from mean 4.06", a mean made of fifteen readings of 1.0 taken from the other game. `AnomalyState::settle` drops the samples (configuration kept) and says so in `history_discarded` (`a462b09`).

### B-116: quick_test dropped the caller's arguments
Must-fix: misleads yes (small). `hypothesis_quick_test_refuses_the_arguments_it_would_ignore` ran 10 iterations for `iterations: 3`. It refuses `iterations` and `timeout` now, naming `test`.

## Change
None. Phase 2 is for when no bug can be worked; four reproduced must-fix bugs of one class were. Left in the backlog: B-120 (an unnamed history followed by a named game stays unverified, found by the review), B-117 (spinner_stops eval flake), B-093, B-092.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals (success / mean calls / invalid) | v0.10.0: 6/6, 5.5, 0 | b3b693e: 6/6, 4.7, 0 |
| held-out evals | v0.10.0: 3/3, 4.7 calls, 0 invalid | 3540366 (release commit): 3/3, 3.7 calls, 0 invalid |
| manual mutants on the touched handlers | n/a | 8 applied one line at a time, 8 killed |
| tests listed (`cargo test --workspace -- --list`) | 460 | 470 |

Mutants (each failed a named test, then was reverted): `adopt` not clearing a stale history (`experiment_execute_drops...`, `experiment_undo_refuses...`), `then != now && false` in observe (`observe_diff_does_not_compare...`) and in anomaly (`anomaly_restart`), `quick_test` refusing nothing (`quick_test_refuses...`), and four arms of `standing_of` (four unit tests). Not run: `cargo mutants` (not installed on this host); the full gate is `scripts/ratchet.sh`, OK on every touched file and on the release commit.

Live proof through a real stdio MCP client against the fixture, one server process across a game restart on the same port. Run against the debug build of 9afe28b and again against the installed 0.11.0 release build; the lines are the same:
```
status.instance (game 1): 2675722-18e38c7d618b5989-0
quick_test with iterations -> Hypothesis tool error: Validation error: `quick_test` fixes `iterations` at 10 and `timeout` at 10 s, so it would ignore `iterations`. `test` takes `iterations`. Nothing was run.
execute modify -> ok True read_back.matches True
history (game 1): count 1 stale False recorded_against 2675722-18e38c7d618b5989-0
--- the game restarts on the same port ---
status.instance (game 2): 2675792-18e38c7dacb64c45-0 | differs: True
history (game 2): count 1 stale True
undo -> isError | history belongs to another game | The 1 recorded action(s) were taken against another game process than the one answering ...
crate weight in game 2 after the refused undo: 42.0 (game 1's original was 1.0 )
observe diff (game 2): baseline True | The earlier `diff` read of this query was of another game (the game restarted since) ...
```
Transcript check (dev, b3b693e): no invalid calls, no repeated identical calls; `spinner_stops` (8 calls) used `watch`, `run bevy_fairy/ring` and `experiment modify` on the spinner, `mover_speed` used `watch` once.

## Metrics
Server LOC 20936 -> 21266 (+330), plugin 3561 -> 3588 (+27), tests 460 -> 470, direct dependencies 52 unchanged (plugin 4: `GameInstance` uses std only; `tests/plugin_leaf.rs` passes), tools 14, clippy 69 unchanged, installed binary 104895024 -> 105158736 bytes (+0.25%). About 190 of the server lines are the experiment history's binding and its unit tests.

## Review
An independent `reviewer` agent with only `e35dd4b` and `b3b693e` found no path where an action reaches another game, and three lesser gaps, all handled in `9afe28b`: (P2) an unverified record quoted a reason saved at record time that can be false of the game now answering. The wording now says the game could not be confirmed as the one that ran the actions; the stronger rule (a process cannot gain a plugin, so an unnamed record followed by a named game is certainly stale) is B-120, same arm in `observe.rs` and `anomaly.rs`. (P3) `history` counted only the undo stack beside a stale check over both: `redoable` added. (P3) the tool description said a silent game leaves the history unverified, when a history recorded against a named game refuses everything: corrected. Skeptic pass: none of the three let an action reach another game; the first is partly benign (the reply does carry a warning).

## Test and eval diffs
- `tests/fixture_ground_truth.rs`: 5 tests added. Two existing ones changed, none weakened: `tools_answer_as_soon_as_the_game_is_up_not_after_the_next_heartbeat` now takes `EXPERIMENT_TURN` (its `experiment undo` read and could clear the process-wide history the other experiment tests use; with the guard it cleared another test's history, 3 of 5 full runs failed, 15 of 15 pass with the turn), and `experiment_reports_a_refused_action_beside_the_ones_that_ran` asserts what `undo` answered before counting crates.
- `tests/anomaly_restart.rs` is new (one test; the detectors are process-wide).
- No eval task, held-out task or protected check changed.

## Release and install
Version 0.10.0 -> 0.11.0 (minor-breaking) · semver-checks: 6 major checks (`EntityFactory` removed; `TestResult`'s `successes`, `failures`, `failure_examples` replaced), 0 minor · Channel: `cargo publish` from this host at 03:23 CDT, the first minute after v0.10.0's tag commit was six hours old; the pre-publish gate ran on the release commit `3540366` (ratchet OK, `RUSTDOCFLAGS=-D warnings cargo doc` OK, package list 100 files with no `docs/loop`, `evals/transcripts` or secrets, `cargo publish --dry-run` OK, held-out 3/3) · Tag: annotated `v0.11.0` on `3540366`, pushed.
crates.io: https://crates.io/crates/bevy_fairy/0.11.0 · docs.rs: https://docs.rs/bevy_fairy/0.11.0 (404 at 03:25, build queued; check next iteration; 0.10.0's docs.rs build succeeded in 1 m 1 s) · Installed binary: 0.11.0 (`cargo install --path . --locked --force` at `c52e818`, 105158736 bytes, `tools/list` 14 tools); a build from crates.io in a scratch `CARGO_HOME` (`cargo install bevy_fairy --version 0.11.0 --locked --no-default-features --features server`, 7 m 52 s) reports 0.11.0 and lists the same 14 tools. The scratch directory is removed.
Downstream: `bevy_emerge` pins this repo by git revision and picks the change up when it moves its pin to `main`; its game, built with the older plugin, gets `game_instance_unverified` on `experiment`, `observe` with `diff` and `detect_anomaly` until it rebuilds.

## Deferred / open
- B-120, B-117, B-093, B-092 (unconfirmed), B-074/B-075/B-080/B-082 (whole-module deletions still need the owner's OK naming each path), B-020, B-021, B-042, B-043, B-049, B-017.
- Issues #1 and #2 (stale, not labelled `consumer`) were not worked; no consumer issue was open.

## Next focus
B-120 first (small; the P2 the review found, the same arm in three files, and the fake endpoint in `tests/anomaly_restart.rs` already changes instance). Then capability work: B-093 (the anomaly detectors read only Velocity, Health and Alive) needs an eval task that fails today before it is built.
