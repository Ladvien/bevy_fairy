# 2026-10-30 anomaly-frames: detect_anomaly told the truth about what it read; observe and detect say which frames they read between

Focus: none given · Commits: c1d3cad, 525ee7e · Outcome: shipped (no release: v0.9.0 was tagged 3.8 h before this pass started, the 6 h rule)

## Summary
`detect_anomaly` returned `anomalies: []` and `entities_analyzed: N` for games in which none of its detectors found anything to read, every detection carried severity exactly 1.0, and its detectors could not match a component the way BRP sends it, so against a real game it could not fire at all. Four bugs (B-087..B-090) are fixed, each with a test that failed on the old code. The one change is frame provenance: `observe` and `detect_anomaly`'s `detect` replies now say which game frames they were read between (the tool contract's "every result carries its provenance").

Host: load 41-48 for the whole pass, 5 GB available, 25 of 31 GB swap in use (four other loops and the model servers). Every cargo command ran under `systemd-run --user --scope -p MemoryMax=8G -p MemorySwapMax=0 ... -j 3`. Following the standing rule for load above 35, dev and held-out evals and mutation testing were not run; see Evidence.

## Bugs fixed
### B-087: severity was exactly 1.0 for every physics and entity-growth detection
Must-fix: misleads yes, blocks no, common/no workaround no (`min_severity` filters on a constant) · `severity = (value / threshold).min(1.0)` ran only inside `if value > threshold`, so the ratio was already above 1.
Fix: `severity_past(value, threshold) = 1 - threshold / value` (0 at the threshold, 0.5 at twice it, towards 1) · Failing-first check: `anomaly_detector::tests::entity_growth_severity_rises_with_how_far_past_the_threshold_it_is` (failed with `1 1 1`) and `physics_severity_rises_with_how_far_past_the_z_score_threshold_it_is` (metamorphic: the same data under a stricter threshold is further past it).

### B-088: two arguments no detector read, three anomaly types no detector emitted
Must-fix: misleads yes · `configure` accepted and echoed `iqr_multiplier` and `performance_threshold` (the latter described to the agent as a frame time in ms); `status` listed `PotentialMemoryLeak`, `PerformanceSpike`, `RapidValueChange`.
Fix: both arguments and the three variants removed; `supported_anomaly_types` comes from `AnomalyDetector::emits` · Failing-first: `tools::anomaly::tests::status_lists_the_anomaly_types_the_detectors_emit_and_no_others` (drives each detector, compares sets; failed with six listed against three emitted), `tool_contract::detect_anomaly_refuses_thresholds_no_detector_reads`.

### B-089: an empty result from detectors that read nothing looked clean
Must-fix: misleads yes · Against the fixture, `detect` answered `anomalies: []`, `entities_analyzed: 102` while PhysicsDetector and ConsistencyDetector found nothing they read and PerformanceDetector held 1 of the 10 samples it needs.
Fix: each detector reports `reads`, `examined`, `evaluated`, `status`; `note` when any did not evaluate; `entities_analyzed` became `entities_read` (it counted what the query returned) · Failing-first: `fixture_ground_truth::detect_anomaly_reports_what_the_detectors_could_not_read_in_the_fixture` (ground truth: the fixture has no Velocity, Health or Alive, and a constant entity count), `tool_contract::detect_anomaly_says_what_each_detector_could_read`.

### B-090: the detectors could not read a real game
Must-fix: misleads yes, blocks yes (the physics and consistency detectors could never fire against any BRP game) · `world.query` keys a component by its full type path and sends a `Vec3` as `[x, y, z]` (the existing `mover_x` helper reads `translation[0]`); the detectors looked up the literal key `Velocity` and an `{x, y, z}` object. An entity with no `Alive` component was reported as "marked as alive".
Fix: `component_named` matches the whole last path segment, `vector_of` reads either form, `health_and_alive` requires both components · Failing-first: `the_physics_detector_reads_a_velocity_as_brp_sends_it`, `the_consistency_detector_reads_health_and_alive_as_brp_sends_them`. `zero_health_with_no_alive_component_is_not_an_inconsistency` and `a_component_whose_name_merely_ends_in_health_is_not_health` passed before the fix only because the detector read nothing; they guard the new matching.

### Found by the independent review, fixed before shipping
`PhysicsDetector` scored a speed against a window that contains it, so among n samples a z-score is at most (n - 1) / sqrt(n) = 2.85 at the default 10, under the default threshold 3; it counted such an entity as `evaluated`. Now it is `examined`, not `evaluated`, and the status says why (`physics_coverage_says_when_the_threshold_cannot_be_reached_with_the_samples_held`). The fixed 0.9 rating of a Health/Alive contradiction is now stated in the tool description and the `min_severity` documentation. A changelog sentence claimed `diff` stamps say how long a change took; reworded.

## Change: frame provenance on `observe` and `detect_anomaly`
Plan: `BevyDebuggerTools::frame_now` reads `bevy_fairy/status`; `stamp_frames` puts `frame` and `frame_until` (the counter before and after the handler) on a successful reply, or `frame_unavailable` with the game's own reason; refusals and `configure`/`status` actions are left alone · Files: src/server/mcp_tools.rs, tests/fixture_ground_truth.rs, tests/tool_contract.rs, CHANGELOG.md, CLAUDE.md, docs/mcp-tools.md · Goal: an agent that reads twice can tell fresh evidence from stale · Proof: `fixture_ground_truth::observe_brackets_the_frame_every_read_was_taken_on` (failed with "the reply carries no frame"; ten reads of the spinner, whose own frame counter must fit every bracket with one constant offset, and frames strictly increase 100 ms apart), `tool_contract::a_read_from_a_game_without_the_plugin_says_why_it_has_no_frame` (no invented frame; `frame_unavailable` names `bevy_fairy/status`).
Candidates considered: B-021 pixel ring buffer beside `burst` (large, needs its own eval task first); a `hypothesis` stress_test audit (the tool's 1,100-line system was last audited at B-067; no failing check found yet); frame provenance (chosen: named in the tool contract, no tool but `watch` had it, ~45 lines).
Cost: two extra `bevy_fairy/status` calls per stamped read. Measured live over stdio: the first `observe` was bracketed `frame` 149 to `frame_until` 153 beside spinner.frames 152, the second 182 to 185 beside 184.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals | 6/6, 5.2 calls, 0 invalid (420019e) | not run (load 41-48) |
| held-out evals | 3/3 (917874c) | not run (no release this pass) |
| mutation score on touched handlers | not run | not run (load) |
| `cargo test --lib anomaly` | 5 failed / 21 passed (new tests, old code) | 27 passed |
| `fixture_ground_truth` | 22 passed + 2 new failing | 26 passed |
| `tool_contract` / `tool_list` / `docs_drift` / `instructions` | | 11 / 7 / 11 / 2 passed |
| `scripts/ratchet.sh` | | OK on c1d3cad's tree and on the final tree (known_red 79 -> 74) |

Live proof through a real MCP client (stdio, `bevy-fairy` debug binary against `examples/fixture_game`; scratch script removed): `detect` twice returned `frame` 215..217 then 218..220, `entities_read` 102, PhysicsDetector "not evaluated: no entity has a `Velocity` component with a `linear` vector", PerformanceDetector "holds 2 of the 10 samples it needs; call detect again", and the `note`. `configure` with `iqr_multiplier` came back as `-32602 unknown field `iqr_multiplier`, expected one of `action`, `window_size`, ...`. `status` carries no frame stamp.

Not measured: the dev eval suite does not exercise `detect_anomaly`; the frame fields are additive to `observe`, whose eval tasks read them as extra keys. This is a weaker claim than "no eval regression" and the next pass with load under 35 should run the dev suite to close it.

## Metrics
server loc 19720 -> 20106 (+386: coverage and `emits` in anomaly_detector.rs, the reachability and BRP-shape handling, the frame helpers, and about 230 lines of new tests inside the same files). plugin loc 3561 unchanged. dependencies unchanged (52). public modules unchanged. MCP tools 14 unchanged. known_red clippy 79 -> 74 (anomaly_detector.rs 5 -> 0: `manual_range_contains` 3 and `cloned_ref_to_slice_refs` 2, in two old tests rewritten to equivalent forms). binary size re-measured at install.

## Review
Independent reviewer (`reviewer` agent, read-only, given the diff and plan): three gaps, all real after the skeptic pass, all fixed above: Samuelson-bound `evaluated` claim (P2), fixed 0.9 severity described as a measurement (P3), changelog over-claim about `diff` (P3). It checked `severity_past`, `vector_of`, `health_and_alive`, the other coverage claims, `stamp_frames` ordering and key collisions, and the shared global state in `tool_contract` and found them sound. One round.

## Test and eval diffs
- `tests/tool_contract.rs::detect_anomaly_analyses_the_live_game_with_the_given_thresholds`: the pinned key `entities_analyzed` became `entities_read`, same value (2) asserted. The old key is the over-claim B-089 removes.
- `src/server/anomaly_detector.rs::test_quartiles` and `test_physics_detector`: `q >= a && q <= b` became `(a..=b).contains(&q)` and `&[entity.clone()]` became `std::slice::from_ref(&entity)`. Equivalent, and they clear five known_red warnings in a file this pass touched.
- `CHANGELOG.md` migration table row for `detect_anomaly`: `iqr_multiplier` and `performance_threshold` removed from the "after" cell, because `docs_drift::the_changelog_migration_table_names_the_fields_the_tools_take` requires the cell to name only fields the tool takes.
- New tests only otherwise. No eval task, held-out task or protected check changed.
- Process note: one edit to CHANGELOG.md was made with a Python one-liner instead of the edit tool.

## Release and install
No release: v0.9.0 was tagged 2026-10-30 14:21 CDT and this pass began 18:08, short of the 6 h rule (next release due after 20:21 CDT). The next release is breaking (semver-checks will say so): `AnomalyType` lost three variants, `AnomalyConfig` lost two fields, `AnomalyDetector` gained two required methods, `detect_anomaly` lost two arguments and `summary.entities_analyzed` was renamed. 0.9.0 is on crates.io and docs.rs built it (`status.json` `doc_status: true`); the installed binary was 0.9.0 at the start. Installed server rebuilt from 525ee7e with `cargo install --path . --locked --force`: `bevy-fairy --version` prints 0.9.0 (the crate version only moves at the release, so the installed binary is ahead of the published 0.9.0), `tools/list` over stdio returns 14 tools, `detect_anomaly` has no `iqr_multiplier` or `performance_threshold`, `observe` mentions `frame_until`, 104850880 bytes. No fixture or server process left running. Downstream copies pick the change up by their own git pin (bevy_emerge) when they move it.

## Deferred / open
- B-074, B-075, B-080, B-082 (whole-module dead-code deletions) still need the owner's OK naming each path.
- B-021: a frame/image ring beside `burst`.
- `detect_anomaly` samples are kept per server process, and `velocity_history` keeps an entry for every entity ever seen with a Velocity, so entity churn grows it without bound. Not reproduced; not fixed.
- The anomaly detectors still read only `Velocity`/`Health`/`Alive` by name. A game that names its components otherwise gets "not evaluated" with the reason, which is honest, but a configurable component name would make the tool useful there.

## Next focus
Run the dev and held-out suites at load under 35 and add an eval task that needs `detect_anomaly` to say a game has no anomaly it could have found (the fixture has none of its components), then decide whether the detectors should take component names from the caller.
