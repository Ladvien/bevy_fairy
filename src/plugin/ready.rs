//! Readiness: when the game may take commands, and the `bevy_fairy/status` report.
//!
//! A game command that runs before the scene is built sees half a world. So every game command
//! refuses until [`DebugReady`] is true. Without `host_signals_ready` the plugin sets it at once and
//! the game needs no extra wiring. With it, the host sets `DebugReady(true)` itself when its scene is
//! finished (the Sanctuary host waits for its loading scenes to settle).

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
    commands: Res<DebugCommands>,
    playback: Option<Res<ScenePlayback>>,
    input: Option<Res<crate::plugin::input::PendingInput>>,
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
        "frame": ticks.0,
        "commands": commands.iter().count(),
        "capture": capture_kind(
            #[cfg(feature = "plugin-screenshot")]
            target.is_some(),
            #[cfg(feature = "plugin-screenshot")]
            !windows.is_empty(),
        ),
        "replay": replay,
        // **Queued versus applied.** `bevy_debugger/input` answers `success: true` on a well-formed
        // command, which says nothing about whether the key ever reached the game: a host whose
        // injections are never folded looks identical from the client. `applied` climbs by one per
        // message actually written, so a caller can tell a dropped injection from an unread key.
        "input": input.as_deref().map(|input| json!({
            "applied": input.applied,
            "applied_this_frame": input.applied_this_frame,
            "queued": input.queued(),
        })),
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
