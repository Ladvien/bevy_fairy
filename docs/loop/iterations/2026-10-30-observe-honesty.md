# 2026-10-30 observe-honesty: `observe` stopped inventing type data, a diff that never changed, and bare entities

Focus: none · Commits: b964dc5..fa6a556 (+ state commit) · Outcome: fixes only (four bugs, no Phase 2 change)

## Summary
No open bug, consumer or enhancement issue (#1 and #2 are stale and already answered). An honesty audit of `observe` against the fixture game found three ways it misled an agent. `reflection: true` returned type metadata the server never read. `diff: true` compared every read with itself. `show entity N` returned every entity with no components. All three are fixed, each with a ground-truth test that failed first. The independent review found a fourth bug in the diff fix: two different queries were compared with each other. That was fixed too, and the server instructions stopped saying `observe` takes an `action`. Phase 2 was skipped because the bugs used the pass. The dead-code deletion I had chosen first needs the owner's OK, because it deletes files (B-074).

## Bugs fixed
### B-071: `observe` `reflection` reported type information the server never read
Must-fix: misleads y, blocks n, common/no workaround n. Evidence: `get_or_create_metadata` cached placeholder metadata (`TypeId::of::<()>`, `is_reflected: false`, `fields: []`, `type_category: Value`). `infer_type_from_value` guessed types from the JSON. The fixture's `Crate { weight: f32 }` came back as an unreflected `Value` whose `weight` was an `f64`.
Fix: removed the `reflection` field and the `reflection_data` block. The description now points at `brp` with registry.schema. A call that sends `reflection` is refused: `unknown field reflection, expected query or diff`.
**Assumed decision** (the owner may override). The options were:
1. remove the field;
2. strip only the fabricated metadata;
3. back it with registry.schema.

I picked (1). `brp registry.schema` already answers the question, no eval transcript ever sent `reflection`, and (3) would duplicate `brp`.
Failing-first check: `tests/fixture_ground_truth.rs::observe_reports_no_type_information_it_did_not_read_from_the_game` (ground truth).

### B-072: `diff: true` always answered "No changes detected"
Must-fix: misleads y, blocks n, common/no workaround y. Evidence: `add_snapshot` replaced `last_snapshot` before `diff_against_last` ran, so `before_timestamp == after_timestamp`. The reply said `changes: []` while the mover moved 1.0 unit.
Fix: compare first, then keep the read. A follow-up from the review (d148e25) keys the baseline to the query. A first read, or a read after the query changed, now answers `"diff": {"baseline": true, ...}`. Before that change, `show entity N` followed by `show entity N components …Transform` reported Name, Mover and GlobalTransform as "Component removed".
I also removed the unreachable `diff_target` branches (`clear`, `history:N`), which no field could set, together with the constant `"diff_target": "last"` metadata.
Failing-first check: `tests/fixture_ground_truth.rs::observe_diff_reports_the_mover_moving_between_two_reads` (ground truth). It failed at 80952d7 with "No changes" and at cb8c933 with no `baseline` key. It also re-pins B-062's entity keying.

### B-073: `show entity N` showed every entity as having no components
Must-fix: misleads y, blocks n, common/no workaround y. Evidence: `make_get_request` sends `components: []`. The game answers `{"components":{},"errors":{}}`, and the reply reported `entity_count: 1`.
Fix: ask `world.get_components` for every registered component (from `world.list_components` with no params), then drop the `COMPONENT_NOT_PRESENT` (-23403) errors. Any other error stays.
I tried `world.list_components {entity}` first and rejected it. It answers Rust type names, not reflected paths: the fixture's `Mover` came back as `fixture_ground_truth::fixture_game::Mover`, and `get_components` answered "Unknown component type".
Failing-first check: `observe_show_entity_lists_the_entitys_components` (ground truth). It also asserts that no "not present" noise is left and that a missing entity is an error.

### Instructions: "Every one of these takes an `action`" included `observe`
`observe` takes `query` and refuses `action` as an unknown field. The instructions now name the four tools that take an `action`. No wording test was added (fa6a556).

## Change
None. The bugs took the pass, so Phase 2 was skipped. The deletion I picked first is recorded as B-074 (blocked):
- `debug_command_processor.rs`, `entity_inspector.rs` and their test, about 2,000 lines;
- about 600 lines of `DebugCommand`/`DebugResponse` types in `brp/mod.rs`.

It needs `git rm` of files this pass did not create. CLAUDE.md requires the owner's OK by name for that, and the loop brief does not count as permission. B-075 is the same: `src/server/bevy_reflection/` (2,953 lines) is unreachable now that `observe` no longer calls it.

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals (success / mean calls / invalid calls) | 37fb141: 6/6 / 6.3 / 0 | fa6a556: 6/6 / 5.3 / 1 |
| held-out evals | not run (no release this pass) | not run |
| mutation score on touched handlers | not run: cargo-mutants is not installed, and the host was at load 33-47 | not run |
| ratchet (`scripts/ratchet.sh <touched files>`) | OK at 80952d7 | OK at fa6a556 |
| fixture_ground_truth | 17 (3 new tests fail on old code) | 20/20 |
| agent_tools / tool_contract / tool_list / docs_drift / instructions / test_state_diff | 10 / 8 / 8 / 11 / 3 / 38 | 9 / 8 / 8 / 11 / 3 / 38 |

The one invalid call after the change is `observe "show fast moving objects"` in mover_speed. It was refused because the fixture registers no `Velocity`, which is B-064's refusal (d373c74). At 37fb141 the same call answered `[]`. Two earlier attempts in this pass hit infrastructure problems and are logged in `evals.jsonl` under `cb8c933-dirty`: one task passed, and one session never reached a model (INFR).

The live proof ran over stdio with a JSON-RPC client against `fixture_game` (`/tmp/fairy-live-proof.py`, deleted after the run):
```
show entity: {"components": {"bevy_ecs::name::Name": "mover", ..., "translation": [10.42, 0, 0]}, ..., "fixture_game::Mover": {}}, "errors": {}}
first diff read: {"baseline": true, "message": "No earlier `diff` read of this exact query ..."}
diff changes: [{"change_type": "ComponentModified", "component_type": "...Transform", "entity_id": 4294967195,
  "old_value": {... "translation": [10.498, 0, 0]}, "new_value": {... "translation": [11.542, 0, 0]}}]
after switching query: {"baseline": true, ...}
reflection: {"code": -32602, "message": "failed to deserialize parameters: unknown field `reflection`, expected `query` or `diff`"}
```
Before the fixes, the same calls answered `"components":{}`, then `"No changes detected"` with equal timestamps, then `"is_reflected":false ... "type_info":"f64"` for the Crate weight.

## Metrics
- Server LOC: 19811 → 19788.
- Plugin LOC: 3535, unchanged.
- MCP tools: 15, unchanged.
- Dependencies: unchanged.
- `known_red` was re-recorded, and every count went down:
  - `manual_strip`: 2 → 0;
  - `vec_init_then_push`: 1 → 0;
  - `dead_code`: 14 → 12;
  - `anomaly_detector.rs`: 8 → 5. 42bad35 had recorded it too high.
- The `--record` run caught `observe_diff…|FAILED` while I was editing that test mid-run. I removed that entry, since the test passes at fa6a556.

## Review
A fresh-context reviewer read the diff, the gate output and the live proof.
- **Query-blind diff baseline.** Survived the skeptic pass. Fixed in d148e25 with a failing-first assertion.
- **Semantic queries never send their velocity thresholds.** Survived. This predates the pass. I removed the claim from the new description and logged it as B-077.
- **`entities_modified` always 0.** Survived weakly. Logged as B-078. `tests/test_state_diff.rs` pins 0, so a fix has to justify changing that expectation.
- **Dismissed:** show-entity dropping real errors (-23403 means only "absent"), diff locking, and the removal of `reflection` weakening a check.

## Test and eval diffs
- `tests/fixture_ground_truth.rs` gains 3 tests plus two helpers, `mover_id` and `observe_diff`. The `observe` helper drops `reflection: false`, because the field is gone.
- `tests/agent_tools.rs` loses `observe_reflection_names_the_entity_that_was_asked_about` and the `ObserveRequest` import. That test read `reflection_data`, which no longer exists. Its B-062 intent, keying a `world.get_components` read on the asked entity, is now asserted by `observe_diff_reports_the_mover_moving_between_two_reads` (`entity_id == mover`) against the real fixture.
- `docs/mcp-tools.md` was regenerated.
- No eval task or protected check changed.

## Release and install
- No release: v0.8.0 was tagged 3.9 h earlier, inside the 6-hour rule. These changes ride the next release.
- The next release is breaking (0.9.0), because `observe.reflection` and two public modules were removed. The CHANGELOG `Unreleased` section covers everything since 0.8.0.
- The installed server was rebuilt from fa6a556 (`cargo install --path . --locked`). It still reports 0.8.0, but its `observe` has these fixes.

## Deferred / open
- **B-074 and B-075:** deleting unreachable modules needs the owner's OK naming the paths (see backlog).
- **B-076:** `observe::handle` still dispatches on `serde_json::Value` keys, and `ObserveState`'s snapshot history is unreachable.
- **B-077:** semantic query thresholds are never applied. It needs a fixture `Velocity` or a fake-game test.
- **B-078:** the diff summary's `entities_modified` is always 0.
- **Not run:** mutation testing and held-out evals.

## Next focus
B-077: `find stuck entities` / `show fast moving objects` return every entity carrying the named components while their `semantic_analysis` text claims a threshold was applied. Add a `Velocity` to the fixture so it can be shown first.
