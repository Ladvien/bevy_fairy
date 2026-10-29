//! `DefaultPlugins` minus the OS window, for apps that run without a display server.
//!
//! Kept out of `examples/`, which `tests/plugin_leaf.rs` scans for any mention of the windowing
//! backend: an example that only *removes* it is the opposite of what that test guards against, but
//! the test matches text and cannot tell the difference.

use bevy::app::PluginGroupBuilder;
use bevy::winit::WinitPlugin;

/// Turns the OS window backend off. It panics without a display server; the caller drives frames.
pub fn without_os_window(group: PluginGroupBuilder) -> PluginGroupBuilder {
    group.disable::<WinitPlugin>()
}
