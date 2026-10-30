# bevy_fairy

**Let a coding agent look inside a running Bevy game, drive it, and ask the person playing it for
help — without touching your desktop.**

[![crates.io](https://img.shields.io/crates/v/bevy_fairy.svg)](https://crates.io/crates/bevy_fairy)
[![docs.rs](https://img.shields.io/docsrs/bevy_fairy)](https://docs.rs/bevy_fairy)
[![Bevy 0.20](https://img.shields.io/badge/bevy-0.20-purple.svg)](https://bevyengine.org)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

`bevy_fairy` is two things in one crate:

- **a Bevy plugin** your game links, which adds debugging methods to the
  [Bevy Remote Protocol](https://docs.rs/bevy_remote) (BRP);
- **an [MCP](https://modelcontextprotocol.io) server**, `bevy-fairy`, which gives an agent such as
  Claude Code tools to read the game's state, run the game's own commands, capture what it shows,
  and inject input.

```text
coding agent ──MCP over stdio──▶ bevy-fairy ──BRP over HTTP :15702──▶ your game + DebuggerPlugin
                                                                            │
                                                        guide card on screen ▼
                                                              the person at the keyboard
```

> [!NOTE]
> This crate was written by an AI agent working from a human's direction. It is tested against a
> fixture game and used against a real one, but it has had no line-by-line human audit.

## See it work

**A scripted tour.** Every reply is the real server's output; every picture is the game frame its
`screenshot` tool returned. The game runs at 10 fps here so the planted bug — the spinner stops at
frame 120 — happens mid-tour.

![MCP tool calls beside the game frames they captured](https://raw.githubusercontent.com/Ladvien/bevy_fairy/main/docs/media/mcp-tour.gif)

**A real agent session.** Claude, with no tools but these, told that something in the game which
should spin forever stops, and asked what it is, which of its values stops changing, and on what
frame. The script runs that question as the `spinner_stops` eval up to three times and draws the
first session that passes; it never draws one that failed.

![An agent finding the frame the spinner stops on](https://raw.githubusercontent.com/Ladvien/bevy_fairy/main/docs/media/agent-session.gif)

Both are recorded from [`examples/demo_game.rs`](https://github.com/Ladvien/bevy_fairy/blob/main/examples/demo_game.rs)
by `python3 scripts/record_demo.py all`.

## Why

An agent that debugs a game by reading source code is guessing. It needs to see the game's actual
state, change it, and check what happened. The obvious way to do that, OS screenshots and OS
input, needs the game window in front: on the game this was built for, a capture with the window
focused had 7,188 distinct colours, and with another app in front it had **one**. Synthetic OS
keystrokes go to whichever window has focus, which is often the one you are typing in.

So nothing here touches the operating system. Captures come from the game's own pixels, input is
written into Bevy's own input streams, and `tests/plugin_leaf.rs` fails the build if the plugin's
dependency graph ever gains something that could synthesise an OS event.

## What an agent can do with it

| | How |
|---|---|
| **Read state** | Query any entity, component or resource over BRP (`brp`, `observe`). |
| **Run game actions** | Call commands your game registers, each with typed params and a worked example (`commands`, `run`). |
| **See the screen** | Capture a PNG from an offscreen render target, or from the game window (`screenshot`). |
| **Press keys, type, click, drag** | Inject keyboard, text, mouse, scroll and cursor input that the game reads like the real thing. |
| **Ask a person** | Show one instruction at a time on the player's screen and watch for the state it asked for (`guide`). |
| **Record and replay** | Save the commands a session ran and replay them when the game starts (`scene`). |

It cannot see frame time, CPU or memory: BRP does not report them, and the tools say so rather than
make up a number.

## Quick start

This takes a game you already have and an agent that speaks MCP, such as Claude Code.

**1. Add the plugin to your game.**

```toml
# Cargo.toml
[dependencies]
bevy = "0.20"
bevy_fairy = { version = "0.4", default-features = false, features = ["plugin"] }
```

```rust
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(DebuggerPlugin::default()) // registers the bevy_fairy/* methods
        .add_plugins(RemoteHttpPlugin::default()) // serves them on port 15702
        .run();
}
```

Run the game, then check it answers:

```sh
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"bevy_fairy/status"}'
# {"jsonrpc":"2.0","id":1,"result":{..., "ready":true, ...}}
```

**2. Install the server.**

```sh
cargo install bevy_fairy --no-default-features --features server
bevy-fairy --version
# bevy-fairy 0.4.0
```

**3. Register it with your agent.** For Claude Code, from your project directory:

```sh
claude mcp add bevy-fairy -- bevy-fairy --stdio
```

For Claude Desktop or a project `.mcp.json`:

```json
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "bevy-fairy",
      "args": ["--stdio"],
      "env": { "BEVY_BRP_HOST": "localhost", "BEVY_BRP_PORT": "15702" }
    }
  }
}
```

**4. Try it.** With the game running, ask the agent: *"Call `status`, then list the game's resources
with `brp`."* It should report `ready: true` and a list of resource type names. From there, ask it
questions about your game.

### If it doesn't work

| Symptom | Cause and fix |
|---|---|
| `status` says no game is answering | The game is not running, or `BEVY_BRP_HOST`/`BEVY_BRP_PORT` do not point at it. The default is `localhost:15702`. |
| The game panics on start | You added a second `RemotePlugin`. `DebuggerPlugin` adds its own; add only a transport such as `RemoteHttpPlugin`. Or `InputPlugin` is missing: `DefaultPlugins` has it, `MinimalPlugins` does not. |
| The agent sees no `bevy-fairy` tools | The client cannot find the binary. Run `bevy-fairy --version`; if that works, give `command` as the full path (`~/.cargo/bin/bevy-fairy`) and restart the client. |
| `screenshot` says `bevy_fairy/screenshot` is not registered | Build the game with the `plugin-screenshot` feature. |
| A `+watch` method is refused by `brp` | Watching methods stream; `brp` returns one answer per call. Call the method without `+watch` again. |

## Guides

### Let the agent run your game's actions

Register commands with typed params and a worked example. The agent finds them with `commands` and
runs them with `run`; a mistyped name is answered with the nearest real one, and wrong params are
refused before your handler runs.

```rust
use bevy_fairy::{DebugCommandInfo, DebugCommandsAppExt};
use serde_json::{json, Value};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnCube { at: [f32; 3] }

fn spawn_cube(In(p): In<SpawnCube>, mut commands: Commands) -> Result<Value, String> {
    commands.spawn(Transform::from_translation(Vec3::from(p.at)));
    Ok(json!({ "spawned_at": p.at }))
}

// After .add_plugins(DebuggerPlugin::default()):
app.add_debug_command(
    DebugCommandInfo::new("demo/spawn_cube", "Spawn a cube at a world position in metres")
        .params("at: [x, y, z]")
        .example(json!({ "at": [0.0, 0.5, 0.0] })),
    spawn_cube,
);
```

Registration is checked at startup: a reserved name (`bevy_fairy/`, `world.`, ...), a duplicate,
or an example that does not parse as the params type panics then, not when the agent calls it.
If your game needs time to load, build the plugin with `DebuggerPlugin::default().host_signals_ready()`
and set the `DebugReady` resource to `true` when it is; until then every command answers "not
ready yet".

### Capture what the game shows

Enable `plugin-screenshot`, then give the plugin an image one of your cameras renders to:

```rust
use bevy_fairy::DebugCaptureTarget;

commands.insert_resource(DebugCaptureTarget { image: render_target_handle });
```

With a `DebugCaptureTarget`, captures come from that image and never depend on the window. Without
one, `screenshot` falls back to the game window. That needs nothing from the OS, but a covered or
minimised window can give a stale frame. A full example with the camera setup is
[`examples/plugin_agent_scene.rs`](https://github.com/Ladvien/bevy_fairy/blob/main/examples/plugin_agent_scene.rs).

### Inject input

The agent sends input through `brp` with the method `bevy_fairy/input`:

```jsonc
{"kind": "Keyboard", "key": "Space"}                      // a tap: pressed one frame, released the next
{"kind": "Keyboard", "text": "site_67"}                   // typed into a focused text field
{"kind": "Cursor", "x": 640, "y": 360}                    // logical window pixels (see below)
{"kind": "Mouse", "button": "Left", "action": "Press"}
{"kind": "Cursor", "clear": true}                         // hand the pointer back to the real mouse
```

Input is written into Bevy's own input messages before its input systems run, so `just_pressed`,
text fields and `ButtonInput` all see it like a real key.

**The pointer reaches picking.** `Cursor` and `Mouse` also write the `WindowEvent` messages a real
mouse produces, so `bevy_ui` buttons, `Interaction` and `On<Pointer<Click | Over | Out | Drag..>>`
observers fire with no code on your side. A press at one position, a `Cursor` move and a release
elsewhere is a drag. A click lands at the last `Cursor` position; with none set the reply carries a
`note` saying so.

**Coordinates.** `x`/`y` are *logical window pixels*, top-left origin, the same space
`Window::cursor_position` reports, unless the host inserted a `DebugCaptureTarget` (feature
`plugin-screenshot`). Then the pointer is routed to that image instead of the window, because a camera
that renders to an image is not hit by a window pointer, and `x`/`y` are pixels of that image. One
route per game: a host with both a window UI and a capture target is driven through the image.

A host that reads the pointer itself rather than through picking has one line to write: **read it through
`bevy_fairy::cursor_position(&window, &debug_cursor)`**, not `Window::cursor_position`. The plugin never
moves the real pointer, because Bevy would move the physical mouse out from under whoever is at the
machine.

### Guide a person through a test

Enable `plugin-guide`. The agent posts a script with `guide`; your game shows one step at a time:

```json
{"steps": [{
  "label": "drop a floor and a wall",
  "goal": "a tile needs two pieces before the solver can match",
  "do": ["walk the library with up and down", "press Enter to bring a row in"],
  "checkpoint": "tile has two members",
  "recovery": "if nothing lands, the filter is hiding every row: press Backspace"
}]}
```

A checkpoint is a condition your game registers by name, so the plugin never needs to know your
game's vocabulary:

```rust
use bevy_fairy::Checkpoints;

let has_two = app.register_system(|_: In<serde_json::Value>, tile: Res<Tile>| tile.members >= 2);
app.world_mut().resource_mut::<Checkpoints>().register("tile has two members", has_two);
```

A step without a checkpoint waits for the person to say it is done. For each step the agent gets
back how many times it came up, how many of those its checkpoint passed, and how long it took.

### Save and replay a session

Every game command the agent runs is recorded. `scene save <path>` writes them to a file;
starting the game with `BEVY_FAIRY_SCENE=<path>` replays them, one step per frame, once the game
reports ready. Scenes record commands, not raw input.

## Reference

- **Tools:** 13 MCP tools. [`docs/mcp-tools.md`](https://github.com/Ladvien/bevy_fairy/blob/main/docs/mcp-tools.md)
  has every description and argument, generated from the server's own `tools/list`.
- **API:** the plugin's types are documented on [docs.rs](https://docs.rs/bevy_fairy).

### BRP methods the plugin registers

| Method | Feature | What it does |
|---|---|---|
| `bevy_fairy/status` | `plugin` | Whether the game is ready, its frame, and what it can capture |
| `bevy_fairy/commands` | `plugin` | The commands the game registered, with params and examples |
| `bevy_fairy/input` | `plugin` | Inject keyboard, text, mouse, scroll and cursor input |
| `bevy_fairy/scene` | `plugin` | Show or clear the recorded commands |
| `bevy_fairy/time` | `plugin` | Pause the game clock or change its speed; the request is re-applied every frame and the reply says whether it `held` (a built-in command). A host that owns `Time<Virtual>` adds `.host_owns_clock()` and reads `ClockRequest` |
| `bevy_fairy/quit` | `plugin` | Ask the game to exit |
| `bevy_fairy/ui_text` | `plugin-ui-text` | The `bevy_ui` text the player can read, with each node's rect (a built-in command, called with `run`) |
| `bevy_fairy/screenshot`, `bevy_fairy/capture` | `plugin-screenshot` | Capture a frame and fetch it as PNG |
| `bevy_fairy/guide`, `bevy_fairy/guide+watch` | `plugin-guide` | Post a guide script; stream its progress |

Bevy's own methods (`world.query`, `world.get_resources`, `registry.schema`, ...) work too.

### Cargo features

| Feature | Use it for | Turns on |
|---|---|---|
| `server` (default) | Building the `bevy-fairy` binary | The MCP server and its dependencies |
| `plugin` | Your game | The plugin; four direct dependencies |
| `plugin-screenshot` | Your game, with capture | `plugin` + Bevy's renderer |
| `plugin-guide` | Your game, with the guide overlay | `plugin` + Bevy UI and picking |
| `plugin-ui-text` | Your game, if an agent should read its HUD text | `plugin` + Bevy UI |

A game should depend on `bevy_fairy` with `default-features = false`, or it compiles the server too.

### Environment variables

| Variable | Read by | Default | Meaning |
|---|---|---|---|
| `BEVY_BRP_HOST` | server | `localhost` | Where the game's BRP endpoint is |
| `BEVY_BRP_PORT` | server | `15702` | Its port |
| `BEVY_FAIRY_CAPTURE_DIR` | server | `./debug_captures` | Where `screenshot` saves PNGs |
| `RUST_LOG` | server | `info` | Log level; logs go to stderr, stdout carries MCP |
| `BEVY_FAIRY_SCENE` | game | unset | A saved scene to replay at startup |

### Examples

None of these need a window or a GPU unless noted. Each prints what happened.

| Command | Shows |
|---|---|
| `cargo run --features plugin --example plugin_injected_input_lands` | An injected key is `just_pressed` for exactly one frame |
| `cargo run --features plugin --example plugin_typed_text_lands` | Typed text reaches a text field |
| `cargo run --features plugin --example plugin_cursor_drag_lands` | A click-drag lands where it was aimed |
| `cargo run --features plugin-guide --example plugin_guided_steps_land` | A three-step guide script, driven to completion |
| `cargo run --features plugin-screenshot --example plugin_agent_scene` | A small game with a command and an offscreen capture target (opens a window) |
| `cargo run --features plugin --example fixture_game` | The headless game the tests and evals check the tools against |
| `cargo run --features plugin-screenshot --example demo_game` | The fixture game rendered offscreen, headless; the GIFs above are recorded from it |

## Development

```sh
cargo test --workspace                                                    # everything
cargo test --no-default-features --features plugin --test plugin_leaf     # the no-OS-access check
UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list                       # regenerate docs/mcp-tools.md
```

Tool behaviour is tested against `examples/fixture_game.rs`, whose state is known from its own
constants. [`evals/`](https://github.com/Ladvien/bevy_fairy/blob/main/evals/README.md) measures
whether an agent with only this server can answer debugging questions about that game.
[`CLAUDE.md`](https://github.com/Ladvien/bevy_fairy/blob/main/CLAUDE.md) explains the design
decisions that are easy to undo by accident.

Issues and pull requests are welcome at [github.com/Ladvien/bevy_fairy](https://github.com/Ladvien/bevy_fairy/issues).

## Changelog

See [`CHANGELOG.md`](https://github.com/Ladvien/bevy_fairy/blob/main/CHANGELOG.md).

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
