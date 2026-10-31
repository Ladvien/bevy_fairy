//! Headless input injection — drives keyboard and mouse without the OS.
//!
//! Writes into **the same message stream `bevy_winit` writes to**, inside the game process. Nothing
//! here reaches the operating system: if the person at the machine is working in another window,
//! injected keys do not go there. That is structural rather than careful — this crate depends on
//! `bevy`, `bevy_remote`, `serde`, `serde_json` and `image`, none of which can synthesise an OS event.
//!
//! # Injected input *is* input, rather than a copy of its effects
//!
//! A real key produces exactly one thing: a [`KeyboardInput`] message. `ButtonInput<KeyCode>` and
//! `ButtonInput<Key>` are Bevy's own **fold** of that stream, done by `keyboard_input_system` in
//! [`InputSystems`](bevy::input::InputSystems).
//!
//! This module used to write the fold and not the source, and that is exactly one half of a key.
//! Everything reading `ButtonInput` saw injected keys; everything reading the stream — **every text
//! field in every Bevy application** — could not, so no agent could type, and could not press Enter or
//! Escape in a field either, because those are read off `logical_key` too. Writing the source instead
//! makes both halves land from one request, which is what "indistinguishable from a real key" has to
//! mean.
//!
//! # Key names come from Bevy, not from a table here
//!
//! `key` and `button` are deserialized straight into [`KeyCode`] and [`MouseButton`], so every key
//! Bevy knows is accepted and the spelling is Bevy's own variant name — `KeyW`, `ArrowLeft`, `F11`,
//! `Numpad7`, `ShiftLeft`, `BracketRight`, `AudioVolumeUp`.
//!
//! This replaced a hand-written `match` covering twelve keys. Such a table is a second, always-stale
//! copy of an enum with nearly two hundred variants: it silently lacked every function key, every
//! modifier, every digit and the whole numpad, and nothing would have told you except an
//! `Unknown key` at the moment you needed one. Deserializing has no drift by construction — a Bevy
//! upgrade that adds a key adds it here.
//!
//! **The logical half comes from the same string through the same mechanism.** `KeyCode` has 194 unit
//! variants and [`Key`] has 306, and **93 are spelled identically** — `Enter`, `Escape`, `Backspace`,
//! `Space`, `Tab`, `Delete`, every arrow, every `F1`–`F35`. For those, [`logical_for`] hands the
//! caller's own name to Bevy's `Key` deserializer and no table exists to go stale. For the rest —
//! `KeyW`, `Numpad7`, `ShiftLeft` — the answer is `Key::Unidentified`, because `KeyW` is `w` on QWERTY
//! and `,` on Dvorak, and guessing which is precisely the table that was deleted above.
//!
//! `MouseButton::Other(u16)` is reachable as `{"Other": 3}`, since that is how serde spells a
//! newtype variant.

use bevy::input::keyboard::{Key, KeyCode, KeyboardInput, NativeKey, NativeKeyCode};
use bevy::input::mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::TouchPhase;
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::remote::{error_codes, BrpError, BrpResult};
use bevy::window::PrimaryWindow;

use super::pointer::PointerRoute;
use serde::Deserialize;
use serde_json::{json, Value};

/// Parameters for the `bevy_fairy/input` BRP method.
///
/// A field this does not name is refused by `deny_unknown_fields`, and a field it names that the
/// chosen `kind` does not read is refused by `refuse_unread_fields` (private): a modifier or a position that
/// changes nothing must not come back as `success: true`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputCommand {
    /// What kind of input to inject.
    pub kind: InputKind,
    /// What action to perform. Defaults to [`InputAction::Tap`], and [`InputKind::Cursor`] ignores it
    /// — a position has no press and no release, so requiring a meaningless one would be a field a
    /// caller has to supply and cannot get right.
    #[serde(default)]
    pub action: InputAction,
    /// Key name — any [`KeyCode`] variant, e.g. `"KeyW"`, `"Space"`, `"ArrowLeft"`, `"F5"`,
    /// `"Numpad7"`, `"ShiftLeft"`.
    ///
    /// **It carries its logical half whenever Bevy spells the same name in [`Key`]**, which covers
    /// every named key a text field matches. So `"Escape"` leaves a tool *and* closes a name field
    /// from one request, the way a real Escape does — see [`logical_for`].
    #[serde(default)]
    pub key: Option<String>,
    /// **What to type**, one message per character. `"site_67"` is one call and one frame.
    ///
    /// Not an alternative spelling of `key`, and not the same request: `key` names a key **on the
    /// keyboard**, `text` is **what should arrive**. Neither can express the other — `text` cannot say
    /// which key produced a character, and `key` cannot produce `é`. Sending both is refused, because
    /// there is no order between two intents.
    #[serde(default)]
    pub text: Option<String>,
    /// Mouse button — any [`MouseButton`] variant: `"Left"`, `"Right"`, `"Middle"`, `"Back"`,
    /// `"Forward"`, or `{"Other": 3}`.
    #[serde(default)]
    pub button: Option<Value>,
    /// Scroll amounts, used by [`InputKind::Scroll`]. Positive `y` scrolls up.
    #[serde(default)]
    pub x: Option<f32>,
    #[serde(default)]
    pub y: Option<f32>,
    /// Scroll unit: `"Line"` (default) or `"Pixel"`.
    #[serde(default)]
    pub unit: Option<String>,
    /// [`InputKind::Cursor`] only: put the pointer back under the real mouse.
    ///
    /// A separate flag rather than "omit `x` and `y`", because omitting a coordinate is much more
    /// likely to be a caller bug than an intent to release the pointer, and the two must not look
    /// alike.
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Deserialize)]
pub enum InputKind {
    Keyboard,
    Mouse,
    Scroll,
    /// **Where the pointer is**, in logical window pixels — `x` right, `y` down from the top-left,
    /// the same frame [`bevy::window::Window::cursor_position`] reports.
    ///
    /// Set it and it stays until moved again or cleared; there is no per-frame stream to keep up.
    /// That is what makes a drag expressible: put the cursor down, press the button, move, release.
    Cursor,
}

#[derive(Debug, Default, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    Press,
    Release,
    /// Pressed for one frame and released the next — what a physical key does, and what a
    /// `just_pressed` reader expects. The default because it is what a caller nearly always means.
    #[default]
    Tap,
}

fn invalid_params(message: String) -> BrpError {
    BrpError { code: error_codes::INVALID_PARAMS, message, data: None }
}

/// The fields each kind of input reads. `Cursor` reads no `action`, but ignores one by design (a
/// position has no press, see [`InputCommand::action`]); `Scroll` has no press either, so an
/// `action` sent to it is the same wrong belief `text` refuses.
fn fields_read_by(kind: &InputKind) -> &'static [&'static str] {
    match kind {
        InputKind::Keyboard => &["kind", "action", "key", "text"],
        InputKind::Mouse => &["kind", "action", "button"],
        InputKind::Scroll => &["kind", "x", "y", "unit"],
        InputKind::Cursor => &["kind", "action", "x", "y", "clear"],
    }
}

/// Refuse a field the call sent that its `kind` does not read, naming the field and what the kind
/// reads. `params` is the caller's own JSON, because a field that was left out and one that was
/// sent with its default value cannot be told apart once parsed.
fn refuse_unread_fields(kind: &InputKind, params: &Value) -> Result<(), BrpError> {
    let read = fields_read_by(kind);
    let Some(field) = params.as_object().and_then(|o| o.keys().find(|k| !read.contains(&k.as_str()))) else {
        return Ok(());
    };
    let mut message = format!(
        "`{field}` is not read by `kind: {kind:?}`, so it would change nothing; that kind reads {}",
        read.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
    );
    if matches!(kind, InputKind::Mouse) && (field == "x" || field == "y") {
        message.push_str(
            ". A click lands where the pointer is: send `kind: \"Cursor\"` with `x` and `y` first, then the click",
        );
    }
    Err(invalid_params(message))
}

/// One validated injection, waiting for the frame that can apply it.
///
/// Parsing happens in the BRP handler so a malformed request still fails loudly and immediately;
/// only the *application* is deferred. By the time a command reaches this enum it is known-good.
enum Injected {
    /// A key, named by its physical [`KeyCode`] and carrying the logical half Bevy spells for that
    /// same name — see [`logical_for`].
    Key { key_code: KeyCode, logical: Key, action: InputAction, window: Entity },
    /// One typed character, **already built as the message it will be written as**.
    ///
    /// Whole rather than a `char`, because `Key::Character` and [`KeyboardInput::text`] both hold a
    /// `SmolStr` and `smol_str` is not one of the five dependencies `tests/leaf.rs` allows. Bevy's own
    /// deserializer builds it in [`typed`] and nothing here ever names the type.
    Typed(KeyboardInput),
    Mouse(MouseButton, InputAction, Entity),
    Scroll { unit: MouseScrollUnit, x: f32, y: f32, window: Entity },
    Cursor(Option<Vec2>),
}

/// **The logical half Bevy itself spells for a physical key's name**, or `Unidentified`.
///
/// Not a mapping table — a second deserialization of the *same string*. `KeyCode` and [`Key`] share
/// 93 spellings, including every named key a text field matches (`Enter`, `Escape`, `Backspace`,
/// `Space`, `Tab`), so for those this is exact and cannot drift: a Bevy upgrade that renames one
/// renames it on both sides at once.
///
/// For the ~101 physical-only names it answers `Key::Unidentified`, and that is a statement rather
/// than a gap. `KeyW` is `w` on QWERTY and `,` on Dvorak; a layout-independent answer does not exist,
/// and inventing one is the twelve-key table this module's header records deleting.
pub fn logical_for(key_code: KeyCode) -> Key {
    match serde_json::to_value(key_code) {
        // A unit variant serializes to its bare name; `KeyCode::Unidentified(..)` serializes to an
        // object and falls through, which is the right answer for it anyway.
        Ok(Value::String(name)) => serde_json::from_value::<Key>(Value::String(name))
            .unwrap_or(Key::Unidentified(NativeKey::Unidentified)),
        _ => Key::Unidentified(NativeKey::Unidentified),
    }
}

/// **One typed character as the message a keyboard would have produced**, or `None` if Bevy will not
/// build a `Key` from it.
///
/// `key_code` is `Unidentified` on purpose: a character that arrived has no physical origin this
/// process can know, and naming one would be a guess that reads as a measurement.
///
/// **A space becomes `Key::Space`, not `Key::Character(" ")`.** It is the one character with a named
/// variant, it is what a space bar produces on every layout, and five separate text handlers in
/// `bevy_emerge_mapper` alone match `Key::Space` and would silently drop a `Character(" ")` — a name with a
/// space in it would lose the space while the call reported success.
pub fn typed(ch: char, window: Entity) -> Option<KeyboardInput> {
    let logical_key = if ch == ' ' {
        Key::Space
    } else {
        let mut buf = [0u8; 4];
        // Bevy's deserializer builds the `SmolStr`; this crate never names the type.
        serde_json::from_value::<Key>(json!({ "Character": ch.encode_utf8(&mut buf) })).ok()?
    };
    // Cloned out of the `Key` we just built, for the same reason.
    let text = match &logical_key {
        Key::Character(s) => Some(s.clone()),
        _ => None,
    };
    Some(KeyboardInput {
        key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
        logical_key,
        state: ButtonState::Pressed,
        text,
        // An injected key is never an OS auto-repeat.
        repeat: false,
        window,
    })
}

/// **Where an injected pointer is**, in logical window pixels — `None` when the real mouse owns it.
///
/// # Why this is a resource and not the window's own cursor
///
/// Bevy has `Window::set_cursor_position`, and it looks like exactly the right call. It is not, and
/// the reason was read out of the pinned engine source rather than guessed.
///
/// `Window::set_cursor_position` writes only an internal field. But Bevy's windowing backend runs a
/// `changed_windows` system that compares that field against a per-window cache each frame, and on a
/// difference asks the platform to **move the physical pointer** — `bevy_winit-0.20.1/src/system.rs`
/// lines 426-430. The cache is `pub(crate)`, so a plugin cannot update it to suppress the diff.
///
/// Writing the window would therefore drag the mouse out from under whoever is at the machine. That
/// is precisely the class of thing this crate exists to avoid, and `tests/leaf.rs` is the ratchet
/// that keeps it avoided; a cursor that moves the real one would satisfy the letter of that test
/// while breaking the whole point of it.
///
/// So the injected position lives here, beside the game's own input, and a host reads it through
/// [`cursor_position`]. Nothing outside the process is touched.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq)]
pub struct DebugCursor(pub Option<Vec2>);

/// **The pointer a host should act on** — the injected position if one is set, else the real one.
///
/// One question with one answer, asked in one place. A host that calls
/// [`Window::cursor_position`](bevy::window::Window::cursor_position) directly is unreachable by an
/// agent; a host that calls this is drivable and behaves identically for a person, because
/// [`DebugCursor`] is `None` until something explicitly sets it.
///
/// Precedence, not fallback: the injected value is only ever present because a caller asked for it,
/// and clearing it is equally explicit. There is no degraded substitute that switches itself on.
pub fn cursor_position(window: &Window, injected: &DebugCursor) -> Option<Vec2> {
    injected.0.or_else(|| window.cursor_position())
}

/// Injected input waiting to be applied in `PreUpdate`.
///
/// # Why this queue exists
///
/// BRP methods registered with `with_method_main` run in **`RemoteLast`** (after `Last`), and a message
/// written there would be read a whole frame late by everything. More importantly, the deferral rules
/// below — one edge per key per frame, cursor ordering, text behind an opening keystroke — have to
/// live in exactly one place, and this is it.
///
/// # The bug this shape was built around, which is worth not re-learning
///
/// Bevy's `keyboard_input_system` runs in **`PreUpdate`** and begins by clearing the just-pressed and
/// just-released sets. A press written straight into `ButtonInput` from a `RemoteLast` handler therefore had
/// its `just_pressed` flag wiped by the very next `PreUpdate`, *before* any `Update` system could read
/// it. **Every `just_pressed`-based action was unreachable by injected input** while the method
/// answered `success: true`. Held `pressed` state survived the clear, which is what made it look
/// intermittent: some actions responded and some silently did not.
///
/// That was first fixed by writing `ButtonInput` **after** the clear. It is now fixed one level
/// further down: [`apply_pending_input`] runs **before** [`InputSystems`](bevy::input::InputSystems)
/// and writes the [`KeyboardInput`] / [`MouseButtonInput`] messages that the clear-then-fold reads, so
/// the edge is produced by Bevy from the same stream a real key travels on. There is one path for
/// input, and now that is literally true rather than true of one half.
#[derive(Resource, Default)]
pub struct PendingInput {
    queue: Vec<Injected>,
    /// Taps press on one frame and release on the next, because that is what a real tap does. Pressing
    /// and releasing within a single frame leaves `pressed` false for the whole frame while both
    /// `just_pressed` and `just_released` are true — a state no physical key can produce, and one that
    /// silently skips every `pressed`-based reader.
    ///
    /// **Whole messages, not key names.** `ButtonInput` is now derived rather than written, and
    /// `ButtonInput::release` is the only thing that clears `pressed` — so a Pressed with no matching
    /// Released leaves the key held forever.
    release_next_keys: Vec<KeyboardInput>,
    release_next_buttons: Vec<MouseButtonInput>,
}

impl PendingInput {
    /// Queue a keyboard action, applied on the next `PreUpdate`.
    ///
    /// The BRP handler is one caller; a host driving itself (a test, an example, a scripted demo) is
    /// another. Both go through this queue rather than writing messages directly, because the ordering
    /// and deferral rules are only correct in one place.
    ///
    /// The logical half is derived by [`logical_for`], so a host driving `KeyCode::Escape` reaches a
    /// text field exactly as the BRP path does. `window` is [`Entity::PLACEHOLDER`] — see
    /// [`apply_pending_input`] for why nothing reads it.
    pub fn queue_key(&mut self, key: KeyCode, action: InputAction) {
        self.queue.push(Injected::Key {
            key_code: key,
            logical: logical_for(key),
            action,
            window: Entity::PLACEHOLDER,
        });
    }

    /// Queue one already-built typed character — see [`typed`], which is how a caller gets one.
    ///
    /// Separate from [`queue_key`](Self::queue_key) because it is a different question, not a
    /// different spelling: this says *a character arrived*, that says *a key went down*.
    pub fn queue_typed(&mut self, message: KeyboardInput) {
        self.queue.push(Injected::Typed(message));
    }

    /// Queue a whole string, one message per character. Returns the character it could not build a
    /// `Key` from, if any — nothing is queued in that case.
    pub fn queue_text(&mut self, text: &str, window: Entity) -> Result<(), char> {
        let built: Result<Vec<_>, char> =
            text.chars().map(|ch| typed(ch, window).ok_or(ch)).collect();
        self.queue.extend(built?.into_iter().map(Injected::Typed));
        Ok(())
    }

    /// Queue a one-frame tap: pressed on the next `PreUpdate`, released the frame after — the shape a
    /// physical key produces, and what `just_pressed` readers expect.
    pub fn queue_tap_key(&mut self, key: KeyCode) {
        self.queue_key(key, InputAction::Tap);
    }

    /// Queue a mouse-button action, applied on the next `PreUpdate`.
    pub fn queue_mouse(&mut self, button: MouseButton, action: InputAction) {
        self.queue.push(Injected::Mouse(button, action, Entity::PLACEHOLDER));
    }

    /// Queue a cursor move — logical window pixels, or `None` to hand the pointer back.
    ///
    /// Through the queue like everything else, so a cursor move and the click that follows it land in
    /// the documented order rather than racing: a click applied a frame before the move would be read
    /// at the old position, which is the drag bug this whole module is shaped around avoiding.
    pub fn queue_cursor(&mut self, at: Option<Vec2>) {
        self.queue.push(Injected::Cursor(at));
    }
}

/// Applies queued injections, in `PreUpdate` **before** Bevy reads input.
///
/// # The ordering, and why it is the opposite of what it used to be
///
/// `keyboard_input_system` clears last frame's edges and *then* folds the [`KeyboardInput`] stream
/// into `ButtonInput` (`bevy_input-0.20.1/src/keyboard.rs`). Writing the messages **ahead** of it
/// therefore survives the clear that erased the old direct `ButtonInput` write — the same fact, used
/// from the other side.
///
/// Registered by [`DebuggerPlugin`](crate::DebuggerPlugin). A host building its own schedule must keep
/// `.before(InputSystems)` **and** must have added `InputPlugin`, or nothing folds these messages and
/// the injection is silently inert. `DebuggerPlugin::finish` asserts the latter rather than leaving it
/// to be discovered.
///
/// It must also `init_resource::<DebugCursor>()` — in Bevy 0.20 a missing `ResMut<T>` panics the system
/// rather than skipping it, so the absence is loud rather than a cursor injection that quietly does
/// nothing.
pub fn apply_pending_input(
    mut pending: ResMut<PendingInput>,
    mut keyboard: MessageWriter<KeyboardInput>,
    mut mouse: MessageWriter<MouseButtonInput>,
    mut wheel: MessageWriter<MouseWheel>,
    mut cursor: ResMut<DebugCursor>,
    mut route: PointerRoute,
) {
    // Taken out first: draining in place would hold a borrow of `pending` across the loop below.
    let release_keys = core::mem::take(&mut pending.release_next_keys);
    let release_buttons = core::mem::take(&mut pending.release_next_buttons);
    let queue = core::mem::take(&mut pending.queue);

    // Last frame's presses end here, one frame after they began. Not optional: `ButtonInput` is
    // derived from this stream now, and `release` is the only thing that clears `pressed`.
    for message in release_keys {
        keyboard.write(message);
    }
    for message in release_buttons {
        route.button(&message, cursor.0.unwrap_or(Vec2::ZERO));
        mouse.write(message);
    }

    // **At most one command per key per frame; the rest wait.**
    //
    // BRP drains every request that arrived since the last frame into a single handler run, so a
    // whole burst can land in this queue together. Applying a burst in one pass destroys edges,
    // because `ButtonInput` records a *state* and only two transitions are visible per frame:
    //
    // - two `Tap`s of one key collapse into a single press, since `ButtonInput::press` only sets
    //   `just_pressed` when the key was not already pressed;
    // - a `Tap` followed by a `Press` leaves a scheduled release that cancels the hold a frame later;
    // - a `Tap` followed by a `Release` produces `pressed == false` with both edges set at once,
    //   the very state this queue exists to prevent.
    //
    // Deferring the remainder spreads them over consecutive frames, which is what a real device does:
    // one key cannot be pressed twice in the same frame either. Relative order is preserved, because
    // the first command for a key marks it and every later one for that key falls through to
    // `deferred` behind it.
    //
    // **Typed characters are exempt, and that is a property of streams rather than a concession.** A
    // burst of characters is a burst of messages, and every text field drains the whole stream once a
    // frame — so `"site_67"` is seven messages in one frame and nothing is lost. There is no edge to
    // destroy. (`ButtonInput<Key>` *does* collapse a repeated character — `wall` yields one `l` edge —
    // which is why the stream is the right thing to read text from and that resource is not.)
    let mut keys_touched: Vec<KeyCode> = Vec::new();
    let mut buttons_touched: Vec<MouseButton> = Vec::new();
    let mut cursor_moved = false;
    let mut deferred: Vec<Injected> = Vec::new();

    for cmd in queue {
        // **Once anything is held back, everything after it is too.**
        //
        // Without this the queue is only ordered *per key*: a later command of a different kind
        // overtakes an earlier deferred one. Measured by `examples/cursor_drag_lands.rs` the day
        // cursor injection landed — in `move, press, move, move, release` the release jumped ahead of
        // the two pending moves and the drag committed at the wrong corner, while every individual
        // rule was behaving as written.
        //
        // The cost is that two *different* keys queued behind a deferral now take a frame each
        // instead of sharing one. That is the right trade: a caller wrote a sequence, and a sequence
        // that arrives out of order is wrong in a way no amount of speed makes up for.
        if !deferred.is_empty() {
            deferred.push(cmd);
            continue;
        }
        match &cmd {
            Injected::Key { key_code, .. } if keys_touched.contains(key_code) => {
                deferred.push(cmd);
                continue;
            }
            Injected::Mouse(button, _, _) if buttons_touched.contains(button) => {
                deferred.push(cmd);
                continue;
            }
            // **Text waits for the frame after the keystroke that opened the field.**
            //
            // Measured against the host rather than invented: every text field in `bevy_emerge_mapper`
            // drains the `KeyboardInput` stream *while shut*, so the key that opened it cannot become
            // its first character — the `xseam` bug, which every field there now guards against. Its
            // key phase runs before its dispatcher phase, so text sent in the same frame as the
            // opening key is eaten by that very guard while this method reports success. That is the
            // original bug's shape, and one frame is the same price the rule above already pays.
            //
            // **Nothing defers behind text**, deliberately: `text:"porch_a"` then `key:"Enter"` in one
            // frame is correct and is the point — type and commit in one round trip.
            Injected::Typed(_) if !keys_touched.is_empty() || !buttons_touched.is_empty() => {
                deferred.push(cmd);
                continue;
            }
            // **A move after a button, or a second move, waits for the next frame.**
            //
            // Position is state and a button edge is read against it, so both orderings matter and
            // only one is expressible per frame. `press, move` applied together would have the game
            // read the press at the *new* position — the press never happens where it was aimed —
            // and two moves in one frame would collapse a drag's path to its endpoint. Deferring
            // gives each its own frame, which is the shape a real mouse produces.
            //
            // A move *before* an untouched button is the one combination that is correct together,
            // and it is the common one: aim, then click there.
            Injected::Cursor(_) if cursor_moved || !buttons_touched.is_empty() => {
                deferred.push(cmd);
                continue;
            }
            _ => {}
        }

        match cmd {
            Injected::Key { key_code, logical, action, window } => {
                keys_touched.push(key_code);
                let message = |state| KeyboardInput {
                    key_code,
                    logical_key: logical.clone(),
                    state,
                    // `None` for a named key, matching what a real one reports: `text` is the
                    // character produced, and `Enter` produces none. A host reading `text` instead of
                    // `logical_key` therefore sees nothing here — stated rather than papered over with
                    // an invented `"\r"`.
                    text: None,
                    repeat: false,
                    window,
                };
                match action {
                    InputAction::Press => {
                        keyboard.write(message(ButtonState::Pressed));
                    }
                    InputAction::Release => {
                        keyboard.write(message(ButtonState::Released));
                    }
                    InputAction::Tap => {
                        keyboard.write(message(ButtonState::Pressed));
                        pending.release_next_keys.push(message(ButtonState::Released));
                    }
                }
            }
            // Never added to `keys_touched`: a character is consumed from a stream, and a stream has
            // no edge for a second one to destroy.
            Injected::Typed(message) => {
                let mut released = message.clone();
                released.state = ButtonState::Released;
                keyboard.write(message);
                pending.release_next_keys.push(released);
            }
            Injected::Mouse(button, action, window) => {
                buttons_touched.push(button);
                let window = route.window_or_primary(window);
                let at = cursor.0.unwrap_or(Vec2::ZERO);
                let message = |state| MouseButtonInput { button, state, window };
                match action {
                    InputAction::Press => {
                        route.button(&message(ButtonState::Pressed), at);
                        mouse.write(message(ButtonState::Pressed));
                    }
                    InputAction::Release => {
                        route.button(&message(ButtonState::Released), at);
                        mouse.write(message(ButtonState::Released));
                    }
                    InputAction::Tap => {
                        route.button(&message(ButtonState::Pressed), at);
                        mouse.write(message(ButtonState::Pressed));
                        pending.release_next_buttons.push(message(ButtonState::Released));
                    }
                }
            }
            // Scroll is a delta, not a state, so several in one frame simply sum — there is no edge
            // to lose and nothing to defer.
            Injected::Scroll { unit, x, y, window } => {
                let message = MouseWheel {
                    unit,
                    x,
                    y,
                    window,
                    // A mouse always reports `Moved`; the variant exists for touch.
                    phase: TouchPhase::Moved,
                };
                route.wheel(&message, cursor.0.unwrap_or(Vec2::ZERO));
                wheel.write(message);
            }
            Injected::Cursor(at) => {
                cursor_moved = true;
                if let Some(to) = at {
                    route.moved(cursor.0, to);
                }
                cursor.0 = at;
            }
        }
    }

    pending.queue = deferred;
}

/// BRP handler: `bevy_fairy/input`.
///
/// Validates now, applies in `PreUpdate` — see [`PendingInput`] for why the write cannot happen here.
pub fn handle_input(
    In(params): In<Option<Value>>,
    mut pending: ResMut<PendingInput>,
    window: Option<Single<Entity, With<PrimaryWindow>>>,
    injected_cursor: Option<Res<DebugCursor>>,
) -> BrpResult {
    // Said back to the caller when a call has something the reply should not leave unsaid; see `Mouse`.
    let mut notes: Vec<&str> = Vec::new();
    // `serde_json::Error` has no `From` into `BrpError`, so `?` cannot carry it — a malformed payload
    // is reported as INVALID_PARAMS with the parser's own message, which is the part a caller can act on.
    let cmd: InputCommand = match params.as_ref() {
        Some(p) => {
            // A positional array parses as a struct, and `deny_unknown_fields` does not police it.
            if !p.is_object() {
                return Err(invalid_params(format!("input params must be a JSON object, got {p}")));
            }
            let cmd: InputCommand = serde_json::from_value(p.clone())
                .map_err(|e| invalid_params(format!("invalid input params: {e}")))?;
            refuse_unread_fields(&cmd.kind, p)?;
            cmd
        }
        None => return Err(invalid_params("Missing input parameters".to_string())),
    };

    // **Not an error when absent, unlike `Scroll`.** A `MouseWheel` has to be addressed to a window;
    // a `KeyboardInput` does not — `keyboard_input_system` never reads the field, and Bevy's focus
    // dispatcher takes the window from its own query rather than from the message. Refusing here would
    // make the crate untestable by its own tests, which run on `MinimalPlugins` with no window at all.
    // Borrowed, not consumed: the `Scroll` arm below still needs the `Option` itself, because for a
    // `MouseWheel` the absence genuinely is an error.
    let addressed = window.as_ref().map_or(Entity::PLACEHOLDER, |w| **w);

    match cmd.kind {
        InputKind::Keyboard => {
            // **One kind, two questions, and sending both is a refusal.** `key` names a key on the
            // keyboard; `text` is what should arrive. There is no order between two intents, and
            // picking one silently is how a caller learns a wrong model of the method.
            if cmd.key.is_some() && cmd.text.is_some() {
                return Err(invalid_params(
                    "send `key` or `text`, not both: `key` names a key on the keyboard, `text` is \
                     what should arrive, and there is no order between them"
                        .to_string(),
                ));
            }
            match (cmd.key, cmd.text) {
                (Some(name), None) => {
                    // Bevy's own variant names, parsed by Bevy's own deserializer.
                    let keycode: KeyCode = serde_json::from_value(Value::String(name.clone()))
                        .map_err(|e| invalid_params(format!("unknown key `{name}`: {e}")))?;
                    pending.queue.push(Injected::Key {
                        key_code: keycode,
                        logical: logical_for(keycode),
                        action: cmd.action,
                        window: addressed,
                    });
                }
                (None, Some(text)) => {
                    // A held character is meaningless — a caller who wrote it believed something, and
                    // swallowing it teaches that belief. `Cursor` ignores `action` because a position
                    // has no press; this is the different case where one was *supplied* and is wrong.
                    if cmd.action != InputAction::Tap {
                        return Err(invalid_params(
                            "`text` has no press and no release; drop `action`, or use `key` if you \
                             meant to hold something down"
                                .to_string(),
                        ));
                    }
                    if text.is_empty() {
                        return Err(invalid_params(
                            "`text` is empty, which injects nothing — omit it or send a character"
                                .to_string(),
                        ));
                    }
                    if let Some(ch) = text.chars().find(|c| c.is_control()) {
                        return Err(invalid_params(format!(
                            "`text` contains the control character {ch:?}; a text field matches \
                             `Key::Enter` and never `Key::Character(\"\\n\")`, so send \
                             `key: \"Enter\"` (or \"Tab\", \"Backspace\", \"Escape\") instead"
                        )));
                    }
                    pending.queue_text(&text, addressed).map_err(|ch| {
                        invalid_params(format!("no logical key exists for the character {ch:?}"))
                    })?;
                }
                (None, None) => {
                    return Err(invalid_params(
                        "keyboard input needs `key` (a key on the keyboard) or `text` (what should \
                         arrive)"
                            .to_string(),
                    ))
                }
                // Refused above; the arm exists so the match is total without a catch-all that could
                // swallow a future case.
                (Some(_), Some(_)) => unreachable!("both-supplied is refused above"),
            }
        }
        InputKind::Mouse => {
            let raw = cmd
                .button
                .ok_or_else(|| invalid_params("Missing 'button' for mouse input".to_string()))?;
            let button: MouseButton = serde_json::from_value(raw.clone())
                .map_err(|e| invalid_params(format!("unknown mouse button `{raw}`: {e}")))?;
            // A press lands where picking last saw the pointer, and picking starts at (0, 0). Refusing
            // would break a host driven by the real cursor, so the reply says it instead.
            let aimed = injected_cursor.is_some_and(|c| c.0.is_some())
                || pending.queue.iter().any(|c| matches!(c, Injected::Cursor(Some(_))));
            if !aimed {
                notes.push(
                    "no injected cursor position: the click lands where the pointer last was \
                     (the origin if it never moved). Send `kind: \"Cursor\"` with `x` and `y` first",
                );
            }
            // `bevy_picking` maps Left, Right and Middle and skips the rest (`mouse_pick_events`), on
            // the window route and the image route alike: the press is in `ButtonInput` and the
            // message stream, and nothing built on picking can see it (B-131).
            if !matches!(button, MouseButton::Left | MouseButton::Right | MouseButton::Middle) {
                notes.push(
                    "this button reaches ButtonInput<MouseButton> and the MouseButtonInput stream \
                     only: bevy_picking has no Back, Forward or Other pointer button, so bevy_ui \
                     buttons and pointer observers never see it",
                );
            }
            pending.queue.push(Injected::Mouse(button, cmd.action, addressed));
        }
        InputKind::Scroll => {
            // This was a stub that returned success and wrote nothing — the worst of both, because a
            // caller scrolling and seeing no movement has no way to tell "ignored" from "the game did
            // not react". A `MouseWheel` message needs a window to be addressed to, so with no primary
            // window it now says so instead of silently doing nothing.
            let window = window.ok_or_else(|| BrpError {
                code: error_codes::INTERNAL_ERROR,
                message: "scroll needs a primary window to address the MouseWheel message to, and \
                          there is none"
                    .to_string(),
                data: None,
            })?;
            let unit = match cmd.unit.as_deref() {
                None | Some("Line") => MouseScrollUnit::Line,
                Some("Pixel") => MouseScrollUnit::Pixel,
                Some(other) => {
                    return Err(invalid_params(format!(
                        "unknown scroll unit `{other}`; expected \"Line\" or \"Pixel\""
                    )))
                }
            };
            if cmd.x.is_none() && cmd.y.is_none() {
                return Err(invalid_params(
                    "a scroll needs an amount: send `x` (sideways) and/or `y` (up is positive); with neither it \
                     would scroll by nothing"
                        .to_string(),
                ));
            }
            let (x, y) = (cmd.x.unwrap_or(0.0), cmd.y.unwrap_or(0.0));
            if !x.is_finite() || !y.is_finite() {
                return Err(invalid_params(format!(
                    "scroll amounts must be finite, got x={x}, y={y}"
                )));
            }
            pending.queue.push(Injected::Scroll { unit, x, y, window: *window });
        }
        InputKind::Cursor => {
            if cmd.clear {
                if cmd.x.is_some() || cmd.y.is_some() {
                    return Err(invalid_params(
                        "`clear` hands the pointer back to the real mouse and takes no position; send \
                         `x` and `y` without `clear` to move it, or `clear` alone to release it"
                            .to_string(),
                    ));
                }
                pending.queue.push(Injected::Cursor(None));
            } else {
                // Both required, and named individually: a caller who sent only `x` has made a
                // mistake, and a `y` defaulted to 0 would put the pointer at the top of the window
                // and look like a working call.
                let x = cmd
                    .x
                    .ok_or_else(|| invalid_params("Missing 'x' for cursor input".to_string()))?;
                let y = cmd
                    .y
                    .ok_or_else(|| invalid_params("Missing 'y' for cursor input".to_string()))?;
                if !x.is_finite() || !y.is_finite() {
                    return Err(invalid_params(format!(
                        "cursor position must be finite, got x={x}, y={y}"
                    )));
                }
                // Deliberately not bounded to the window here. `Window::cursor_position` already
                // reports `None` outside the window area, so a host reading through
                // `cursor_position` sees "off the window" — which is a real state a mouse produces
                // and a thing worth being able to test.
                pending.queue.push(Injected::Cursor(Some(Vec2::new(x, y))));
            }
        }
    }

    let mut reply = json!({
        "success": true,
        // Deliberately not "Input injected": it has been accepted and validated, and lands on the next
        // `PreUpdate`. Claiming it already reached the game is what made the old schedule bug so hard
        // to see — the reply said the key had been delivered while the game never saw it.
        "message": "Input queued; applied on the next frame's PreUpdate",
    });
    if !notes.is_empty() {
        reply["note"] = json!(notes.join(". "));
    }
    Ok(reply)
}
