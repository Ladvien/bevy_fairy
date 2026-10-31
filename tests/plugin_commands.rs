//! Behaviour of game-registered commands, readiness, time and scene replay.
//!
//! These drive the handlers the way the BRP server does: each method is looked up in `RemoteMethods`
//! and its `SystemId` is run with the request's params. No socket is opened, so the tests need no
//! network and no GPU. Research behind the shape: Gregory, *Game Engine Architecture* 3rd ed.
//! §10.4–10.6 (game-registered console commands and pause); Ostrowski & Aroudj 2013, DOI
//! 10.7603/s40601-013-0010-4 (the game answers function calls over a connection). Scene replay follows
//! Xu, "AI Agent Systems" 2026, DOI 10.48550/arXiv.2601.01743: a recorded scene is a list of named
//! calls with their exact params, so an agent can read and rerun it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use bevy::prelude::*;
use bevy::remote::{error_codes, BrpResult, RemoteMethodSystemId, RemoteMethods};
use bevy::time::Virtual;
use bevy_fairy::{
    DebugCommandInfo, DebugCommandsAppExt, DebugReady, DebuggerPlugin, SceneLog, SceneStep,
};
use serde::Deserialize;
use serde_json::{json, Value};

const SCENE_VAR: &str = "BEVY_FAIRY_SCENE";

/// Serialises app construction: `DebuggerPlugin` reads `BEVY_FAIRY_SCENE` while it builds, and the
/// test harness runs tests on several threads, so one test's variable must not leak into another's app.
static BUILD_LOCK: Mutex<()> = Mutex::new(());

fn build_lock() -> MutexGuard<'static, ()> {
    // A panic in one `should_panic` test poisons the lock; the guard is still usable.
    BUILD_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Builds an app the way a host does: `MinimalPlugins`, `InputPlugin` (required by the plugin), and
/// `DebuggerPlugin`. `host_signals` makes the game report not ready until the test flips it.
fn new_app(host_signals: bool, scene: Option<&Path>) -> App {
    let _guard = build_lock();
    match scene {
        Some(path) => std::env::set_var(SCENE_VAR, path),
        None => std::env::remove_var(SCENE_VAR),
    }
    let plugin = if host_signals {
        DebuggerPlugin::default().host_signals_ready()
    } else {
        DebuggerPlugin::default()
    };
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, plugin));
    app.init_resource::<Counted>();
    app
}

/// Records every value the `demo/count` command was called with, in call order.
#[derive(Resource, Default)]
struct Counted(Vec<i64>);

#[derive(Deserialize)]
struct Count {
    n: i64,
}

fn count(In(p): In<Count>, mut seen: ResMut<Counted>) -> Result<Value, String> {
    seen.0.push(p.n);
    Ok(json!({ "counted": p.n }))
}

fn register_count(app: &mut App) {
    app.add_debug_command(
        DebugCommandInfo::new("demo/count", "Count one, and remember it")
            .params("n: i64, the number to count")
            .example(json!({ "n": 2 })),
        count,
    );
}

/// Runs a registered BRP method by name, the way the server's request path does.
fn call(app: &mut App, method: &str, params: Option<Value>) -> BrpResult {
    let id = match app.world().resource::<RemoteMethods>().get(method) {
        Some(RemoteMethodSystemId::Instant(id)) => *id,
        _ => panic!("no instant method `{method}` registered"),
    };
    app.world_mut()
        .run_system_with(id, params)
        .expect("a registered handler system runs")
}

fn temp_scene(name: &str, body: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "bdm-{name}-{}-{nanos}.json",
        std::process::id()
    ));
    std::fs::write(&path, body).expect("temp scene file is writable");
    path
}

#[test]
fn a_registered_command_is_listed_with_its_description_and_example() {
    let mut app = new_app(false, None);
    register_count(&mut app);

    let listed = call(&mut app, "bevy_fairy/commands", None).expect("commands answers");
    let entries = listed["commands"].as_array().expect("commands is a list");
    let entry = entries
        .iter()
        .find(|c| c["name"] == "demo/count")
        .expect("demo/count is listed");
    assert_eq!(entry["description"], "Count one, and remember it");
    assert_eq!(entry["example"], json!({ "n": 2 }));
}

#[test]
fn a_call_runs_the_handler_and_records_exactly_one_step() {
    let mut app = new_app(false, None);
    register_count(&mut app);

    let result = call(&mut app, "demo/count", Some(json!({ "n": 3 }))).expect("the call succeeds");
    assert_eq!(result, json!({ "counted": 3 }));
    assert_eq!(app.world().resource::<Counted>().0, vec![3]);
    assert_eq!(
        app.world().resource::<SceneLog>().0,
        vec![SceneStep {
            method: "demo/count".to_string(),
            params: json!({ "n": 3 }),
        }]
    );
}

#[test]
fn bad_params_name_the_command_and_its_example_and_record_nothing() {
    let mut app = new_app(false, None);
    register_count(&mut app);

    let err = call(&mut app, "demo/count", Some(json!({ "m": 1 }))).expect_err("bad params fail");
    assert_eq!(err.code, error_codes::INVALID_PARAMS);
    assert!(err.message.contains("demo/count"), "{}", err.message);
    assert!(err.message.contains(r#"{"n":2}"#), "{}", err.message);
    assert!(app.world().resource::<Counted>().0.is_empty());
    assert!(app.world().resource::<SceneLog>().0.is_empty());
}

#[test]
fn a_command_waits_while_the_game_is_not_ready() {
    let mut app = new_app(true, None);
    register_count(&mut app);

    let err = call(&mut app, "demo/count", Some(json!({ "n": 1 }))).expect_err("not ready yet");
    assert_eq!(err.code, error_codes::INTERNAL_ERROR);
    assert!(err.message.contains("game not ready yet"), "{}", err.message);

    app.world_mut().resource_mut::<DebugReady>().0 = true;
    call(&mut app, "demo/count", Some(json!({ "n": 1 }))).expect("ready game answers");
}

#[test]
#[should_panic(expected = "registered twice")]
fn registering_a_name_twice_panics_at_startup() {
    let mut app = new_app(false, None);
    register_count(&mut app);
    register_count(&mut app);
}

#[test]
#[should_panic(expected = "is reserved or empty")]
fn a_reserved_world_name_panics_at_startup() {
    let mut app = new_app(false, None);
    app.add_debug_command(
        DebugCommandInfo::new("world.foo", "reserved")
            .example(json!({ "n": 1 })),
        count,
    );
}

#[test]
#[should_panic(expected = "its example does not parse as its params")]
fn an_example_that_does_not_parse_panics_at_startup() {
    let mut app = new_app(false, None);
    app.add_debug_command(
        DebugCommandInfo::new("demo/bad_example", "example is a string").example(json!({ "n": "two" })),
        count,
    );
}

#[test]
fn time_pauses_virtual_time() {
    let mut app = new_app(false, None);
    call(&mut app, "bevy_fairy/time", Some(json!({ "pause": true }))).expect("time answers");
    assert!(app.world().resource::<Time<Virtual>>().is_paused());
}

#[test]
fn time_refuses_a_non_positive_speed() {
    let mut app = new_app(false, None);
    let err = call(&mut app, "bevy_fairy/time", Some(json!({ "speed": 0.0 })))
        .expect_err("zero speed is refused");
    assert!(err.message.contains("speed must be > 0"), "{}", err.message);
}

#[test]
fn a_scene_file_replays_its_steps_in_order_one_per_frame() {
    let body = r#"{"format":"bevy_fairy_scene","version":1,"steps":[
        {"method":"demo/count","params":{"n":1}},
        {"method":"demo/count","params":{"n":2}}]}"#;
    let path = temp_scene("replay", body);
    let mut app = new_app(false, Some(&path));
    register_count(&mut app);

    for _ in 0..3 {
        app.update();
    }

    assert_eq!(app.world().resource::<Counted>().0, vec![1, 2]);
    let status = call(&mut app, "bevy_fairy/status", None).expect("status answers");
    assert_eq!(status["replay"]["done"], 2);
    assert_eq!(status["replay"]["total"], 2);
    assert_eq!(status["replay"]["failed"], 0);
    std::fs::remove_file(path).ok();
}

#[test]
fn a_scene_file_with_the_wrong_format_replays_nothing_and_does_not_panic() {
    let body = r#"{"format":"x","version":1,"steps":[{"method":"demo/count","params":{"n":1}}]}"#;
    let path = temp_scene("bad-format", body);
    let mut app = new_app(false, Some(&path));
    register_count(&mut app);

    for _ in 0..3 {
        app.update();
    }

    assert!(app.world().resource::<Counted>().0.is_empty());
    let status = call(&mut app, "bevy_fairy/status", None).expect("status answers");
    // B-130: the rejection is said in the reply. `null` here meant "no scene was asked for", the same
    // answer a game with no BEVY_FAIRY_SCENE gives, so an agent could not tell a typo'd path from
    // nothing; the `error!` line went to a log nobody reads.
    let why = status["replay"]["rejected"].as_str().unwrap_or_else(|| panic!("replay does not say it was rejected: {status}"));
    assert!(why.contains("format"), "says why: {why}");
    assert_eq!(status["replay"]["path"], path.to_str().expect("utf-8 path"), "{status}");
    assert_eq!(status["replay"]["total"], 0, "{status}");
    std::fs::remove_file(path).ok();
}

/// **B-130.** A scene file that does not exist is as invisible as one with the wrong format, and
/// no scene requested at all is still `null`: the two are different answers.
#[test]
fn a_scene_file_that_cannot_be_read_is_reported_and_no_scene_stays_null() {
    let missing = std::env::temp_dir().join("bevy_fairy_scene_that_does_not_exist.json");
    std::fs::remove_file(&missing).ok();
    let mut app = new_app(false, Some(&missing));
    app.update();
    let status = call(&mut app, "bevy_fairy/status", None).expect("status answers");
    let why = status["replay"]["rejected"].as_str().unwrap_or_else(|| panic!("{status}"));
    assert!(why.contains("cannot read"), "{why}");

    let mut app = new_app(false, None);
    app.update();
    let status = call(&mut app, "bevy_fairy/status", None).expect("status answers");
    assert!(status["replay"].is_null(), "no scene was asked for: {status}");
}

#[test]
fn scene_get_returns_the_file_format_with_the_recorded_steps() {
    let mut app = new_app(false, None);
    register_count(&mut app);
    call(&mut app, "demo/count", Some(json!({ "n": 5 }))).expect("the call succeeds");

    let scene = call(&mut app, "bevy_fairy/scene", Some(json!({ "action": "get" })))
        .expect("scene get answers");
    assert_eq!(scene["format"], "bevy_fairy_scene");
    assert_eq!(scene["version"], 1);
    assert_eq!(scene["steps"][0]["method"], "demo/count");
    assert_eq!(scene["steps"][0]["params"], json!({ "n": 5 }));
}
