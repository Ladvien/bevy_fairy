# 2026-10-30 profiler-history: B-018 fixed, frame-history timestamps were ages, not times

Focus: priority order · Outcome: fixes only (patch release)

## Summary
Open issues #1 (stale, answered by owner) and #2 (a question) are not workable. Of the open backlog, B-018's ignored test turned out to be a real bug: `SystemProfiler::get_system_history` stamped each sample with `frame.start_time.elapsed()`, the frame's age, so timestamps fell as time passed.

## Bugs fixed
### B-018: ignored `test_profiling_history`
Must-fix: misleads yes (data labelled timestamp, non-monotonic), blocks no. Reached only through the library API, not a registered MCP tool.
Fix: `FrameData` stores `started_at_us` (wall-clock, same clock as `collect_sample`); history uses it. Failing-first check: `tests/system_profiler_integration_tests.rs::test_profiling_history` (un-ignored; failed with "Timestamps should be ordered" before, passes after).

## Evidence
Evals not run: no tool, schema or description changed. Workspace tests green; `scripts/ratchet.sh`: OK; `known_red` lost `test_profiling_history` (ignored tests 4 -> 3).

## Test and eval diffs
Removed `#[ignore]` from `test_profiling_history`; assertions untouched.

## Release and install
0.6.0 -> 0.6.1 (patch).

## Deferred / open
B-013 remainder, B-044, B-047, B-020 clippy debt.

## Next focus
B-044/B-047 description fixes measured with evals, or the next B-013 deletion.
