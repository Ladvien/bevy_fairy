# 2026-10-30 ring-buffer: bevy_fairy/ring records a query per frame inside the game

Focus: B-021 (always-on ring buffer with trigger) · Outcome: shipped (v0.7.3)

## Summary
No bug issues were open (#1, #2 answered earlier). Added `bevy_fairy/ring`, a built-in command (reached with `run`) that samples a `world.query` every frame in `Last`, keeps the newest 1-600, stops on an optional JSON-pointer trigger, and dumps samples with frame numbers. State only; a pixel ring beside `burst` stays in B-021.

## Change: bevy_fairy/ring
Plan: `src/plugin/ring.rs` (new, ~230 lines), register in `mod.rs` · Goal: an agent can read the frames before a value stopped changing without polling gaps · Proof: `tests/plugin_ring.rs` (4 tests, ground truth = a counter the test bumps; fail-before: method unregistered).
Honesty catch from the test: non-strict `world.query` returns an empty array for an unknown component path, so the ring defaults `strict: true` and refuses at `start` with the game's message.
Candidates considered: clippy debt (B-020), fmt debt (B-042), ring buffer (picked; the loop's own next-focus).

## Evidence
- Live, fixture game over BRP: Spinner samples frames 468-470 all `angle 6.000004768371582` (120 frames x 0.05, the planted stop), frame numbers consecutive; `nope::X` refused: "Component `nope::X` isn't registered or used in the world".
- `cargo test --workspace` all ok; `scripts/ratchet.sh src/plugin/ring.rs src/plugin/mod.rs`: OK.
- Dev/held-out evals not rerun: no server tool or description changed (the command surfaces through the existing `commands`/`run`). Mutation testing not run (host load).

## Metrics
plugin loc 3260 -> see metrics.json (new module; no new dependency).

## Test and eval diffs
New `tests/plugin_ring.rs` and its `[[test]]` entry; nothing existing changed.

## Deferred / open
- Frame (pixel) ring buffer; dev evals have no ring task yet.

## Next focus
B-020 clippy debt, or an eval task that needs `ring`.
