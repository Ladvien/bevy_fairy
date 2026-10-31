# Changelog

## Unreleased

Breaking: the `replay` MCP tool is removed, and `detect_anomaly` lost two arguments it never read. Migration is below.

### Removed

- `detect_anomaly`'s `iqr_multiplier` and `performance_threshold` arguments (`AnomalyConfig::iqr_multiplier` and `performance_threshold`), the anomaly types `PotentialMemoryLeak`, `PerformanceSpike` and `RapidValueChange`, and the same three from `status`'s `supported_anomaly_types`. `configure` accepted both arguments and echoed them back, `performance_threshold` was described as a frame time in milliseconds, and no detector read either; no detector ever emitted the three types. A call that names the arguments is now refused as an unknown field. `supported_anomaly_types` is derived from what the detectors can emit (`AnomalyDetector::emits`), and a test drives each detector to prove it. The entity count and the velocity are what the detectors read; there is no frame-time detector.
- The `replay` tool, and `ReplayRequest`, `ReplayAction`, `StepDirection`, `MergeStrategyArg`, `EntityModification` and `EntityRemoval` from `server::mcp_tools`. Its playback (`play`, `pause`, `seek`, `step`, `set_speed`, `playback_status`) answered "Playback started" or "Stepped forward" while nothing reached the game: the sync that was to write each recorded frame back was a `TODO` that returned `Ok`. Against the fixture game, after `record`, `stop` and `play`, the mover stood at x = 6.57 while the recording had seen it only between x = 0.53 and 2.55. Its branches (`create_branch` to `branch_tree`) compared the recording with the caller's own edits, which nothing ran, and `merge_branch` with `strategy: manual` silently took the source. A recording could not be read back either: `load` of the file `save` had just written failed with "failed to fill whole buffer", and no action returned a recorded frame. **Migration:** for state over time, use `watch` (polls one BRP method, up to 30 s) or the plugin's `bevy_fairy/ring` command through `run` (records a `world.query` every frame inside the game, and stops on a trigger); to re-run a session, use `scene`.

### Added

- `observe` replies and `detect_anomaly`'s `detect` replies carry `frame` and `frame_until`: the game's frame counter (`bevy_fairy/status`) before and after the read. An agent that reads twice can tell how far apart the reads were and whether a value is stale; with `diff: true`, the earlier reply's stamp and this one's bound how many frames lay between the two reads (the diff itself keeps wall-clock timestamps, not frames). It is a bracket, not one frame, for the reason `watch` brackets: the handler's own requests are each served on a frame. A game that serves BRP without this crate's plugin has no counter, so the reply carries `frame_unavailable` and the game's own reason instead, and a refusal is left unstamped. Each stamped read costs two extra `bevy_fairy/status` calls.
- `watch` replies carry `fields` when the sampled value holds two or more values (a component's fields, the rows of a query): each value gets its own `changed`, `distinct_values`, `first_change_frame` and `constant_since_frame`, beside a `pointer` that can be passed back to `watch` as `pointer`. The whole-value summary cannot say which value stopped: a spinner whose `angle` froze while its `frames` counter kept counting differed from its first sample at every sample, so the reply read `changed: true, constant_since_frame: null` and named no stopped field. Values that moved between values they held and then stopped are listed first, then the other values that changed, then values that never moved (each group in natural pointer order, so `/2` before `/10`), at most 40; `fields_omitted` counts the rest, and the cut never drops a stopped value for a moving one. An array position is a position, so a despawn that shifts the rows of a `world.query` shifts the fields after it. A value one sample lacked counts as a change, not as null: `absent_samples` counts the samples that lacked it, a value that only appeared or skipped a sample is not listed as stopped, and a value gone from the last sample is never reported as constant.

### Changed

- `detect_anomaly` `detect` says what each detector could read. The reply has a `detectors` list (`detector`, `reads`, `examined`, `evaluated`, `status`), and a `note` when one did not evaluate. Before, `anomalies: []` beside `entities_analyzed: 2` was the answer for a game in which no entity carried a component any detector reads, which an agent takes for a clean result. `summary.entities_analyzed` is now `summary.entities_read`: it counted what the query returned, not what any detector could use of it. `status` lists what each detector reads. The tool's description says which components the detectors read.
- `status`'s description says what its `replay` field is: the progress of a scene file the game is replaying, null when none.
- `detect_anomaly`'s physics detector no longer reports itself as judging a speed it cannot flag. A speed is scored against the window it belongs to, so among n samples its z-score is at most (n - 1) / sqrt(n): 2.85 among 10, under the default threshold of 3. Until the samples held reach that bound the entity counts as examined and not evaluated, and `status` says so with the bound; the `z_score_threshold` documentation states it. Found by the independent review of B-087..B-090.

### Fixed

- `detect_anomaly` severity was exactly 1.0 for every physics and entity-growth detection: `(value / threshold).min(1.0)` was computed only once `value > threshold`, so the ratio was already above 1 and the cap read 1. `min_severity` could not tell a reading just past its threshold from one a hundred times past it. Severity is now how far past the threshold a reading is, `1 - threshold / value`: 0 at the threshold, 0.5 at twice it, towards 1 (B-087). A state inconsistency keeps a fixed 0.9 rating, which is a rule's rating and not a measurement; the field's documentation says so.
- `detect_anomaly`'s physics and consistency detectors could not fire against a real game. `world.query` keys a component by its full type path (`my_game::physics::Velocity`) and sends a `Vec3` as `[x, y, z]`; the detectors looked for the literal key `Velocity` and an `{x, y, z}` object. They now match a component on the last segment of its path (`MaxHealth` is not `Health`) and read a vector in either form (B-090). An entity with `Health.current` at or below zero and no `Alive` component was reported as "marked as alive": the absence was read as `true`. Both components must be present now.
- `observe` refused every component path with a digit or a generic argument as "Unrecognized query pattern", including the full path its own "component not registered" error tells the agent to use: `bevy_mesh::components::Mesh3d`, `Camera3d`, `Camera2d`, `Mesh2d`, every component of `avian3d`, and `bevy_time::time::Time<bevy_time::real::Real>`. The five query shapes now take letters, digits, `_` and `::` in a path, with optional generic arguments (`<>`, `()` and spaces allowed inside them), and a list of components (`find entities with A, B`, `show entity N components A, B`) splits only at the commas outside `<>` and `()`, so `Pair<a::A, b::B>` is one component in every shape, including `without` and `find N` (issue #14).
- `bevy_fairy/time` accepted any finite positive `speed`, and a speed of about 1.2e21 or more panicked the game on the next frame: `Time<Virtual>` multiplies the frame's raw delta by it with `Duration::mul_f64` before the `max_delta` clamp, and the request is re-applied every frame, so the host could not reset it first. `speed` must now be above 0 and at most 1000 (the engine caps a frame at `max_delta`, 250 ms by default, so anything above about 15 at 60 fps gives the same clock); a larger value is refused with the bound and an example (issue #12).
- `bevy_fairy/time` answered `held: false` when the engine, not the host, slowed the clock: `{"speed": 20}` on 16 ms frames advances 250 ms, an effective 15.625x, and the reply read as a host overriding the request. `held` is now true when the speed in force is the request's, and the reply gains `clamped_by_max_delta` (`null` with `unavailable` until a frame has run under the request). A host that owns the clock and ignores the request is still `held: false` (issue #12).
- `detect_anomaly`'s entity growth was "per sample" in the description and `window_size` in the arithmetic: the change across the buffer was divided by the configured window, not by the samples it spans, so 20 entities per sample read as 0.2 and a detector with the default window of 100 first reported a leak after 52 samples. It is now divided by the samples spanned, and the anomaly's metadata carries `samples_spanned`. `configure` now resizes the buffers the detectors measure over (the entity-count window and each entity's velocity history), so `window_size: 10` after `configure` is a window of 10, not the 100 the buffer was built with. A buffer no longer reserves its configured size up front, so resizing to a very large `window_size` costs nothing (issue #13).
- `detect_anomaly` `configure` refuses a `window_size` below `min_samples` (or below 2) with an error naming both fields and what to change. A detector keeps `window_size` samples and needs `min_samples` of them before it reports, so such a window never reported anything while `status` echoed it as configured; the resizing above would otherwise have made `configure {"window_size": 5}` under the default `min_samples` of 10 do that silently. Lower `min_samples` in the same call.
- `replay`'s tiny `sample_rate` panic (issue #11) is gone with the `replay` tool above; nothing the server runs reaches that code.

## 0.9.0 - 2026-10-30

Covers everything since 0.8.0. Breaking (semver-checks: 9 major checks): `observe` lost its `reflection` argument and its semantic phrases, `QueryParser::parse` returns a `ParsedQuery` and lost `parse_semantic`, `observe::handle` takes `&ObserveRequest`, `ObserveState` lost its unreachable snapshot history, and the unreachable `server::replay_actor` and `server::resource_manager` public modules were deleted. No MCP tool was removed; `observe` callers that sent `reflection` or a semantic phrase now get a refusal naming what to use instead.

### Removed

- `observe`'s `reflection` argument and the `reflection_data` it added. Its type claims were constants and guesses: every component was reported `is_reflected: false`, `type_category: "Value"` with no `fields`, and each value's `type_info` was inferred from its JSON (the fixture's `f32` weight came back as `f64`, a `Vec3` as `Vec<T>`). The advertised custom inspectors for `Option<T>`, `Vec<T>` and `HashMap` never ran. A call that still sends `reflection` is refused as an unknown field. For a component's fields and their types, call `brp` with `registry.schema`.
- `server::replay_actor` and `server::resource_manager` (about 1,420 lines), and `BrpClient::with_resource_manager`: nothing the binary runs constructed them.
- `observe`'s semantic phrases (`find stuck entities`, `show fast moving objects`, `find overlapping colliders`, `find memory leaks`, `find inconsistent state`, `find physics violations`, `find stuck and fast entities`) and the `semantic_analysis` and `metadata.resolved_component_names` fields they produced. Each claimed a selection the server never made: the velocity and health thresholds were built but never sent to the game, so `find stuck entities` would list every entity with the named components, and `find memory leaks` answered the fixture's mover as a leak candidate at "confidence" 0.6 because it has a `Transform`. They are now refused as unrecognised queries, with the shapes that work. To find entities by a value, read the component with `observe` or `brp` and compare the values yourself. `QueryParser::parse_semantic` is gone with them.
- `ObserveState`'s ten-snapshot history and the methods only it served (`diff_against_history`, `clear_history`, `history_size`, `max_history_size`), plus `with_diff_config`, `configure_diff`, `diff_against_last`, `has_last_snapshot` and `last_snapshot`: no tool reached them. `observe::handle` now takes the `ObserveRequest` the schema is generated from instead of a JSON object it read back by key with silent defaults.

### Fixed

- `observe` with `diff: true` compared each read with itself and always answered "No changes detected". It now compares with the previous `diff` read of the same query; a read with nothing to compare with (the first one, or the first after the query changed) answers `"diff": {"baseline": true, ...}` instead of leaving `diff` out. Comparing two different queries would have reported what the second one left out as components the game removed. The metadata's constant `"diff_target": "last"` is gone, and `ObserveState::add_snapshot` takes the query and the snapshot.
- `observe`'s `show entity N` asked the game for no components and answered `{"components": {}}`, so every entity read as one with none. It now asks for every registered component and leaves out the "not present" errors for the ones the entity lacks.
- `observe`'s description claimed it reads resources (no query shape does; use `brp` with `world.get_resources`), and the server instructions said `observe` takes an `action` (it takes a `query`).
- `observe` with `diff: true` reported `entities_modified: 0` beside every component change, because it counted only a change kind the diff never produces. It now counts the distinct entities with a component added, removed or modified.
- `observe`'s `find N entities with component Y` parsed N and ignored it, returning every match. It now returns at most N rows, and `metadata` adds `limit` and `matched` (how many matched before the cut). With `diff: true` the comparison covers every matched entity, not only the N returned, and `diff.compares` says so: the game answers in table order, so a diff of the cut alone would report an entity that only moved into the cut as spawned. `find 0 entities ...` is refused.

## 0.8.0 - 2026-10-30

Covers everything since 0.7.2. Breaking (semver-checks: 13 checks) because the unreachable `server::tool_orchestration` and `server::tools::orchestration` public modules were deleted; no MCP tool or parameter was removed except `detect_anomaly`'s `start_monitoring`/`stop_monitoring`.

### Removed

- `server::tool_orchestration` and `server::tools::orchestration` (about 1,170 lines): a pipeline/workflow executor with an unbounded-lifetime result cache and "auto-record" / "auto-experiment" hooks that only logged "placeholder". Nothing the binary runs constructed it and it backed no tool; the tool list is unchanged. `ToolOrchestrator`, `ToolContext`, `ToolExecutor`, `WorkflowDSL` and the rest were `pub` items of the library (breaking for code that used them).
- `detect_anomaly` actions `start_monitoring` and `stop_monitoring`, and `status`'s `is_monitoring`. They set a flag and reported "Continuous monitoring started" while nothing sampled the game. Use `detect` repeatedly, or `watch`. Asking for them now returns the "Invalid action" error listing `detect, configure, status`.

### Added

- `bevy_fairy/ring` (a built-in command, called with `run`): an always-on, bounded recording of a `world.query` result, one sample per frame, taken inside the game at the end of each frame. `start` (query, `frames` 1-600, optional `trigger` {JSON pointer, value} that stops recording on the first matching frame), `dump` (optionally the newest `last` N), `stop`, `status`. Every sample has its frame number and elapsed seconds; the reply says whether it is recording, how many samples were evicted and the trigger frame. The query defaults to `strict`, because `world.query` otherwise matches nothing for a component path it cannot find and the buffer would fill with empty arrays; a result over 256 KiB ends the recording with the reason. State only: pixels are `burst`'s job.
- `burst` tool: a reference frame plus 1-7 captures at 0.5-10 fps (default 4) as one contact sheet (2 rows x 4 columns at most). Each cell is
  labelled with the game frame the plugin reported for it and the milliseconds since the sequence began, so a sprite that stops
  moving or a flash is visible in one image; a cell with no frame reported is labelled `F-`. Saves `sheet.png` and `ref.png`
  (and every frame with `full_frames`). Fails, saving nothing, when any capture fails or the frame size changes. The server now
  depends on `image` (already a plugin dependency) for it.
- `screenshot` and `bevy_fairy/capture` report provenance: `source` (the one actually read, `offscreen` or `window`),
  `frame_requested` and `frame_captured` (the plugin's frame count, comparable with `status`'s `frame`). A game whose
  plugin predates this is reported as "source and frame unavailable" rather than shown without them. `Provenance` is a
  new public type and `CaptureState::Done` has a new field (breaking for code matching it).

### Fixed

- `experiment`: `rollback_available: true` on a `modify` or `delete` was a constant. Neither had a reverse action, so `undo` answered
  "No reverse action available" and had already dropped the entry from `history`. `rollback_available` is now true only when `undo`
  will run a recorded reverse: a `spawn` (deleted again) or a `modify` whose components were all readable beforehand (original values
  restored; they are now read by name, which the old empty-list request never returned). A `delete` cannot be put back and says
  `false`. A refused or failed `undo` keeps its history entry; undo/redo no longer add their own history entries; `redo` of a spawn
  records the new entity. An atomic `batch` that fails reports how many successful actions it could not reverse instead of claiming it
  rolled back.
- `hypothesis` (`test`): `entity_exists` and `component_equals` conditions always evaluated true without asking the game, so every such
  hypothesis was reported confirmed. They now query the game (`count` on `entity_exists` is an exact match; `component_equals` with no
  `entity_id` holds when any entity has the value; short names match the end of a registered path). `component_matches` is refused
  with an error, since the server has no predicate evaluator.
- `observe` semantic queries (`find memory leaks`, `find stuck entities`, ...) were refused after the unregistered-name check because they
  are built from short names (`Transform`). A short name that ends exactly one registered path is now rewritten and listed in
  `metadata.resolved_component_names`; unknown or ambiguous names are still refused.
- `observe` kept a 5-minute cache of every query and answered a repeat with the first read's rows under a fresh timestamp, so a
  moving object looked frozen. The cache is deleted (`QueryCache`, `QueryMetrics`, `QueryResult` and `cache_hit` go with it); metadata now
  has `read_at`. Check: `tests/fixture_ground_truth.rs::observe_reads_the_live_game_every_time_it_is_asked`.
- `observe` with a filter that names a component the game does not register (a short name such as the `Name` its own description
  advertised) returned `[]` and `entity_count: 0`; it is now an error that lists the registered paths ending in that name. The
  description's examples use full paths. `list components` reported its type count as `entity_count`; it is `component_type_count`.
  The semantic queries (`find stuck entities`, ...) name short, game-specific components and now refuse until they resolve full paths (B-065).
  Check: `observe_refuses_a_component_name_the_game_does_not_register`.
- `screenshot` answered a missing `width` or `height` from the plugin as `0x0`; it is now an error.
- `replay` advertised `action` (an open string), `checkpoint_id` (read by no handler) and `speed` (always sent as 1.0, so
  `set_speed` without a speed silently set 1x), while its 21 actions read 25 arguments the schema refused: `save` could never be
  given a `filename`, `seek` a `frame`, `add_modification` anything. `action` is now an enum, every argument its handlers read is
  advertised with the actions it belongs to, `direction` and `strategy` are enums, `checkpoint_id` is removed, and `set_speed`
  without `speed` is an error. `observe` loses `detailed` (nothing read it). Every advertised argument now has a description.
  Checks: `tests/tool_list.rs::replay_advertises_its_actions_and_every_argument_they_read`, `every_advertised_argument_has_a_description`.
- `hypothesis` advertised `variation_type` values `parameter` and `scenario` (and `parameter_name`, `parameter_values`) that the
  handler refused or never read, and hid the ones it runs. `variation_type` is now an enum of `none`, `random`, `grid`,
  `boundary`, `fuzz`, each with its own advertised fields; `parameter_name` and `parameter_values` are removed (nothing read
  them). `stress_test` no longer overwrites a caller's `iterations`, `variation_type`, `max_mutations` or
  `mutation_probability`. Check: `tests/tool_contract.rs::every_advertised_variation_type_runs`.
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
  | `hypothesis` | `hypothesis`, `confidence`, `context` | `action`, `description`, `success_condition`, `actions`, `variation_type`, `numeric_min`, `numeric_max`, `string_pool`, `iterations`, `timeout`, `seed`, `threshold` |
  | `detect_anomaly` | `detection_type`, `sensitivity`, `window` | `action`, `window_size`, `z_score_threshold`, `min_samples`, `entity_growth_threshold`, `min_severity` |

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
