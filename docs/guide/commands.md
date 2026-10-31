# Let the agent run your game's actions

Register commands with typed params and a worked example. The agent finds them with `commands` and
runs them with `run`; a mistyped name is answered with the nearest real one, and wrong params are
refused before your handler runs.

```rust,no_run
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

Registration is checked at startup: a reserved name (one beginning `bevy_fairy/` or `world.`), a
duplicate, or an example that does not parse as the params type panics then, not when the agent
calls it.

`serde(deny_unknown_fields)` is worth the line: without it a typo in the agent's params becomes a
silent default, and the agent learns a shape that is not the real one.

If your game needs time to load, build the plugin with `DebuggerPlugin::default().host_signals_ready()`
and set the `DebugReady` resource to `true` when it is; until then every command answers "not
ready yet".

## Recording and replay

Every command that changes the game is recorded, so `scene save` can write the run out as a file
the game replays at startup. A command registered read-only (`bevy_fairy/ring` and
`bevy_fairy/ui_text` are) is not recorded, because replaying it would change nothing. See
[scene.md](scene.md).
