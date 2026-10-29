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
