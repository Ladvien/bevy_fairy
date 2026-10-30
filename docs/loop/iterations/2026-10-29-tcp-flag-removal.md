# 2026-10-29 dead-code: `--tcp`/`--server` removed (B-012)

Focus: none · Outcome: shipped (v0.4.0)

## Summary
`bevy-fairy --tcp` bound a port, accepted a connection and dropped it unread; a terminal start with no flag fell into the same mode. stdio is now the only transport; the flags are refused as unknown (exit 2) and a bare terminal start exits 2 with a usage hint.

## Bugs fixed
### B-012: dead TCP mode
Must-fix: blocks yes (held a port open, served nothing) — `src/main.rs` `run_tcp_mode`.
Failing-first check: `tests/cli_args.rs::the_tcp_flags_that_never_served_anything_are_refused` (round-trip on the binary). Before: hung until killed (300 s timeout, serving TCP). After: passes.

## Evidence
Ratchet: `scripts/ratchet.sh src/main.rs` → OK. `cargo test --workspace -j 3` all green. Evals not run: no tool schema or handler changed (server arg parsing only); stated honestly. Library `McpServer` untouched (integration harness uses it).

## Metrics
main.rs shrank by ~90 lines; no deps added. Test diff: one additive test in `tests/cli_args.rs`.

## Release and install
0.3.5 → 0.4.0 (breaking CLI flag removal, pre-1.0 minor).

## Deferred / open
B-011 (`cli.rs` never compiled), B-013 (unreachable modules), B-010/B-021 (burst capture/ring buffer).

## Next focus
Delete unreachable modules module-by-module (B-013), or burst capture (B-021).
