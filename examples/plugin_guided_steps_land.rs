//! **An agent can tell a person what to try — shown in a terminal, no GPU.**
//!
//! This crate could already put input *into* an app and get frames *out* of it. What it could not do
//! was say a sentence to the person at the keyboard, so every instruction went to a terminal they had
//! to look away from their work to read, and every answer came back as prose the agent then had to
//! guess its way from.
//!
//! `bevy_fairy/guide` posts a script; the app renders one step; `bevy_fairy/guide+watch` waits
//! on a condition the *host* named, advances itself, and records what actually happened.
//!
//! Run it:
//!
//! ```sh
//! cargo run --features plugin-guide --example plugin_guided_steps_land
//! ```
//!
//! There is no window, no socket and no person: the two handlers are ordinary Bevy systems, and a
//! stand-in "author" below satisfies each condition a few frames after it is asked for. What the run
//! prints is the transcript an agent would receive.
//!
//! Four things to look for:
//!
//! 1. **The overlay shows one step**, never the script. Andersen et al. 2012 (CHI, N = 45,318) found
//!    instructions delivered in context beat an up-front manual by 40% progress in the complex,
//!    unconventional interface -- which is what an editor is.
//! 2. **A step is Carroll's guided-exploration card**: a label, why it is being asked, two to four
//!    hints, a checkpoint, and what to do when it does not happen. Chauvergne et al. 2023 could not
//!    find that last field in a single one of twenty-one shipped tutorials.
//! 3. **The watch stream answers only when something happens.** While the condition is unmet the
//!    handler returns `Ok(None)` and the engine parks the request -- the frames printed as `.` below.
//! 4. **The transcript is `k/n`, never a boolean.** Bryant, *Game Testing All in One* 4e: a tester who
//!    ran the steps twice and saw it twice will report 100%, and it is just as likely to be 50%.

use bevy::prelude::*;
use bevy_fairy::{handle_guide, watch_guide, Checkpoints, Guide, GuideOverlayPlugin};
use bevy::ecs::system::RunSystemOnce;
use serde_json::{json, Value};

/// The host's own state. This crate never learns what any of it means -- it only runs the conditions
/// the host registered, by the names the host chose.
#[derive(Resource, Default)]
struct Tile {
    members: usize,
    saved: bool,
}

fn main() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        // In a real host this is `DebuggerPlugin`, which inits both resources and adds the overlay.
        // Spelled out here so the example shows what a host actually needs.
        .init_resource::<Guide>()
        .init_resource::<Checkpoints>()
        .add_plugins(GuideOverlayPlugin)
        .init_resource::<Tile>();

    // The seam: one-shot systems answering `bool`, registered under the host's own words.
    // **Every checkpoint takes `In<Value>`, and a step supplies it with `with`.** One shape, so a
    // condition can always be as specific as the step that claims it -- a checkpoint too vague for
    // its step is how a transcript comes to report a pass for work nobody did.
    let has_n = app.register_system(|args: In<Value>, tile: Res<Tile>| {
        let n = args.0.get("n").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        tile.members >= n
    });
    let is_saved = app.register_system(|_: In<Value>, tile: Res<Tile>| tile.saved);
    {
        let mut checkpoints = app.world_mut().resource_mut::<Checkpoints>();
        checkpoints.register("tile has members", has_n);
        checkpoints.register("tile is saved", is_saved);
    }

    // The script. Note the ordering: the step most likely to go wrong is first, because van der Meij
    // & van der Meij measured coverage decaying down a procedure -- a preview line holds 90-98% while
    // everything below it drops toward a 70% mean.
    let script = json!({"steps": [
        {
            "label": "drop a floor and a wall",
            "goal": "a tile needs at least two pieces before the solver has anything to match on",
            "do": [
                "walk the library with up and down",
                "press Enter to bring the selected row in",
                "press Enter again for a second piece"
            ],
            "checkpoint": "tile has members",
            "with": {"n": 2},
            "recovery": "if nothing lands, the library filter is hiding every row: press Backspace"
        },
        {
            "label": "does the wall sit flush",
            "goal": "the envelope is derived from the contents, so a gap here is a real gap",
            "do": ["look at the tile from the front"],
            "recovery": "shift and an arrow puts the focused piece against that side"
        },
        {
            "label": "save it",
            "goal": "so the next step reads it back off disk rather than out of memory",
            "do": ["press Cmd+S"],
            "checkpoint": "tile is saved",
            "recovery": "if the panel says nothing, the tile has no id yet: press N and name it"
        }
    ]});

    println!("POST bevy_fairy/guide");
    let posted = call(&mut app, handle_guide, Some(script));
    println!("  {}\n", posted["message"]);

    println!("the app is showing:");
    app.update();
    for line in on_screen(&mut app) {
        println!("  | {line}");
    }
    println!();

    println!("GET bevy_fairy/guide+watch   (one row per frame; `.` means parked)");
    // A stand-in for the person at the keyboard: does what the current step asks, a few frames late.
    let mut parked = 0;
    for frame in 0..40 {
        match watch(&mut app) {
            None => {
                parked += 1;
                print!(".");
            }
            Some(answer) => {
                if parked > 0 {
                    println!();
                }
                parked = 0;
                report(&answer);
                if answer["done"] == json!(true) {
                    break;
                }
                // The one step no machine can judge gets skipped here, which is what a person saying
                // "yes that looks right" does through `bevy_fairy/guide`.
                if answer["waiting_on_a_person"] == json!(true) {
                    println!("     (a person judges this one; sending skip)");
                    call(&mut app, handle_guide, Some(json!({"skip": true})));
                }
            }
        }
        act(&mut app, frame);
        app.update();
    }
    println!();

    let transcript = call(&mut app, handle_guide, Some(json!({"read": true})));
    println!("the transcript an agent gets back:\n");
    println!("  {:<26} {:>5}  {:>7}", "step", "k/n", "seconds");
    let empty = vec![];
    for row in transcript["guide"]["steps"].as_array().unwrap_or(&empty) {
        let label = row["step"].as_str().unwrap_or("?");
        let kn = format!("{}/{}", row["passes"], row["runs"]);
        let secs = row["seconds"].as_f64().unwrap_or(0.0);
        println!("  {label:<26} {kn:>5}  {secs:>7.2}");
    }
    println!(
        "\n  A step showing 0/1 is not a failure to report -- it is the one that needs a person's\n  \
         judgement, or the one whose instruction made no sense. Both are findings."
    );
}

/// The stand-in author. Each condition becomes true a few frames after its step comes up, so the
/// parked frames in the output are real waiting rather than a formality.
fn act(app: &mut App, frame: usize) {
    match frame {
        3 => app.world_mut().resource_mut::<Tile>().members = 1,
        5 => app.world_mut().resource_mut::<Tile>().members = 2,
        12 => app.world_mut().resource_mut::<Tile>().saved = true,
        _ => {}
    }
}

fn report(answer: &Value) {
    if let Some(passed) = answer["passed"].as_str() {
        println!("  PASS {passed}");
    } else if answer["waiting_on_a_person"] == json!(true) {
        println!("  ASK  {}", answer["step"].as_str().unwrap_or("?"));
    } else if answer["done"] == json!(true) {
        println!("  DONE");
    }
}

fn call<M>(
    app: &mut App,
    handler: impl IntoSystem<In<Option<Value>>, bevy::remote::BrpResult, M>,
    params: Option<Value>,
) -> Value {
    match app.world_mut().run_system_once_with(handler, params) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            println!("  refused: {}", e.message);
            Value::Null
        }
        Err(e) => {
            println!("  could not run: {e}");
            Value::Null
        }
    }
}

fn watch(app: &mut App) -> Option<Value> {
    match app.world_mut().run_system_once_with(watch_guide, None) {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            println!("  refused: {}", e.message);
            None
        }
        Err(e) => {
            println!("  could not run: {e}");
            None
        }
    }
}

fn on_screen(app: &mut App) -> Vec<String> {
    let mut q = app.world_mut().query::<&Text>();
    q.iter(app.world()).map(|t| t.0.clone()).collect()
}
