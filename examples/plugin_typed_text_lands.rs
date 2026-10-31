//! **An agent can type — shown in a terminal, no GPU.**
//!
//! For as long as this crate existed it could press keys and not type into them, and the difference
//! was invisible from the outside because `bevy_fairy/input` answered `success: true` either way.
//!
//! The cause was that it wrote `ButtonInput` — Bevy's *fold* of the input stream — rather than the
//! stream. Everything reading `ButtonInput` saw injected keys; **every text field in every Bevy
//! application** reads `MessageReader<KeyboardInput>` and matches on `logical_key`, and saw nothing.
//! Not just letters: `Enter` and `Escape` are read off `logical_key` too, so a field could not even be
//! committed or cancelled. Three separate verifications were blocked by this in one day.
//!
//! The system below is the shape a real text field has — the same shape ten handlers in
//! `bevy_grimoire_mapper` have. It is driven here with no MCP server, no network and no window.
//!
//! Run it:
//!
//! ```sh
//! cargo run -p bevy_fairy --example typed_text_lands
//! ```
//!
//! Four things to look for, and the last two are the ones that cost something to learn:
//!
//! 1. **A whole word arrives in one frame.** A stream is not a state, so there is no edge to lose and
//!    nothing to spread out — naming a composition is one call, not one call per letter.
//! 2. **`Enter` in the same batch commits it**, because nothing defers behind text.
//! 3. **A space arrives as `Key::Space`, not `Key::Character(" ")`** — the one character with a named
//!    variant, and the one a text field would otherwise drop while the call reported success.
//! 4. **Text queued alongside the key that *opens* a field waits a frame.** Fields drain the stream
//!    while shut so the opening keystroke cannot become their first character; sent together, the
//!    text would be eaten by that guard and the method would still report success.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy_fairy::{InputAction, PendingInput};

/// A text field with the shape every real one has.
#[derive(Resource, Default)]
struct Field {
    open: bool,
    value: String,
    committed: Option<String>,
}

/// One line per frame, so the run can be printed as a table.
#[derive(Resource, Default)]
struct Log(Vec<String>);

fn main() {
    let mut app = App::new();
    // `MinimalPlugins` gives a schedule and nothing else — no window, no renderer. `InputPlugin` is
    // added on its own because it owns the systems that fold this crate's messages into `ButtonInput`;
    // without it the injection is accepted and read by nobody, which is what `DebuggerPlugin::finish`
    // asserts against.
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::input::InputPlugin)
        .init_resource::<PendingInput>()
        .init_resource::<bevy_fairy::DebugCursor>()
        .init_resource::<Field>()
        .init_resource::<Log>()
        .add_systems(
            PreUpdate,
            // Before, not after: the messages have to be in the stream when Bevy reads it.
            bevy_fairy::apply_pending_input.before(InputSystems),
        )
        .add_systems(Update, field_keys);

    // Frame 1 — the field is shut, and the key that opens it is queued together with the text. The
    // text must NOT land yet.
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_key(KeyCode::KeyM, InputAction::Tap);
        if let Err(ch) = pending.queue_text("porch a", Entity::PLACEHOLDER) {
            println!("  no logical key exists for {ch:?}");
            return;
        }
    }
    app.update(); // opens the field; the text is held back
    app.update(); // the text arrives

    // Then commit it — in one batch with nothing before it, so it lands the same frame.
    app.world_mut()
        .resource_mut::<PendingInput>()
        .queue_key(KeyCode::Enter, InputAction::Tap);
    app.update();

    println!("\n  frame  what the field saw");
    println!("  -----  ------------------");
    for (n, line) in app.world().resource::<Log>().0.iter().enumerate() {
        println!("  {:>5}  {line}", n + 1);
    }

    let field = app.world().resource::<Field>();
    println!(
        "\n  The opening keystroke and the text were queued together; the text waited a frame,\n  \
         because a field drains the stream while shut and would have eaten it.\n  \
         `porch a` then arrived whole, in ONE frame, space included as `Key::Space`.\n"
    );

    // An example that only prints is an example that can rot. This is the same reason
    // `cursor_drag_lands` asserts: it found a real ordering bug the day it landed.
    assert_eq!(
        field.committed.as_deref(),
        Some("porch a"),
        "the whole name must have been typed and committed"
    );
    println!("  committed: {:?}  — asserted, so this example fails loudly if it rots\n", field.committed);
}

/// The shape of a real text field: read the stream, match `logical_key`, ignore releases.
fn field_keys(
    mut events: MessageReader<KeyboardInput>,
    mut field: ResMut<Field>,
    mut log: ResMut<Log>,
) {
    // **Drained even while shut**, so the keystroke that opens the field cannot become its first
    // character. This guard is why text queued in the same frame as the opening key has to wait.
    if !field.open {
        let opened = events.read().any(|e| {
            e.state.is_pressed() && matches!(e.logical_key, Key::Character(ref s) if s == "m")
                || e.state.is_pressed() && e.key_code == KeyCode::KeyM
        });
        if opened {
            field.open = true;
            log.0.push("field opened (stream drained, nothing typed)".to_owned());
        } else {
            log.0.push("shut".to_owned());
        }
        return;
    }

    let mut arrived = Vec::new();
    for event in events.read() {
        if !event.state.is_pressed() {
            continue;
        }
        match &event.logical_key {
            Key::Character(text) => {
                field.value.push_str(text);
                arrived.push(text.to_string());
            }
            // Its own arm, not `Character(" ")` — a space bar produces this on every layout.
            Key::Space => {
                field.value.push(' ');
                arrived.push("Space".to_owned());
            }
            Key::Backspace => {
                field.value.pop();
                arrived.push("Backspace".to_owned());
            }
            Key::Enter => {
                field.committed = Some(field.value.clone());
                field.open = false;
                arrived.push("Enter".to_owned());
            }
            Key::Escape => {
                field.value.clear();
                field.open = false;
                arrived.push("Escape".to_owned());
            }
            _ => {}
        }
    }
    log.0.push(if arrived.is_empty() {
        format!("open, nothing arrived  (value {:?})", field.value)
    } else {
        format!("{}  (value {:?})", arrived.join(" "), field.value)
    });
}
