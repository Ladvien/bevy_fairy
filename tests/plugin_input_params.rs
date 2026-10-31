//! **`bevy_fairy/input` refuses what it would otherwise ignore.**
//!
//! The method used to accept any field it did not know and any field the chosen `kind` does not
//! read, and answer `success: true`: `{"kind":"Keyboard","key":"KeyS","modifiers":["ControlLeft"]}`
//! tapped a plain S, and `{"kind":"Mouse","button":"Left","x":300,"y":200}` clicked wherever the
//! cursor already was. An agent that believes its modifier or its position was honoured has no way
//! to learn otherwise, so a field that cannot change the result is a refusal that names it.

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use bevy::remote::BrpResult;
use bevy::window::PrimaryWindow;
use bevy_fairy::{handle_input, PendingInput};
use serde_json::{json, Value};

/// An app with what the handler reads: the pending queue and a primary window (a scroll is
/// addressed to one).
fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).init_resource::<PendingInput>();
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app
}

fn input(app: &mut App, params: Value) -> BrpResult {
    app.world_mut().run_system_once_with(handle_input, Some(params)).expect("the handler runs")
}

/// The refusal's message; fails when the call was accepted.
fn refused(app: &mut App, params: Value) -> String {
    match input(app, params.clone()) {
        Err(error) => error.message,
        Ok(reply) => panic!("{params} was accepted ({reply}) though a field of it changes nothing"),
    }
}

fn accepted(app: &mut App, params: Value) {
    if let Err(error) = input(app, params.clone()) {
        panic!("{params} is a well-formed call and was refused: {}", error.message);
    }
}

#[test]
fn a_field_the_method_does_not_know_is_refused_by_name() {
    let mut app = app();
    for (params, field) in [
        (json!({"kind": "Keyboard", "key": "KeyS", "modifiers": ["ControlLeft"]}), "modifiers"),
        (json!({"kind": "Keyboard", "key": "KeyS", "ctrl": true}), "ctrl"),
        (json!({"kind": "Mouse", "button": "Left", "buton": "Right"}), "buton"),
        (json!({"kind": "Scroll", "y": 1.0, "hold": 3}), "hold"),
    ] {
        let message = refused(&mut app, params);
        assert!(message.contains(field), "the refusal names `{field}`: {message}");
    }
}

#[test]
fn a_field_that_another_kind_reads_is_refused_by_name_and_says_what_the_kind_reads() {
    let mut app = app();
    for (params, field) in [
        (json!({"kind": "Mouse", "button": "Left", "x": 300, "y": 200}), "x"),
        (json!({"kind": "Mouse", "button": "Left", "key": "KeyA"}), "key"),
        (json!({"kind": "Keyboard", "key": "KeyA", "button": "Left"}), "button"),
        (json!({"kind": "Keyboard", "text": "hi", "x": 1.0}), "x"),
        (json!({"kind": "Cursor", "x": 1.0, "y": 2.0, "button": "Left"}), "button"),
        (json!({"kind": "Scroll", "y": 1.0, "key": "KeyA"}), "key"),
        (json!({"kind": "Scroll", "y": 1.0, "action": "Press"}), "action"),
    ] {
        let message = refused(&mut app, params.clone());
        assert!(message.contains(&format!("`{field}`")), "the refusal names `{field}`: {message}");
        let kind = params["kind"].as_str().expect("kind");
        assert!(message.contains(kind), "and the kind that does not read it: {message}");
    }
    // The case an agent is most likely to write: a click is not aimed by the click.
    let message = refused(&mut app, json!({"kind": "Mouse", "button": "Left", "x": 300, "y": 200}));
    assert!(message.contains("Cursor"), "and where the position goes instead: {message}");
}

#[test]
fn a_scroll_with_no_amount_is_refused_instead_of_scrolling_by_nothing() {
    let mut app = app();
    for params in [json!({"kind": "Scroll"}), json!({"kind": "Scroll", "unit": "Pixel"})] {
        let message = refused(&mut app, params);
        assert!(message.contains('x') && message.contains('y'), "it says which fields carry the amount: {message}");
    }
}

/// The calls the README documents, and the quiet ones the refusals above must not catch.
#[test]
fn well_formed_calls_of_every_kind_are_still_accepted() {
    let mut app = app();
    for params in [
        json!({"kind": "Keyboard", "key": "Space"}),
        json!({"kind": "Keyboard", "key": "Space", "action": "Press"}),
        json!({"kind": "Keyboard", "text": "site_67"}),
        json!({"kind": "Cursor", "x": 640, "y": 360}),
        // A position has no press, and `action` is ignored by design (see `InputCommand::action`).
        json!({"kind": "Cursor", "x": 640, "y": 360, "action": "Tap"}),
        json!({"kind": "Cursor", "clear": true}),
        json!({"kind": "Mouse", "button": "Left", "action": "Press"}),
        json!({"kind": "Mouse", "button": {"Other": 3}}),
        json!({"kind": "Scroll", "y": -2.0}),
        json!({"kind": "Scroll", "x": 1.0, "unit": "Pixel"}),
    ] {
        accepted(&mut app, params);
    }
}
