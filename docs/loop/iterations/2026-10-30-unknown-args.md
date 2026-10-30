# 2026-10-30 tool-contract: observe, guide and replay refuse unknown arguments

Focus: none given · Commits: 74db8a2.. · Outcome: shipped (no release, 6 h rule: v0.7.2 was 1 h old)

## Summary
Three of the 14 tools lacked `deny_unknown_fields`, so an invented or misspelled argument was dropped and the call ran as if it were absent. The tool contract requires refusal. A test over `tools/list` now guards every tool.

## Change: deny_unknown_fields on ObserveRequest, GuideRequest, ReplayRequest
Plan: add the attribute; add a `tools/list` test requiring `additionalProperties: false` · Files: src/server/mcp_tools.rs, tests/tool_list.rs, CHANGELOG.md · Goal: no tool silently ignores an argument · Proof: `tool_list::every_tool_refuses_arguments_it_does_not_advertise` failed with `["guide","observe","replay"]` before, passes after.
Candidates considered: B-021 burst capture (large, needs eval task first); B-020 clippy debt (186 left); B-024 wrapper structs.
Issues: #1 (stale, answered earlier), #2 (question) — nothing workable.

## Evidence
- Before: test panicked "these tools silently drop unknown arguments: [guide, observe, replay]".
- After: `cargo test --workspace --all-features` all green; `scripts/ratchet.sh src/server/mcp_tools.rs` OK.
- Dev/held-out evals not re-run: host load ~20, and the change only rejects arguments no description advertises. Not a release pass.

## Metrics
No change in LOC beyond +3 lines, deps, tools (14).

## Test and eval diffs
tests/tool_list.rs: one test added; none weakened.

## Release and install
No release: v0.7.2 tagged 1 h ago (6 h rule); rides in next (0.8.0, breaking). Installed binary not rebuilt this pass (no release).

## Next focus
B-021 burst capture with its eval task written first.
