# MCP tools

Generated from the server's `tools/list` by `tests/tool_list.rs`; do not edit by hand.
Regenerate with `UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list`.

## `brp`

Call any Bevy Remote Protocol method the game serves, for example world.query or registry.schema, and return its result. Use `run` for the game's own commands. Watching methods (names ending in +watch) are refused: they stream, and this returns one answer per call.

| Argument | Type | Required | Description |
|---|---|---|---|
| `method` | string | yes | The BRP method, for example `world.query` or `registry.schema`. |
| `params` | any | no | The method's params as JSON, omitted for a method that takes none. Shapes, one per line: - `world.query`: `{"data":{"components":["bevy_ecs::name::Name"]},"filter":{"with":[],"without":[]}}` (full type paths, never short names: take the game's own from `world.list_components`; `filter` is optional) - `registry.schema`: `{"with_crates":["bevy_transform"],"type_limit":{"with":["Component"]}}` (every field optional and a list; omit one rather than sending null) - `world.list_components`: no params for every registered component, or `{"entity":4294967296}` for one entity - `rpc.discover`: no params; lists every method the game serves with its param schema |

## `burst`

Capture a short sequence of frames and return it as one contact sheet (a PNG, at most 2 rows x 4 columns), so a change over time can be seen: a sprite that stops moving, a flash, a transition. The first cell is a reference frame taken just before the sequence (labelled REF); the rest are `frames` captures at `fps` (default 4, 0.5-10), 1-7 frames. Every cell is labelled with the game frame the capture was delivered on (F<frame>) and the milliseconds since the reference was requested (+<ms>MS), both as reported, so two cells with the same F<frame> are the same game frame (the game is slower than `fps`). Each frame needs a round trip to the game, so the real spacing can exceed 1/fps: read the labels, not the request. Saves sheet.png and ref.png in a new directory (or full-resolution frame-<n>.png files too with full_frames true) and returns their paths. The default offscreen source leaves out UI; source 'window' includes it. Fails, saving nothing, when any capture fails or the frame size changes mid-sequence.

| Argument | Type | Required | Description |
|---|---|---|---|
| `fps` | number | no | Requested captures per second, 0.5 to 10. Default 4. The real spacing is in the labels. |
| `frames` | integer | no | Frames to capture after the reference, 1 to 7. Default 4. |
| `full_frames` | boolean | no | Also save every frame at full resolution as `frame-<n>.png`. Default false. |
| `inline` | boolean | no | Also return the contact sheet inline, so the agent sees it in the reply. |
| `save_dir` | string | no | Directory to write sheet.png and ref.png into. Default: a new `burst-<unix_ms>` directory under `$BEVY_FAIRY_CAPTURE_DIR`, or `./debug_captures/` when that variable is unset. |
| `source` | string | no | `offscreen` (the default) renders the game's own image and leaves out UI. `window` captures the game window, UI included. |

## `commands`

List the commands the game registered, each with its parameters and a working example. Call this before `run`. The built-in bevy_fairy/time is listed last.

No arguments.

## `detect_anomaly`

Look for anomalies in the game's entities. `action` picks the operation: `detect` reads every entity once and runs three detectors, `configure` sets the thresholds below, `status` lists what each detector reads. The detectors read only particular components, matched on the last segment of the type path: the linear vector of a component named Velocity (compared with that entity's own recent speeds), the current field of Health beside a boolean Alive, and the number of entities across `detect` calls. Speeds and the entity count are judged only after `min_samples` readings, kept between calls, so the first calls judge nothing. `detect` returns a list of detectors saying, for each, how many entities it read and whether it could judge them, because an empty anomalies list from a detector that read nothing says nothing about the game. Severity is how far past its threshold a reading is, from 0 to 1. Like `observe`, the reply says which game frames it was read between. A detection is a lead to check, not a verdict.

| Argument | Type | Required | Description |
|---|---|---|---|
| `action` | string | yes | The operation to run. One of `detect`, `configure`, `status`. |
| `entity_growth_threshold` | number | no | The entities added per `detect` call (measured across the window) above which the entity count counts as growing abnormally. |
| `limit` | integer | no | How many anomalies to report at most. |
| `min_samples` | integer | no | The fewest samples needed before a verdict is possible; at most `window_size`. |
| `min_severity` | number | no | The lowest severity to report, from 0 to 1. Severity is how far past its threshold a reading is: 0.5 at twice the threshold. |
| `window_size` | integer | no | How many samples the window covers. At least `min_samples` and at least 2: a detector keeps this many samples and cannot report before it has `min_samples` of them, so `configure` refuses a smaller window. |
| `z_score_threshold` | number | no | How many standard deviations from its own recent speeds an entity's velocity must be to count as anomalous. |

## `experiment`

Run controlled experiments on your Bevy game: spawn, modify or delete entities and read back what changed. `action` picks the operation; `execute` carries the `actions` to run.

| Argument | Type | Required | Description |
|---|---|---|---|
| `action` | string | yes | The operation to run. One of `execute`, `undo`, `redo`, `history`, `clear_history`, `archetypes`. |
| `actions` | array | no | The actions to carry out. Required by `execute`; ignored by the other operations. |

## `guide`

Guide the HUMAN through something in the running app, and get back what actually happened. This is the only tool here that talks to the person rather than the game: you post a short script, the app draws ONE step at a time over their own window, and it advances itself when a named condition arrives. Use it to walk somebody through reproducing a bug, to hand over an acceptance test, or any time you would otherwise type a numbered list into the chat and hope they map it onto an interface you cannot see.

HOW TO COLLABORATE (this part is not optional -- getting it wrong strands them):
1. Post steps, then TELL THEM IN CHAT that the guide is up and to look at the app window. They will not know otherwise; nothing pops up to get their attention.
2. Poll with {read:true}. When it answers waiting_on_a_person:true, that step has NO machine check and will never advance on its own. STOP AND ASK THEM IN CHAT, in words, naming the step. The card tells them 'tell the agent' -- that promise is yours to keep. Only after they answer, call {skip:true}.
3. When they say a step made no sense or did not work, that is DATA, not a detour. Record it and fix the step or the app. The transcript is k/n per step -- passes over attempts -- because a boolean 'it worked' is the thing people get wrong.
4. Never post a step whose checkpoint the app has not registered. The app refuses it by name and lists what would have worked, but the person is already standing there by then.

Write steps the way you would brief a colleague: say WHY before what (people ignore a suggestion whose reason they cannot see), two to four short imperatives, and always fill in recovery -- what to do when the step does not work is the field that makes this better than a list in a chat window.

| Argument | Type | Required | Description |
|---|---|---|---|
| `clear` | boolean | no | Take the script down. The transcript survives, because the transcript is the result. |
| `read` | boolean | no | Return the transcript and where the person is, changing nothing. Safe to call at any time, and the way to find out whether the current step is waiting on them. |
| `skip` | boolean | no | Move past the current step WITHOUT its condition passing. This is how a step only a person can judge gets advanced -- after they have told you, never before. |
| `steps` | array | no | The steps to post, replacing whatever is up. Each step is an object: `label` (required), `goal`, `do` (array of 2-4 short imperatives), `checkpoint` (a condition name the APP registered, or null when only a person can judge it), and `recovery` (what to do when it does not work). |
| `visible` | boolean | no | Show or hide the card without discarding the script. |

## `hypothesis`

Test a claim about your game against the running game. `action` picks the operation; `test` needs a `description` of the claim and a `success_condition` that decides it.

| Argument | Type | Required | Description |
|---|---|---|---|
| `action` | string | yes | The operation to run. One of `test`, `quick_test` (10 iterations, 10 s), `stress_test` (fuzz variation, 1000 iterations unless `iterations` or `variation_type` say otherwise), `validate` (checks the claim parses; runs nothing). |
| `actions` | array | no | The actions to try while testing the claim. |
| `bool_probability` | number | no | For `random`: the probability a varied boolean is true. Default 0.5. |
| `bool_values` | array | no | For `grid`: the booleans to try. Default true and false. |
| `description` | string | no | What the claim is, in words. `test` and `quick_test` require it. |
| `include_empty` | boolean | no | For `boundary`: try empty strings and collections. Default true. |
| `include_max` | boolean | no | For `boundary`: try the largest values. Default true. |
| `include_negative` | boolean | no | For `boundary`: try negative values. Default true. |
| `include_zero` | boolean | no | For `boundary`: try zero. Default true. |
| `iterations` | integer | no | How many times to run the test. |
| `max_mutations` | integer | no | For `fuzz`: the most mutations applied to one run. Default 3. |
| `mutation_probability` | number | no | For `fuzz`: the probability each mutation applies. Default 0.5. |
| `numeric_max` | number | no | For `random`: the high end of the numeric range to vary over. Default 100. |
| `numeric_min` | number | no | For `random`: the low end of the numeric range to vary over. Default 0. |
| `numeric_values` | array | no | For `grid`: the numbers to try. Default 0, 50, 100. |
| `seed` | integer | no | The seed, so a run can be repeated exactly. |
| `string_pool` | array | no | For `random`: the strings to vary over. Default none. |
| `string_values` | array | no | For `grid`: the strings to try. Default `default`. |
| `success_condition` | any | no | The condition that decides whether the claim held. `test` and `quick_test` require it. |
| `threshold` | number | no | The confidence a result needs before it counts as confirmed. |
| `timeout` | integer | no | How long the test may run, in seconds. |
| `variation_type` | any | no | How to vary the test between iterations. Default `none`. `random` varies within `numeric_min`..`numeric_max`, `string_pool` and `bool_probability`; `grid` walks `numeric_values`, `string_values` and `bool_values`; `boundary` tries edge values chosen by the `include_*` flags; `fuzz` mutates the actions up to `max_mutations` times, each with probability `mutation_probability`. |

## `observe`

Read the game's entities with a short phrase query and return them with their component values: every entity, one entity by id, the entities with or without a component, or the registered component types. `diff` compares a read with the previous `diff` read of the same query. The reply says which game frames the read lies between ("frame" to "frame_until", the counter before and after; the game serves each request on a frame of its own), or "frame_unavailable" with the reason when the game does not serve the plugin's status method. Entities only: for a resource use `brp` with world.get_resources, and for a component's fields and their types use `brp` with registry.schema.

| Argument | Type | Required | Description |
|---|---|---|---|
| `diff` | boolean | no | Compare this read with the previous `diff: true` read of the same query in this server session. The first such read is the baseline and says so. With find N entities the diff compares every matched entity, not only the N rows returned. |
| `query` | string | yes | A phrase in one of the shapes the parser accepts; a bare word such as "score" or "crate" is refused. Shapes, each a complete example: `list all entities`, `list components`, `show entity 4294967298`, `find entities with component bevy_ecs::name::Name`, `find entities without component bevy_ecs::name::Name`, `find 5 entities with component bevy_ecs::name::Name`. Component names are full type paths, and a path may hold digits and generic arguments: `find entities with component bevy_mesh::components::Mesh3d`, `find entities with component bevy_time::time::Time<bevy_time::real::Real>`. In a list only the commas outside angle brackets separate components. A name the game does not register is an error that lists the registered paths ending in it. The metadata carries entity_count (component_type_count for the list-components query) and read_at; find N entities returns at most N rows and adds limit and matched, the number that matched before the cut. For any other read, such as one resource, use `brp`. |

## `quit`

Ask the game to quit cleanly.

No arguments.

## `run`

Run one command the game registered, by name, with its parameters. Waits up to 120 s for the game to be ready. An unknown name is refused with the closest known one. Every command that runs is recorded, so `scene save` can write the run out as a replayable file.

| Argument | Type | Required | Description |
|---|---|---|---|
| `command` | string | yes | The command name, `<game>/<verb>`, exactly as `commands` lists it. |
| `params` | any | no | The command's parameters as a JSON object. Omit it for a command that takes none. |

## `scene`

Work with the commands recorded in this game session. save writes them to a replayable scene file on this machine (path required); show returns them; clear forgets them.

| Argument | Type | Required | Description |
|---|---|---|---|
| `action` | any | yes | `save`, `show` or `clear`. |
| `path` | string | no | Required for `save`: the file to write, on this machine. |

## `screenshot`

Capture the game to a PNG file on this machine and return its path, size, the source actually read (offscreen or window) and the game frames the request and the delivery happened on (compare with `status`'s frame to judge freshness). The default offscreen capture leaves out UI; source 'window' captures the game window with its UI included. Pass inline true to also get the image in the reply.

| Argument | Type | Required | Description |
|---|---|---|---|
| `inline` | boolean | no | Also return the PNG inline, so the agent sees the image as well as the saved file. |
| `region` | any | no | Crop to this rectangle before saving. |
| `save_to` | string | no | File to write on this machine. Default: `$BEVY_FAIRY_CAPTURE_DIR/capture-<unix_ms>.png`, or `./debug_captures/` when that variable is unset. |
| `source` | string | no | `offscreen` (the default) renders the game's own image and leaves out UI. `window` captures the game window, UI included. |
| `zoom` | number | no | Magnify the captured image by this factor. Default 1. |

## `status`

Report whether the game is answering and ready for commands: its frame, how many commands it registered, where captures come from, and the progress of a scene file the game is replaying (null when none). The reply also carries the URL this server talks to. Call this first.

No arguments.

## `watch`

Sample one BRP method repeatedly and return the series, each sample stamped with the game frames it was read between ("frame" to "frame_until", usually one apart or equal) and the milliseconds since the first sample. Use it for anything that happens over time: a value that stops changing, a position's speed, a counter that drifts. The reply says whether the value changed, how many distinct values were seen, the frame of the first change, and "constant_since_frame" (the frame from which the last value never changed, null when it changed at the last sample). Samples are polled over HTTP at `interval_ms` (20-5000), so changes between samples are not seen; 2-100 samples, 30 s in total at most. Pass `pointer` (JSON pointer into each result) to keep samples small; a pointer that matches nothing is an error. Streaming `+watch` methods are refused.

| Argument | Type | Required | Description |
|---|---|---|---|
| `interval_ms` | integer | no | Milliseconds between samples, 20 to 5000. Default 100. `samples` x `interval_ms` may not exceed 30000. |
| `method` | string | yes | The one-shot BRP method to sample, for example `world.query` or `world.get_resources`. Names ending in `+watch` are refused. |
| `params` | any | no | The method's params, exactly as for `brp`. |
| `pointer` | string | no | A JSON pointer (RFC 6901) into each result, for example `/0/components/my_game::Health/current`. Without it each sample carries the whole result, so use it for anything larger than a few values. A pointer that matches nothing is an error. |
| `samples` | integer | no | How many samples to take, 2 to 100. Default 10. |
