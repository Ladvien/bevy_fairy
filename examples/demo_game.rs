//! The fixture game, rendered offscreen and headless, so the README's GIFs can be recorded from
//! the server's own `screenshot` tool.
//!
//! The world is [`fixture_game`]'s, unchanged: the same six crates, the same score, the same
//! mover and the same spinner with its planted bug. This file only adds meshes, a camera that
//! renders into an `Image`, a light, and one game command, `demo/spawn_cube`. Nothing here writes
//! a fixture value, so every answer an agent gets from this game is the answer the fixture's
//! constants predict.
//!
//! It opens no window and needs no display: the camera renders into the [`DebugCaptureTarget`]
//! image, and `ScheduleRunnerPlugin` drives the frames.
//!
//! ```sh
//! cargo run --features plugin-screenshot --example demo_game            # port 15702, 60 fps
//! FIXTURE_PORT=15990 FIXTURE_FPS=10 cargo run --features plugin-screenshot --example demo_game
//! ```
//!
//! `scripts/record_demo.py` records the README's GIFs from it.

#[allow(dead_code)]
#[path = "fixture_game.rs"]
mod fixture_game;
#[path = "../tests/helpers/headless.rs"]
mod headless;

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy::render::render_resource::TextureFormat;
use bevy::window::ExitCondition;
use bevy_fairy::{DebugCaptureTarget, DebugCommandInfo, DebugCommandsAppExt, DebuggerPlugin};
use fixture_game::{Crate, Mover, Score, Spinner};
use serde::Deserialize;
use serde_json::{json, Value};

/// The params of `demo/spawn_cube`. `deny_unknown_fields` makes a typo a named error, not a silent
/// default, so the agent learns the right shape from the error and the example.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnCube {
    at: [f32; 3],
}

fn spawn_cube(
    In(params): In<SpawnCube>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) -> Result<Value, String> {
    let [x, y, z] = params.at;
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::default())),
        MeshMaterial3d(materials.add(StandardMaterial::from_color(Color::srgb(0.6, 0.3, 0.8)))),
        Transform::from_xyz(x, y, z),
    ));
    Ok(json!({ "spawned_at": [x, y, z] }))
}

fn setup_view(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // The camera renders into an image, not a window, so the capture needs no display. The same
    // handle is the plugin's capture target.
    let target = images.add(Image::new_target_texture(
        640,
        360,
        TextureFormat::Bgra8UnormSrgb,
        None,
    ));
    commands.spawn((
        Camera3d::default(),
        RenderTarget::from(target.clone()),
        Transform::from_xyz(1.0, 5.0, 11.0).looking_at(Vec3::new(1.0, 1.2, 0.0), Vec3::Y),
    ));
    commands.insert_resource(DebugCaptureTarget { image: target });
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(4.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// Gives each crate a box, in a row. A crate that weighs less than nothing is red.
fn dress_crates(
    crates: Query<(Entity, &Crate), Added<Crate>>,
    mut placed: Local<u32>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, krate) in &crates {
        let colour = if krate.weight < 0.0 {
            Color::srgb(0.85, 0.2, 0.2)
        } else {
            Color::srgb(0.8, 0.65, 0.4)
        };
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(0.8, 0.8, 0.8))),
            MeshMaterial3d(materials.add(StandardMaterial::from_color(colour))),
            Transform::from_xyz(-5.0 + 2.0 * *placed as f32, 0.4, -2.0),
        ));
        *placed += 1;
    }
}

/// Gives the mover a ball. Only `y` and `z` are set: the fixture measures `MOVER_SPEED` on `x`.
fn dress_mover(
    mut movers: Query<(Entity, &mut Transform), Added<Mover>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, mut transform) in &mut movers {
        transform.translation.y = 0.5;
        transform.translation.z = 2.0;
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Sphere::new(0.5))),
            MeshMaterial3d(
                materials.add(StandardMaterial::from_color(Color::srgb(0.25, 0.45, 0.9))),
            ),
        ));
    }
}

/// Gives the spinner a box, high enough above the crates that it never hides one.
fn dress_spinner(
    spinners: Query<Entity, Added<Spinner>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for entity in &spinners {
        commands.entity(entity).insert((
            Mesh3d(meshes.add(Cuboid::new(1.5, 1.5, 1.5))),
            MeshMaterial3d(
                materials.add(StandardMaterial::from_color(Color::srgb(0.3, 0.75, 0.4))),
            ),
            Transform::from_xyz(2.0, 3.0, 0.0),
        ));
    }
}

/// Turns the spinner's box to the spinner's angle, so the picture freezes exactly when the data
/// does.
fn show_spin(mut spinners: Query<(&Spinner, &mut Transform)>) {
    for (spinner, mut transform) in &mut spinners {
        transform.rotation = Quat::from_rotation_x(0.4) * Quat::from_rotation_y(spinner.angle);
    }
}

fn main() {
    let port = fixture_game::env_or("FIXTURE_PORT", fixture_game::DEFAULT_PORT);
    let fps = fixture_game::env_or("FIXTURE_FPS", fixture_game::DEFAULT_FPS);
    assert!(
        fps.is_finite() && fps > 0.0,
        "the demo's frame cap must be a finite number above zero, got {fps}"
    );
    App::new()
        .add_plugins(
            // No OS window backend, which panics without a display server; ScheduleRunnerPlugin
            // drives frames.
            headless::without_os_window(DefaultPlugins.set(WindowPlugin {
                primary_window: None,
                // No window ever exists, so the default condition would exit on frame one.
                exit_condition: ExitCondition::DontExit,
                ..default()
            })),
        )
        .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
            1.0 / fps,
        )))
        // Owns the BRP RemotePlugin. Never add a second RemotePlugin next to it.
        .add_plugins(DebuggerPlugin::default())
        .add_plugins(RemoteHttpPlugin::default().with_port(port))
        .register_type::<Crate>()
        .register_type::<Score>()
        .register_type::<Mover>()
        .register_type::<Spinner>()
        .insert_resource(Score {
            points: fixture_game::SCORE_POINTS,
        })
        .insert_resource(ClearColor(Color::srgb(0.10, 0.11, 0.15)))
        .add_debug_command(
            DebugCommandInfo::new(
                "demo/spawn_cube",
                "Spawn a purple cube at a world position in metres",
            )
            .params("at: [x, y, z], the cube's centre in metres")
            .example(json!({ "at": [0.0, 0.5, 0.0] })),
            spawn_cube,
        )
        .add_systems(Startup, (fixture_game::spawn_world, setup_view))
        .add_systems(
            Update,
            (
                fixture_game::move_mover,
                (fixture_game::spin, show_spin).chain(),
                dress_crates,
                dress_mover,
                dress_spinner,
            ),
        )
        .run();
}
