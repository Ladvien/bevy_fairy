//! `bevy_fairy/time` is a request a host can honour, not a write the host overwrites (issue #10).
//!
//! Each test builds an app with a host system that fights the agent every `Update`, drives a fixed
//! 16 ms per frame through `TimeUpdateStrategy`, and checks `Time<Virtual>` against what the
//! request asked for.

use std::time::Duration;

use bevy::prelude::*;
use bevy::remote::{BrpResult, RemoteMethodSystemId, RemoteMethods};
use bevy::time::{TimeUpdateStrategy, Virtual};
use bevy_fairy::{ClockRequest, DebuggerPlugin};
use serde_json::{json, Value};

const FRAME: Duration = Duration::from_millis(16);

fn app_with(plugin: DebuggerPlugin) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, plugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(FRAME));
    // The first update only primes the clock; run it so later frames each advance by FRAME.
    app.update();
    app
}

fn call(app: &mut App, params: Option<Value>) -> BrpResult {
    let id = match app.world().resource::<RemoteMethods>().get("bevy_fairy/time") {
        Some(RemoteMethodSystemId::Instant(id)) => *id,
        _ => panic!("bevy_fairy/time is not registered"),
    };
    app.world_mut().run_system_with(id, params).expect("handler runs")
}

fn run(app: &mut App, frames: u32) -> Duration {
    let before = app.world().resource::<Time<Virtual>>().elapsed();
    for _ in 0..frames {
        app.update();
    }
    app.world().resource::<Time<Virtual>>().elapsed() - before
}

/// A host that unpauses the clock every frame, as Sanctuary's `apply_time_holds` does.
fn host_unpauses(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

/// A host that owns the speed every frame, as Sanctuary's `slow_the_world` does.
fn host_sets_speed_to_one(mut time: ResMut<Time<Virtual>>) {
    time.set_relative_speed(1.0);
}

/// A host that holds its own pause every frame, as a pause menu does.
fn host_pauses(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

/// A host that runs the world in slow motion of its own, as a bullet-time effect does.
fn host_slows_to_half(mut time: ResMut<Time<Virtual>>) {
    time.set_relative_speed(0.5);
}

#[test]
fn a_pause_holds_against_a_host_that_unpauses_every_frame() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_unpauses);
    let free = run(&mut app, 10);
    assert!(free > Duration::ZERO, "the clock runs before any request");

    call(&mut app, Some(json!({ "pause": true }))).expect("time answers");
    let moved = run(&mut app, 60);
    assert_eq!(moved, Duration::ZERO, "virtual time moved while the agent held a pause");

    let read = call(&mut app, None).expect("an empty call reads the clock");
    assert_eq!(read["held"], true, "{read}");
    assert_eq!(read["host_owned"], false, "{read}");
}

#[test]
fn a_speed_holds_against_a_host_that_resets_it_every_frame() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_sets_speed_to_one);
    let baseline = run(&mut app, 60);

    call(&mut app, Some(json!({ "speed": 4.0 }))).expect("time answers");
    let fast = run(&mut app, 60);

    let ratio = fast.as_secs_f64() / baseline.as_secs_f64();
    assert!((ratio - 4.0).abs() < 0.1, "clock ran {ratio}x, wanted 4x");
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["held"], true, "{read}");
    assert!((read["effective_speed"].as_f64().unwrap() - 4.0).abs() < 1e-6, "{read}");
}

#[test]
fn a_host_that_owns_the_clock_is_never_written_to_and_learns_the_request() {
    let mut app = app_with(DebuggerPlugin::default().host_owns_clock());
    app.add_systems(Update, host_unpauses);

    let reply = call(&mut app, Some(json!({ "pause": true, "speed": 2.0 }))).expect("answers");
    assert_eq!(reply["host_owned"], true, "{reply}");
    let request = app.world().resource::<ClockRequest>();
    assert!(request.pause);
    assert_eq!(request.speed, 2.0);

    let moved = run(&mut app, 30);
    assert!(moved > Duration::ZERO, "the crate wrote Time<Virtual> in host-owned mode");
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["held"], false, "a host that ignores the request is not held: {read}");
    assert_eq!(read["host_owned"], true, "{read}");
}

#[test]
fn a_request_survives_a_host_state_change_until_cleared() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_unpauses);
    call(&mut app, Some(json!({ "pause": true }))).expect("time answers");
    run(&mut app, 5);

    // The host resets its own world state (a new game); the agent's request is not part of it.
    app.world_mut().insert_resource(Time::<Virtual>::default());
    assert_eq!(run(&mut app, 30), Duration::ZERO, "the pause was lost with the host's reset");

    call(&mut app, Some(json!({ "pause": false }))).expect("clear");
    assert!(run(&mut app, 5) > Duration::ZERO, "clearing the pause did not resume the clock");
}

#[test]
fn an_empty_call_reports_the_clock_and_whether_the_request_held() {
    let mut app = app_with(DebuggerPlugin::default());
    run(&mut app, 3);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["paused"], false, "{read}");
    assert_eq!(read["speed"], 1.0, "{read}");
    assert_eq!(read["held"], true, "{read}");
    assert_eq!(read["host_owned"], false, "{read}");
}

#[test]
fn a_request_is_not_called_held_before_a_frame_has_run_under_it() {
    let mut app = app_with(DebuggerPlugin::default());
    run(&mut app, 3);
    let reply = call(&mut app, Some(json!({ "pause": true }))).expect("answers");
    assert!(reply["held"].is_null(), "held claimed with no frame observed: {reply}");
    assert!(reply["unavailable"].is_string(), "{reply}");
}

/// `Time<Virtual>` multiplies each frame's raw delta by the speed with `Duration::mul_f64`, which
/// panics when the product does not fit a `Duration`. A speed that does that is refused, and the
/// clock never sees it (issue #12).
#[test]
fn a_speed_the_clock_cannot_represent_is_refused_not_applied() {
    let mut app = app_with(DebuggerPlugin::default());
    for bad in [1e30, 1e21, f32::MAX, 1000.5] {
        let err = call(&mut app, Some(json!({ "speed": bad }))).expect_err("a speed above the bound");
        assert!(err.message.contains("speed"), "{err:?}");
    }
    assert_eq!(app.world().resource::<ClockRequest>().speed, 1.0, "a refused speed was stored");
    run(&mut app, 3); // before the fix this panicked inside bevy_time's advance_with_raw_delta
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["speed"], 1.0, "{read}");
}

/// The bound itself is accepted and the game keeps running at it.
#[test]
fn the_largest_speed_the_command_accepts_runs() {
    let mut app = app_with(DebuggerPlugin::default());
    call(&mut app, Some(json!({ "speed": 1000.0 }))).expect("the bound is allowed");
    run(&mut app, 3);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["speed"], 1000.0, "{read}");
}

/// A speed the engine clamps is not a speed the host overrode. `Time<Virtual>` caps each frame at
/// `max_delta` (250 ms), so 20x on 16 ms frames advances 250 ms, an effective 15.625x. `held`
/// said false there, which read as "the host is fighting the clock" when nothing touched it.
#[test]
fn a_speed_the_engine_clamps_is_held_and_says_it_was_clamped() {
    let mut app = app_with(DebuggerPlugin::default());
    call(&mut app, Some(json!({ "speed": 20.0 }))).expect("answers");
    run(&mut app, 3);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["held"], true, "no host system touched the clock: {read}");
    assert_eq!(read["clamped_by_max_delta"], true, "{read}");
    assert!((read["effective_speed"].as_f64().unwrap() - 15.625).abs() < 1e-3, "{read}");
    assert_eq!(read["speed"], 20.0, "the speed in force is the one asked for: {read}");

    // A speed under the clamp is not reported as clamped.
    call(&mut app, Some(json!({ "speed": 4.0 }))).expect("answers");
    run(&mut app, 3);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["held"], true, "{read}");
    assert_eq!(read["clamped_by_max_delta"], false, "{read}");
}

/// Clamped is not a licence for `held`: a host that owns the clock and ignores the request is
/// still not held, because its speed is not the request's.
#[test]
fn a_host_that_ignores_a_clamped_speed_is_still_not_held() {
    let mut app = app_with(DebuggerPlugin::default().host_owns_clock());
    call(&mut app, Some(json!({ "speed": 20.0 }))).expect("answers");
    run(&mut app, 3);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["held"], false, "{read}");
    assert_eq!(read["clamped_by_max_delta"], false, "the host's 1x speed was not clamped: {read}");
}

/// **B-129.** The agent asks for the clock to run and the host holds it paused. `held` said true,
/// because an unpause was never something it checked, while `paused: true` sat in the same reply:
/// the agent read "the clock obeyed" next to a clock that had not moved.
#[test]
fn an_unpause_the_host_overrides_is_not_held() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_pauses);
    run(&mut app, 5);

    call(&mut app, Some(json!({ "pause": false }))).expect("answers");
    assert_eq!(run(&mut app, 30), Duration::ZERO, "the host's pause held the clock");
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["paused"], true, "{read}");
    assert_eq!(read["held"], false, "asked to run, clock paused, and held said true: {read}");
}

/// **B-129.** The same for speed: a request of 1.0 was never checked against the clock, so a host
/// running the world at half speed read as `held: true` beside `effective_speed: 0.5`.
#[test]
fn a_speed_of_one_the_host_overrides_is_not_held() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_slows_to_half);
    run(&mut app, 5);

    call(&mut app, Some(json!({ "speed": 1.0 }))).expect("answers");
    run(&mut app, 30);
    let read = call(&mut app, None).expect("read");
    assert!((read["effective_speed"].as_f64().unwrap() - 0.5).abs() < 1e-6, "{read}");
    assert_eq!(read["held"], false, "asked for real time, clock at half, and held said true: {read}");
}

/// The other side of the two above: with nobody fighting, the same requests are held, so the fix
/// did not turn `held` into a constant `false` for requests that ask for nothing unusual.
#[test]
fn an_unpause_and_a_speed_of_one_nobody_overrides_are_held() {
    let mut app = app_with(DebuggerPlugin::default());
    call(&mut app, Some(json!({ "pause": true }))).expect("answers");
    run(&mut app, 5);
    call(&mut app, Some(json!({ "pause": false, "speed": 1.0 }))).expect("answers");
    run(&mut app, 5);
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["paused"], false, "{read}");
    assert_eq!(read["held"], true, "{read}");
}

/// A pause asks for no speed (the request's speed stays 1.0, which the plugin never writes over a
/// host's own), so a host that runs slow motion of its own does not make a pause that worked read as
/// not held. While the clock is frozen the speed it will resume at is not what was asked about.
#[test]
fn a_pause_is_held_beside_a_hosts_own_slow_motion() {
    let mut app = app_with(DebuggerPlugin::default());
    app.add_systems(Update, host_slows_to_half);
    run(&mut app, 5);

    call(&mut app, Some(json!({ "pause": true }))).expect("answers");
    assert_eq!(run(&mut app, 30), Duration::ZERO, "the pause froze the clock");
    let read = call(&mut app, None).expect("read");
    assert_eq!(read["paused"], true, "{read}");
    assert_eq!(read["held"], true, "a frozen clock obeyed a pause: {read}");
}
