//! **The guide channel's contract, pinned.**
//!
//! The half of this crate that talks to a *person* has a failure mode the other two do not: it can be
//! wrong in a way that still returns `success: true` and still renders. So what is checked here is
//! mostly the behaviour over *time* — what the watch stream sends on the second frame, and the
//! third — because that is where a condition-watcher goes wrong.
//!
//! Four properties:
//!
//! 1. **A checkpoint decides, and the host owns it.** A registered condition that answers false parks
//!    the request; one that answers true advances the step and records the pass. A name nobody
//!    registered is refused *by name*, listing what would have worked.
//! 2. **Every non-advancing answer is sent once.** The handler re-runs every frame, so "this step has
//!    no checkpoint" and "the script is done" would otherwise be pushed sixty times a second down a
//!    stream whose entire contract is *something happened*.
//! 3. **The transcript is `k/n`, and it survives a re-post.** Bryant, *Game Testing All in One* 4e:
//!    a boolean pass is the thing a person reports and gets wrong. Counts that reset on every run
//!    would be the same boolean wearing a fraction's clothes.
//! 4. **The overlay shows one step, and only while there is one.** Never the script (Andersen et al.
//!    2012: instructions given in context beat an up-front manual by 40% progress), and nothing at all
//!    when idle.
//!
//! Every test here was verified by putting the bug back — the discipline the tiles contract records.
//! Three of them exist *because* the bug was there. Two `announced_once` tests: the first draft of
//! `watch_guide` answered both non-advancing cases unconditionally, at sixty frames a second.
//!
//! And `a_skipped_step_is_not_confirmed_as_a_pass`, which is the one worth reading twice — **a full
//! green suite is not evidence the overlay is right.** Every test here passed while the card
//! congratulated a person for reporting that a step made no sense, because both outcomes rendered
//! text and nothing asserted which. A devshot capture of the real editor is what noticed. When the
//! output is something a human reads, at some point somebody has to look at it.

use bevy::prelude::*;
use bevy_fairy::{
    evaluate_checkpoint, handle_guide, watch_guide, Checkpoints, DebuggerPlugin, Guide,
    GuideOverlayPlugin, Step, BEAT_SECONDS,
};
use bevy::ecs::system::RunSystemOnce;
use serde_json::{json, Value};

/// Whether the host's condition is currently true. Stands in for `Build::open.members.len() >= 2`.
#[derive(Resource, Default)]
struct Ready(bool);

/// An app with the guide's resources and one registered checkpoint, and **no transport** — the
/// handlers are ordinary systems, so none of this needs a socket, a window or a GPU.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Guide>()
        .init_resource::<Checkpoints>()
        .init_resource::<Ready>()
        // The one place a checkpoint is evaluated. The stream below only reports what it recorded,
        // so a test that wants a step to pass runs a frame.
        .add_systems(Last, evaluate_checkpoint);
    register_ready(&mut app);
    app
}

/// Every checkpoint takes `In<Value>`. This one reads it, so the suite exercises the path that
/// makes a condition as strong as the step claiming it: `{"want": false}` asks for the opposite.
fn register_ready(app: &mut App) {
    let id = app.register_system(|args: In<Value>, ready: Res<Ready>| {
        let want = args.0.get("want").and_then(|v| v.as_bool()).unwrap_or(true);
        ready.0 == want
    });
    app.world_mut().resource_mut::<Checkpoints>().register("ready", id);
}

/// An app built the way a host builds one: the whole plugin, so a frame runs exactly what the plugin
/// registers and nothing the test added. Still no transport, no window, no GPU.
fn plugin_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, DebuggerPlugin::default()))
        .init_resource::<Ready>();
    register_ready(&mut app);
    app.update();
    app
}

fn post(app: &mut App, params: Value) -> Value {
    match app.world_mut().run_system_once_with(handle_guide, Some(params)) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => panic!("handler refused: {}", e.message),
        Err(e) => panic!("could not run handler: {e}"),
    }
}

fn refusal(app: &mut App, params: Value) -> String {
    match app.world_mut().run_system_once_with(handle_guide, Some(params)) {
        Ok(Err(e)) => e.message,
        Ok(Ok(v)) => panic!("expected a refusal, got {v}"),
        Err(e) => panic!("could not run handler: {e}"),
    }
}

/// One turn of the watch stream: `None` means *parked*, which is the answer under test as often as a
/// value is.
fn watch(app: &mut App) -> Option<Value> {
    match app.world_mut().run_system_once_with(watch_guide, None) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => panic!("watch refused: {}", e.message),
        Err(e) => panic!("could not run watch: {e}"),
    }
}

/// Posts a script the way a host's own Rust does, straight into [`Guide`]. The BRP method refuses a
/// checkpoint nobody registered, so this is the only way to get one in.
fn post_from_rust(app: &mut App, params: Value) {
    let steps: Vec<Step> = serde_json::from_value(params["steps"].clone()).expect("steps");
    app.world_mut().resource_mut::<Guide>().post(steps);
}

fn watch_refusal(app: &mut App) -> String {
    match app.world_mut().run_system_once_with(watch_guide, None) {
        Ok(Err(e)) => e.message,
        Ok(Ok(v)) => panic!("expected a refusal, got {v:?}"),
        Err(e) => panic!("could not run watch: {e}"),
    }
}

fn script(checkpoint: Value) -> Value {
    json!({"steps": [
        {"label": "drop a floor", "goal": "the tile needs something in it",
         "do": ["press Enter"], "checkpoint": checkpoint,
         "recovery": "if nothing lands, the library row is empty"},
        {"label": "save it", "do": ["press Cmd+S"]}
    ]})
}

// ── 1. the checkpoint decides ────────────────────────────────────────────────────────────────────

#[test]
fn a_checkpoint_that_is_not_met_parks_the_request() {
    let mut app = app();
    post(&mut app, script(json!("ready")));

    // The host's condition is false, so there is nothing to say and the request stays open. This is
    // the whole reason the method is `with_watching_method_main` rather than something that polls.
    assert_eq!(watch(&mut app), None);
    assert_eq!(watch(&mut app), None, "still parked on the next frame");

    let report = post(&mut app, json!({"read": true}));
    assert_eq!(report["guide"]["at"], 1, "still on the first step");
}

#[test]
fn a_checkpoint_that_passes_advances_and_records_it() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();

    let answer = watch(&mut app).expect("the checkpoint passed, so the stream answers");
    assert_eq!(answer["passed"], "drop a floor", "names the step that just completed");
    assert_eq!(answer["guide"]["at"], 2, "and the app has moved on");

    let rows = &answer["guide"]["steps"];
    assert_eq!(rows[0]["passes"], 1);
    assert_eq!(rows[0]["runs"], 1, "1/1");
    assert_eq!(rows[1]["runs"], 1, "the next step is now being attempted");
    assert_eq!(rows[1]["passes"], 0);
}

#[test]
fn a_checkpoint_nobody_registered_is_refused_by_name() {
    let mut app = app();
    post_from_rust(&mut app, script(json!("tile is saved")));

    let message = watch_refusal(&mut app);
    assert!(message.contains("tile is saved"), "names what was asked for: {message}");
    assert!(
        message.contains("ready"),
        "and what would have worked, so the script can be fixed without reading this crate: {message}"
    );
    // The failure this prevents: a watching handler that answered `Ok(None)` for an unknown name
    // would park for ever, and a script that never advances is indistinguishable from a person who
    // has not got round to it.
    assert!(message.contains("for ever") || message.contains("forever"));
}

// ── 2. every non-advancing answer is sent once ───────────────────────────────────────────────────

#[test]
fn a_step_with_no_checkpoint_is_announced_once_not_every_frame() {
    let mut app = app();
    // `checkpoint: null` is a real state: "does this look right?" is not a machine question.
    post(&mut app, script(Value::Null));

    let first = watch(&mut app).expect("says so once");
    assert_eq!(first["waiting_on_a_person"], true);
    assert_eq!(first["step"], "drop a floor");

    assert_eq!(watch(&mut app), None, "and then parks");
    assert_eq!(watch(&mut app), None);

    // A skip moves the step on, which re-arms the announcement for the new one.
    post(&mut app, json!({"skip": true}));
    let second = watch(&mut app).expect("the next step announces itself in turn");
    assert_eq!(second["step"], "save it");
}

#[test]
fn the_end_of_the_script_is_announced_once_not_every_frame() {
    let mut app = app();
    post(&mut app, json!({"steps": [{"label": "the only step", "checkpoint": "ready"}]}));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();

    assert_eq!(watch(&mut app).expect("the step passes")["passed"], "the only step");

    let done = watch(&mut app).expect("then the script reports itself finished");
    assert_eq!(done["done"], true);
    assert_eq!(done["guide"]["steps"][0]["passes"], 1);

    assert_eq!(watch(&mut app), None, "once, not sixty times a second");
    assert_eq!(watch(&mut app), None);
}

// ── 1b. the app advances a step by itself, watched or not ────────────────────────────────────────

/// **B-127.** The MCP `guide` tool can open no watch stream, so when the checkpoint was evaluated
/// only by the stream a step with a checkpoint never advanced through it, while the tool's
/// description told the agent the app moves on by itself. Every test above drove the stream, so
/// none saw it.
#[test]
fn a_checkpoint_that_passes_advances_the_step_with_nobody_watching() {
    let mut app = plugin_app();
    post(&mut app, script(json!("ready")));
    app.update();
    let before = post(&mut app, json!({"read": true}));
    assert_eq!(before["guide"]["at"], 1, "its condition is false, so it stays: {before}");
    assert_eq!(before["guide"]["checkpoint"], "ready", "a read names what the app is waiting for");

    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();
    let after = post(&mut app, json!({"read": true}));
    assert_eq!(after["guide"]["at"], 2, "the condition is true, so the app moved on: {after}");
    assert_eq!(after["guide"]["steps"][0]["passes"], 1);
    assert_eq!(after["guide"]["steps"][0]["runs"], 1, "1/1");
}

/// The stream is a reporter now: a pass the app recorded before anyone watched is still told, once,
/// to the first watcher that asks, and the next step is announced after it.
#[test]
fn the_watch_stream_reports_a_pass_the_app_recorded_before_it_attached() {
    let mut app = plugin_app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();

    let passed = watch(&mut app).expect("the pass is reported to the first watcher");
    assert_eq!(passed["passed"], "drop a floor");
    assert_eq!(passed["guide"]["at"], 2, "with the script's position now");

    let next = watch(&mut app).expect("then the next step names itself");
    assert_eq!(next["waiting_on_a_person"], true);
    assert_eq!(watch(&mut app), None, "and nothing is repeated");
}

/// A checkpoint the app cannot run leaves the step where it is, so a `read` has to say why: before,
/// only the watch stream knew, and a client that never opened one saw a step that did not move.
#[test]
fn a_checkpoint_the_app_cannot_run_is_said_by_a_read() {
    let mut app = plugin_app();
    // Posted from Rust, as a host's own startup code might: the BRP method refuses an unknown name.
    let step: Step = serde_json::from_value(json!({"label": "save it", "checkpoint": "tile is saved"}))
        .expect("a step");
    app.world_mut().resource_mut::<Guide>().post(vec![step]);
    app.update();

    let read = post(&mut app, json!({"read": true}));
    let why = read["guide"]["checkpoint_error"].as_str().unwrap_or_else(|| panic!("a read is silent: {read}"));
    assert!(why.contains("tile is saved") && why.contains("ready"), "names it and what would work: {why}");
}

// ── 3. k/n, and it survives a re-post ────────────────────────────────────────────────────────────

#[test]
fn a_skip_is_an_attempt_that_did_not_pass() {
    let mut app = app();
    post(&mut app, script(json!("ready")));

    let answer = post(&mut app, json!({"skip": true}));
    assert_eq!(answer["skipped"], "drop a floor");

    let rows = &answer["guide"]["steps"];
    assert_eq!(rows[0]["runs"], 1);
    assert_eq!(
        rows[0]["passes"], 0,
        "0/1 — the escape hatch Choong et al. found is needed, because 18 of 20 of their \
         participants followed a suggestion that made no sense rather than say so"
    );
}

#[test]
fn re_running_the_same_script_accumulates_the_counts() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();

    // The same script again — the tester having another go, which is the entire point of `k/n`.
    post(&mut app, script(json!("ready")));
    let after = post(&mut app, json!({"read": true}));
    assert_eq!(after["guide"]["steps"][0]["runs"], 2, "n counts every attempt");
    assert_eq!(after["guide"]["steps"][0]["passes"], 1, "k counts the ones that worked");

    app.update();
    let again = post(&mut app, json!({"read": true}));
    assert_eq!(again["guide"]["steps"][0]["passes"], 2, "2/2");
}

#[test]
fn a_different_script_starts_its_own_counts() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    post(&mut app, json!({"skip": true}));

    post(&mut app, json!({"steps": [{"label": "something else"}]}));
    let report = post(&mut app, json!({"read": true}));
    assert_eq!(report["guide"]["steps"][0]["step"], "something else");
    assert_eq!(report["guide"]["steps"][0]["runs"], 1, "not carried over from a different script");
}

#[test]
fn an_empty_script_is_refused_rather_than_posted() {
    let mut app = app();
    let message = refusal(&mut app, json!({"steps": []}));
    assert!(message.contains("clear"), "and points at the verb that was meant: {message}");
}

#[test]
fn clearing_takes_the_script_down_and_keeps_the_answer() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();

    post(&mut app, json!({"clear": true}));
    let report = post(&mut app, json!({"read": true}));
    assert_eq!(
        report["guide"]["steps"][0]["passes"], 1,
        "the transcript survives, because it is what the exercise was for"
    );
}

// ── 3b. a call that is refused changes nothing, and a call that is accepted did what it said ─────

/// **B-128.** The description tells the agent an unregistered checkpoint is refused by name; only
/// the unreachable watch stream did.
#[test]
fn a_script_naming_an_unregistered_checkpoint_is_refused_when_posted() {
    let mut app = app();
    let message = refusal(&mut app, script(json!("tile is saved")));
    assert!(message.contains("tile is saved"), "names what was asked for: {message}");
    assert!(message.contains("drop a floor"), "and the step that asked: {message}");
    assert!(message.contains("ready"), "and what would have worked: {message}");
    assert!(message.contains("Nothing was posted"), "{message}");

    let read = post(&mut app, json!({"read": true}));
    assert_eq!(read["guide"]["of"], 0, "nothing was posted: {read}");
}

/// **B-128.** `{clear, skip}` ran `clear` and answered `success: true` for both.
#[test]
fn a_call_naming_two_verbs_is_refused_and_does_nothing() {
    let mut app = app();
    post(&mut app, script(json!("ready")));

    let message = refusal(&mut app, json!({"clear": true, "skip": true}));
    assert!(message.contains("one verb per call"), "{message}");
    assert!(message.contains("clear") && message.contains("skip"), "names both: {message}");
    let message = refusal(&mut app, json!({"read": true, "steps": [{"label": "x"}]}));
    assert!(message.contains("read") && message.contains("steps"), "{message}");

    let read = post(&mut app, json!({"read": true}));
    assert_eq!(read["guide"]["of"], 2, "a refused call changed nothing: {read}");
    assert_eq!(read["guide"]["at"], 1);
}

/// **B-128.** `skip` with no step to move past answered `success: true, skipped: null`.
#[test]
fn skipping_with_nothing_to_skip_is_refused() {
    let mut app = app();
    let message = refusal(&mut app, json!({"skip": true}));
    assert!(message.contains("no current step"), "{message}");

    post(&mut app, json!({"steps": [{"label": "the only step"}]}));
    post(&mut app, json!({"skip": true}));
    let message = refusal(&mut app, json!({"skip": true}));
    assert!(message.contains("no current step"), "a finished script has none either: {message}");
}

/// **B-136.** A checkpoint advances the step by itself, so the step a `read` showed can be gone by
/// the time a `skip` arrives, and the skip moved past the NEXT step instead, which the person had
/// not seen: it was recorded as attempted and not passed. A skip that names the step it means is
/// refused when that is not the current one, and skips nothing.
#[test]
fn a_skip_naming_a_step_the_script_has_moved_past_is_refused_and_skips_nothing() {
    let mut app = plugin_app();
    post(&mut app, script(json!("ready")));
    app.update();
    let read = post(&mut app, json!({"read": true}));
    let seen = read["guide"]["step"].as_str().expect("a current step").to_string();
    assert_eq!(seen, "drop a floor");

    // Between the read and the skip the person does it, and the app moves on by itself.
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();
    let message = refusal(&mut app, json!({"skip": true, "step": seen}));
    assert!(message.contains("drop a floor") && message.contains("save it"), "names both steps: {message}");
    assert!(message.contains("skipped nothing"), "{message}");
    assert!(!message.contains("\"skip\""), "the example must not tell the agent to skip the step it has not seen: {message}");

    let read = post(&mut app, json!({"read": true}));
    assert_eq!(read["guide"]["at"], 2, "the script is still on `save it`: {read}");
    assert_eq!(read["guide"]["step"], "save it");
    assert_eq!(read["guide"]["steps"][0]["passes"], 1, "the pass was the app's, and is kept");
}

/// The `step` guard compares labels, so two steps with one label would let a skip meant for the
/// first land on the second (found in review of B-136). A label is also what names a row in the
/// transcript, so a script that repeats one is refused when it is posted and nothing is posted.
#[test]
fn two_steps_with_one_label_are_refused_when_posted() {
    let mut app = app();
    let message = refusal(&mut app, json!({"steps": [
        {"label": "click it", "checkpoint": "ready"},
        {"label": "wait"},
        {"label": "click it"},
    ]}));
    assert!(message.contains("click it") && message.contains("labelled"), "names the label: {message}");
    assert!(message.contains("Nothing was posted"), "{message}");
    let read = post(&mut app, json!({"read": true}));
    assert_eq!(read["guide"]["of"], 0, "nothing was posted: {read}");
}

#[test]
fn a_skip_naming_the_current_step_skips_it() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    let answer = post(&mut app, json!({"skip": true, "step": "drop a floor"}));
    assert_eq!(answer["skipped"], "drop a floor", "{answer}");
    assert_eq!(answer["guide"]["at"], 2);
}

/// `step` names what a skip means; beside any other verb it would be read by nothing.
#[test]
fn a_step_without_a_skip_is_refused_by_name() {
    let mut app = app();
    post(&mut app, script(json!("ready")));
    let message = refusal(&mut app, json!({"read": true, "step": "drop a floor"}));
    assert!(message.contains("step") && message.contains("skip"), "{message}");
    let read = post(&mut app, json!({"read": true}));
    assert_eq!(read["guide"]["at"], 1, "a refused call changed nothing: {read}");
}

/// **B-128.** `{steps, visible: false}` posted a visible script: `post` set `visible` after the
/// field was applied. And `visible` beside a verb it cannot act on was ignored.
#[test]
fn visible_is_honoured_with_a_post_and_refused_where_it_cannot_act() {
    let mut app = app();
    post(&mut app, json!({"steps": [{"label": "quiet"}], "visible": false}));
    assert!(!app.world().resource::<Guide>().visible, "the script was posted hidden");

    let message = refusal(&mut app, json!({"read": true, "visible": true}));
    assert!(message.contains("visible") && message.contains("read"), "{message}");
    assert!(!app.world().resource::<Guide>().visible, "and the refusal changed nothing");
}

// ── 4. the overlay ───────────────────────────────────────────────────────────────────────────────

/// What the overlay is currently saying, as the lines a person would read.
fn on_screen(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&Text>();
    q.iter(app.world()).map(|t| t.0.clone()).collect()
}

fn shown(app: &mut App) -> bool {
    let mut q = app.world_mut().query_filtered::<&Node, With<bevy_fairy::GuideOverlay>>();
    q.iter(app.world()).any(|n| n.display != Display::None)
}

fn ui_app() -> App {
    let mut app = app();
    app.add_plugins(GuideOverlayPlugin);
    app.update();
    app
}

#[test]
fn the_overlay_says_nothing_until_there_is_a_step() {
    let mut app = ui_app();
    assert!(!shown(&mut app), "an app with the plugin on and no script shows no card");
    assert!(on_screen(&mut app).is_empty());
}

#[test]
fn the_overlay_shows_one_step_and_not_the_script() {
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.update();

    let lines = on_screen(&mut app).join("\n");
    assert!(shown(&mut app));
    assert!(lines.contains("STEP 1 OF 2"), "segmented, van der Meij: {lines}");
    assert!(lines.contains("drop a floor"));
    assert!(lines.contains("the tile needs something in it"), "the why — Choong, 13 of 20");
    assert!(lines.contains("press Enter"));
    assert!(lines.contains("the library row is empty"), "the recovery field 21 of 21 shipped \
        tutorials did not have");
    assert!(
        !lines.contains("save it"),
        "and NOT the next step. One at a time is the finding, not a layout preference: {lines}"
    );
}

#[test]
fn a_finished_step_is_confirmed_before_the_next_one_arrives() {
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();
    app.update();

    let held = on_screen(&mut app).join("\n");
    assert!(held.contains("OK"), "the beat: {held}");
    assert!(held.contains("drop a floor"), "confirming the step that just passed: {held}");
    assert!(!held.contains("save it"), "the next step has not arrived yet: {held}");

    // Past the beat, the next step appears. Stepped in slices because `MinimalPlugins` advances
    // `Time` by real elapsed time, so one long sleep is the only way to make this deterministic —
    // and a loop of updates is cheaper than a sleep.
    let start = std::time::Instant::now();
    while start.elapsed().as_secs_f32() < BEAT_SECONDS + 0.2 {
        app.update();
    }
    let next = on_screen(&mut app).join("\n");
    assert!(next.contains("save it"), "then the next step: {next}");
    assert!(!next.contains("OK"), "and the confirmation is gone: {next}");
}

#[test]
fn hiding_the_overlay_keeps_the_script() {
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.update();
    assert!(shown(&mut app));

    // Iacovides et al. 2015: experts spend attention on an overlay merely because it is there, and
    // removing it raised their sense of control. So a host can bind a dismiss key — and it must not
    // cost the script.
    post(&mut app, json!({"visible": false}));
    app.update();
    assert!(!shown(&mut app));

    let report = post(&mut app, json!({"read": true}));
    assert_eq!(report["guide"]["at"], 1, "still on step one, just not saying so");
}

/// **B-128.** A script re-posted while step 1 is up has the same step index and the same phase as
/// the one it replaced, which was the overlay's whole render key, so the card kept the old text.
#[test]
fn re_posting_a_script_while_step_one_is_up_replaces_the_card() {
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.update();
    assert!(on_screen(&mut app).join("\n").contains("drop a floor"));

    post(&mut app, json!({"steps": [{"label": "a different first step", "do": ["press Tab"]}]}));
    app.update();
    let lines = on_screen(&mut app).join("\n");
    assert!(lines.contains("a different first step"), "the card still shows the old script: {lines}");
    assert!(!lines.contains("drop a floor"), "{lines}");
}

/// **The step text has to be ASCII**, because Bevy's embedded default font is 95 codepoints and this
/// crate cannot ship one without widening the dependency list `leaf.rs` pins. An em-dash in a step
/// draws as tofu in any host that has not installed a font.
///
/// Checked on the **example**, because that is the only file in this crate that ships an actual
/// script — not at the door, since refusing a caller's non-ASCII step would be this plugin deciding
/// what font a host has installed. The example is what a reader copies, so it is what must be right.
#[test]
fn the_one_script_this_crate_ships_is_ascii() {
    let rel = "examples/plugin_guided_steps_land.rs";
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let Ok(src) = std::fs::read_to_string(root.join(rel)) else {
        panic!("{rel} is named by this test and must exist");
    };
    for (n, line) in src.lines().enumerate() {
        // A doc comment is read in an editor, not rendered by Bevy.
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with('*') {
            continue;
        }
        assert!(
            code.is_ascii(),
            "{rel}:{} has a non-ASCII character in code that may become step copy; Bevy's default \
             font has 95 codepoints and would draw it as tofu:\n  {line}",
            n + 1
        );
    }
}

/// **The host says where the card goes, and this is why.**
///
/// The plugin's default is 12 px from the top. In `bevy_emerge_mapper` that is exactly the tab bar, and
/// the first devshot capture came back with ANIM reading through STEP 1 OF 8 at 0.92 alpha — the
/// card sitting on the chrome it was supposed to sit beside. No test could have caught that, because
/// the plugin cannot know a host has a tab bar; only a frame could.
///
/// What a test *can* pin is that the host's answer is honoured, and honoured **whichever order the
/// plugins are added in**. `DebuggerPlugin` uses `init_resource`, which is insert-if-absent, so a
/// host that `insert_resource`s its own value wins from either side. Getting that backwards would put
/// the card back on the tab bar and nothing would say so.
#[test]
fn a_host_can_move_the_card_off_its_own_chrome() {
    use bevy_fairy::GuidePlacement;

    let mut app = app();
    // The host speaks first, as `bevy_emerge_mapper`'s GuidePlugin does.
    app.insert_resource(GuidePlacement { top: 58.0, width: 520.0 });
    app.add_plugins(GuideOverlayPlugin);
    app.update();

    assert_eq!(
        app.world().resource::<GuidePlacement>().top,
        58.0,
        "GuideOverlayPlugin's init_resource must not overwrite a host's value"
    );

    post(&mut app, script(json!("ready")));
    app.update();

    let mut q = app
        .world_mut()
        .query_filtered::<&Node, With<bevy_fairy::GuideOverlay>>();
    let node = q.iter(app.world()).next().cloned().expect("the overlay exists");
    assert_eq!(node.top, Val::Px(58.0), "the card moved to where the host asked");
    assert_eq!(node.width, Val::Px(520.0));
    assert_eq!(
        node.margin.left,
        Val::Px(-260.0),
        "and stayed centred: the pull-back is half the width, not a constant that only matched the \
         default"
    );
}

/// **A skipped step is not an OK, and the card has to say so.**
///
/// It said `OK <step>` in green for both outcomes, and every test here passed while it did: both
/// branches rendered text, and nothing asserted *which*. A devshot capture of the real editor is what
/// caught it — three of four frames showed a green tick over a step the person had just told the
/// script did not work.
///
/// Which is the failure the escape hatch exists to prevent, running backwards. Choong et al. found 18
/// of 20 participants followed an AI suggestion that made no sense rather than report it; `skip` is
/// how this channel lets them report it, and congratulating them for it is a way of teaching them not
/// to bother.
#[test]
fn a_skipped_step_is_not_confirmed_as_a_pass() {
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));

    post(&mut app, json!({"skip": true}));
    app.update();
    let skipped = on_screen(&mut app).join("\n");
    assert!(skipped.contains("SKIPPED"), "says what actually happened: {skipped}");
    assert!(skipped.contains("drop a floor"), "and to which step: {skipped}");
    assert!(!skipped.contains("OK"), "and does NOT congratulate them for it: {skipped}");

    // The passing branch is unchanged, and asserted here too so the two cannot converge again.
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.world_mut().resource_mut::<Ready>().0 = true;
    app.update();
    app.update();
    let passed = on_screen(&mut app).join("\n");
    assert!(passed.contains("OK"), "a real pass still reads as one: {passed}");
    assert!(!passed.contains("SKIPPED"), "{passed}");
}

/// **A `read` has to answer what the stream announced once, or a reconnecting client is blind.**
///
/// `waiting_on_a_person` is sent on the watch stream exactly once per step — it has to be, or a step
/// with no checkpoint pushes sixty frames a second. The consequence nobody noticed until it happened:
/// a client that attaches *after* that announcement sees silence, and silence is also what an unmet
/// checkpoint looks like. It waits on a machine that is waiting on it.
///
/// This was hit for real, driving a guided run: the watcher process died, the replacement attached
/// mid-script, and the run looked stalled when it was in fact holding for a human call.
///
/// Re-announcing on the stream is not the fix — there is no per-request identity to key it on, and
/// per-frame is the flood the latch exists to stop. The fix is that the *state* query answers it.
#[test]
fn a_read_says_when_the_current_step_is_waiting_on_a_person() {
    let mut app = app();
    post(&mut app, script(Value::Null));

    // Consume the one announcement, the way a watcher that later dies would have.
    assert_eq!(watch(&mut app).expect("announced once")["waiting_on_a_person"], true);
    assert_eq!(watch(&mut app), None, "and the stream now says nothing at all");

    // A client attaching now has only this to go on, and it has to be enough.
    let report = post(&mut app, json!({"read": true}))["guide"].clone();
    assert_eq!(report["waiting_on_a_person"], true, "the read still knows: {report}");
    assert_eq!(report["step"], "drop a floor", "and which step it is: {report}");

    // A step with a checkpoint is the other answer, and must not read as waiting.
    post(&mut app, script(json!("ready")));
    let report = post(&mut app, json!({"read": true}))["guide"].clone();
    assert_eq!(
        report["waiting_on_a_person"], false,
        "this one is waiting on a condition, which is not the same thing: {report}"
    );
    assert_eq!(report["step"], "drop a floor");
}

/// **The card says when the step is waiting on the person.**
///
/// A step with no checkpoint never advances on its own -- that is the whole point of it, the question
/// no machine can answer. Until this line existed its card was indistinguishable from a card watching
/// a condition, so the reasonable reading was "the editor has not noticed yet", and an author sat in
/// front of a step that was sitting in front of them.
///
/// Found from the keyboard, mid-run, by somebody who had been told in advance that this step would
/// need their call and *still* could not tell that it was the moment. The fact was on the wire the
/// whole time; it was never anywhere a person looks. That is the same failure as
/// `a_read_says_when_the_current_step_is_waiting_on_a_person`, one layer further out -- which is why
/// it is worth pinning twice.
#[test]
fn the_card_says_when_a_step_is_waiting_on_the_person() {
    let mut app = ui_app();
    post(&mut app, script(Value::Null));
    app.update();

    let lines = on_screen(&mut app).join("\n");
    assert!(lines.contains("drop a floor"), "the step is up: {lines}");
    assert!(
        lines.contains("yours to judge"),
        "the card says it is waiting on a person, in words, on the screen: {lines}"
    );
    assert!(
        lines.contains("no key advances this step"),
        "and says no key will do it, because this crate binds none -- an author hunting for the \
         key that does not exist is worse off than one told plainly: {lines}"
    );

    // A step with a checkpoint must NOT say it -- a prompt that appears on every step is one nobody
    // reads by the third (docs/ui.md 3.4, the alert budget).
    let mut app = ui_app();
    post(&mut app, script(json!("ready")));
    app.update();
    let lines = on_screen(&mut app).join("\n");
    assert!(lines.contains("drop a floor"));
    assert!(
        !lines.contains("yours to judge"),
        "this one is watched, so saying otherwise would be a lie every third step: {lines}"
    );
}
