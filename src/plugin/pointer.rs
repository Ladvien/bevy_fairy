//! Where an injected pointer goes, so that **picking and `bevy_ui` see it**.
//!
//! Injecting `MouseButtonInput` and a [`DebugCursor`](super::input::DebugCursor) position reaches the
//! hosts that read those two. It reaches nothing built on `bevy_picking`, which is every Bevy UI
//! button and every `On<Pointer<Click>>` observer: `mouse_pick_events` reads the aggregated
//! [`WindowEvent`] stream that `bevy_winit` writes, and takes a click's position from the last
//! [`WindowEvent::CursorMoved`] it saw (`bevy_picking-0.20.0/src/input.rs`, `mouse_pick_events`). So
//! an agent could aim and press and the game's own button never knew (issue #9, measured in
//! Sanctuary pass 67).
//!
//! This writes the same messages a real mouse produces, inside the process; nothing here touches the
//! OS or a [`Window`](bevy::window::Window) field, which is what `tests/plugin_leaf.rs` keeps true.
//!
//! # Two routes, chosen by [`DebugCaptureTarget`](super::screenshot::DebugCaptureTarget)
//!
//! - **No capture target: the window route.** [`WindowEvent`] variants addressed to the primary
//!   window. `x`/`y` are logical window pixels.
//! - **A capture target (feature `plugin-screenshot`): the image route.** A picking pointer addressed
//!   to that `Image`, because a camera that renders to an image is skipped by a pointer whose
//!   location is the window, and `bevy_ui`'s picking backend skips cameras whose target is not the
//!   pointer's. `x`/`y` are then pixels of that image (logical, at the target's scale factor 1.0).
//!
//! Exactly one route per call: two pointers writing `PointerId::Mouse` in one frame would fight over
//! its location and hover would flicker between them.

use bevy::ecs::message::Messages;
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

#[cfg(feature = "plugin-screenshot")]
use bevy::camera::{ImageRenderTarget, NormalizedRenderTarget};
#[cfg(feature = "plugin-screenshot")]
use bevy::input::{mouse::MouseButton, ButtonState};
#[cfg(feature = "plugin-screenshot")]
use bevy::picking::pointer::{Location, PointerAction, PointerButton, PointerId, PointerInput};

/// Every sink an injected pointer can reach. All of them optional, so a host that wires
/// `apply_pending_input` by hand without these messages keeps working — it just reaches fewer readers.
#[derive(SystemParam)]
pub struct PointerRoute<'w, 's> {
    primary: Query<'w, 's, Entity, With<PrimaryWindow>>,
    window_events: Option<ResMut<'w, Messages<WindowEvent>>>,
    cursor_messages: Option<ResMut<'w, Messages<CursorMoved>>>,
    #[cfg(feature = "plugin-screenshot")]
    target: Option<Res<'w, super::screenshot::DebugCaptureTarget>>,
    #[cfg(feature = "plugin-screenshot")]
    pointer: Option<ResMut<'w, Messages<PointerInput>>>,
}

impl PointerRoute<'_, '_> {
    /// The window a message should be addressed to: the one given, or the primary when the caller had
    /// none to give ([`Entity::PLACEHOLDER`]).
    pub fn window_or_primary(&self, given: Entity) -> Entity {
        if given == Entity::PLACEHOLDER {
            self.primary.single().unwrap_or(given)
        } else {
            given
        }
    }

    #[cfg(feature = "plugin-screenshot")]
    fn image_location(&self, at: Vec2) -> Option<Location> {
        let target = self.target.as_ref()?;
        self.pointer.as_ref()?;
        Some(Location {
            target: NormalizedRenderTarget::Image(ImageRenderTarget {
                handle: target.image.clone(),
                scale_factor: 1.0,
            }),
            position: at,
        })
    }

    /// The pointer moved from `from` (`None` before any injected move) to `to`.
    pub fn moved(&mut self, from: Option<Vec2>, to: Vec2) {
        let window = self.primary.single();
        if let Ok(window) = window {
            let event = CursorMoved { window, position: to, delta: from.map(|f| to - f) };
            if let Some(messages) = self.cursor_messages.as_mut() {
                messages.write(event.clone());
            }
            #[cfg(feature = "plugin-screenshot")]
            if self.move_on_image(from, to) {
                return;
            }
            if let Some(events) = self.window_events.as_mut() {
                events.write(WindowEvent::CursorMoved(event));
            }
        } else {
            // No window at all: only the image route can still deliver.
            #[cfg(feature = "plugin-screenshot")]
            self.move_on_image(from, to);
        }
    }

    /// Writes the move to the capture image's pointer; `false` when there is no such route.
    #[cfg(feature = "plugin-screenshot")]
    fn move_on_image(&mut self, from: Option<Vec2>, to: Vec2) -> bool {
        let Some(location) = self.image_location(to) else { return false };
        if let Some(pointer) = self.pointer.as_mut() {
            // `mouse_pick_events` computes `position - cursor_last`, with `cursor_last` starting at 0.
            let delta = to - from.unwrap_or(Vec2::ZERO);
            pointer.write(PointerInput::new(PointerId::Mouse, location, PointerAction::Move { delta }));
        }
        true
    }

    /// A button edge at `at` (the injected cursor, or the origin if none was ever set).
    pub fn button(&mut self, message: &MouseButtonInput, at: Vec2) {
        #[cfg(feature = "plugin-screenshot")]
        if let Some(location) = self.image_location(at) {
            // The same three buttons `mouse_pick_events` maps; it drops the rest.
            let button = match message.button {
                MouseButton::Left => PointerButton::Primary,
                MouseButton::Right => PointerButton::Secondary,
                MouseButton::Middle => PointerButton::Middle,
                _ => return,
            };
            let action = match message.state {
                ButtonState::Pressed => PointerAction::Press(button),
                ButtonState::Released => PointerAction::Release(button),
            };
            if let Some(pointer) = self.pointer.as_mut() {
                pointer.write(PointerInput::new(PointerId::Mouse, location, action));
            }
            return;
        }
        #[cfg(not(feature = "plugin-screenshot"))]
        let _ = at;
        if let Some(events) = self.window_events.as_mut() {
            events.write(WindowEvent::MouseButtonInput(message.clone()));
        }
    }

    /// A scroll at `at`.
    pub fn wheel(&mut self, message: &MouseWheel, at: Vec2) {
        #[cfg(feature = "plugin-screenshot")]
        if let Some(location) = self.image_location(at) {
            if let Some(pointer) = self.pointer.as_mut() {
                pointer.write(PointerInput::new(
                    PointerId::Mouse,
                    location,
                    PointerAction::Scroll {
                        x: message.x,
                        y: message.y,
                        unit: message.unit,
                        phase: message.phase,
                    },
                ));
            }
            return;
        }
        #[cfg(not(feature = "plugin-screenshot"))]
        let _ = at;
        if let Some(events) = self.window_events.as_mut() {
            events.write(WindowEvent::MouseWheel(*message));
        }
    }
}
