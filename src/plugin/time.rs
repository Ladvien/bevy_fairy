//! `bevy_fairy/time`: pause the game clock and set its speed.
//!
//! Gregory, *Game Engine Architecture* (3rd ed., §10.4–10.6), lists a pause and a time scale among
//! the features that make in-engine debugging useful: an agent can freeze a frame, then inspect it.
//! This drives `Time<Virtual>`, the clock that gameplay and animation read, so pausing it freezes
//! the simulation but not the renderer or BRP itself.
//!
//! The command writes a [`ClockRequest`] and does not fight the host for `Time<Virtual>`. A system in
//! `First`, ahead of bevy_time's own clock system, applies the request every frame, so a host that
//! writes `Time<Virtual>` later in the frame (an `unpause`, a `set_relative_speed`) cannot undo it.
//! A host that must combine the request with its own holds and slow motion turns that system off with
//! `DebuggerPlugin::host_owns_clock` and reads [`ClockRequest`] itself. Either way the reply says
//! whether the request held, measured on the frame after it was made (issue #10).

use bevy::prelude::*;
use bevy::time::TimeSystems;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::plugin::commands::{add_builtin_command, DebugCommandInfo};

/// Params of `bevy_fairy/time`. Both fields are optional; an empty call only reports the clock.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeParams {
    pub pause: Option<bool>,
    pub speed: Option<f32>,
}

/// What the agent asked of the game clock. `bevy_fairy/time` writes it; it stays until the agent
/// changes it (a pause survives a new game or a phase change). A host that owns the clock reads it:
/// OR `pause` into its own hold and multiply `speed` into its own factor.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct ClockRequest {
    /// Hold `Time<Virtual>` paused.
    pub pause: bool,
    /// The speed multiplier asked for; `1.0` is real time.
    pub speed: f32,
}

impl Default for ClockRequest {
    fn default() -> Self {
        Self { pause: false, speed: 1.0 }
    }
}

/// What the clock did on the last frame, and under which request.
#[derive(Clone, Copy)]
struct Observation {
    request: ClockRequest,
    delta: std::time::Duration,
    effective_speed: f32,
}

/// Private bookkeeping: whether the host owns the clock, what this crate has overridden (so a
/// cleared request is undone exactly once), and the last observation.
#[derive(Resource, Default)]
pub(crate) struct ClockState {
    pub(crate) host_owned: bool,
    paused_by_us: bool,
    speed_by_us: bool,
    observed: Option<Observation>,
}

/// Registers `bevy_fairy/time` as a recorded built-in command.
pub(crate) fn register(app: &mut App, host_owned: bool) {
    app.init_resource::<ClockRequest>()
        .insert_resource(ClockState { host_owned, ..default() })
        .add_systems(First, observe_clock.after(TimeSystems));
    if !host_owned {
        app.add_systems(First, apply_request.before(TimeSystems));
    }
    add_builtin_command::<TimeParams, _>(
        app,
        DebugCommandInfo::new(
            "bevy_fairy/time",
            "Pause or unpause the game clock (Time<Virtual>) and set its speed multiplier. The request persists and is re-applied every frame against a host that writes the clock. Returns the clock state, `held` (did the clock obey on the last frame; null with `unavailable` until a frame has run) and `host_owned`.",
        )
        .params("pause: bool, optional; speed: f32 > 0, optional (1.0 is real time)")
        .example(json!({"pause": true})),
        handle_time,
    );
}

/// Applies the request to the clock, and undoes what this crate set once the request is cleared.
fn apply(request: &ClockRequest, state: &mut ClockState, clock: &mut Time<Virtual>) {
    if request.pause {
        clock.pause();
        state.paused_by_us = true;
    } else if state.paused_by_us {
        clock.unpause();
        state.paused_by_us = false;
    }
    if request.speed != 1.0 {
        clock.set_relative_speed(request.speed);
        state.speed_by_us = true;
    } else if state.speed_by_us {
        clock.set_relative_speed(1.0);
        state.speed_by_us = false;
    }
}

fn apply_request(
    request: Res<ClockRequest>,
    mut state: ResMut<ClockState>,
    mut clock: ResMut<Time<Virtual>>,
) {
    apply(&request, &mut state, &mut clock);
}

fn observe_clock(request: Res<ClockRequest>, clock: Res<Time<Virtual>>, mut state: ResMut<ClockState>) {
    state.observed = Some(Observation {
        request: *request,
        delta: clock.delta(),
        effective_speed: clock.effective_speed(),
    });
}

fn handle_time(
    In(params): In<TimeParams>,
    mut request: ResMut<ClockRequest>,
    mut state: ResMut<ClockState>,
    mut clock: ResMut<Time<Virtual>>,
) -> Result<Value, String> {
    if let Some(speed) = params.speed {
        if !speed.is_finite() || speed <= 0.0 {
            return Err("speed must be > 0".to_string());
        }
    }
    if let Some(speed) = params.speed {
        request.speed = speed;
    }
    if let Some(pause) = params.pause {
        request.pause = pause;
    }
    // Apply now so the reply reads the new clock; the host-owned mode never writes the clock.
    if !state.host_owned {
        apply(&request, &mut state, &mut clock);
    }

    let mut reply = json!({
        "paused": clock.is_paused(),
        "speed": clock.relative_speed(),
        "effective_speed": clock.effective_speed(),
        "requested": { "pause": request.pause, "speed": request.speed },
        "host_owned": state.host_owned,
    });
    match state.observed.filter(|seen| seen.request == *request) {
        Some(seen) => {
            let pause_held = !request.pause || seen.delta.is_zero();
            let speed_held = request.pause
                || request.speed == 1.0
                || (seen.effective_speed - request.speed).abs() <= 1e-3 * request.speed;
            reply["held"] = json!(pause_held && speed_held);
        }
        None => {
            reply["held"] = Value::Null;
            reply["unavailable"] = json!(
                "no frame has run under this request yet; call bevy_fairy/time with no params to read whether it held"
            );
        }
    }
    Ok(reply)
}
