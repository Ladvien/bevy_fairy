//! Readiness: when the game may take commands, and the `bevy_fairy/status` report.
//!
//! A game command that runs before the scene is built sees half a world. So every game command
//! refuses until [`DebugReady`] is true. Without `host_signals_ready` the plugin sets it at once and
//! the game needs no extra wiring. With it, the host sets `DebugReady(true)` itself when its scene is
//! finished (the Sanctuary host waits for its loading scenes to settle).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy::remote::BrpResult;
use serde_json::{json, Value};

use crate::plugin::commands::DebugCommands;
use crate::plugin::scene::ScenePlayback;

/// Whether the game accepts debug commands yet. Read by every game command's wrapper.
#[derive(Resource)]
pub struct DebugReady(pub bool);

/// Frames since startup, counted by the plugin. `bevy::diagnostic::FrameCount` would do the same, but
/// it needs `bevy_diagnostic`, which this crate does not enable.
#[derive(Resource, Default)]
pub(crate) struct FrameTicks(pub u32);

/// Names this game process, for the whole of its life: `bevy_fairy/status` reports it as `instance`.
///
/// A server keeps state about a game (an undo history, entity ids) for longer than the game lives,
/// and a restarted game reuses the same ids for other things. Comparing this against the value
/// the server saw when it recorded the state is how it tells the two apart. It is the process id,
/// the start time in nanoseconds and a per-process counter (two apps started in one process, as a
/// test does, would otherwise share the first); nothing is drawn from the OS.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct GameInstance(pub String);

impl GameInstance {
    pub(crate) fn new() -> Self {
        static STARTED_IN_THIS_PROCESS: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let nth = STARTED_IN_THIS_PROCESS.fetch_add(1, Ordering::Relaxed);
        Self(format!("{}-{nanos:x}-{nth}", std::process::id()))
    }
}

/// Counts one frame per `Last`.
pub(crate) fn tick_frames(mut ticks: ResMut<FrameTicks>) {
    ticks.0 = ticks.0.wrapping_add(1);
}

/// BRP handler: `bevy_fairy/status`. Not ready-gated, so it is the way to ask whether the game is
/// ready.
pub(crate) fn handle_status(
    In(_): In<Option<Value>>,
    ready: Res<DebugReady>,
    ticks: Res<FrameTicks>,
    instance: Res<GameInstance>,
    commands: Res<DebugCommands>,
    playback: Option<Res<ScenePlayback>>,
    #[cfg(feature = "plugin-screenshot")] target: Option<Res<crate::plugin::screenshot::DebugCaptureTarget>>,
    #[cfg(feature = "plugin-screenshot")] windows: Query<(), With<bevy::window::PrimaryWindow>>,
) -> BrpResult {
    let replay = match playback {
        Some(p) => json!({
            "path": p.path,
            "done": p.next,
            "total": p.steps.len(),
            "failed": p.failed,
        }),
        None => Value::Null,
    };
    Ok(json!({
        "ready": ready.0,
        "instance": instance.0,
        "frame": ticks.0,
        "commands": commands.iter().count(),
        "capture": capture_kind(
            #[cfg(feature = "plugin-screenshot")]
            target.is_some(),
            #[cfg(feature = "plugin-screenshot")]
            !windows.is_empty(),
        ),
        "replay": replay,
    }))
}

/// Which capture path a screenshot request would take right now.
fn capture_kind(
    #[cfg(feature = "plugin-screenshot")] offscreen: bool,
    #[cfg(feature = "plugin-screenshot")] window: bool,
) -> &'static str {
    #[cfg(feature = "plugin-screenshot")]
    {
        if offscreen {
            return "offscreen";
        }
        if window {
            return "window";
        }
    }
    "none"
}
