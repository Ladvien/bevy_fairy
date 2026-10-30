# 2026-10-30 tooling: request structs are the only argument list (B-024)

Focus: none · Outcome: shipped (no release, 6 h rule; last tag v0.7.2 is 5.5 h old)

## Summary
`experiment`, `hypothesis` and `detect_anomaly` built the handler's argument object from a hand-written `json!` list of the request struct's fields, a second place a field could be added and forgotten. They now serialise the request struct (`request_args`), so the advertised schema and the handler input cannot drift.

## Change
Files: `src/server/mcp_tools.rs` (Serialize on three request structs, `request_args`, three handlers). Proof: refactor with no behaviour change; `tests/tool_contract.rs` (sends every advertised field and enum value), `tests/docs_drift.rs`, `tests/fixture_ground_truth.rs` pass. No failing-first check exists for a pure single-source-of-truth refactor; stated plainly.

## Evidence
Ratchet: OK, `mcp_tools.rs` clippy-clean. Workspace tests: see LOG. Evals/mutation not re-run: no behaviour change and host load ~28.

## Test and eval diffs
none

## Release and install
No release (6 h rule). Installed server not rebuilt for an unreleased refactor.

## Next focus
B-021 frame ring buffer with trigger dump; B-020 clippy debt.
