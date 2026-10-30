# 2026-10-30 clippy-sweep: unused imports/get(0) cleared in bevy_reflection/inspector.rs

Focus: B-020 · Outcome: shipped (no release, 6 h rule; v0.8.0 published 76 min earlier)

`cargo clippy --fix --lib --bins` fixed 3 files; two were reverted because the fix removed `json!`/`FieldMetadata`
imports that the `bevy-reflection` feature and the unit tests need (autofix ran without them). Kept: `inspector.rs`.
Ratchet green (`scripts/ratchet.sh`), `cargo test --workspace` green, baseline re-recorded (counts only down).
No test or eval diffs. Leftover B-020 warnings: see `known_red` in metrics.json.
