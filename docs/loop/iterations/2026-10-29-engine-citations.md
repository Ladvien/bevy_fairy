# 2026-10-29 engine-citations: plugin comments and CLAUDE.md cite the Bevy 0.20 engine, not 0.19 (B-016)

Focus: (none) · Outcome: shipped (docs/comments only)

## Summary
Host load was 25-38 with ~50 rustc/lld running, so this pass took the smallest backlog item that is a
docs-versus-code contradiction (B-016). Issue #1 (Bevy 0.16 / WebSocket) is stale; commented, left open
for the reporter. #2 is a question, not work.

## Bugs fixed
### B-016: plugin comments cite 0.19 paths and say handlers run in `Last`
Must-fix: misleads (docs only), no code path. Fix: `bevy_remote`/`bevy_winit`/`bevy_input` citations
re-read in `~/.cargo/registry` 0.20.1 (remote lib.rs:632, :832-835, :1559; winit system.rs:426-430);
`Last` -> `RemoteLast` in `input.rs`. No failing-first check applies to comments.

## Evidence
`cargo check --lib --no-default-features --features plugin` ok; `plugin_leaf` 3/3 ok;
`cargo test --workspace` rc=0. NOT run: ratchet, all-features, mutants, evals, semver-checks (comment-only
diff; host overloaded). Server-side "Bevy 0.19" comments (brp_validation.rs etc.) remain: not verified against 0.20 method lists.

## Release and install
0.3.1 -> 0.3.2 (patch).
