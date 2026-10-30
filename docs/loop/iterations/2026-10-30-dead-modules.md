# 2026-10-30 dead-modules: deleted unreachable replay_actor and resource_manager

Focus: none · Outcome: shipped (deletion), no release (v0.8.0 published minutes earlier; 6 h rule)

## Summary
No open bug or enhancement issue: #1 and #2 were answered by the owner as stale/non-issues; bevy_emerge #97 is complete. Phase 2 priority 2: removed `src/server/replay_actor.rs` (552 lines, referenced by nothing) and `src/server/resource_manager.rs` (869 lines, only reachable through `BrpClient::with_resource_manager`, which no caller invoked), and the dead `resource_manager` field on `BrpClient`.

## Evidence
- `grep` for `ReplayActor|replay_actor|ResourceManager|with_resource_manager` outside the two files: only `mod.rs` and `brp_client.rs` plumbing.
- `cargo test --workspace` (capped scope, -j 3): all `test result: ok`, 0 failed.
- `scripts/ratchet.sh`: OK; `--record` lowered known_red: dead_code 26->22, unused_variables 21->20, inherent_to_string 4->2.
- Fail-before check: n/a (pure deletion; compile + existing tests prove nothing depended on it). Mutation, evals not run: no tool behaviour changed.

## Metrics
server loc -1421 lines; no deps changed.

## Test and eval diffs
Tests inside the two deleted modules (8 in resource_manager) went with them; no other test changed.

## Release and install
No release: 6 h since v0.8.0 not elapsed; changes ride the next release. Installed server unchanged (no behaviour change).

## Deferred / open
Remaining unreachable modules: timeline_branching/playback/recording (used only by `replay` tool), entity_inspector + debug_command_processor (used by tests only), bevy_reflection (observe). Pixel ring buffer. B-042 fmt debt.

## Next focus
Check `entity_inspector`/`debug_command_processor` reachability from `BrpClient::with_debug_router` and delete if only tests use them.
