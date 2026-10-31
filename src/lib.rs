/*
 * bevy_fairy - an agentic debugger for Bevy games
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

//! # bevy_fairy
//!
//! An agentic debugger for Bevy games. It has two halves, and a game uses one of them while
//! an agent uses the other.
//!
//! - **The plugin** ([`plugin`]) is what a *game* links. It registers BRP methods
//!   (`bevy_fairy/status`, `commands`, `screenshot`, `capture`, `input`, `scene`, `guide`,
//!   `quit`) and lets the game register its own typed commands.
//! - **The server** ([`server`]) is the `bevy-fairy` binary. It speaks the Model Context
//!   Protocol to a coding agent and the Bevy Remote Protocol to a running game.
//!
//! The two halves do not depend on each other. A game that has never heard of this crate can
//! still be inspected with Bevy's own BRP methods, and a game that links the plugin can be
//! driven by any BRP client, not only this one.
//!
//! ## Which feature do I want?
//!
//! | you are | you want | your dependency |
//! |---|---|---|
//! | a game | the plugin | `bevy_fairy = { version = "0.2", default-features = false, features = ["plugin"] }` |
//! | an agent | the server | `cargo install bevy_fairy` |
//!
//! A game that takes the plugin as a dependency compiles **none** of the server's
//! dependencies. That is not a promise about the future; `tests/plugin_leaf.rs` fails the
//! build if it stops being true.
//!
//! ## Quick start: a game
//!
//! ```toml
//! [dependencies]
//! bevy = { version = "0.20", features = ["default"] }
//! bevy_fairy = { version = "0.2", default-features = false, features = ["plugin"] }
//! ```
//!
//! This example belongs to the *game*, not to this crate: it uses `bevy`, which this crate
//! only depends on when the `plugin` feature is on, so it is shown rather than compiled.
//!
//! ```text
//! use bevy::prelude::*;
//! use bevy::remote::http::RemoteHttpPlugin;
//! use bevy_fairy::plugin::DebuggerPlugin;
//!
//! fn main() {
//!     App::new()
//!         .add_plugins(DefaultPlugins)
//!         // Owns the BRP RemotePlugin. Never add a second RemotePlugin next to it.
//!         .add_plugins(DebuggerPlugin::default())
//!         .add_plugins(RemoteHttpPlugin::default())
//!         .run();
//! }
//! ```
//!
//! `DebuggerPlugin` needs `bevy::input::InputPlugin` in the app. `DefaultPlugins` includes it;
//! `MinimalPlugins` does not.
//!
//! ## Quick start: an agent
//!
//! ```sh
//! cargo install bevy_fairy
//! bevy-fairy --help
//! ```
//!
//! Then point it at a running game. The game listens on `127.0.0.1:15702` by default; set
//! `BEVY_BRP_HOST` and `BEVY_BRP_PORT` to change that.
//!
//! ## The tool contract
//!
//! Every tool's advertised schema is generated from the same Rust type its handler parses, so
//! the two cannot drift apart. Every result carries the frame it was observed on. A value that
//! cannot be measured is refused with a reason, never answered with a plausible number.

/// The Bevy plugin: what a game links.
#[cfg(feature = "plugin")]
pub mod plugin;

/// The MCP server: what an agent talks to.
#[cfg(feature = "server")]
pub mod server;

// The plugin's own items are re-exported at the root, so a game writes
// `use bevy_fairy::DebuggerPlugin` rather than `use bevy_fairy::plugin::DebuggerPlugin`. The
// module is still there for anyone who wants to be explicit.
#[cfg(feature = "plugin")]
pub use plugin::{
    apply_pending_input, cursor_position, handle_input, ClockRequest, logical_for, typed, DebugCommandInfo,
    DebugCommands, DebugCommandsAppExt, DebugCursor, DebugReady, DebuggerPlugin, InputAction,
    InputCommand, InputKind, PendingInput, SceneLog, ScenePlayback, SceneStep,
};

// The guide overlay and the capture target, which are their own features.
#[cfg(feature = "plugin-guide")]
pub use plugin::{
    evaluate_checkpoint, handle_guide, watch_guide, Checkpoints, Guide, GuideOverlay,
    GuideOverlayPlugin, GuideParams, GuidePlacement, Step, StepRecord, BEAT_SECONDS,
};

#[cfg(feature = "plugin-screenshot")]
pub use plugin::{handle_screenshot, CaptureState, Captures, DebugCaptureTarget, Provenance, ScreenshotParams};

#[cfg(feature = "server")]
pub use server::prelude;

#[cfg(feature = "server")]
pub use server::brp as brp_messages;
