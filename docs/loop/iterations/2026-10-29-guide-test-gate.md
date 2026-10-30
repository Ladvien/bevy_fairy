# 2026-10-29 guide-test-gate: the 19 guide-channel tests were compiled out; now run (B-053)

Focus: (none) · Outcome: fixes only

## Summary
`tests/plugin_guide_steps.rs` carried `#![cfg(feature = "guide")]`; no such feature exists, so the file built to nothing and the guide contract CLAUDE.md cites was unchecked. Host load ~27, so smallest honest bug taken.

## Bugs fixed
### B-053
Must-fix: misleads (a pinned contract that never ran). Failing-first: removing the gate made the file compile and it failed twice: unresolved `BEAT_SECONDS`/`GuidePlacement` (not re-exported at the crate root) and a stale example path (`examples/guided_steps_land.rs`, now `plugin_guided_steps_land.rs`). Fix: re-export the two items in `src/lib.rs`, correct the path, drop the gate (`required-features = ["plugin-guide"]` in Cargo.toml is the real gate).
Check: `cargo test --no-default-features --features plugin-guide --test plugin_guide_steps` → 19 passed.

## Evidence
`scripts/ratchet.sh src/lib.rs tests/plugin_guide_steps.rs` → OK; `cargo test --workspace` no failures. NOT run: evals, mutants, semver-checks (additive re-exports only).

## Test and eval diffs
`tests/plugin_guide_steps.rs`: removed dead cfg line; example path corrected to the renamed file. No assertion loosened.

## Release and install
0.3.2 → 0.3.3 (patch).

## Next focus
B-010/B-045 (watch/input tools), or B-011 deletion of dead cli.rs references.
