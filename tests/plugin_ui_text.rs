//! **`bevy_fairy/ui_text` reports what the player can read** (issue #8).
//!
//! A real `bevy_ui` tree is laid out under the real UI plugins (no GPU, no OS window), and the
//! command is called through `RemoteMethods` the way the BRP server calls it. The tree is the one
//! the issue describes: a prompt and a toast that say the same sentence, a toast slot that expired
//! (`Display::None`) and kept its old string with `InheritedVisibility` still true, and a node
//! pushed out of an `Overflow::clip` ancestor. Every expectation below is a constant of that tree.

use bevy::prelude::*;
use bevy::remote::{BrpResult, RemoteMethodSystemId, RemoteMethods};
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy_fairy::DebuggerPlugin;
use serde_json::{json, Value};

#[path = "helpers/headless.rs"]
mod headless;

const SENTENCE: &str = "No stone on N1: Couriers carry them up by day";

fn label(parent: &mut bevy::ecs::relationship::RelatedSpawner<ChildOf>, name: &str, text: &str, left: i32, top: i32) {
    parent.spawn((
        Name::new(name.to_string()),
        Text::new(text),
        Node {
            position_type: PositionType::Absolute,
            left: px(left),
            top: px(top),
            width: px(200),
            height: px(40),
            ..default()
        },
    ));
}

fn app(ui_scale: f32) -> App {
    let mut app = App::new();
    app.add_plugins(
        headless::without_os_window(DefaultPlugins.build())
            .set(RenderPlugin {
                render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                    backends: None,
                    ..default()
                })),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    resolution: (1000, 600).into(),
                    ..default()
                }),
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            }),
    )
    .add_plugins(DebuggerPlugin::default())
    .insert_resource(UiScale(ui_scale));

    app.world_mut().spawn(Camera2d);
    app.world_mut()
        .spawn((
            Name::new("HUD"),
            Node {
                width: percent(100),
                height: percent(100),
                ..default()
            },
        ))
        .with_children(|hud| {
            // Prompt: visible, at (100, 100).
            hud.spawn((
                Name::new("PromptRoot"),
                Node { position_type: PositionType::Absolute, ..default() },
            ))
            .with_children(|p| label(p, "PromptLabel", SENTENCE, 100, 100));
            // Toast that is up: the same sentence, 300 px lower.
            label(hud, "ToastLive", SENTENCE, 100, 400);
            // Toast slot that expired: Display::None, string kept, visibility untouched.
            hud.spawn((
                Name::new("ToastExpired"),
                Node { display: Display::None, ..default() },
            ))
            .with_children(|p| label(p, "ToastText", SENTENCE, 500, 100));
            // A pooled label that never had text.
            label(hud, "PooledEmpty", "", 700, 100);
            // A 100 x 20 clip box with a label shoved 300 px outside it.
            hud.spawn((
                Name::new("ClipBox"),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(500),
                    top: px(300),
                    width: px(100),
                    height: px(20),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ))
            .with_children(|p| label(p, "ClippedAway", "you cannot read me", 300, 0));
            // Fully transparent text.
            hud.spawn((
                Name::new("Ghost"),
                Text::new("faded out"),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.0)),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(700),
                    top: px(500),
                    width: px(200),
                    height: px(40),
                    ..default()
                },
            ));
        });

    app.finish();
    app.cleanup();
    for _ in 0..4 {
        app.update();
    }
    app
}

fn call(app: &mut App, params: Value) -> BrpResult {
    let id = match app.world().resource::<RemoteMethods>().get("bevy_fairy/ui_text") {
        Some(RemoteMethodSystemId::Instant(id)) => *id,
        other => panic!("bevy_fairy/ui_text is not an instant method: {}", other.is_some()),
    };
    app.world_mut().run_system_with(id, Some(params)).unwrap()
}

fn rows(reply: &Value) -> Vec<&Value> {
    reply["nodes"].as_array().unwrap().iter().collect()
}

fn names(reply: &Value) -> Vec<&str> {
    rows(reply).iter().map(|n| n["name"].as_str().unwrap()).collect()
}

#[test]
fn only_what_can_be_read_is_returned_by_default() {
    let mut app = app(1.0);
    let reply = call(&mut app, json!({})).unwrap();
    // The expired slot, the empty label, the clipped label and the transparent one are all `Text`
    // nodes with `InheritedVisibility` true, and none of them is on screen.
    assert_eq!(names(&reply), ["PromptLabel", "ToastLive"], "{reply}");
    assert_eq!(reply["truncated"], json!(false));
    assert_eq!(reply["target_size"], json!([1000, 600]));
    assert_eq!(reply["scale_factor"], json!(1.0));
}

#[test]
fn the_same_sentence_twice_is_two_rows_with_their_own_rects() {
    let mut app = app(1.0);
    let reply = call(&mut app, json!({"contains": SENTENCE})).unwrap();
    let rows = rows(&reply);
    assert_eq!(rows.len(), 2, "{reply}");
    assert_ne!(rows[0]["entity"], rows[1]["entity"]);
    assert_eq!(rows[0]["rect"], json!([100, 100, 200, 40]));
    assert_eq!(rows[1]["rect"], json!([100, 400, 200, 40]));
    // Sorted top to bottom, and the nearest named ancestors come back nearest first.
    assert_eq!(rows[0]["ancestors"], json!(["PromptRoot", "HUD"]));
    assert_eq!(rows[1]["ancestors"], json!(["HUD"]));
}

#[test]
fn hidden_nodes_come_back_on_request_with_the_reason() {
    let mut app = app(1.0);
    let reply = call(&mut app, json!({"include_hidden": true})).unwrap();
    let why = |name: &str| -> String {
        let row = rows(&reply)
            .into_iter()
            .find(|n| n["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing: {reply}"));
        assert_eq!(row["visible"], json!(false), "{name}: {row}");
        row["hidden_because"].as_str().unwrap().to_string()
    };
    assert!(why("ToastText").starts_with("zero_size"));
    assert_eq!(why("PooledEmpty"), "empty_text");
    assert_eq!(why("ClippedAway"), "clipped_by_ancestor_overflow");
    assert_eq!(why("Ghost"), "text_alpha_zero");
    assert_eq!(rows(&reply).len(), 6, "{reply}");
}

#[test]
fn a_region_keeps_the_node_it_covers_and_not_one_300px_away() {
    let mut app = app(1.0);
    let reply = call(
        &mut app,
        json!({"region": {"x": 100, "y": 100, "width": 200, "height": 40}}),
    )
    .unwrap();
    assert_eq!(names(&reply), ["PromptLabel"], "{reply}");
}

#[test]
fn rect_is_in_physical_pixels_under_ui_scale() {
    let mut app = app(2.0);
    let reply = call(&mut app, json!({"contains": SENTENCE})).unwrap();
    // `px(100)` at UiScale 2 is 200 physical pixels, and the target is still 1000 x 600.
    assert_eq!(rows(&reply)[0]["rect"], json!([200, 200, 400, 80]), "{reply}");
    assert_eq!(reply["target_size"], json!([1000, 600]));
}

#[test]
fn limit_is_applied_after_sorting_and_says_so() {
    let mut app = app(1.0);
    let reply = call(&mut app, json!({"limit": 1})).unwrap();
    assert_eq!(names(&reply), ["PromptLabel"]);
    assert_eq!(reply["truncated"], json!(true));
    assert_eq!(reply["matched"], json!(2));
}

/// `region.x + region.width` was summed in `u32`: a region reaching past `u32::MAX` panicked the
/// game in a debug build and wrapped to an empty region in a release build, where `matched: 0` and
/// `nodes: []` read as "no text on screen". The rect is a rect, wherever its far edge falls.
#[test]
fn a_region_reaching_past_the_u32_range_is_a_region_not_an_overflow() {
    let mut app = app(1.0);
    let everything = call(&mut app, json!({})).unwrap();
    assert!(!rows(&everything).is_empty(), "the tree has readable text: {everything}");

    let huge = call(&mut app, json!({"region": {"x": 1, "y": 1, "width": u32::MAX, "height": u32::MAX}})).unwrap();
    assert_eq!(names(&huge), names(&everything), "a region that covers the whole target keeps every node: {huge}");

    let beyond = call(&mut app, json!({"region": {"x": u32::MAX, "y": 0, "width": 2, "height": 10}})).unwrap();
    assert_eq!(beyond["matched"], json!(0), "nothing is that far right: {beyond}");
}

#[test]
fn bad_params_are_refused_not_defaulted() {
    let mut app = app(1.0);
    assert!(call(&mut app, json!({"limit": 0})).is_err());
    assert!(call(&mut app, json!({"region": {"x": 0, "y": 0, "width": 0, "height": 5}})).is_err());
    assert!(call(&mut app, json!({"visible": true})).is_err(), "unknown field must be refused");
}
