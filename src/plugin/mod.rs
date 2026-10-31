//! The game-side half of `bevy_fairy`: a Bevy plugin that lets a coding agent
//! drive a running game over BRP (JSON-RPC over HTTP), without a shared filesystem.
//!
//! Adding [`DebuggerPlugin`] gives the game:
//!
//! - **Game commands.** [`DebugCommandsAppExt::add_debug_command`] registers a typed
//!   handler under a `<game>/<verb>` name. Its params are parsed, and its example is
//!   checked at startup. Commands are discovered at run time through
//!   `bevy_fairy/commands`, so an agent learns the vocabulary from the game itself.
//! - **Readiness.** `bevy_fairy/status` says whether the game has finished loading.
//!   Commands answer "not ready" until then. Use [`DebuggerPlugin::host_signals_ready`]
//!   to take control of that signal.
//! - **Captures.** `bevy_fairy/screenshot` starts a capture from the host's
//!   offscreen image or from the primary window. `bevy_fairy/capture` returns the
//!   PNG as base64 over BRP. Nothing is written to disk unless a path is given.
//! - **Scenes.** Every recorded command call is logged. `bevy_fairy/scene` returns
//!   that log as a replayable file. Setting `BEVY_FAIRY_SCENE` replays a file at
//!   startup, one step per frame.
//! - **Time and quit.** `bevy_fairy/time` pauses the virtual clock or changes its
//!   speed. `bevy_fairy/quit` exits cleanly.
//! - **Ring buffer.** `bevy_fairy/ring` keeps the newest N frames of a `world.query`
//!   result, recorded inside the game every frame, and dumps them with frame numbers.
//! - **UI text.** With `plugin-ui-text`, `bevy_fairy/ui_text` lists the `bevy_ui` text the
//!   player can read, each node with its on-screen rect and, when hidden, why.
//! - **Input.** `bevy_fairy/input` injects keys, clicks, drags and text as the
//!   window would receive them.
//!
//! Research behind the design: Gregory, *Game Engine Architecture* 3rd ed., §10.4–10.6
//! (game-registered console commands, a debug camera and pause); Ostrowski & Aroudj,
//! "Automated Regression Testing within Video Game Development", GSTF JoC 3:10 (2013),
//! DOI 10.7603/s40601-013-0010-4 (the game is driven over a network connection, possibly
//! on another machine); Yang et al., SWE-agent, DOI 10.48550/arXiv.2405.15793 (a small
//! action set, concise feedback, guardrails that catch mistakes early); Mindermann, Keck
//! & Wagner, QRS 2018, DOI 10.1109/QRS.2018.00028 (every parameter is explained, and each
//! command ships a checked example); Matthews, *Idiomatic Rust* (2024) ch. 6 (few
//! dependencies, few feature flags); Xu, "AI Agent Systems" (2026),
//! DOI 10.48550/arXiv.2601.01743 (stable tool schemas that an agent can learn in context).
//!
//! # Quick start
//!
//! ```text
//! use bevy::prelude::*;
//! use bevy::remote::http::RemoteHttpPlugin;
//! use bevy_fairy::{DebugCommandInfo, DebugCommandsAppExt, DebuggerPlugin};
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     // Owns the BRP RemotePlugin. Never add a second RemotePlugin next to it.
//!     .add_plugins(DebuggerPlugin::default())
//!     .add_plugins(RemoteHttpPlugin::default())
//!     .add_debug_command(
//!         DebugCommandInfo::new("demo/spawn_cube", "Spawn a unit cube at a point.")
//!             .params("at: [x, y, z] in metres")
//!             .example(serde_json::json!({"at": [0.0, 0.0, 0.0]})),
//!         |In(p): In<SpawnCube>, mut commands: Commands| -> Result<serde_json::Value, String> {
//!             commands.spawn(Transform::from_translation(Vec3::from(p.at)));
//!             Ok(serde_json::json!({"spawned": true}))
//!         },
//!     )
//!     .run();
//! # #[derive(serde::Deserialize)] struct SpawnCube { at: [f32; 3] }
//! ```
//!
//! `DebuggerPlugin` needs `bevy::input::InputPlugin` in the app. `DefaultPlugins`
//! includes it; `MinimalPlugins` does not.

use bevy::prelude::*;
use bevy::remote::{BrpError, RemotePlugin};

mod commands;
mod input;
mod quit;
mod ready;
mod pointer;
mod ring;
mod scene;
mod time;

#[cfg(feature = "plugin-guide")]
mod guide;
#[cfg(feature = "plugin-ui-text")]
mod ui_text;
#[cfg(feature = "plugin-screenshot")]
mod screenshot;

pub use commands::{DebugCommandInfo, DebugCommands, DebugCommandsAppExt};
pub use input::{
    apply_pending_input, cursor_position, logical_for, typed, DebugCursor, InputAction,
    InputCommand, InputKind, PendingInput,
};
pub use ready::DebugReady;
pub use time::ClockRequest;
pub use scene::{SceneLog, ScenePlayback, SceneStep};

#[cfg(feature = "plugin-guide")]
pub use guide::{
    evaluate_checkpoint, handle_guide, watch_guide, Checkpoints, Guide, GuideOverlay,
    GuideOverlayPlugin, GuideParams, GuidePlacement, Step, StepRecord, BEAT_SECONDS,
};
#[cfg(feature = "plugin-screenshot")]
pub use screenshot::{
    handle_screenshot, CaptureState, Captures, DebugCaptureTarget, Provenance, ScreenshotParams,
};

// The handlers above are public so that tests and examples can drive them directly.
// `run_system_once_with` can run each one under `MinimalPlugins`, without a running
// BRP server.
pub use input::handle_input;

/// Builds the JSON-RPC error that every BRP handler in this crate returns.
pub(crate) fn brp_error(code: i16, message: impl Into<String>) -> BrpError {
    BrpError {
        code,
        message: message.into(),
        data: None,
    }
}

/// The game-side plugin. Add it before `RemoteHttpPlugin`, and do not add a second
/// `RemotePlugin`: this plugin owns it and registers every `bevy_fairy/*` method.
///
/// Requires `bevy::input::InputPlugin` in the app.
#[derive(Default)]
pub struct DebuggerPlugin {
    host_signals_ready: bool,
    host_owns_clock: bool,
}

impl DebuggerPlugin {
    /// Stops the plugin from reporting ready on its own. The host sets
    /// [`DebugReady`] to `true` when its scenes have loaded. Until then, every game
    /// command answers "not ready yet".
    pub fn host_signals_ready(mut self) -> Self {
        self.host_signals_ready = true;
        self
    }
}

impl DebuggerPlugin {
    /// Stops the plugin from writing `Time<Virtual>`. `bevy_fairy/time` then only records a
    /// [`ClockRequest`]; the host reads it, ORs `pause` into its own hold and multiplies `speed` into
    /// its own factor. Without this, the plugin re-applies the request every frame, before
    /// bevy_time's clock system, so it wins against a host that writes the clock later in the frame.
    pub fn host_owns_clock(mut self) -> Self {
        self.host_owns_clock = true;
        self
    }
}

impl Plugin for DebuggerPlugin {
    fn build(&self, app: &mut App) {
        let remote = RemotePlugin::default()
            .with_method_main("bevy_fairy/input", input::handle_input)
            .with_method_main("bevy_fairy/status", ready::handle_status)
            .with_method_main("bevy_fairy/commands", commands::handle_commands)
            .with_method_main("bevy_fairy/scene", scene::handle_scene)
            .with_method_main("bevy_fairy/quit", quit::handle_quit);
        #[cfg(feature = "plugin-screenshot")]
        let remote = remote
            .with_method_main("bevy_fairy/screenshot", screenshot::handle_screenshot)
            .with_method_main("bevy_fairy/capture", screenshot::handle_capture);
        #[cfg(feature = "plugin-guide")]
        let remote = remote
            .with_method_main("bevy_fairy/guide", guide::handle_guide)
            .with_watching_method_main("bevy_fairy/guide+watch", guide::watch_guide);
        app.add_plugins(remote);

        app.insert_resource(DebugReady(!self.host_signals_ready))
            .init_resource::<DebugCommands>()
            .init_resource::<SceneLog>()
            .init_resource::<ready::FrameTicks>()
            .insert_resource(ready::GameInstance::new())
            .add_systems(Last, ready::tick_frames)
            .add_systems(Update, scene::replay_next_step);
        #[cfg(feature = "plugin-screenshot")]
        app.init_resource::<Captures>();

        time::register(app, self.host_owns_clock);
        ring::register(app);
        #[cfg(feature = "plugin-ui-text")]
        ui_text::register(app);

        if let Ok(path) = std::env::var(scene::SCENE_ENV) {
            scene::start_playback(app, &path);
        }

        #[cfg(feature = "plugin-guide")]
        app.add_plugins(guide::GuideOverlayPlugin)
            .init_resource::<guide::Guide>()
            .init_resource::<guide::Checkpoints>()
            // After the host's frame, so a checkpoint reads the state this frame ended with, and
            // ahead of `RemoteLast`, where the watch stream reports what this recorded.
            .add_systems(Last, guide::evaluate_checkpoint);

        // Injected keys and clicks are written as messages. `apply_pending_input` runs
        // before `InputSystems`, which folds those messages into `ButtonInput`.
        app.init_resource::<input::PendingInput>()
            .add_message::<bevy::input::keyboard::KeyboardInput>()
            .add_message::<bevy::input::mouse::MouseButtonInput>()
            .add_message::<bevy::input::mouse::MouseWheel>()
            // What `bevy_picking` reads instead of the three above; see `pointer`.
            .add_message::<bevy::window::WindowEvent>()
            .add_message::<bevy::window::CursorMoved>()
            .init_resource::<input::DebugCursor>()
            .add_systems(
                PreUpdate,
                input::apply_pending_input.before(bevy::input::InputSystems),
            );
        // The image route writes `PointerInput` directly, which picking consumes in `ProcessInput`;
        // ahead of it, so a click lands the frame it is applied rather than the one after.
        #[cfg(feature = "plugin-screenshot")]
        app.configure_sets(
            PreUpdate,
            bevy::picking::PickingSystems::ProcessInput.after(input::apply_pending_input),
        );
    }

    fn finish(&self, app: &mut App) {
        assert!(
            app.is_plugin_added::<bevy::input::InputPlugin>(),
            "DebuggerPlugin needs bevy::input::InputPlugin: injected keys and clicks are written as KeyboardInput/MouseButtonInput messages, and InputPlugin owns the systems that fold those into ButtonInput. Without it every injection is accepted and read by nobody. Add InputPlugin (DefaultPlugins includes it; MinimalPlugins does not)."
        );
    }
}
