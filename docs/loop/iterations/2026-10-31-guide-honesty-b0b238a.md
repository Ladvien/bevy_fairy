# 2026-10-31 guide-honesty: the guide's checkpoint advances the step by itself, and five more audit defects fixed

Focus: open bugs (B-127..B-133, the code-read audit of the last pass) · Commits: ee2dd84..b0b238a · Outcome: fixes only (no Phase 2 change; six bugs is over the budget) · released as v0.12.0

## Summary
Every B-127..B-133 item that the last pass logged from a read-only audit was reproduced or checked against the code, and six were fixed failing-first. The big one: through the MCP `guide` tool a step with a checkpoint could never advance, though the tool's description said the app moves on by itself, because the checkpoint was evaluated only inside a BRP *watching* method that the MCP server cannot open. A plugin system evaluates it now. No consumer or enhancement issue was open (#1 and #2 are a stale bug report that the owner already answered and a question).

## Bugs fixed
### B-127: a step with a checkpoint never advanced through the MCP tool
Must-fix: misleads yes, blocks yes, common/no workaround no — `tests/plugin_guide_steps.rs::a_checkpoint_that_passes_advances_the_step_with_nobody_watching` stayed at step 1 on the old src; the only evaluator was `watch_guide`, and neither `brp` nor `watch` can open a `+watch` stream.
Fix: `evaluate_checkpoint` (a plain system in `Last` that queues the exclusive work only while a checkpointed step is current) runs the host's checkpoint once a frame, advances on true and queues the pass in `Guide::unreported`; `watch_guide` is a reporter and drains it. `read` names the checkpoint and carries `checkpoint_error`. Failing-first: the test above, `a_checkpoint_the_app_cannot_run_is_said_by_a_read` (ground-truth through the plugin).
Assumed decision (owner may override): evaluate in the plugin rather than make the server hold a stream. Options: plugin system, or a long-lived stream client in the server. Picked the plugin: it is where the host's checkpoints live, needs no dependency, no streaming client and no long-lived request, and a `guide+watch` consumer keeps working unchanged.

### B-128: four guide honesty defects
Must-fix: misleads yes. Unregistered checkpoint accepted at post time (the description said it is refused), `{clear, skip}` ran one and answered success for both, `skip` with nothing to skip answered success, `{steps, visible:false}` posted a visible script, a re-post while step 1 showed kept the old card (the render key did not change).
Fix: all refused or honoured as above; `Guide::generation` joins the render key. Failing-first (all red on the old src): `a_script_naming_an_unregistered_checkpoint_is_refused_when_posted`, `a_call_naming_two_verbs_is_refused_and_does_nothing`, `skipping_with_nothing_to_skip_is_refused`, `visible_is_honoured_with_a_post_and_refused_where_it_cannot_act`, `re_posting_a_script_while_step_one_is_up_replaces_the_card`.

### B-129: `time` `held` was true for an unpause or a speed of 1.0 the clock did not obey
Must-fix: misleads yes. `held` now means the clock matched the request (paused iff asked, at the asked speed). Failing-first: `tests/plugin_clock.rs::an_unpause_the_host_overrides_is_not_held`, `a_speed_of_one_the_host_overrides_is_not_held` (red), a control that an unpause nobody overrides is held. The independent review found a regression in my first fix (a pause beside a host's slow motion read as not held); fixed with `a_pause_is_held_beside_a_hosts_own_slow_motion` (red first).

### B-130: `status.replay` was null for a rejected scene file
Must-fix: misleads yes. Now `{path, rejected: <why>}` (no counts: the review pointed out `0 of 0` is what an accepted empty file says). Failing-first: `tests/plugin_commands.rs::a_scene_file_that_cannot_be_read_is_reported_and_no_scene_stays_null`, `a_scene_file_with_the_wrong_format_replays_nothing_and_does_not_panic`. Live: a game started with `BEVY_FAIRY_SCENE=/nonexistent/scene.json` answers `"replay":{"path":"/nonexistent/scene.json","rejected":"cannot read: No such file or directory (os error 2)"}`.

### B-131: a Back/Forward/Other mouse press says nothing about picking
The audit's claim was half wrong: `bevy_picking` skips those buttons on *both* routes (`bevy_picking-0.20.0/src/input.rs:167`), the press does reach `ButtonInput<MouseButton>`. What was true: the reply said only success. Now `note` says so. Failing-first: `tests/plugin_input_params.rs::a_mouse_button_picking_cannot_see_says_so_and_a_button_it_can_see_does_not`.

### B-133: `run` and the README said every command that runs is recorded
Wording only (read-only commands are not recorded; the recorder was right). No failing-first check exists for a sentence.

## Evidence
| Suite | Before (67fa91d) | After (6cc4282) |
|---|---|---|
| dev evals (success / mean calls / invalid calls) | 6/6, 5.3, 1 | 6/6, 4.7, 1 (the known `brp` ask for `Time<Real>`) |
| held-out evals | 3/3, 3.7 calls, 0 invalid (3540366) | 3/3, 4.0 calls, 0 invalid (debug build of 94f03e2) |
| manual mutants on the touched handlers | n/a | 12 applied at once, 12 killed (each by the test named for it) |
| baseline `cargo test --workspace` | green (88 s) | green; feature-gated plugin tests green |

Live through a real MCP stdio client against a throwaway game with `plugin-guide` and one registered checkpoint (`/home/ladvien/target/fairy-loop/tmp/live_guide.py`, output below, game and example removed after):
```
-- 1. refused at post time
Validation error: the app refused `bevy_fairy/guide`: step "x" names checkpoint `tile is saved`, which this app has not registered, so it would wait for ever. This app registers: flag is set. ... Nothing was posted.
-- 2. read + skip: one verb per call
-- 3. post a real script, then read
at=1/2 checkpoint=flag is set waiting_on_a_person=False
-- 4. run tmp/set_flag through the MCP `run` tool; nobody opens a watch stream
at=2/2 step='does it look right' waiting_on_a_person=True
transcript: [('set the flag', '1/1'), ('does it look right', '0/1')]
-- 5. skip on a finished script: refused ("no current step to skip")
```

Transcript excerpt (before/after, the same call): before, `guide {read:true}` after the condition became true answered `at: 1`, for ever; after, `at: 2` within a frame.

## Metrics
Plugin LOC 3747 → 3929 (+182: the evaluator and the unreported queue, the verb/visible/skip refusals, `SceneRejected`, the `held` rewrite, the input note); server LOC 21413 → 21413 (description text only); tools 14 → 14; direct dependencies unchanged, none added; release binary 105158736 → 105161160 bytes; tests listed by `cargo test --workspace` 491 → 497 (+6), feature-gated `plugin_guide_steps` 19 → 27.

### Found by the pre-publish gate: a public doc linked a private fn
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace` failed on `InputCommand`'s doc linking the private `refuse_unread_fields` (B-121's text, 0fabe1c). The last pass did not release, so the release doc gate had not run since. Fixed in b0b238a (plain code formatting). Not a tool defect.

## Review
Round 1 (reviewer agent, diff + plan only): four gaps. (1) the uncommitted `status` description had not regenerated `docs/mcp-tools.md` — benign by then, the full run regenerated it before the commit. (2) real: a pause beside a host's slow motion read as not held (fixed, test first). (3) real: a rejected scene reported `done/total/failed: 0` (fixed). (4) `skip` can land on step k+1 when step k's checkpoint passes between `read` and `skip`: survives the skeptic pass as a real but contained risk (the reply names the step actually skipped; no lie), logged as B-136. Round 2: no new defect; one doc gap (the `held` text did not name the pause exception), fixed.

## Test and eval diffs
- `tests/plugin_guide_steps.rs`: harness `app()` adds `evaluate_checkpoint`; six tests that drove a pass through `watch(&mut app)` run a frame first (assertions unchanged); `a_checkpoint_nobody_registered_is_refused_by_name` posts through `Guide::post` because the BRP method now refuses the name at the door (`post_from_rust`). 
- `tests/plugin_commands.rs::a_scene_file_with_the_wrong_format_replays_nothing_and_does_not_panic`: its `status["replay"].is_null()` assertion pinned the bug; it now asserts the rejection. Its two original claims (nothing replayed, no panic) are kept.
- No eval task or protected check changed.

## Release and install
Version 0.11.0 → 0.12.0 (minor-breaking pre-1.0) · semver-checks (default features plus plugin-guide, plugin-screenshot, plugin-ui-text): 1 major, `handle_guide` takes 3 parameters instead of 2 · Channel: local `cargo publish` at the 6 h mark (v0.11.0's commit is dated 02:06 CDT, so 08:06) after the pre-publish gate: ratchet OK on the release commit, `cargo doc` with `-D warnings`, `cargo package --list` (103 files, 1.8 MiB), `cargo publish --dry-run`, held-out evals · Tag: v0.12.0 on b0b238a (the first commit whose packaged files are the published ones; the release commit 6cc4282 predates the doc fix).
crates.io: https://crates.io/crates/bevy_fairy/0.12.0 · docs.rs: https://docs.rs/bevy_fairy/0.12.0 (checked next iteration) · Installed binary: `bevy-fairy 0.12.0` from the release tree (`cargo install --path . --locked --force`), `tools/list` over stdio answers the 14 tools

## Deferred / open
- B-136 (new): `guide` `skip` has no way to say which step it means, so it can land on the next step if a checkpoint passes between `read` and `skip`. A `step` guard on `skip` would close it (`GuideRequest.skip` schema line says "Move past the current step").
- B-126 (certainly_another_game is one-directional; flagged, not silent), B-132 (contact-sheet labels clipped), B-134, B-135 (unreproduced), B-117, B-093, B-092, B-074/B-075/B-080/B-082 (deletions need the owner's OK naming each path), B-020, B-021, B-042, B-043, B-049, B-017.
- Issues #1 and #2 are not consumer or enhancement issues (a stale bug report already answered by the owner, and a question); not worked.

## Next focus
B-136 (skip guard) is the last guide gap; then B-132 (labels on a contact sheet that cannot be read are the evidence an agent leans on) and B-126.
