# The tools, the methods and the settings

## The 14 MCP tools

One line each; [`../mcp-tools.md`](../mcp-tools.md) is the generated page with every argument,
default and refusal, written from the server's own `tools/list`.

| Tool | What it does |
|---|---|
| `status` | Whether the game is answering and ready: its frame, its instance, its command count, where captures come from, and the progress of a scene it is replaying. Call this first. |
| `brp` | Call any BRP method the game serves — `world.query`, `world.get_resources`, `registry.schema`, `rpc.discover` — and return its result. Watching methods are refused: they stream. |
| `observe` | Read entities with a short phrase query (`list all entities`, `show entity 4294967298`, `find 5 entities with component <path>`), optionally diffing against the previous read. |
| `watch` | Sample one method N times and report which values moved, which stopped, and on which frame. |
| `burst` | Capture a short run of frames as one labelled contact sheet. |
| `commands` | The commands the game registered, each with its params and a worked example. |
| `run` | Run one registered command by name. A mistyped name is answered with the nearest real one. |
| `experiment` | Spawn, modify, delete or batch entities, with `undo`, `redo` and `history`. |
| `hypothesis` | Test a claim against the running game over many iterations, putting the game back after each. |
| `detect_anomaly` | Score entity velocities, state inconsistencies and entity growth against thresholds you set. |
| `screenshot` | Capture a PNG from the offscreen target or the window, with a crop and a zoom. |
| `scene` | Save, show or clear the commands this session ran, as a replayable file. |
| `guide` | Post a script for the person at the keyboard, and read back what happened. |
| `quit` | Ask the game to exit cleanly. |

## BRP methods the plugin registers

| Method | Feature | What it does |
|---|---|---|
| `bevy_fairy/status` | `plugin` | Whether the game is ready, its frame, and what it can capture |
| `bevy_fairy/commands` | `plugin` | The commands the game registered, with params and examples |
| `bevy_fairy/input` | `plugin` | Inject keyboard, text, mouse, scroll and cursor input |
| `bevy_fairy/scene` | `plugin` | Show or clear the recorded commands |
| `bevy_fairy/time` | `plugin` | Pause the game clock or change its speed (above 0, at most 1000); the request is re-applied every frame and the reply says whether it `held`, and whether the engine cut the last frame to the clock's `max_delta` (`clamped_by_max_delta`) (a built-in command). A host that owns `Time<Virtual>` adds `.host_owns_clock()` and reads `ClockRequest` |
| `bevy_fairy/ring` | `plugin` | Record the newest N frames (1-600) of a `world.query` result inside the game, with an optional trigger that freezes the buffer on the frame it matches; `dump` returns the samples with frame numbers (a built-in command, called with `run`) |
| `bevy_fairy/quit` | `plugin` | Ask the game to exit |
| `bevy_fairy/ui_text` | `plugin-ui-text` | The `bevy_ui` text the player can read, with each node's rect (a built-in command, called with `run`) |
| `bevy_fairy/screenshot`, `bevy_fairy/capture` | `plugin-screenshot` | Capture a frame and fetch it as PNG |
| `bevy_fairy/guide`, `bevy_fairy/guide+watch` | `plugin-guide` | Post a guide script; stream its progress |

Bevy's own methods (`world.query`, `world.get_resources`, `registry.schema`, ...) work too.

## Environment variables

| Variable | Read by | Default | Meaning |
|---|---|---|---|
| `BEVY_BRP_HOST` | server | `localhost` | Where the game's BRP endpoint is |
| `BEVY_BRP_PORT` | server | `15702` | Its port |
| `BEVY_FAIRY_CAPTURE_DIR` | server | `./debug_captures` | Where `screenshot` saves PNGs |
| `RUST_LOG` | server | `info` | Log level; logs go to stderr, stdout carries MCP |
| `BEVY_FAIRY_SCENE` | game | unset | A saved scene to replay at startup |

## Examples

None of these need a window or a GPU unless noted. Each prints what happened.

| Command | Shows |
|---|---|
| `cargo run --features plugin --example plugin_injected_input_lands` | An injected key is `just_pressed` for exactly one frame |
| `cargo run --features plugin --example plugin_typed_text_lands` | Typed text reaches a text field |
| `cargo run --features plugin --example plugin_cursor_drag_lands` | A click-drag lands where it was aimed |
| `cargo run --features plugin-guide --example plugin_guided_steps_land` | A three-step guide script, driven to completion |
| `cargo run --features plugin-screenshot --example plugin_agent_scene` | A small game with a command and an offscreen capture target (opens a window) |
| `cargo run --features plugin --example fixture_game` | The headless game the tests and evals check the tools against |
| `cargo run --features plugin-screenshot --example demo_game` | The fixture game rendered offscreen, headless; the tour GIF is recorded from it |

## Development

```sh
cargo test --workspace                                                    # everything
cargo test --no-default-features --features plugin --test plugin_leaf     # the no-OS-access check
UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list                       # regenerate docs/mcp-tools.md
```
