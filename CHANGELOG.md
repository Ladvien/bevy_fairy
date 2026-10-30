# Changelog

## Unreleased

### Fixed

- `observe`, `guide` and `replay` accepted and silently dropped arguments they do not have (`observe` with `limit: 5` ran
  unlimited). They now refuse unknown arguments like every other tool; `tests/tool_list.rs::every_tool_refuses_arguments_it_does_not_advertise`
  checks every tool in `tools/list` advertises `additionalProperties: false`.

### Breaking

- `server::error::Error::WithContext { context }` now holds `Box<ErrorContext>`. It was 150+ bytes inline, which made every
  `Result<T, Error>` in the server a `clippy::result_large_err` site (485 of them); `Error` is now small enough that none remain.

### Removed

- `server::tools::replay_v2`, `server::tools::observe_optimized`, `server::query_optimization`, `server::parallel_query_executor`,
  the one bench that exercised them, and the `criterion` dev-dependency. No registered tool reached any of them.
- 38 server modules (about 21,000 lines) that nothing the binary runs could reach, and the tests and
  benches that only exercised them: the legacy `mcp_server` (the live one is `mcp_server_v2`), the
  session manager, system/memory profilers, issue detector, performance budget, query builder,
  visual overlays, connection pool, heartbeat, and the caches and pools behind them. None was
  reachable from a registered tool, and the tool list is unchanged. These were `pub` items of the
  library; nothing downstream of the binary used them.
- `docs/tools/`, `docs/api/`, `docs/api-reference.md` and `docs/troubleshooting/`: hand-written tool
  references whose examples sent arguments the tools refuse (`experiment_type`, `detection_type`) and
  called tools that do not exist (`orchestrate`, `resource_metrics`, `performance_dashboard`). The
  generated `docs/mcp-tools.md` is the reference.

### Added

- Docs-drift tests: every tool-call example (`callTool` with a name and an argument object) in a doc names a registered tool and arguments it
  takes, and every relative markdown link resolves.

## 0.7.2 - 2026-10-30

### Changed

- The `observe` tool's `query` field now carries its grammar in the schema: eight complete example
  queries (`list all entities`, `show entity <id>`, `find entities with component <Name>`, ...) and a
  pointer to `brp` for anything else. `observe` refuses bare nouns such as "score" or "crate", and
  agents kept sending them because nothing advertised the shapes. A test feeds every advertised
  example to the parser `observe` uses. No behaviour change in the handler.

## 0.7.1 - 2026-10-30

### Changed

- The `brp` tool's `params` schema now shows the params shape for `world.query`,
  `registry.schema`, `world.list_components` and `rpc.discover`, including that every
  `registry.schema` field is a list and must be omitted rather than sent as null. Agents had
  guessed `with_crates: null` and been refused. A ground-truth test runs every example in the
  advertised schema against the fixture game. No behaviour change.

## 0.7.0 - 2026-10-30

### Removed

- **Breaking:** `server::stress_test_system`, `server::tools::stress`, the `stress-testing` Cargo
  feature, and the `debug_performance` pipeline template (its only step called the `stress` tool).
  Nothing reached them: `stress_test` left `tools/list` in 0.3.x and the legacy server that called
  them was deleted in 0.4.0. No MCP tool, flag or behaviour changes. (The `hypothesis` tool's
  `stress_test` action is separate and stays.)

## 0.6.1 - 2026-10-30

### Fixed

- `SystemProfiler::get_system_history` returned each sample's `timestamp` as the *age* of its frame
  (`start_time.elapsed()`), so timestamps fell as time passed and later samples looked earlier.
  They are now the frame's wall-clock start in Unix microseconds, the same clock live samples use.
  The previously ignored `test_profiling_history` now runs and passes. No MCP tool output changes.

## 0.6.0 - 2026-10-30

### Removed

- **Breaking:** `server::brp_client_v2` (`BrpClientV2`) and `server::brp_client_refactored`: two
  unused duplicates of `server::brp_client::BrpClient`, the one client every tool uses. The
  published server has one BRP client now.
- `src/server/cli.rs` (426 lines): it was never declared as a module, so it was never compiled.
  No MCP tool, flag or behaviour changes.

## 0.5.0 - 2026-10-29

### Removed

- **Breaking:** the `observability` feature and the `server::observability` module (2.7k lines).
  It never compiled (42 errors), so `full-debugging` and `cargo build --all-features` failed with
  it; nothing in the 14 registered tools reached it. `full-debugging` no longer lists it, and
  `cargo build --all-features` is green again.
- **Breaking:** `Config::observability` and `ObservabilityConfig`, and the environment variables
  that only filled it (`METRICS_ENABLED`, `METRICS_PORT`, `TRACING_ENABLED`, `JAEGER_ENDPOINT`,
  `OTEL_EXPORTER_OTLP_ENDPOINT`, `HEALTH_CHECK_ENABLED`, `HEALTH_CHECK_PORT`,
  `OTEL_TRACES_SAMPLER_ARG`, `DEPLOYMENT_ENVIRONMENT`). They were parsed and then ignored; an
  invalid value was refused for a setting that did nothing.
- Unused optional dependencies `opentelemetry*`, `tracing-opentelemetry`, `prometheus`, `metrics`,
  `metrics-exporter-prometheus`.

## 0.4.0 — 2026-10-29

### Removed

- **Breaking:** the `--tcp` / `--server` flags of `bevy-fairy`. They bound a port, accepted a
  connection and dropped it without reading a byte; nothing could ever be served on it. They are
  now refused as unknown arguments (exit 2). stdio is the only transport. Migration: register the
  server in an MCP client as `bevy-fairy --stdio`.

### Changed

- Started from a terminal with no `--stdio`, the binary exits 2 with a usage message instead of
  falling into the dead TCP mode. Logs always go to stderr.

## 0.3.5 — 2026-10-29

### Added

- `watch` tool: samples one one-shot BRP method 2-100 times (20-5000 ms apart, 30 s at most) and
  returns the series. Every sample carries the game frames it was read between (`frame` to
  `frame_until`: the method is sent between two `bevy_fairy/status` reads in one batch, which
  bevy_remote serves request by request, so one frame is not guaranteed) and its time since the
  first sample. The reply states whether the value changed, how many distinct
  values were seen, the frame of the first change, and `constant_since_frame`, so "the angle stops
  at frame N" is one call instead of dozens of `brp` polls. `pointer` (RFC 6901) picks the value to
  watch; a pointer matching nothing, an out-of-range count and a `+watch` method are refused with
  the valid range. Polled, not streamed: changes between samples are not seen, and the reply says so.
- `BrpClient::send_batch`.

### Changed

- The `brp` refusal of a `+watch` method now points at `watch`.

## 0.3.4 — 2026-10-29

### Fixed

- Handler failures from `observe`, `guide`, `experiment`, `hypothesis`, `detect_anomaly` and `replay`
  now come back as `isError` tool results carrying the message, not JSON-RPC internal errors that
  MCP clients may hide from the model.
- `experiment` with `action: "execute"` and no `actions` used to succeed with zero results (the
  request struct defaults `actions` to `[]`). It now refuses, says nothing was run, and shows the shape.

## 0.3.3 — 2026-10-29

### Fixed

- `tests/plugin_guide_steps.rs` (19 tests pinning the guide channel) was gated on
  `#![cfg(feature = "guide")]`, a feature that does not exist, so it compiled to nothing and CI never
  ran it. The gate is gone (the manifest's `required-features = ["plugin-guide"]` is the real one);
  the test had also rotted against a renamed example path, now corrected. `GuidePlacement` and
  `BEAT_SECONDS` are re-exported from the crate root next to the other guide items.

## 0.3.2 — 2026-10-29

### Fixed

- Plugin doc comments and CLAUDE.md cited Bevy 0.19 engine paths and line numbers, and one said BRP
  handlers run in `Last`; they run in `RemoteLast`. Citations now point at `bevy_remote`,
  `bevy_winit` and `bevy_input` 0.20.1 and were checked against those sources. No behaviour change.

## 0.3.1 — 2026-10-29

### Added

- **`bevy_fairy/time` holds against a host that writes the clock (issue #10).** The command now
  records a public `ClockRequest { pause, speed }` resource instead of writing `Time<Virtual>` once.
  A system in `First`, ahead of bevy_time's clock system, re-applies it every frame, so a host that
  `unpause()`s or `set_relative_speed()`s later in the frame no longer undoes the agent's pause or
  speed; the request survives a game reset until the agent clears it. A host that must combine the
  request with its own holds and slow motion adds `DebuggerPlugin::host_owns_clock()`: the crate then
  never writes `Time<Virtual>`, and the host ORs `pause` and multiplies `speed` itself. The reply
  gains `held` (the clock obeyed on the last frame; `null` with `unavailable` until a frame has run
  under the request), `host_owned`, `effective_speed` and `requested`. A host that does nothing
  behaves as before.

- **`bevy_fairy/ui_text` (issue #8).** A built-in read-only command, behind the new `plugin-ui-text`
  feature, that answers "what text can the player read?" as a query instead of a screenshot. Each
  node of the `bevy_ui` tree that carries `Text` (spans joined) comes back with its `rect`
  (`[x, y, w, h]`, physical pixels of its render target, the space of `screenshot`'s `region`),
  `visible`, the `Name`s of up to three named ancestors and, for a hidden node, `hidden_because`:
  `empty_text`, `inherited_visibility_false`, `zero_size` (an expired `Display::None` slot that kept
  its string), `text_alpha_zero`, `clipped_by_ancestor_overflow` or `outside_render_target`.
  Hidden nodes are omitted unless `include_hidden`; `contains`, `region` and `limit` filter, and the
  limit applies after sorting top to bottom. Two nodes with one sentence are two rows. Called with
  `run`, so the tool count is unchanged. `Text2d` is out of scope.

## 0.3.0 — 2026-10-29

Breaking for a host that calls `apply_pending_input` or `handle_input` as plain functions: both
systems gained a parameter (`PointerRoute`, and an optional `DebugCursor`). A host that only adds
`DebuggerPlugin` is unaffected.

### Fixed

- **An injected `Cursor` and `Mouse` now reach `bevy_picking` (issue #9).** They used to write only
  `MouseButtonInput` and the `DebugCursor` resource, which picking never reads, so an agent could
  aim and click and no `bevy_ui` button or `On<PointerClick>` observer noticed. `bevy_fairy/input`
  now also writes the `WindowEvent` messages a real mouse produces (plus `CursorMoved`); hover,
  click and drag land. A host with a `DebugCaptureTarget` is driven through a picking pointer
  located on that image instead, and `x`/`y` are then pixels of the image. A `Mouse` call with no
  cursor position set replies with a `note` saying where the click will land.

### Changed

- `plugin-screenshot` now also turns on `bevy/bevy_picking`, which the image route needs.

## 0.2.3 — 2026-10-23

A documentation release: the README now shows the server working. The library and the server
are unchanged.

### Added

- **Two GIFs in the README.** A scripted tour puts real MCP tool calls beside the game frames the
  server's own `screenshot` tool captured, and runs long enough to show the fixture's planted bug:
  the spinner stops turning at frame 120. A real agent session, Claude with no tools but this
  server's, finds that bug and the frame it happens on.
- **`examples/demo_game.rs`**, the fixture game rendered offscreen and headless: no window, no
  display, one `demo/spawn_cube` command. It reuses the fixture's world unchanged, so every
  answer it gives is the one the fixture's constants predict.
- **`scripts/record_demo.py`**, which records both GIFs from it:
  `python3 scripts/record_demo.py all`. A tool error aborts the recording, and the agent GIF is
  drawn only from a session that passed the `spinner_stops` eval.

## 0.2.2 — 2026-10-23

A documentation release: the README crates.io shows is rewritten from scratch. No code changed.

### Changed

- **The README** now says what the crate is and why it exists, then gives a four-step quick
  start in which every step has a check you can run, and a table of what to do when a step fails.
  After that come short guides (game commands, capture, input, the guide channel, scenes), and
  reference tables for the BRP methods, Cargo features, environment variables and examples, each
  example with the feature flags it actually needs. It links the generated
  [`docs/mcp-tools.md`](docs/mcp-tools.md) rather than copying it. The checkpoint example now
  takes the `In<Value>` its `SystemId` type requires; the old one did not compile.
- **`examples/plugin_agent_scene.rs`** gave a run command that did not work
  (`-p bevy_fairy --example agent_scene`); it now says
  `cargo run --features plugin-screenshot --example plugin_agent_scene`.

## 0.2.1 — 2026-10-23

A patch: two tools that told an agent something untrue, and a tool reference generated from
the server itself.

### Fixed

- **`brp` said "no game answering" when the game had answered.** A watching method such as
  `world.get_components+watch` streams a new answer every time the value changes; `brp` read
  it as one reply, timed out after 5 s, and reported the game as down. An agent in the evals
  took that at its word. `brp` now refuses `+watch` methods and names the one-shot method to
  call instead, and any other reply it cannot read is reported as an unreadable answer from
  the game, not as silence.
- **`detect_anomaly`'s description named a `detection_type` field the tool does not have**, and
  refuses as unknown. It now says what `detect` does: read every entity once and run the
  detectors over them with the advertised thresholds.

### Added

- **[`docs/mcp-tools.md`](docs/mcp-tools.md)**, every tool's description and arguments,
  generated from the server's `tools/list` by `tests/tool_list.rs`. A test fails when it no
  longer matches, and another fails when a description names a field or value the tool does
  not have.

## 0.2.0 — 2026-10-21

A minor bump, and a breaking one: the crate is renamed and merged, the tool schemas
changed, and the BRP method names changed.

The gap from 0.1.8 (2025-08-23) to here is large — a year of work, most of it in a
Bevy plugin that was not published at all until now.

### Breaking

- **The crate is renamed and the two halves are merged.** `bevy_debugger_mcp` and
  `bevy_debugger_bevy` are now one crate, `bevy_fairy`. The names stuttered, and a game
  that wanted the plugin had to depend on a crate whose name said it was about the server.

  The two halves are two *features* of the one crate, and they stay independent: Cargo
  resolves dependencies per feature, so a game that takes the plugin compiles the plugin's
  closure and none of the server's.

  | | before | after |
  |---|---|---|
  | a game | `bevy_debugger_bevy = "0.2"` | `bevy_fairy = { version = "0.2", default-features = false, features = ["plugin"] }` |
  | an agent | `cargo install bevy_debugger_mcp` | `cargo install bevy_fairy --no-default-features --features server` |
  | the binary | `bevy-debugger-mcp` | `bevy-fairy` |
  | the BRP methods | `bevy_debugger/*` | `bevy_fairy/*` |
  | the environment | `BEVY_DEBUGGER_*` | `BEVY_FAIRY_*` |
  | the control script | `bevy-debugger-control` | `bevy-fairy-control` |

  The plugin's public items are re-exported at the crate root, so a game writes
  `use bevy_fairy::DebuggerPlugin` — the same as it did when the plugin was its own crate.

  `bevy_debugger_mcp` 0.1.8 stays published and is not yanked. It will not receive further
  releases; this is the one that replaces it.

- **The `experiment`, `hypothesis` and `detect_anomaly` tools take different arguments.**
  Each tool's parameters are now generated from the same Rust type its handler parses, so
  the advertised schema and the handler cannot drift apart again. The old schemas advertised
  `experiment_type`, `hypothesis` and `detection_type`; the handlers
  dispatched on `action`, so every one of those requests was dropped and a default ran
  instead. The new schema names the field the handler reads.

  | tool | before | after |
  |---|---|---|
  | `experiment` | `experiment_type`, `params`, `duration` | `action`, `actions` |
  | `hypothesis` | `hypothesis`, `confidence`, `context` | `action`, `description`, `success_condition`, `actions`, `variation_type`, `numeric_min`, `numeric_max`, `string_pool`, `parameter_name`, `parameter_values`, `iterations`, `timeout`, `seed`, `threshold` |
  | `detect_anomaly` | `detection_type`, `sensitivity`, `window` | `action`, `window_size`, `z_score_threshold`, `iqr_multiplier`, `min_samples`, `performance_threshold`, `entity_growth_threshold`, `min_severity` |

  An agent that learned the old names has to learn the new ones. Nothing that worked stops
  working: the old names were never read.

- **`stress_test` is removed.** It could not be made honest: every spawn and change it sent
  used a short type name (`Transform`) or a component no game has (`Health`, `Component_0`),
  so against a real game every operation failed while the tool answered `"success": true`.
  Its frame-time percentiles were `16.67 + rand * 10`, its CPU figures were random, and it
  ran for 30 s whatever `duration` it was given.

- **`bevy` is now 0.20.** The plugin is a Bevy 0.20 plugin.

### Added

- **A Bevy plugin**, behind the `plugin` feature, published for the first time. It
  owns the BRP `RemotePlugin`, so a host adds `DebuggerPlugin` plus a transport and never a
  second `RemotePlugin` — adding one twice panics the game on start.
- **The guarantee that the plugin cannot reach the operating system is now checked against
  the resolved dependency graph** rather than against a list. `tests/plugin_leaf.rs` runs
  `cargo tree` for the plugin's feature set and fails if anything in it can synthesise an
  OS event, which is stronger than the manifest check it replaces: the graph cannot be
  quietly widened by adding a dependency to a feature.
- **Game-registered commands.** A host registers a typed handler under a `<game>/<verb>` name
  with `add_debug_command`; the agent discovers the vocabulary from the game itself through
  `bevy_fairy/commands`, and each command ships a checked example.
- **Capture from an offscreen target or the window.** `bevy_fairy/screenshot` asks for
  `offscreen` when a `DebugCaptureTarget` is present, and falls back to the window when it is
  not. Neither path raises, focuses or captures the screen the way a window manager would.
- **Injectable input that really arrives.** Input is written into Bevy's own input message
  stream, inside the game process, ahead of the input systems — so it behaves like a key
  somebody pressed, and an agent can type into a text field. The cursor is injectable too, and
  a host reads it through `bevy_fairy::cursor_position`. The window's own cursor is
  deliberately never written: Bevy's windowing backend would turn that into a request to move
  the *physical* pointer, dragging the mouse out from under whoever is at the machine.
- **A guide channel.** `bevy_fairy/guide` posts a script and the host renders one step;
  `bevy_fairy/guide+watch` waits on a condition and records `k/n`. The plugin never learns
  the host's vocabulary: checkpoints are one-shot systems the host registers by name.
- **Replayable scenes.** Every recorded command call is logged, and `bevy_fairy/scene`
  returns that log as a file. `BEVY_FAIRY_SCENE` replays one step per frame at startup.
- **A stdio transport that Claude Desktop can use.** Logging goes to stderr, never stdout, in
  stdio mode.

### Fixed

- **The server kept its tools in a router nothing read**, and quit right after `initialize`.
- **Injected input was written as the fold rather than the source**, so `just_pressed`-based
  actions could never fire and no text field could be typed into.
- **Four deadlocks and three correctness bugs** in the plugin's own internals.
- **The three tools above now do what they are asked**, and `hypothesis` measures the entity and
  component counts from the game instead of reporting constants. The three metrics BRP cannot
  report — frame time, memory, CPU — are refused with a reason rather than answered with a
  number.
- **`replay` records the game's entities** instead of returning an empty map. Events are not
  observable over BRP, so a frame says so rather than showing an empty list that reads as
  "nothing happened".
- **`tools/list` returns the tools sorted by name.** The order came from a `HashMap` in the
  router, so it changed on every launch — four launches gave four orders.
- **A missing `bevy_fairy/*` method names the feature that registers it.** `screenshot`
  answered a bare "Method `bevy_fairy/screenshot` not found", and `guide` told the agent to
  build with `--features debugger`, a feature this crate does not have. Both now say which of
  `plugin`, `plugin-screenshot` or `plugin-guide` the game was built without.
- **A refusal comes back as a tool error.** `observe`, `detect_anomaly`, `hypothesis`,
  `experiment` and `replay` answered an unknown action or an unparseable query as a
  successful result whose body held `"error": ...`, so a client that branches on `isError`
  took "Query parsing failed" for an answer. Such a result now has `isError` set.
- **The docs match the code.** `CLAUDE.md`, the README and `book/SCREENSHOT_SETUP.md` no
  longer tell a reader to hand-register a method the plugin already registers, and the
  capture description says what the code does: the fallback is to window capture, which is
  weaker than offscreen but is not a focus-stealing path.

### Migration

An agent that calls the three tools above with the old field names will get a structured error
naming the bad value, listing the valid ones, and giving one corrected call. Read the tool's
schema from `tools/list` rather than from memory.

A host that linked the plugin by git now depends on
`bevy_fairy = { version = "0.2", default-features = false, features = ["plugin"] }`. A host
that hand-registered `bevy_debugger/screenshot` must delete that registration: the plugin
owns it, and registering it twice panics the game on start.

A host that registered a game command under a `bevy_debugger/` name must rename it to
`bevy_fairy/`: the BRP method names changed with the crate.

## 0.1.8 — 2025-08-23

The last release before this one. See the git history for its contents.
