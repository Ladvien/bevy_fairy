# 2026-10-30 clippy-sweep: B-020 clippy warnings 186 -> 144

Focus: none · Outcome: shipped (debt paydown, no release: 6 h rule, v0.7.2 tag under 6 h old)

## Summary
`cargo clippy --fix --lib --bins` on default features removed autofixable warnings in 10 server files. The first
run also touched `src/server/bevy_reflection/` (feature-gated); it removed imports that feature needs, so
`--all-features` stopped compiling. Those files were reverted and the baseline re-recorded: every feature builds.

## Evidence
- `scripts/ratchet.sh --record`: clippy total 186 -> 144, 0 failing features.
- `cargo test --workspace --all-features`: all suites ok (1 pre-existing ignored doctest).
- Lesson: autofix under default features is unsafe for feature-gated modules; check `--all-features` after.

## Test and eval diffs
none. Evals not rerun (no tool behaviour changed). Not a Phase 2 change; no release.

## Deferred
B-020 remaining 144 warnings (bevy_reflection files, tests/). Next focus: B-021 burst capture ring buffer.
