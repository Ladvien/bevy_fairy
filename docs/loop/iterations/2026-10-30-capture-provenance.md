# 2026-10-30 capture: screenshots carry source and frame provenance

Focus: none given · Commits: see git log (capture-provenance) · Outcome: shipped (no release, 6 h rule: v0.7.2 about 2 h old)

## Summary
The tool contract says every capture carries its frame and source. `screenshot` returned only a path and size, so an agent could not tell a stale window frame from a fresh one, nor which source a default had picked. The plugin now records the source and the frame the request and the delivery happened on; the server prints them.

## Change: Provenance on captures
Plan: `Provenance {source, frame_requested, frame_captured}` in `CaptureState::Done`, returned by `bevy_fairy/capture`, printed by `screenshot` (or "unavailable" for an older plugin) · Files: src/plugin/screenshot.rs, src/plugin/mod.rs, src/lib.rs, src/server/mcp_tools.rs, tests/agent_tools.rs, CHANGELOG.md, docs/mcp-tools.md · Goal: no capture without provenance · Proof: `agent_tools::screenshot_writes_a_png_after_one_pending_reply` (asserts the reply ends with the source and frames; the old reply ended at the size) and `plugin::screenshot::tests::a_done_capture_reports_where_and_when_it_was_taken`.
Candidates considered: B-021 burst capture (needs an eval task first; this is its prerequisite, frame stamps); B-020 clippy debt; B-024 wrapper structs.
Issues: #1 (stale, answered), #2 (question); bevy_emerge #114 (agent judges captured frames) needs frame stamps to pair with state, which this adds.

## Evidence
- `cargo test --workspace --all-features` green; `scripts/ratchet.sh src/plugin/screenshot.rs src/server/mcp_tools.rs` OK.
- Not exercised: a live capture with a GPU (the fixture is headless; the plugin's real capture path cannot run in CI here). Evals not re-run: host load ~13 and the change only adds text to one reply.

## Test and eval diffs
tests/agent_tools.rs: the fake game's `capture` reply gains the three fields, and the screenshot assertion `ends_with("(1x1)")` became `contains("(1x1)")` plus an assertion on the provenance text (the reply format was extended on purpose).

## Release and install
No release (6 h rule); rides in next (0.8.0, breaking: `CaptureState::Done` field). Installed binary not rebuilt this pass.

## Next focus
B-021 burst capture, with its eval task written first.
