//! `bevy_fairy/ring` records a query once per frame inside the game, bounded, and dumps it with
//! frame numbers. Ground truth is the counter the test itself increments: sample `k` must hold the
//! value the game had on that frame, and a gap or a stale repeat is a lie.

use std::time::Duration;

use bevy::prelude::*;
use bevy::remote::{BrpResult, RemoteMethodSystemId, RemoteMethods};
use bevy::time::TimeUpdateStrategy;
use bevy_fairy::DebuggerPlugin;
use serde_json::{json, Value};

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Counter(u32);

const PATH: &str = "plugin_ring::Counter";

fn bump(mut counters: Query<&mut Counter>) {
    for mut c in &mut counters {
        c.0 += 1;
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, DebuggerPlugin::default()));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(16)));
    app.register_type::<Counter>();
    app.world_mut().spawn(Counter(0));
    app.add_systems(Update, bump);
    app.update();
    app
}

fn ring(app: &mut App, params: Value) -> BrpResult {
    let id = match app.world().resource::<RemoteMethods>().get("bevy_fairy/ring") {
        Some(RemoteMethodSystemId::Instant(id)) => *id,
        _ => panic!("bevy_fairy/ring is not registered"),
    };
    app.world_mut().run_system_with(id, Some(params)).expect("handler runs")
}

fn start(frames: usize) -> Value {
    json!({"action": "start", "frames": frames, "query": {"data": {"components": [PATH]}}})
}

fn counter_in(sample: &Value) -> u64 {
    sample["result"][0]["components"][PATH].as_u64().unwrap_or_else(|| panic!("no counter in {sample}"))
}

#[test]
fn keeps_exactly_the_newest_frames_and_each_holds_its_own_frames_value() {
    let mut app = app();
    ring(&mut app, start(5)).expect("start");
    for _ in 0..12 {
        app.update();
    }
    let dump = ring(&mut app, json!({"action": "dump"})).expect("dump");
    let samples = dump["samples"].as_array().expect("samples");
    assert_eq!(samples.len(), 5, "{dump}");
    assert_eq!(dump["evicted"], 7, "{dump}");
    assert_eq!(dump["recording"], true);
    let frames: Vec<u64> = samples.iter().map(|s| s["frame"].as_u64().unwrap()).collect();
    assert!(frames.windows(2).all(|w| w[1] == w[0] + 1), "frame gap: {frames:?}");
    // The counter rises by one per frame, so value - frame is constant across the buffer.
    let offsets: Vec<i64> = samples.iter().map(|s| counter_in(s) as i64 - s["frame"].as_i64().unwrap()).collect();
    assert!(offsets.iter().all(|o| *o == offsets[0]), "stale or skipped samples: {offsets:?}");
    assert_eq!(dump["newest_frame"], frames[4]);
}

#[test]
fn a_trigger_ends_the_buffer_on_the_frame_that_matched() {
    let mut app = app();
    let mut params = start(50);
    params["trigger"] = json!({"pointer": format!("/0/components/{PATH}"), "equals": 9});
    ring(&mut app, params).expect("start");
    for _ in 0..30 {
        app.update();
    }
    let dump = ring(&mut app, json!({"action": "dump", "last": 2})).expect("dump");
    assert_eq!(dump["recording"], false, "{dump}");
    let samples = dump["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 2, "{dump}");
    let last = samples.last().unwrap();
    assert_eq!(counter_in(last), 9, "{dump}");
    assert_eq!(dump["triggered_at_frame"], last["frame"], "{dump}");
    assert_eq!(dump["newest_frame"], last["frame"], "recording went on past the trigger: {dump}");
}

#[test]
fn stop_freezes_the_buffer() {
    let mut app = app();
    ring(&mut app, start(10)).expect("start");
    for _ in 0..3 {
        app.update();
    }
    let stopped = ring(&mut app, json!({"action": "stop"})).expect("stop");
    assert_eq!(stopped["recording"], false);
    assert_eq!(stopped["buffered"], 3, "{stopped}");
    for _ in 0..5 {
        app.update();
    }
    let dump = ring(&mut app, json!({"action": "dump"})).expect("dump");
    assert_eq!(dump["samples"].as_array().unwrap().len(), 3, "{dump}");
}

#[test]
fn refuses_instead_of_recording_something_else() {
    let mut app = app();
    let dump = ring(&mut app, json!({"action": "dump"})).expect_err("nothing started");
    assert!(dump.message.contains("start"), "{}", dump.message);
    for bad in [
        json!({"action": "start"}),
        start(0),
        start(601),
        json!({"action": "start", "frames": 4, "query": {"data": {"components": ["no::Such"]}}}),
        json!({"action": "start", "query": {"data": {"components": [PATH]}}, "trigger": {"pointer": "0/x", "equals": 1}}),
    ] {
        ring(&mut app, bad.clone()).expect_err(&format!("accepted {bad}"));
    }
    let status = ring(&mut app, json!({"action": "dump"}));
    assert!(status.is_err(), "a refused start must not leave a buffer behind");
}
