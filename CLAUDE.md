# bevy_fairy — notes for agents

> **This is our code. Edit it here.**
>
> It is not a third-party dependency, not a vendored upstream, and not something to work around. If
> the debugger is missing a capability you need, **add the capability** — that is an ordinary change
> in this repo, reviewed like any other. The one thing that is *not* ordinary is widening the OS
> boundary below; that costs a deliberate argument.
>
> The word "vendored" appears in the history because this arrived by `git subtree add` rather than
> being written here. It says where the code came from, not who may change it. Every previous agent
> that treated a missing feature here as a fixed constraint was wrong: the cursor-position gap in
> `bevy_fairy/input` sat unfixed through two whole steps of work for exactly that reason, and it
> was a forty-line change.

An MCP server plus a companion Bevy plugin that let an agent inspect and drive a **running** Bevy game: live entity/component/resource queries, offscreen frame capture, and keyboard/mouse injection.

## Force pushes and deletions need the user's express permission

Never do any of the following unless the user has approved that specific action, by name, in the current conversation. A loop brief, a skill, a standing instruction such as "commit, push, PR, merge", or an earlier session's approval is not permission.

- Force-push in any form: `git push --force`, `--force-with-lease`, `--force-if-includes`, a `+` refspec, or pushing rewritten history. Deleting a remote branch or tag (`git push --delete`, `gh pr merge --delete-branch`, an API delete).
- Throwing away commits or uncommitted work: `git reset --hard`, `--merge` or `--keep`; `git checkout -- <path>` or `git restore` over changes you did not make; `git clean`; `git stash drop` or `clear`; `git branch -D`; `git worktree remove --force`; `git rebase` or `git commit --amend` on pushed commits; `git filter-repo` or `filter-branch`.
- Deleting files, directories, build caches, target dirs, models, datasets, database rows, issues, PRs, releases or cloud resources that you did not create in this session (`rm`, `rm -rf`, `git rm`, `cargo clean`, an API delete).

When one of these seems necessary, stop and ask. Name the exact command, what it destroys and whose work it touches. A commit that landed on the wrong branch is fixed with a new commit or a question, never by resetting a shared working tree or force-pushing. Files you created yourself in this session (scratch scripts, temp dirs, your own worktrees) you may delete.

## Source of truth

This standalone repo, `Ladvien/bevy_fairy`, is the source of truth for the crate, on Bevy 0.20 (Sanctuary decision 0232, 2026-10-19). It was a pinned git dependency, then vendored into `Ladvien/foundation_vs_slop` with `git subtree add` and mirrored back out of it. The monorepo's copy is now a frozen Bevy 0.19 copy that is never mirrored again, and `Ladvien/bevy_emerge` depends on this repo by git. A bug found while driving a game is an ordinary edit here, followed by a push.

## One crate, two halves

`bevy_fairy` is one published crate with two halves, and it was two crates until 2026-10-21. The
names `bevy_debugger_mcp` and `bevy_debugger_bevy` stuttered, and a game that wanted the plugin
had to depend on a crate whose name said it was about the server.

- **`src/plugin/`** — the Bevy plugin the game links. Behind the `plugin` feature. ~400 lines, four
  direct dependencies.
- **`src/server/`** — the MCP server, and the `bevy-fairy` binary. Behind the `server` feature, which
  is the default. A separate process: speaks MCP over stdio to an agent, BRP over JSON-RPC/HTTP
  (:15702) to the game. ~24k lines.

The two halves do **not** depend on each other, and that is a property of the *features*, not of
the packages. Cargo resolves dependencies per feature, so a game that takes the plugin with
`--no-default-features --features plugin` compiles the plugin's closure and none of the server's.
The reverse holds: the server talks to any game that exposes BRP, including one that never heard of
this crate.

A consequence worth remembering: the `bevy` dependency is declared once, with
`default-features = false`, and each feature adds what it needs. A feature that turns on the
plugin's capture turns on `bevy/bevy_render`; one that turns on the guide turns on
`bevy/ui_api` and `bevy/bevy_picking`. Adding a Bevy feature is a deliberate act, not a side
effect of depending on the crate.

## The non-negotiable: nothing here may touch the OS

The whole reason this exists is that capturing a window requires raising it (stealing focus, possibly switching workspaces) and driving the OS keyboard sends keystrokes to whatever actually has focus. Measured on the game this was built for: **7,188 distinct colours** captured with the window focused, **1** with another app in front.

So:

- **Capture never reaches outside the game.** It reads either an `Image` a camera renders to
  (`offscreen`) or the game's own swapchain (`window`) — both are the game's own pixels, and
  neither raises, focuses or captures the screen the way a window manager would. `bevy_fairy/
  screenshot` asks for `offscreen` when a `DebugCaptureTarget` is present, and falls back to
  `window` when it is not; an explicit `source: "offscreen"` with no target fails loudly. The
  fallback is not a focus-stealing path, but it is a weaker one: the OS keeps a window's surface
  current only while the window is on screen, so a covered or minimised window can give a stale
  frame, and on macOS an unfocused window has been measured to give one flat colour. A host that
  wants a capture that is always fresh inserts the target and lets the default stand.
- **Input writes into Bevy's own input message stream**, inside the game process — the same
  `Messages<KeyboardInput>` / `Messages<MouseButtonInput>` buffers `bevy_winit` appends to.

`tests/plugin_leaf.rs` is the ratchet, and it is the enforcement of the paragraph above. It
checks the plugin's *resolved dependency closure* — it runs `cargo tree` for the plugin's feature
set — and fails if anything in it can synthesise an operating-system event, if the plugin's direct
dependencies grow past `bevy`, `serde`, `serde_json` and `image`, or if any plugin source mentions
`enigo`, `xdotool`, `CGEvent`, `SendInput`, `winit` or `unsafe`. Widening any of those lists is a
design decision and should cost a deliberate edit in that file.

This is stronger than the check it replaced. When the plugin was its own package, the guarantee
rested on a list somebody maintained; now it rests on the dependency graph Cargo resolves, which
cannot be quietly widened by adding a dependency to a feature.

## BRP handlers run in `RemoteLast` — after everything — and that has bitten input

`DebuggerPlugin::build` registers its methods with `RemotePlugin::with_method_main(..)`, so handlers execute in **`RemoteLast`**, a schedule `RemotePlugin` inserts *after* `Last` (`bevy_remote-0.20.1/src/lib.rs:832-835`). It is the very end of the frame. Bevy's `keyboard_input_system` clears `just_pressed`/`just_released` in the **next** frame's `PreUpdate`.

A press written there therefore has its `just_pressed` flag cleared before any `Update` system reads it, so **`just_pressed`-based actions could never be triggered by injected input** — the method returned `success: true` and the game did not move. Held `pressed` state survives the clear, which is why some actions appeared to work and others did not. See `src/plugin/input.rs` for how this is handled now; if you change the schedule or the action shape, re-verify against a real game rather than a unit test.

(This paragraph said `Last` for as long as it existed, and it was close enough to be useful and wrong enough to mislead anyone reasoning about ordering against another `Last` system.)

## A watching method is how you wait, and the engine already owns it

`with_watching_method_main` (`bevy_remote-0.20.1/src/lib.rs:632`) re-runs a handler **every frame** in `RemoteLast` with exclusive `World`. `Ok(None)` means *not yet* and parks the request in `RemoteWatchingRequests`; `Ok(Some(v))`/`Err` sends a frame to the client and **keeps** the request open; `remove_closed_watching_requests` reaps it when the client goes away. The HTTP transport serves it as a `BrpHttpResponse::Stream` of `application/json` chunks, so `curl -N` consumes it — refused only inside a batch.

`bevy_fairy/guide+watch` is the first user of this, and it is why the guide channel needed **zero new dependencies**: a condition-watcher with lifecycle management, already written and tested by the engine.

The trap it walked into first: **the handler runs every frame, so any answer that does not change state is re-sent sixty times a second.** Both non-advancing answers (`waiting_on_a_person`, `done`) now announce once per step index and then park. See `Guide::announced`, and `tests/guide_steps.rs` for the two tests that exist because the first draft did not.

## The guide channel: the plugin talks to the *person*, not just the app

`bevy_fairy/guide` posts a script and the host renders one step; `bevy_fairy/guide+watch` waits on a condition and records `k/n`. The design and its citations are in `docs/bevy_fairy.md`. The parts that constrain edits here:

- **The plugin never learns the host's vocabulary.** Checkpoints are one-shot systems the host registers by name (`Checkpoints`), the same seam `PendingInput`'s public `queue_*` methods are. Anything that made this crate know what a tile is would be the mistake.
- **The overlay spawns no camera**, because a second one silently breaks any host `Single<&Camera>` query — the trap the mirror camera already argues about.
- **Step copy must be ASCII.** Bevy's embedded font is 95 codepoints and this crate cannot ship one without widening `tests/plugin_leaf.rs`. `the_one_script_this_crate_ships_is_ascii` pins the example.

## Injected input is the *source*, not the fold — and that is what lets an agent type

`apply_pending_input` runs in `PreUpdate` **`.before(InputSystems)`** and writes `KeyboardInput` / `MouseButtonInput` **messages**. `ButtonInput<KeyCode>` and `ButtonInput<Key>` are Bevy's own fold of that stream, done by `keyboard_input_system`, which clears last frame's edges and *then* folds — so a message written ahead of it survives the clear that erased the old direct write.

**This reverses the first fix, and the reversal is the point.** Writing `ButtonInput` directly reached everything that reads `ButtonInput` and nothing that reads the stream — which is **every text field in every Bevy app**, since they use `MessageReader<KeyboardInput>` and match `logical_key`. So an agent could press keys and not type into them, and could not press Enter or Escape *in a field* either. That was `FVS-R-12`, and it blocked three verifications in one day.

Two consequences worth knowing before you touch this:

- **`InputPlugin` is now required.** Without it nothing folds the stream and every injection is accepted and read by nobody. `DebuggerPlugin::finish` asserts it — `finish`, not `build`, so it holds whichever order the host adds plugins in.
- **`kind: "Keyboard"` takes `key` *or* `text`, and sending both is refused.** `key` names a key on the keyboard and carries its logical half whenever Bevy spells the same name in `Key` (93 of them do, including `Enter`, `Escape`, `Backspace`, `Space`, `Tab`); `text` is what should arrive, one message per character, so `"site_67"` is one call and one frame. A separate `kind: "Text"` was designed and rejected: `Escape` is spelled identically in both enums, so an agent would have had to pick a kind based on which of the *host's* systems was listening — knowledge it cannot obtain — and each kind would produce only half of what a real key produces.

`text` refuses control characters by name (`use key: "Enter"`), and a space becomes `Key::Space` rather than `Key::Character(" ")`, because that is what a space bar produces and text handlers match the former.

## The cursor is a resource, not the window's

`bevy_fairy/input` accepts `kind: "Cursor"` with `x`/`y` in logical window pixels, or `clear: true`. It writes `DebugCursor`, **not** `Window::set_cursor_position`.

That looks like the obvious call and it is the wrong one, read out of the pinned engine rather than guessed: `Window::set_cursor_position` writes an internal field, and Bevy's windowing backend diffs that field against a per-window cache every frame and asks the platform to **move the physical pointer** (`bevy_winit-0.20.1/src/system.rs:426-430`). The cache is `pub(crate)`, so a plugin cannot suppress the diff. Writing the window would drag the mouse out from under whoever is at the machine — the precise class of thing this crate exists to avoid, and it would pass `tests/plugin_leaf.rs` while defeating its whole purpose.

**Picking does not need that.** `src/plugin/pointer.rs` also writes the `WindowEvent::{CursorMoved, MouseButtonInput, MouseWheel}` messages `bevy_picking`'s `mouse_pick_events` reads (it takes a click's position from the last `CursorMoved`), so `bevy_ui` buttons and `On<PointerClick>` observers fire. With a `DebugCaptureTarget` present it instead writes `PointerInput` located on that image (a window-addressed pointer misses cameras that render to an image), and `x`/`y` are pixels of the image. One route per call, never both: two writers to `PointerId::Mouse` fight over its location. `tests/plugin_pointer.rs` drives a real `bevy_ui` node under `DefaultPlugins` (no winit, no GPU) through `handle_input`. Issue #9.

**A host that reads the pointer itself has to use `bevy_fairy::cursor_position(&window, &debug_cursor)`** or the injected half never reaches it. A host that calls `Window::cursor_position` directly is undrivable by an agent, and that is not a bug in the plugin.

Ordering is load-bearing and is handled in `apply_pending_input`: a move *after* a button, or a second move in one frame, is deferred to the next frame. Applying `press, move` together makes the game read the press at the new position — the click never happens where it was aimed — and two moves in one frame collapse a drag's path to its endpoint.

## `DebuggerPlugin` owns `RemotePlugin`

`DebuggerPlugin::build` adds `RemotePlugin::default().with_method_main(..)` itself. Bevy rejects a duplicate plugin by name, so a host app that also adds `RemotePlugin` panics the moment the feature is switched on. A host should add `DebuggerPlugin` plus **only** a transport (e.g. `RemoteHttpPlugin`).

## Profiles

There is deliberately no `[profile.*]` in `Cargo.toml`; the manifest comment records what was removed and why `panic = "abort"` is a real difference.

## Build and test

One package, so `cargo test --workspace` covers everything. The halves are selected by feature,
and the plugin's tests declare `required-features` so they are simply not built when the plugin is
off:

```sh
cargo test --workspace                    # the server, and the plugin's tests
cargo test --no-default-features --features plugin --test plugin_leaf
                                          # the OS-boundary ratchet, on its own
cargo install --path . --no-default-features --features server
                                          # the bevy-fairy binary, without Bevy
UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list
                                          # regenerate docs/mcp-tools.md after a tool changes
```

The plugin's examples need their own features: `plugin-screenshot` for the capture example,
`plugin-guide` for the guided-steps example, plain `plugin` for the rest.

## `watch` samples by polling, and brackets its frames

The `watch` tool does not use bevy_remote's `+watch` streams: those report only changes to one entity's components, and `brp` refuses them. `watch` polls a one-shot method, sending `bevy_fairy/status`, the method and `bevy_fairy/status` as one JSON-RPC batch. **A batch is not one frame**: `bevy_remote::http::process_request_batch` awaits each request's answer before sending the next, so the requests can land on different frames (measured: the value was one frame off the first status in about half of the runs). Each sample therefore carries `frame` and `frame_until`, the two status readings, and claims only that the value was read between them. `tests/fixture_ground_truth.rs::watch_brackets_the_frame_every_sample_was_read_on` pins that against the spinner's own frame counter. A single-frame stamp needs a plugin-side method; none exists. No progress notifications yet (30 s cap instead).

A whole-result summary cannot say which value stopped. The fixture's spinner freezes `angle` while `frames` keeps counting, so the component differs from its first sample at every sample and `constant_since_frame` is null: an agent that watched the whole component read "still changing" and spent a dozen calls finding the field. `src/server/watch_fields.rs` therefore gives every leaf of the sampled value its own `changed`, `distinct_values`, `first_change_frame` and `constant_since_frame` (`fields` in the reply, absent when the value holds fewer than two leaves), and the whole-value numbers come from the same `movement` function, so the two cannot disagree on what "constant" means. Pointers are prefixed with the request's `pointer`, so any of them can be passed back. An array index is a position, not an identity; `tests/fixture_ground_truth.rs::watch_without_a_pointer_says_which_spinner_field_froze` pins the spinner case.

## `observe` and `detect_anomaly` say which frames they read between

`BevyDebuggerTools::stamp_frames` reads `bevy_fairy/status` before and after the handler and puts `frame` and `frame_until` on the reply, so an agent that reads twice can tell how far apart the reads were and whether a value is stale. Like `watch`, it claims a bracket and not a frame: the handler's own requests are served one per frame. A game without the plugin has no `bevy_fairy/status`; the reply then carries `frame_unavailable` and the game's own reason, never a made-up frame, and a refusal is left unstamped. `tests/fixture_ground_truth.rs::observe_brackets_the_frame_every_read_was_taken_on` checks the bracket against the spinner's own frame counter.

`detect_anomaly`'s detectors read particular components, and an empty `anomalies` list from a detector that read nothing is not a clean result. `DetectorCoverage` (`examined`, `evaluated`, `status`) is what `detect` returns beside the list, and `AnomalyDetector::emits` is what `status` lists, so neither can drift from the detectors. BRP keys a component by its full type path and sends a `Vec3` as `[x, y, z]`, so the detectors match the last path segment and read either vector form; a detector that looks for a literal short key cannot fire against a real game. Severity is `1 - threshold / value`, never a `.min(1.0)` on a ratio already above 1.

## `bevy_fairy/ui_text` answers "what can the player read", and says why a node is not

A `Text` node's `InheritedVisibility` stays true after its slot is set to `Display::None` (the game keeps the old string), so visibility alone reports stale strings as on screen. `src/plugin/ui_text.rs` checks, in order: empty text, `InheritedVisibility`, a zero `ComputedNode.size`, text alpha, an ancestor's `CalculatedClip`, overlap with the render target; a hidden node names the first check it failed. `rect` is physical pixels of the node's target (the space of `screenshot`'s `region`), cut to the target when visible. It is a built-in *command*, reached with `run`, not a tool. `tests/plugin_ui_text.rs` runs a real laid-out `bevy_ui` tree. Not measured: the issue's "reply at most a fifth of a raw `world.query`" — the fixture game has no HUD.

## `bevy_fairy/time` is a request, not a write

The command writes `ClockRequest`; `time.rs` re-applies it in `First`, `.before(TimeSystems)`, every frame, because a host (Sanctuary) that writes `Time<Virtual>` in `Update` overwrote the old one-shot write within a frame. `DebuggerPlugin::host_owns_clock()` removes that system; the host reads `ClockRequest`. `held` is measured by a system `.after(TimeSystems)` and compared with the *current* request, so it is `null` until a frame has run under it. `tests/plugin_clock.rs` pins this with hostile host systems. Issue #10.

`speed` is capped at `MAX_SPEED` (1000): `Time<Virtual>` multiplies the frame's raw delta by it with `Duration::mul_f64` *before* its `max_delta` clamp, so a huge speed panics the game on the next frame, and the request is re-applied every frame, so the host cannot reset it first. A frame the engine does clamp (`speed` above about 15 at 60 fps under the default 250 ms `max_delta`) advances slower than the speed in force; that is `clamped_by_max_delta`, and the request still counts as `held` when the speed in force is the request's, because nothing overrode it. Issue #12.

## `bevy_fairy/ring` records state inside the game, so it samples every frame

It runs `process_remote_query_request` (public in bevy_remote) from an exclusive system in `Last`, after `tick_frames`, so a sample's `frame` is the same counter `status` reports. The query is made `strict` unless the caller says otherwise: non-strict `world.query` silently matches nothing for an unknown component path, and a buffer of empty arrays reads as "no entity has it". `tests/plugin_ring.rs` checks each sample against a counter the test itself bumps. Samples are query results, not pixels; the frame half of the old B-021 idea (a frame ring beside `burst`) is not built.

## The fixture game is the ground truth

`examples/fixture_game.rs` is a headless Bevy app whose state is known from its own `pub const`s:
six `Crate`s with fixed weights (one deliberately negative), a `Score` resource, a `Mover` at a
fixed speed, and a `Spinner` with a **planted bug** -- it stops turning after a fixed frame.
`tests/fixture_ground_truth.rs` runs it in-process and checks the server's tools against those
constants. A tool test that asserts against a value the tool itself produced proves nothing;
assert against the fixture.

```sh
cargo run --example fixture_game --features plugin    # FIXTURE_PORT, FIXTURE_FPS to override
```

Things that need a window -- a UI button, a pickable object, a text field -- are not in it yet,
because nothing headless can click them honestly.

`examples/demo_game.rs` is the same world rendered offscreen, still headless, and it is what the
README's GIFs are recorded from (`scripts/record_demo.py`). It includes `fixture_game.rs` by
`#[path]` and calls its systems, which is why `spawn_world`, `move_mover`, `spin` and `env_or`
are `pub`; it only adds meshes, a camera, a light and `demo/spawn_cube`, and must never write a
fixture value. Rendering's first frames are black, so a capture taken in the first second or two
shows only the clear colour.

## The evals ask an agent, not a test

`evals/run.py` runs a fresh headless `omp` session per task, against the fixture, with only this
server's tools: no repo, no files, no shell. It scores the final answer against the fixture's
constants and appends one line per task to `docs/loop/evals.jsonl`. A tool change that passes
every test can still leave an agent unable to answer; this is where that shows.

```sh
python3 evals/run.py --server-bin target/debug/bevy-fairy     # dev suite; --suite heldout
```

`evals/tasks/heldout/` exists to show a change generalised. Do not tune a tool description to
it, and do not read its answers while working on a change. `evals/README.md` has the scoring
and the isolation check.

- Consult Bevy documentation often. It can be found at codex_fs/offline_reference_docs/bevy-0.20-book/
