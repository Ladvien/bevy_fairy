# 2026-10-30 anomaly: start/stop_monitoring removed, they only flipped a flag

Focus: honesty audit of detect_anomaly · Outcome: fixes only

## Summary
`detect_anomaly` `start_monitoring` set an in-process flag and answered "Continuous monitoring started"; no task ever sampled the game (the source comment says so). `status` then reported `is_monitoring: true`. Both actions and the field are deleted; asking for them is the structured "Invalid action" error listing `detect, configure, status`.

## Bugs fixed
### B-069
Must-fix: misleads yes — the agent is told the game is being watched when it is not.
Fix: delete handlers, `is_monitoring` state and status field; schema doc and `docs/mcp-tools.md` regenerated. Failing-first check: `server::tools::anomaly::tests::monitoring_actions_are_refused_not_pretended` (failed on old code, passes now).

## Evidence
Workspace tests green (`cargo test --workspace -j 3` under 8G cap); `scripts/ratchet.sh src/server/tools/anomaly.rs`: OK. Evals and live MCP pass not re-run this iteration (no tool added; removal only).

## Test and eval diffs
Replaced `test_anomaly_monitoring_control`, which asserted the fabricated `is_monitoring: true`, with the test above.

## Release and install
No release: v0.7.2 cut ~5 h ago, under the 6 h rule. Installed server not rebuilt. Breaking for any caller using the two actions (they did nothing); goes in the next release's CHANGELOG as Removed.

## Deferred / open
- B-070: `configure` resets unspecified fields to defaults; free-string action returns Ok(error JSON).

## Next focus
B-070, then B-021 ring buffer.
