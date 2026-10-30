# 2026-10-30 replay-schema: replay's schema matches what its handlers read

Focus: none given - Commits: this one - Outcome: shipped (no release: 6 h rule, v0.7.2 was 2.4 h old)

## Summary
`replay`'s schema offered `action: string`, an unread `checkpoint_id`, and `speed` defaulted to 1.0, while the handlers behind 21 actions read 25 arguments `deny_unknown_fields` refused. `observe.detailed` was read by nothing.

## Bugs fixed
### B-061 (widened): replay/observe/scene schema
Must-fix: misleads yes (set_speed without speed set 1x silently; ignored `checkpoint_id`/`detailed`), blocks yes (save filename, seek frame, branch args unreachable).
Fix: `ReplayRequest` carries every argument with per-action docs; `ReplayAction`, `StepDirection`, `MergeStrategyArg` enums; `Option<f32>` speed; `checkpoint_id` and `observe.detailed` removed; descriptions added. Failing-first checks: `tests/tool_list.rs::replay_advertises_its_actions_and_every_argument_they_read`, `every_advertised_argument_has_a_description` (schema vs handlers; the old schema lacked every field).
Live (stdio, no game): `set_speed` without speed -> `Missing 'speed' parameter` (was silent 1x); `step direction=sideways` -> unknown variant, expected forward|backward; `save checkpoint_id` -> unknown field.

## Evidence
Ratchet OK; `cargo test --workspace --all-features` green. Evals not run (host load ~20; description-only change to a tool the fixture evals do not use).

## Metrics
Tool count unchanged (13); mcp_tools.rs grew by the argument structs (schema is the one source).

## Review
No subagent review (schema-only change, checked by tools/list tests).

## Test and eval diffs
Added two tests to tests/tool_list.rs; none weakened.

## Release and install
No release: v0.7.2 tag 2.4 h old (<6 h). Next is 0.8.0 (breaking: removed `checkpoint_id`, `observe.detailed`, `variation_type` values).

## Deferred / open
B-021 burst capture, B-020 clippy debt (186), B-024, B-042. Issues #1 (stale, answered) and #2 (question) not workable.

## Next focus
B-021 burst capture: write the eval task first.
