# Adding the plugin to a game

The setup is one plugin. It owns the BRP `RemotePlugin` and registers every `bevy_fairy/*` method,
so there is nothing else to wire up.

- Bevy 0.20
- Rust 1.95 or newer
- `InputPlugin` in the app: `DefaultPlugins` has it, `MinimalPlugins` does not

## Add the dependency

A game takes the plugin with the server off, so it compiles none of the server's tree:

```toml
# Cargo.toml
[dependencies]
bevy = { version = "0.20", features = ["default"] }
bevy_fairy = { version = "0.12", default-features = false, features = ["plugin"] }
```

## Add the plugin

```rust,no_run
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        // Owns the BRP RemotePlugin. Never add a second RemotePlugin next to it.
        .add_plugins(DebuggerPlugin::default())
        .add_plugins(RemoteHttpPlugin::default())
        .add_systems(Startup, setup)
        .add_systems(Update, game_systems)
        .run();
}
```

**Do not add `RemotePlugin` yourself.** `DebuggerPlugin` adds it, and Bevy rejects a duplicate
plugin by name — a game that adds it twice panics on start. Add a transport such as
`RemoteHttpPlugin` and nothing else.

## Check it answers

```sh
cargo run
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"bevy_fairy/status"}'
```

The reply carries `ready: true`, the current frame, and what the game can capture.

## Port and address

`RemoteHttpPlugin` defaults to `127.0.0.1:15702`. To move it:

```rust,no_run
use bevy::remote::http::RemoteHttpPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(DebuggerPlugin::default())
        .add_plugins(RemoteHttpPlugin::default().with_port(15703))
        .run();
}
```

```sh
export BEVY_BRP_PORT=15703
```

`with_address("0.0.0.0")` binds every interface. Only do that on a network you trust: BRP can
spawn and delete entities, and nothing on the path authenticates the caller.

## Only in debug builds

```rust,no_run
fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);

    #[cfg(debug_assertions)]
    app.add_plugins(DebuggerPlugin::default())
        .add_plugins(RemoteHttpPlugin::default());

    app.run();
}
```

The same shape takes an environment variable: add the two plugins only when
`std::env::var("BEVY_DEBUG").is_ok()`, then run with `BEVY_DEBUG=1 cargo run`.

## Features

| Feature | Use it for | Turns on |
|---|---|---|
| `server` (default) | Building the `bevy-fairy` binary | The MCP server and its dependencies |
| `plugin` | Your game | The plugin; four direct dependencies |
| `plugin-screenshot` | Your game, with capture | `plugin` + Bevy's renderer |
| `plugin-guide` | Your game, with the guide overlay | `plugin` + Bevy UI, picking and a font |
| `plugin-ui-text` | Your game, if an agent should read its HUD text | `plugin` + Bevy UI |

Each `plugin-*` feature turns on the BRP methods listed in
[tools.md](tools.md#brp-methods-the-plugin-registers).

## A host that loads for a while

If the game needs time to load, build the plugin with
`DebuggerPlugin::default().host_signals_ready()` and set the `DebugReady` resource to `true` when it
is; until then every command answers "not ready yet" instead of failing halfway.

A host that owns `Time<Virtual>` (its own pause or slow motion) adds `.host_owns_clock()` and reads
`ClockRequest`; see [`bevy_fairy/time`](tools.md#brp-methods-the-plugin-registers).
