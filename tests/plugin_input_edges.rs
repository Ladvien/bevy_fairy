//! **The injected-input contract, pinned.**
//!
//! Injected input has one job: be indistinguishable from a real key. That is entirely a question of
//! *when* the write happens and *how many* writes land in one frame, so none of it is visible to a
//! test that only calls `ButtonInput::press` directly — the bug this file guards against was invisible
//! to exactly that kind of test for as long as it existed.
//!
//! Three properties are pinned here:
//!
//! 1. **Ordering.** `apply_pending_input` must run **before** [`InputSystems`], because it writes the
//!    `KeyboardInput`/`MouseButtonInput` messages that `keyboard_input_system` clears-then-folds into
//!    `ButtonInput`. Written after, every edge arrives a frame late.
//!
//!    This reverses an earlier fix, and the fact underneath is unchanged: the clear happens at the top
//!    of `PreUpdate`. When this module wrote `ButtonInput` **directly**, the write had to land after
//!    the clear or it was erased — the original bug, where the BRP method answered `success: true` and
//!    the game never moved. Now that it writes the *source* instead of the fold, it has to land before
//!    the read. Same line in the engine, opposite side.
//! 2. **One command per key per frame.** BRP drains a whole burst of requests into one handler run, and
//!    `ButtonInput` can only show two transitions per frame, so applying a burst at once silently
//!    destroys edges.
//! 3. **Typing.** A whole string is one frame's worth of messages, every character releases, and text
//!    waits a frame behind the keystroke that opened the field it is going into.

use bevy::input::keyboard::{Key, KeyboardInput, NativeKey};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy_fairy::{apply_pending_input, typed, DebugCursor, InputAction, PendingInput};

/// What an ordinary `Update` system saw, one entry per frame.
#[derive(Resource, Default)]
struct Seen(Vec<Edges>);

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Edges {
    just_pressed: bool,
    pressed: bool,
    just_released: bool,
}

fn observe(keys: Res<ButtonInput<KeyCode>>, mut seen: ResMut<Seen>) {
    seen.0.push(Edges {
        just_pressed: keys.just_pressed(KeyCode::KeyQ),
        pressed: keys.pressed(KeyCode::KeyQ),
        just_released: keys.just_released(KeyCode::KeyQ),
    });
}

/// An app wired the way [`DebuggerPlugin`](bevy_fairy::DebuggerPlugin) wires it.
fn app_correctly_ordered() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        // Not optional, and no longer merely convenient: `ButtonInput` is now Bevy's fold of the
        // message stream this crate writes, and `InputPlugin` owns the systems that do the folding.
        // `DebuggerPlugin::finish` asserts its presence for the same reason.
        .add_plugins(bevy::input::InputPlugin)
        .init_resource::<PendingInput>()
        // `apply_pending_input` writes the injected pointer here, and in Bevy 0.19 a missing
        // `ResMut<T>` panics the system rather than skipping it. `DebuggerPlugin` inits this; a host
        // that registers the system by hand — as these tests do — has to init it too.
        .init_resource::<bevy_fairy::DebugCursor>()
        .init_resource::<Seen>()
        .add_systems(PreUpdate, apply_pending_input.before(InputSystems))
        .add_systems(Update, observe);
    app
}

/// What an ordinary `Update` system read off the *stream*, one entry per frame.
///
/// Separate from [`Seen`] because it is a different question: `ButtonInput` is a state and this is the
/// sequence, and text is only legible in the sequence — `ButtonInput<Key>` collapses a repeated
/// character, so `wall` would show one `l`.
#[derive(Resource, Default)]
struct Heard(Vec<Vec<Key>>);

fn listen(mut events: MessageReader<KeyboardInput>, mut heard: ResMut<Heard>) {
    heard.0.push(
        events
            .read()
            .filter(|e| e.state.is_pressed())
            .map(|e| e.logical_key.clone())
            .collect(),
    );
}

/// [`app_correctly_ordered`] plus a reader of the message stream itself.
fn app_listening() -> App {
    let mut app = app_correctly_ordered();
    app.init_resource::<Heard>().add_systems(Update, listen);
    app
}

/// The characters a `Key::Character` sequence spells, for readable assertions.
fn spelled(frame: &[Key]) -> String {
    frame
        .iter()
        .map(|k| match k {
            Key::Character(s) => s.to_string(),
            Key::Space => " ".to_owned(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// **Queued is not applied, and the counter says which.** A caller whose injections are never folded
/// -- the failure the plugin asserts about for a missing `InputPlugin` -- could not tell the two apart,
/// because `bevy_debugger/input` answers `success: true` at the queue. One tap is a press written on
/// the first frame and a release written on the second: two, and nothing at all if the system is not
/// running. Measured in Sanctuary, where this turned "no key arrives" into "the key arrives and is
/// dropped downstream".
#[test]
fn one_tap_reports_two_applied_writes_and_none_waiting() {
    let mut app = app_correctly_ordered();
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    app.update();
    app.update();
    app.update();
    let pending = app.world().resource::<PendingInput>();
    assert_eq!(
        pending.applied, 2,
        "a tap writes a press and then a release, and both are counted"
    );
    assert_eq!(pending.queued(), 0, "and nothing is left waiting");
    assert_eq!(
        pending.applied_this_frame, 0,
        "the third frame wrote nothing, so this frame's count is zero"
    );
}

fn queue_key(app: &mut App, key: KeyCode, action: InputAction) {
    app.world_mut()
        .resource_mut::<PendingInput>()
        .queue_key(key, action);
}

fn seen(app: &App) -> Vec<Edges> {
    app.world().resource::<Seen>().0.clone()
}

#[test]
fn a_tap_presses_for_one_frame_and_releases_the_next() {
    let mut app = app_correctly_ordered();
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    app.update();
    app.update();
    app.update();

    let frames = seen(&app);
    assert!(
        frames[0].just_pressed && frames[0].pressed,
        "the tap must be visible to `just_pressed` on the frame it lands: {frames:?}"
    );
    assert!(
        frames[1].just_released && !frames[1].pressed,
        "and released on the very next frame, the way a physical key behaves: {frames:?}"
    );
    assert_eq!(
        frames[2],
        Edges::default(),
        "and leave nothing behind afterwards: {frames:?}"
    );
}

/// **Misordered, an injected key now arrives a frame late rather than never** — and the difference
/// between those two sentences is the whole change.
///
/// This test replaces `ordering_before_input_systems_erases_the_press_entirely`, which pinned the same
/// engine fact from the other side: `keyboard_input_system` clears last frame's edges at the top of
/// `PreUpdate` and *then* folds the message stream. While this crate wrote `ButtonInput` **directly**,
/// a write placed before that clear was **destroyed** — the original bug. Now that it writes the
/// **source**, a write placed after the fold is merely **read next frame**.
///
/// So the cost of getting the ordering wrong dropped from "every `just_pressed` action is unreachable
/// while the method reports success" to "one frame of latency". Worth pinning precisely: if this ever
/// starts reporting a *lost* press again, something has gone back to writing the fold.
#[test]
fn ordering_after_input_systems_costs_a_frame() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::input::InputPlugin)
        .init_resource::<PendingInput>()
        // `apply_pending_input` writes the injected pointer here, and in Bevy 0.19 a missing
        // `ResMut<T>` panics the system rather than skipping it. `DebuggerPlugin` inits this; a host
        // that registers the system by hand — as these tests do — has to init it too.
        .init_resource::<bevy_fairy::DebugCursor>()
        .init_resource::<Seen>()
        // The one ordering constraint inverted.
        .add_systems(PreUpdate, apply_pending_input.after(InputSystems))
        .add_systems(Update, observe);

    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    app.update();
    app.update();

    let frames = seen(&app);
    assert!(
        !frames[0].just_pressed,
        "misordered, the press cannot be visible on the frame it was written: {frames:?}"
    );
    assert!(
        frames[1].just_pressed,
        "but it must still arrive on the next frame — a message survives the clear, where a direct \
         `ButtonInput` write did not. If this is false, the write has moved back to the fold: \
         {frames:?}"
    );
}

#[test]
fn two_taps_in_one_batch_produce_two_separate_press_edges() {
    // Both requests arrive before a single frame boundary, which is the normal case when an agent
    // sends two calls in quick succession. Applied in one pass they would collapse into one press,
    // because `ButtonInput::press` only sets `just_pressed` when the key was not already pressed.
    let mut app = app_correctly_ordered();
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    for _ in 0..4 {
        app.update();
    }

    let presses = seen(&app).iter().filter(|e| e.just_pressed).count();
    assert_eq!(
        presses,
        2,
        "two taps must produce two press edges, not one: {:?}",
        seen(&app)
    );
}

#[test]
fn a_tap_followed_by_a_press_leaves_the_key_held() {
    // "Step once, then keep walking." The tap's deferred release must not cancel the hold.
    let mut app = app_correctly_ordered();
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Press);
    for _ in 0..4 {
        app.update();
    }

    let frames = seen(&app);
    let last = frames.last().copied().unwrap_or_default();
    assert!(
        last.pressed,
        "the key must still be held after the tap's release has run: {frames:?}"
    );
}

#[test]
fn a_tap_and_a_release_never_show_both_edges_with_the_key_unpressed() {
    // Press and release inside one frame yields `pressed == false` while both edges are set — a state
    // no physical key can produce, and one every `pressed`-based reader silently skips.
    let mut app = app_correctly_ordered();
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Tap);
    queue_key(&mut app, KeyCode::KeyQ, InputAction::Release);
    for _ in 0..4 {
        app.update();
    }

    for (n, frame) in seen(&app).iter().enumerate() {
        assert!(
            !(frame.just_pressed && frame.just_released && !frame.pressed),
            "frame {n} shows a press and a release at once with the key unpressed: {frame:?}"
        );
    }
}

// ------------------------------------------------------------------------------------------------
// Cursor position — the half that makes a drag expressible
// ------------------------------------------------------------------------------------------------

/// A move lands, stays put without being re-sent, and clearing hands the pointer back.
#[test]
fn a_cursor_move_lands_and_clearing_releases_it() {
    let mut app = app_correctly_ordered();
    app.world_mut()
        .resource_mut::<PendingInput>()
        .queue_cursor(Some(Vec2::new(120.0, 64.0)));
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(120.0, 64.0)));

    // No per-frame stream to keep up: a position is state, not an edge.
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(120.0, 64.0)));

    app.world_mut().resource_mut::<PendingInput>().queue_cursor(None);
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, None);
}

/// **Aim, then click, in one frame** — the one ordering that is correct together, and the common one.
#[test]
fn a_move_before_an_untouched_button_applies_in_the_same_frame() {
    let mut app = app_correctly_ordered();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_cursor(Some(Vec2::new(10.0, 20.0)));
        pending.queue_mouse(MouseButton::Left, InputAction::Press);
    }
    app.update();
    assert_eq!(
        app.world().resource::<DebugCursor>().0,
        Some(Vec2::new(10.0, 20.0)),
        "the aim has to land on the frame the press is read"
    );
    assert!(app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Left));
}

/// **A press must be read where it was aimed**, so a move queued after one waits a frame.
///
/// Applied together, the game would see the press at the *new* position and the click would never
/// happen where it was pointed.
#[test]
fn a_move_after_a_button_waits_for_the_next_frame() {
    let mut app = app_correctly_ordered();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_cursor(Some(Vec2::new(10.0, 10.0)));
        pending.queue_mouse(MouseButton::Left, InputAction::Press);
        pending.queue_cursor(Some(Vec2::new(90.0, 90.0)));
    }
    app.update();
    assert_eq!(
        app.world().resource::<DebugCursor>().0,
        Some(Vec2::new(10.0, 10.0)),
        "the press frame must still read the position it was aimed at"
    );
    app.update();
    assert_eq!(
        app.world().resource::<DebugCursor>().0,
        Some(Vec2::new(90.0, 90.0)),
        "and the drag continues on the next frame"
    );
}

/// Two moves in one batch are two frames, so a drag has a path and not just an endpoint.
#[test]
fn two_moves_in_one_batch_are_spread_over_two_frames() {
    let mut app = app_correctly_ordered();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_cursor(Some(Vec2::new(1.0, 1.0)));
        pending.queue_cursor(Some(Vec2::new(2.0, 2.0)));
    }
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(1.0, 1.0)));
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(2.0, 2.0)));
}

/// **The injected pointer wins while it is set, and the window's own is read otherwise.**
///
/// One question with one answer. A host calling `Window::cursor_position` directly is undrivable by
/// an agent; a host calling this behaves identically for a person, because `DebugCursor` is `None`
/// until something explicitly sets it.
#[test]
fn the_injected_pointer_takes_precedence_and_hands_back_when_cleared() {
    use bevy::window::{Window, WindowResolution};
    let window = Window {
        resolution: WindowResolution::new(800, 600),
        ..Default::default()
    };
    assert_eq!(
        bevy_fairy::cursor_position(&window, &DebugCursor(Some(Vec2::new(5.0, 6.0)))),
        Some(Vec2::new(5.0, 6.0))
    );
    // A bare `Window` has no real cursor, so clearing gives what the window gives: nothing.
    assert_eq!(
        bevy_fairy::cursor_position(&window, &DebugCursor(None)),
        window.cursor_position()
    );
}

/// **A later command must never overtake an earlier deferred one.**
///
/// The regression this pins was found by `examples/cursor_drag_lands.rs` on the day cursor injection
/// landed, and every individual rule was behaving as written: the queue was ordered *per key*, so in
/// `move, press, move, move, release` the release — a button nothing had touched yet that frame —
/// jumped ahead of the two still-pending moves and the drag committed at the wrong corner.
#[test]
fn a_release_does_not_overtake_the_moves_queued_before_it() {
    let mut app = app_correctly_ordered();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_cursor(Some(Vec2::new(100.0, 100.0)));
        pending.queue_mouse(MouseButton::Left, InputAction::Press);
        pending.queue_cursor(Some(Vec2::new(140.0, 130.0)));
        pending.queue_cursor(Some(Vec2::new(180.0, 160.0)));
        pending.queue_mouse(MouseButton::Left, InputAction::Release);
    }
    // Frame 0: aim and press together — the one pairing that is correct in one frame.
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(100.0, 100.0)));
    assert!(app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Left));

    // Frame 1: the drag moves, and the button is still held.
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(140.0, 130.0)));
    assert!(
        app.world().resource::<ButtonInput<MouseButton>>().pressed(MouseButton::Left),
        "the release must not have jumped the queue"
    );

    // Frame 2: the last move, and only then the release — at the far corner, where it was aimed.
    app.update();
    assert_eq!(app.world().resource::<DebugCursor>().0, Some(Vec2::new(180.0, 160.0)));
    assert!(app.world().resource::<ButtonInput<MouseButton>>().just_released(MouseButton::Left));
}

// ------------------------------------------------------------------------------------------------
// Typing — the half that was unreachable, and the reason FVS-R-12 existed
// ------------------------------------------------------------------------------------------------

fn queue_text(app: &mut App, text: &str) {
    app.world_mut()
        .resource_mut::<PendingInput>()
        .queue_text(text, Entity::PLACEHOLDER)
        .unwrap_or_else(|ch| panic!("no logical key for {ch:?}"));
}

fn heard(app: &App) -> Vec<Vec<Key>> {
    app.world().resource::<Heard>().0.clone()
}

/// **A whole word is one frame**, because a stream is not a state.
///
/// This is the property the whole redesign turns on. `ButtonInput` shows two transitions per frame, so
/// key edges have to be spread out; `Messages<KeyboardInput>` is append-only and every text field
/// drains it whole once a frame, so seven characters written together are seven characters read
/// together. Spreading them would have made naming a composition seven round trips.
#[test]
fn a_whole_word_lands_in_one_frame() {
    let mut app = app_listening();
    queue_text(&mut app, "site_67");
    app.update();

    let frames = heard(&app);
    assert_eq!(
        spelled(&frames[0]),
        "site_67",
        "the whole string must arrive in order, in one frame: {frames:?}"
    );
}

/// **Every character releases, or it is held forever.**
///
/// `ButtonInput` is derived from the stream now, and `ButtonInput::release` is the only thing that
/// clears `pressed` — nothing else would ever call it. A Pressed with no matching Released is a leak
/// that no `just_pressed` assertion would notice.
#[test]
fn every_typed_character_is_released_and_nothing_stays_pressed() {
    let mut app = app_listening();
    queue_text(&mut app, "wall");
    for _ in 0..3 {
        app.update();
    }

    let world = app.world();
    assert_eq!(
        world.resource::<ButtonInput<Key>>().get_pressed().count(),
        0,
        "a typed character that is never released stays pressed for the life of the app"
    );
    assert_eq!(
        world.resource::<ButtonInput<KeyCode>>().get_pressed().count(),
        0,
        "and the physical side must be clean too"
    );
}

/// **One request, both halves** — the property a separate `Text` kind could not have.
///
/// `Escape` is spelled identically in `KeyCode` and `Key`. A real Escape leaves a tool *and* closes a
/// text field, because it is one key producing one message that Bevy folds. Splitting the request in
/// two would have made a caller choose based on which of the host's systems happened to be listening,
/// which is not knowledge an agent can obtain.
#[test]
fn a_named_key_carries_both_halves() {
    let mut app = app_listening();
    queue_key(&mut app, KeyCode::Escape, InputAction::Tap);
    app.update();

    assert!(
        app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::Escape),
        "the physical half must reach a `ButtonInput` reader"
    );
    assert!(
        heard(&app)[0].contains(&Key::Escape),
        "and the logical half must reach the message stream, from the same request: {:?}",
        heard(&app)
    );
}

/// **A physical-only key says so, rather than guessing.**
///
/// `KeyW` is `w` on QWERTY and `,` on Dvorak. There is no layout-independent logical answer, and
/// inventing one would be the twelve-key table this module's header records deleting — so it reports
/// `Unidentified`, which is the enum's own word for it.
#[test]
fn a_physical_only_key_states_that_it_has_no_logical_value() {
    let mut app = app_listening();
    queue_key(&mut app, KeyCode::KeyW, InputAction::Tap);
    app.update();

    assert!(
        app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::KeyW),
        "the physical half is exact and must still land"
    );
    assert!(
        heard(&app)[0].contains(&Key::Unidentified(NativeKey::Unidentified)),
        "and the logical half must state the absence: {:?}",
        heard(&app)
    );
}

/// **A space is the space bar, not the character `" "`.**
///
/// The one character with a named variant. Five separate text handlers in `bevy_emerge_mapper` match
/// `Key::Space` and none matches `Key::Character(" ")`, so the wrong choice would drop the space out
/// of a typed name while the call reported success — a silent corruption of exactly the input this
/// feature exists to deliver.
#[test]
fn a_space_is_the_space_bar_not_a_character() {
    let mut app = app_listening();
    queue_text(&mut app, "a b");
    app.update();

    let frame = &heard(&app)[0];
    assert_eq!(frame.len(), 3, "three characters: {frame:?}");
    assert_eq!(frame[1], Key::Space, "the middle one must be `Space`: {frame:?}");
}

/// **Text waits for the frame after the keystroke that opened the field.**
///
/// Every text field in `bevy_emerge_mapper` drains the stream *while shut*, so the key that opened it
/// cannot become its first character — the `xseam` bug. Its key phase runs before its dispatcher
/// phase, so text sent in the same frame as the opening key would be eaten by that very guard while
/// this method reported success. One frame is the same price the one-edge-per-key rule already pays.
#[test]
fn text_waits_for_the_frame_after_the_key_that_opened_the_field() {
    let mut app = app_listening();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending.queue_key(KeyCode::KeyM, InputAction::Tap);
        pending
            .queue_text("porch", Entity::PLACEHOLDER)
            .unwrap_or_else(|ch| panic!("no logical key for {ch:?}"));
    }
    app.update();
    app.update();

    let frames = heard(&app);
    assert_eq!(
        spelled(&frames[0]),
        "Unidentified(Unidentified)",
        "frame 0 carries the opening key and no characters: {frames:?}"
    );
    assert_eq!(
        spelled(&frames[1]),
        "porch",
        "and the characters follow on the next frame: {frames:?}"
    );
}

/// **Nothing defers behind text**, which is the point: type and commit in one round trip.
///
/// The inverse of the rule above, and deliberately not symmetric. A field is already open by the time
/// text is being sent, so `Enter` in the same frame is read after the characters it is committing.
#[test]
fn an_enter_after_text_commits_in_the_same_frame() {
    let mut app = app_listening();
    {
        let mut pending = app.world_mut().resource_mut::<PendingInput>();
        pending
            .queue_text("porch_a", Entity::PLACEHOLDER)
            .unwrap_or_else(|ch| panic!("no logical key for {ch:?}"));
        pending.queue_key(KeyCode::Enter, InputAction::Tap);
    }
    app.update();

    let frame = &heard(&app)[0];
    assert_eq!(
        spelled(frame),
        "porch_aEnter",
        "the name and its commit must land together, in that order: {frame:?}"
    );
}

/// Two text commands in one frame concatenate in order — a burst of BRP calls is still one string.
#[test]
fn two_text_commands_in_one_frame_concatenate_in_order() {
    let mut app = app_listening();
    queue_text(&mut app, "si");
    queue_text(&mut app, "te");
    app.update();

    assert_eq!(spelled(&heard(&app)[0]), "site");
}

/// `typed` refuses nothing printable, and builds the message a keyboard would have.
#[test]
fn a_typed_character_is_shaped_like_a_real_one() {
    let message = typed('q', Entity::PLACEHOLDER).unwrap_or_else(|| panic!("`q` must build"));
    assert_eq!(message.logical_key, Key::Character("q".into()));
    // Deliberately not naming the windowing backend here: `tests/leaf.rs` scans string literals as
    // well as code, and it is right to — a marker in an error message is how a forbidden dependency
    // would announce itself.
    assert_eq!(
        message.text.as_deref(),
        Some("q"),
        "`text` carries the character produced, matching what a real keyboard event reports"
    );
    assert!(!message.repeat, "an injected key is never an OS auto-repeat");
    assert_eq!(
        message.key_code,
        KeyCode::Unidentified(bevy::input::keyboard::NativeKeyCode::Unidentified),
        "a character that arrived has no physical origin this process can know"
    );
}
