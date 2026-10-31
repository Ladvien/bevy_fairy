# 2026-10-31 honesty-audit: B-120 and a read-only audit of the tools nobody had traced; six honesty bugs fixed

Focus: (none given) · Commits: 69cc6e4..67fa91d · Outcome: fixes shipped (pushed to `origin/main`), no release (6 h rule: v0.11.0's commit is 02:06, the window opens at 08:06)

## Summary
B-120 (the first backlog bug) was fixed, and because the tools that earlier passes audited (`observe`, `experiment`, `hypothesis`, `detect_anomaly`, `watch`) were done, two read-only scouts audited the ones nobody had traced end to end: capture and guide (`screenshot`, `burst`, `guide`, `scene`) and control (`run`, `commands`, `brp`, `status`, `quit`, and the plugin methods `input`, `time`, `ui_text`, `ring`). They reported 16 confirmed defects from reading the code. Five of the most misleading were reproduced against fakes or the plugin and fixed failing-first; the rest are logged (B-126..B-135). An independent review of the six fixes found seven more real gaps; all seven were fixed. Phase 2 was skipped: six reproduced bugs is over the budget the loop sets.

## Bugs fixed
### B-120: state recorded against a game with no plugin stayed "unverified" after a restart with one
Must-fix: misleads yes (the reply carried `game_instance_unverified`, but `experiment undo` still despawned the new game's entity 7, `observe` reported two games' difference as a change, `detect_anomaly` scored the new game against the old samples). Open since the last pass's review.
Fix: `BrpClient::game_instance` returns `Unnamed { why, answered_without_a_name }`; true when the game *answered* `bevy_fairy/status` with no `instance` or `method not found`. A process does not gain a plugin, so `certainly_another_game(then, now)` (one comparison, three stores) calls that pair another game: history `stale` (refused and discarded), diff baseline new, samples dropped. A game that was only not asked successfully (timeout, internal error, HTTP failure) says nothing and stays unverified.
Failing-first: `tests/restart_unnamed.rs` (fake BRP endpoint: no plugin, older plugin, failing status; experiment and observe), `tests/anomaly_restart.rs` (one more step). All red on 3540366 for the right reason (`stale: null`; a `ComponentModified` diff with `game_instance_unverified`; `game_instance_unverified` instead of `history_discarded`). Mutants killed: `answered_without_a_name` true for every error, false for the older plugin, `certainly_another_game`'s `Err` arm always true, the guard dropped in `standing_of` (4 of 4).

### B-121: `bevy_fairy/input` ignored what it did not read
Must-fix: misleads yes. `{Keyboard, KeyS, modifiers: [ControlLeft]}` tapped a plain S, `{Mouse, Left, x: 300, y: 200}` clicked wherever the cursor was, `{Scroll}` scrolled by nothing, each `success: true`. Found by the control audit, confirmed by a failing test.
Fix: `deny_unknown_fields`, `fields_read_by` + `refuse_unread_fields` (the refusal names the field and what the kind reads; a Mouse `x`/`y` points at a `Cursor` call), a scroll needs `x` and/or `y`. The review added: `Cursor` `clear: true` with a position, and params sent as a JSON array (serde parses a sequence as a struct, `deny_unknown_fields` does not police it).
Failing-first: `tests/plugin_input_params.rs` (3 of 4 red on 3540366, the guard test green; 2 more red on 4ff1e19's src for the review's gaps).

### B-122: a ring trigger written as an integer never matched a float field
Must-fix: misleads yes (`triggered_at_frame: null` reads as "it never happened"). `serde_json` calls `9` and `9.0` different; an `f32` reads back widened (`0.1` is `0.10000000149011612`).
Fix: `same_value` / `same_number`; two integers exact; an f32-exact sample matches the shortest decimal of that f32. The review found the first rule matched `16777217` against an f64 `16777216.0` and `1e-50` against `0.0`; tightened.
Failing-first: `tests/plugin_ring.rs` (integer and decimal triggers red on 3540366; the neighbour-value test red on the first rule).

### B-123: `ui_text` `region` overflowed
Must-fix: blocks (a debug game panicked) and misleads (a release game wrapped to an empty region: `matched: 0`). Failing-first: `tests/plugin_ui_text.rs` panicked at `ui_text.rs:182 attempt to add with overflow`. Fix: sum in f32.

### B-124: `screenshot` `zoom` had no upper bound
Must-fix: blocks. `zoom: 1e9` died with `memory allocation of 256000000000 bytes failed` inside the game's own observer. Fix: `zoomed_size` refuses a zoom in over 4096 x 4096 output pixels (a failed capture that names `region`); a zoom out is never refused (the review found the first draft refused an 8K window at 0.9). Failing-first: `screenshot::tests::a_zoom_that_would_make_a_huge_image_is_refused_before_it_is_allocated` (the process aborted).

### B-125: a slow game or an HTTP 500 was reported as "no game answering"
Must-fix: misleads yes, and an agent that believed it restarted a running game or sent the request twice. Fix: `Error::Timeout` / `Error::Brp` from the client, `why_no_reply` shared by `call_game` and the batch sampler (so `watch` too), a 2 s connect timeout (a host that drops packets is still "no game answering", a game in a long frame is "did not answer; the request may still run; check `status`"). Failing-first: `tests/slow_game.rs` (fake endpoint that sleeps 7 s / answers 500; `brp`, then `watch`).

## Change
None. Six reproduced must-fix bugs of one class (a tool answering something other than the truth) were worked instead; Phase 2 is for when no bug can be worked.
Candidates left for later: B-127/B-128 (the guide: no checkpoint is evaluated through the MCP tool, and four smaller guide honesty defects), B-129 (`time` `held`), B-093 (anomaly detectors name Velocity/Health/Alive only).

## Evidence
| Suite | Before | After |
|---|---|---|
| dev evals (success / mean calls / invalid) | b3b693e: 6/6, 4.7, 0 | 67fa91d: 6/6, 5.3, 1 (a game-side refusal) |
| held-out evals | 3540366: 3/3, 3.7 calls | not run: no release |
| manual mutants on the touched code | n/a | 4 on B-120, all killed |
| tests listed (`cargo test --workspace -- --list`) | 470 | 491 (+32 feature-gated plugin tests, run separately) |

Live proof through a real stdio MCP client (a throwaway Python client, outside the repo) against the fixture and two stub endpoints, debug build of 67fa91d:
```
input with a modifier it does not read -> isError | invalid input params: unknown field `modifiers`, expected one of `kind`, `action`, `key`, `text`, `button`, `x`, `y`, `unit`, `clear`
the same key without it               -> ok | {"message":"Input queued; applied on the next frame's PreUpdate","success":true}
click with x/y                        -> isError | `x` is not read by `kind: Mouse` ... send `kind: "Cursor"` with `x` and `y` first, then the click
cursor clear with a position          -> isError | `clear` hands the pointer back to the real mouse and takes no position ...
params as a positional array          -> isError | input params must be a JSON object, got ["Mouse","Tap",null,null,"Left",3,4]
ring, trigger equals the integer 4 on a crate weight of 4.0 -> recording False, triggered_at_frame 256 (before: never)
brp against a game in a 7 s frame     -> did not answer `world.query` ... may still act on it ... check with `status` before sending it again
watch against the same game           -> the same text (before: "no game answering")
brp against a game answering HTTP 500 -> answered `world.query` with a failure ... HTTP 500 Internal Server Error
brp against a port nothing listens on -> no game answering at http://127.0.0.1:15794
```
Not exercised live: `screenshot` zoom and `ui_text` region (the fixture has no capture target and no UI; both are proven by unit and plugin tests that run in the real code path), and the restart cases (the fake-endpoint tests are the proof; the fixture always has the plugin).
Transcript check (dev, 67fa91d): no repeated identical calls; the one invalid call is `mover_speed` asking `brp` for `Time<Real>`, which the game itself cannot serialise.

## Metrics
Server LOC 21266 -> 21413 (+147: `Unnamed`/`certainly_another_game`/`instance_from` and their tests, `why_no_reply`), plugin 3588 -> 3747 (+159: input refusals, ring number matching, zoom bound, with their tests), tests 470 -> 491, direct dependencies 52 and 4 unchanged (`tests/plugin_leaf.rs` passes), tools 14, clippy unchanged (the ratchet found two `unused_imports` from this pass and they were removed; `known_red` was not lowered), release binary size unchanged (no release).

## Review
An independent `reviewer` agent with only the commit range found 8 gaps; 7 were real and fixed in `67fa91d`, each failing-first where it could be (the connect-timeout and the HTTP-500 wording are covered by a unit test of the classification and by the live stub run, not by a red test): `Cursor` `clear` with a position (P2), `watch` still said "no game answering" for a slow game (P2), a connect timeout read as "the request was sent" (P2), the f32 rule over-matching (P3), a zoom out refused on a large capture (P3), params as an array (P3), the HTTP-500 wording in `game_instance` (P3). One was logged, not fixed (B-126: `certainly_another_game` is one-directional; nothing is silent and the existing `anomaly_restart` assertion pins the current reply). Skeptic pass: each of the seven was confirmed in the code or by a red test before the fix; none had a benign reading.

## Test and eval diffs
- `src/server/tools/experiment.rs`: `a_history_recorded_without_a_name_is_never_called_current_or_stale` pinned the B-120 behaviour itself (a game that answered without a name, then one that names itself, came out `Unverified`). It is split into three: both unnamed stays `Unverified`; answered-without-a-name then named is `Stale`; not-asked-successfully then named stays `Unverified`. The review judged this a legitimate change, not a weakened check.
- `tests/anomaly_restart.rs`: one step added at the end; no assertion changed.
- New: `tests/restart_unnamed.rs`, `tests/slow_game.rs`, `tests/plugin_input_params.rs` (with a `[[test]]` entry in `Cargo.toml`), unit tests in `brp_client.rs`, plus additions to `tests/plugin_ring.rs`, `tests/plugin_ui_text.rs` and `plugin/screenshot.rs`.
- No eval task, held-out task or protected check changed. `tests/plugin_guide_steps.rs`'s `a_checkpoint_nobody_registered_is_refused_by_name` is untouched (B-128 would need it to post through `Guide::post`).
- A finding about the gate itself: `cargo test --workspace` (default features) does not run `plugin_pointer`, `plugin_ui_text`, `plugin_guide_steps` or the screenshot unit tests. They were run here with `--features plugin-screenshot,plugin-ui-text,plugin-guide`, and CLAUDE.md now says so.

## Release and install
No release: the last tag is v0.11.0 on 3540366 (committed 02:06 on 2026-10-31); the 6 h window opens at 08:06, and this pass ended at about 04:45. The changes (CHANGELOG `Unreleased`) ride in the next release. Nothing yanked, republished or force-pushed.
Installed binary: rebuilt from the pushed commit with `cargo install --path . --locked --force` (still reports 0.11.0, the version in `Cargo.toml`).
Downstream: `Ladvien/bevy_emerge` depends on this repo by git revision and picks the fixes up when its pin moves; nothing in a consumer-facing call shape changed except that `bevy_fairy/input` now refuses fields it used to ignore.

## Deferred / open
- B-126 (one-directional `certainly_another_game`), B-127 (guide checkpoints never evaluated through the MCP tool: a design fork), B-128 (four guide honesty defects), B-129 (`time` `held`), B-130 (`status.replay` null for a rejected scene), B-131 (image-route buttons dropped), B-132 (contact-sheet label clipping), B-133 (`run` says every command is recorded), B-134 (ring `start` replaces silently, unconfirmed), B-135 (nine unreproduced suspicions). All from read-only audits; B-127..B-133 are code-read, not run.
- B-117, B-093, B-092, B-074/B-075/B-080/B-082 (deletions need the owner's OK naming each path), B-020, B-021, B-042, B-043, B-049, B-017.
- Issues #1 and #2 (stale, not labelled `consumer`) were not worked; no consumer issue was open.

## Next focus
The guide (B-127, then B-128): it is the one tool whose headline promise (the app advances itself when a named condition arrives) cannot be kept through the MCP server, and four smaller defects sit beside it. Decide first whether checkpoints are evaluated by a plugin system (the stream then only reports) or the server holds the stream. After that B-129 (`time` `held`) and B-130 (`status.replay`), both small and plugin-only.
