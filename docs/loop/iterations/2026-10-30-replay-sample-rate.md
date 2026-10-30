# 2026-10-30 replay-sample-rate: B-066 fixed, record honors and validates sample_rate

Focus: honesty audit · Outcome: fixes only

## Bugs fixed
### B-066: replay record ignored sample_rate above 30 and panicked on <= 0
Must-fix: misleads yes — the response echoed `sample_rate: 60` while the recorder task polled every 33 ms; `sample_rate: 0` made `Duration::from_secs_f32(inf)` panic inside the task, leaving `status` "recording" with no frames.
Fix: `parse_recording_config` returns Err for non-finite or <= 0 rates (tool error with example); the task sleeps one sample period (`poll_interval`). Schema text says the reached rate is bounded by the BRP round trip.
Failing-first checks: `replay::tests::record_refuses_a_sample_rate_that_cannot_be_a_period`, `recorder_polls_at_the_requested_rate_not_a_fixed_33_ms` (fail to compile against the old signature; unit tests, not fixture ground truth, no live capture of rate).

## Evidence
Workspace tests green, ratchet OK with replay.rs clean, tool_list docs regenerated. Evals not run (host load ~30; replay is not in the eval set).

## Test and eval diffs
Two existing replay unit tests gained `.unwrap()` for the new Result; assertions unchanged.

## Release and install
No release: last tag v0.7.2 under 6 h old. Installed server not rebuilt (no release).

## Next focus
B-021 ring buffer; B-024 single source of truth for args.
