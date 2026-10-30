# 2026-10-30 scene-docs: B-022 documented, no release

Focus: none · Outcome: fixes only (docs)

## Summary
Scene scripts record only registered debug commands. `bevy_fairy/input` is not one, so injected keys and clicks are absent from a saved scene and replay is not frame-exact. `src/plugin/scene.rs` module docs and the README now say so (README already said "not raw input"; it now names `bevy_fairy/input` and non-determinism).

## Bugs fixed
### B-022: scene docs omitted what is not recorded
Must-fix: misleads y — an agent could assume a replay repeats its clicks. Fix: docs only. Check: `cargo test --test docs_drift` green (11 passed).

## Not worked
- #1 (rmcp error on a hand-written `initialize` lacking `protocolVersion`/`clientInfo`, old crate name, 2025-09): request is invalid MCP, not reproducible as a server bug.
- #2: a question, not a request.
- Release: none; last tag v0.7.2 is under 6 h old, and this rides in the next release.

## Next focus
B-010 (input/watch tools) or B-052 (box Error, minor release).
