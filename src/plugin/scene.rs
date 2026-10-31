//! Scenes: the successful debug-command calls, kept as a list and replayable from a file.
//!
//! # Why a scene is a list of calls
//!
//! A scene is what an agent did to the game to get a state worth inspecting. Storing the calls, not
//! a snapshot of the world, keeps the file small and readable, and it replays the same code paths the
//! agent used. Ostrowski & Aroudj (GSTF Journal of Computing 3:10, 2013, DOI
//! 10.7603/s40601-013-0010-4) build automated game regression tests the same way: the game is driven
//! by function calls it processes over a network connection, possibly from another machine, with no
//! shared filesystem. Here the calls arrive over BRP and the file is read on the game's own machine.
//!
//! # The file
//!
//! `{"format": "bevy_fairy_scene", "version": 1, "steps": [{"method": "...", "params": {...}}, ...]}`.
//! A file whose format or version differs is refused with one `error!` and nothing is replayed. A bad
//! file never panics.
//!
//! # What a scene does not record
//!
//! Only calls to registered debug commands are recorded. `bevy_fairy/input` (keys, text, cursor,
//! buttons) is a built-in method, not a command, so a keypress or click an agent injected is **not**
//! in the file and a replay will not repeat it. Replay is also command-level, not frame-exact: Bevy
//! is not deterministic by default, so the same calls on a later run can land on a different world.
//! Drive input after the replay, or register the action as a command.
//!
//! # Replay
//!
//! Set `BEVY_FAIRY_SCENE` to a file path before the game starts. Steps run one per frame, and only
//! while [`DebugReady`](crate::DebugReady) is true, so a scene cannot run into a half-built world. A
//! step that names no recorded command, or whose command fails, is counted in `failed` and replay
//! goes on. Replayed calls are recorded again, so saving a replayed session gives the same file.

use bevy::prelude::*;
use bevy::remote::{BrpResult, RemoteMethodSystemId, RemoteMethods};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::plugin::commands::DebugCommands;
use crate::plugin::ready::DebugReady;

/// The `format` string every scene file carries.
pub(crate) const FORMAT: &str = "bevy_fairy_scene";
/// The only `version` this crate reads or writes.
pub(crate) const VERSION: u32 = 1;
/// The environment variable that names a scene file to replay at startup.
pub(crate) const SCENE_ENV: &str = "BEVY_FAIRY_SCENE";

/// One recorded call: the BRP method and the params it was called with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SceneStep {
    pub method: String,
    pub params: Value,
}

/// The calls made so far this session, in order.
#[derive(Resource, Default)]
pub struct SceneLog(pub Vec<SceneStep>);

/// A scene being replayed. `next` is the index of the next step to run.
#[derive(Resource)]
pub struct ScenePlayback {
    pub path: String,
    pub steps: Vec<SceneStep>,
    pub next: usize,
    pub failed: usize,
}

#[derive(Deserialize)]
struct SceneFile {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    steps: Vec<SceneStep>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneParams {
    action: String,
}

/// The scene file text for `steps`.
pub(crate) fn scene_json(steps: &[SceneStep]) -> Value {
    json!({ "format": FORMAT, "version": VERSION, "steps": steps })
}

/// Reads and checks a scene file. Every failure is a sentence, never a panic.
pub(crate) fn load(path: &str) -> Result<Vec<SceneStep>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read: {e}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("not JSON: {e}"))?;
    let format = value.get("format").and_then(Value::as_str).unwrap_or("");
    if format != FORMAT {
        return Err(format!("format is \"{format}\", expected \"{FORMAT}\""));
    }
    let version = value.get("version").and_then(Value::as_u64);
    if version != Some(u64::from(VERSION)) {
        return Err(format!("version is {version:?}, expected {VERSION}"));
    }
    let file: SceneFile =
        serde_json::from_value(value).map_err(|e| format!("bad steps: {e}"))?;
    Ok(file.steps)
}

/// A scene file that was asked for and refused. `bevy_fairy/status` reports it: the one `error!`
/// line goes to a log an agent cannot read, and `replay: null` would say that no scene was asked for,
/// which is what a game started without `BEVY_FAIRY_SCENE` says (B-130).
#[derive(Resource)]
pub(crate) struct SceneRejected {
    pub(crate) path: String,
    pub(crate) why: String,
}

/// Starts replaying `path`. A file that cannot be read is logged once, recorded as
/// [`SceneRejected`], and nothing is replayed.
pub(crate) fn start_playback(app: &mut App, path: &str) {
    match load(path) {
        Ok(steps) => {
            if steps.is_empty() {
                info!("bevy_fairy: scene {path} replayed (0 steps, 0 failed)");
            }
            app.world_mut().insert_resource(ScenePlayback {
                path: path.to_string(),
                steps,
                next: 0,
                failed: 0,
            });
        }
        Err(why) => {
            error!("bevy_fairy: scene {path}: {why}");
            app.world_mut().insert_resource(SceneRejected { path: path.to_string(), why });
        }
    }
}

/// BRP handler: `bevy_fairy/scene`. `{"action": "get"}` returns the recorded calls in file format.
/// `{"action": "clear"}` empties the log.
pub(crate) fn handle_scene(In(params): In<Option<Value>>, mut log: ResMut<SceneLog>) -> BrpResult {
    let params: SceneParams = serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(|e| {
        crate::plugin::brp_error(
            bevy::remote::error_codes::INVALID_PARAMS,
            format!("bevy_fairy/scene: {e}; example: {}", json!({"action": "get"})),
        )
    })?;
    match params.action.as_str() {
        "get" => Ok(scene_json(&log.0)),
        "clear" => {
            let cleared = log.0.len();
            log.0.clear();
            Ok(json!({ "cleared": cleared }))
        }
        other => Err(crate::plugin::brp_error(
            bevy::remote::error_codes::INVALID_PARAMS,
            format!("action must be get or clear, not \"{other}\""),
        )),
    }
}

/// Exclusive `Update` system: runs at most one replay step per frame, once the game is ready.
pub(crate) fn replay_next_step(world: &mut World) {
    if !world.get_resource::<DebugReady>().is_some_and(|r| r.0) {
        return;
    }
    let Some(playback) = world.get_resource::<ScenePlayback>() else {
        return;
    };
    let Some(step) = playback.steps.get(playback.next).cloned() else {
        return;
    };

    let outcome = run_step(world, &step);

    let Some(mut playback) = world.get_resource_mut::<ScenePlayback>() else {
        return;
    };
    playback.next += 1;
    if let Err(why) = &outcome {
        playback.failed += 1;
        warn!("bevy_fairy: scene step {} ({}) failed: {why}", playback.next, step.method);
    }
    if playback.next == playback.steps.len() {
        info!(
            "bevy_fairy: scene {} replayed ({} steps, {} failed)",
            playback.path,
            playback.steps.len(),
            playback.failed
        );
    }
}

/// Runs one step through the same handler a live call would use. A method that is not a recorded
/// command is refused here, so a scene file cannot call a read-only query or a method this game does
/// not have.
fn run_step(world: &mut World, step: &SceneStep) -> Result<(), String> {
    let recorded = world
        .get_resource::<DebugCommands>()
        .is_some_and(|commands| commands.iter().any(|c| c.name == step.method && c.records));
    if !recorded {
        return Err(format!("{} is not a recorded debug command here", step.method));
    }
    let id = match world
        .get_resource::<RemoteMethods>()
        .and_then(|methods| methods.get(&step.method))
    {
        Some(RemoteMethodSystemId::Instant(id)) => *id,
        _ => return Err(format!("{} has no instant handler", step.method)),
    };
    match world.run_system_with(id, Some(step.params.clone())) {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(brp)) => Err(brp.message),
        Err(e) => Err(format!("{e:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_json_has_the_format_and_version() {
        let steps = vec![SceneStep { method: "demo/x".into(), params: json!({"n": 1}) }];
        let file = scene_json(&steps);
        assert_eq!(file["format"], FORMAT);
        assert_eq!(file["version"], VERSION);
        assert_eq!(file["steps"][0]["method"], "demo/x");
    }
}
