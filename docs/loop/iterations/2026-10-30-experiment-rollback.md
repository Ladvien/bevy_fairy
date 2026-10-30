# 2026-10-30 experiment-rollback: B-068 fixed, rollback_available is true only when undo can run

Focus: honesty audit (experiment tool) · Outcome: fixes only

## Bugs fixed
### B-068: experiment claimed a rollback undo could not perform
Must-fix: misleads yes — `modify`/`delete` returned `rollback_available: true` (a constant keyed on a BRP read succeeding); `create_reverse_action` returned None for both, so `undo` said "No reverse action available" after popping the entry off `history`. The pre-read used `components: []`, which returns nothing. Atomic `batch` rollback used the same function, so it reported "rolled back" with modifies/deletes still applied.
Fix: the executor records the reverse when it runs (modify: original values read by name, only when all were readable; spawn: delete; delete: none). `rollback_available` derives from that. Refused/failed undo keeps its entry; undo/redo bypass history (`dispatch`); redo of a spawn rebuilds its entry; failed atomic batch reports the count of unreversed actions.
Failing-first checks (ground truth, fixture over real BRP): `fixture_ground_truth::experiment_undo_restores_a_modified_component_when_it_claims_rollback`, `experiment_delete_does_not_claim_a_rollback_undo_cannot_perform`.

## Evidence
Workspace tests green; ratchet OK with experiment_system.rs clean. Evals and mutation testing not run (host load 14-19; experiment is not in the eval set).

## Test and eval diffs
Added two tests and a turn-taking mutex (experiment history is process-global) in tests/fixture_ground_truth.rs; no existing assertion changed.

## Release and install
No release: last tag v0.7.2 cut 02:10, under 6 h old. Installed server not rebuilt.

## Deferred / open
- #1 (rmcp stream error) already answered as stale; #2 is a question, not work.
- `experiment` `action` is a free string with an Ok(error JSON) for unknown values (not an enum/tool error); detect_anomaly audit not yet done.

## Next focus
detect_anomaly honesty audit; B-021 ring buffer.
