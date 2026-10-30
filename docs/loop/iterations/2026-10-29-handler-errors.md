# 2026-10-29 tools: handler errors are tool errors; empty `experiment execute` refused

Focus: none · Outcome: fixes only

## Bugs fixed
### B-048: handler `Err`s left as JSON-RPC internal errors
Must-fix: misleads no, blocks no — client may hide protocol errors from the model.
Fix: six wrappers in `src/server/mcp_tools.rs` return `tool_err(..)` · Failing-first check: `tests/tool_contract.rs::a_handler_error_comes_back_as_a_tool_error_the_model_can_read` (failed before: protocol error).
### New: `experiment execute` with no `actions` succeeded with zero results
Must-fix: misleads yes (empty success). Cause: `ExperimentRequest.actions` defaults to `[]`. Fix: `handle_execute` refuses an empty list. Same test (found by it).

## Evidence
Workspace `cargo test --workspace --no-fail-fast` all green; `scripts/ratchet.sh src/server/mcp_tools.rs src/server/tools/experiment.rs`: OK. Dev/held-out evals and mutation testing not run (host load ~25, loop run without an LLM budget); no tool description changed.

## Release and install
0.3.3 → 0.3.4 (patch; no signature change).
## Next focus
B-010/B-045: input and watch tools.
