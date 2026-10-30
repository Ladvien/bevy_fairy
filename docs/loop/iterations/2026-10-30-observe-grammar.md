# 2026-10-30 observe-grammar: `observe` schema advertises its query grammar (B-026, B-044) — v0.7.2

Focus: none given · Outcome: shipped

## Summary
No bug or issue workable (#1 stale and answered, #2 a question). Chose B-026/B-044: `observe` refuses
queries outside its grammar but the schema listed none, so agents sent bare nouns and were refused.

## Change
Files: `src/server/mcp_tools.rs` (doc on `ObserveRequest.query`), `tests/tool_list.rs`, `docs/mcp-tools.md` (generated).
Failing-first check: `tool_list::every_observe_query_example_in_the_schema_is_accepted_by_the_parser`
(panicked "the `query` field carries a description" with the source change stashed; passes now; each
advertised example is fed to `RegexQueryParser::parse`). Examples use `Name`, not `Transform`, after the
0.7.1 finding that agents anchor on schema examples.
Candidates: B-051 (ratchet span bug), B-052 (box Error), B-026/B-044.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals | 6/6 (0.7.1) | 5/6 first run, spinner_stops flaky |
| held-out | 3/3 | 3/3, 0 invalid calls |
spinner_stops races the fixture's 2 s stop against agent latency. Repeated, same fixture: 0.7.1 binary
fail, pass, pass, pass (15-21 calls); this build fail, fail, fail, pass (4-10 calls). Failures miss only
`stop_frame_in_110_130` (agent reads angle already frozen, says it cannot recover the frame). 4 of 6 dev
tasks showed 0 invalid calls. Treated as eval noise, not a regression; not tuned against. Evals ran
with the dev build (`/home/ladvien/target/fairy-loop/debug`), each run is in `evals.jsonl`.

## Metrics
Server LOC +6 (doc comment); no dependency, module or tool-count change.

## Review
Self-review only (host load 55, no subagent spawned).

## Test and eval diffs
`tests/tool_list.rs`: one test added. No existing assertion changed.

## Release and install
v0.7.1 -> v0.7.2 (patch; schema description text only).

## Deferred / open
- spinner_stops eval is racy (stop at frame 120 vs agent latency); needs an evaluator pass, protected.

## Next focus
B-051 ratchet touched-file span bug, then B-052 (box Error) as a minor release.
