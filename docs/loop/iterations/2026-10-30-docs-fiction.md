# 2026-10-30 docs-fiction: deleted the hand-written tool references that described tools and arguments that do not exist

Focus: (none; B-040) · Commits: see git log · Outcome: shipped (docs + test only; no release, last tag under 6 h)

## Summary
`docs/tools/`, `docs/api/`, `docs/api-reference.md` and `docs/troubleshooting/` (about 2,700 + 1,500 lines) taught agents `experiment_type`, `detection_type`, and tools like `orchestrate` that the server refuses or lacks. They are deleted; `docs/mcp-tools.md` (generated) is the reference. Two docs_drift tests keep it from growing back.

## Change: docs_drift checks examples and links
Plan: parse `docs/mcp-tools.md` (generated, pinned by `tool_list`) for tools and arguments; scan every markdown file's `callTool("x", {...})`; check relative links resolve.
Proof: both new tests failed first (the first listed ~50 bad calls across 4 doc trees; the second `troubleshooting.md`, `tools/`, `api-reference.md` links), pass after.
Candidates considered: B-010 (stale: input/watch exist), B-052 (Error boxing; public signature sweep, too big for this host load), B-040 (chosen).

## Evidence
`cargo test --test docs_drift`: 11 passed. Evals not run (no tool or description change).

## Metrics
No Rust source change; tests/docs_drift.rs +~90 lines. Docs −4,200 lines.

## Review
No subagent review: docs deletion plus a test, no behaviour.

## Test and eval diffs
tests/docs_drift.rs: two tests added; `doc_files()` skips `.omp/` (loop command templates, not docs). Nothing loosened.

## Release and install
No release: v0.7.2 tagged 30 minutes earlier, 6-hour rule; rides in next (CHANGELOG "Unreleased"). Installed binary unchanged (no code change).

## Deferred / open
- `docs/tutorials/` and `docs/installation/` prose not audited beyond callTool and links.

## Next focus
B-052 (box Error, 937 result_large_err) when host load permits.
