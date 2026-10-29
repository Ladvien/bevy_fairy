//! **An injected pointer reaches `bevy_picking`** (issue #9).
//!
//! `bevy_picking` reads the aggregated `WindowEvent` stream, not `MouseButtonInput` and not
//! `DebugCursor`, so a click an agent injected used to be accepted, folded into `ButtonInput`, and
//! seen by no UI button and no `On<PointerClick>` observer. These tests run a real `bevy_ui` node
//! under the real picking plugins and drive it through `bevy_fairy/input`'s own handler, so the only
//! thing under test is whether the game's own picking noticed.
//!
//! The app has a window entity and a camera but no GPU and no OS window: `WinitPlugin` is off and
//! the renderer is created with no backend.

use bevy::app::AppExit;
use bevy::camera::RenderTarget;
use bevy::ecs::system::RunSystemOnce;
use bevy::image::Image;
use bevy::picking::events::{
    PointerClick, PointerDrag, PointerDragEnd, PointerDragStart, PointerOut, PointerOver,
};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::settings::{RenderCreation, WgpuSettings};
use bevy::render::RenderPlugin;
use bevy::winit::WinitPlugin;
use bevy_fairy::{handle_input, DebugCaptureTarget, DebuggerPlugin};
use serde_json::{json, Value};

#[derive(Resource, Default, Debug)]
struct Seen {
    clicks: Vec<Entity>,
    overs: usize,
    outs: usize,
    drag_start: usize,
    drags: usize,
    drag_end: usize,
}

/// The button, at known layout: left 100, top 100, 200 x 100 logical pixels.
const BUTTON_CENTRE: Vec2 = Vec2::new(200.0, 150.0);
const ELSEWHERE: Vec2 = Vec2::new(700.0, 500.0);

fn app(offscreen: bool) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<WinitPlugin>()
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
    .init_resource::<Seen>();

    let mut camera = app.world_mut().spawn(Camera2d);
    if offscreen {
        let mut image = Image::new_target_texture(1000, 600, TextureFormat::Rgba8UnormSrgb, None);
        image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(DebugCaptureTarget { image: handle.clone() });
        camera = app.world_mut().spawn(Camera2d);
        camera.insert(RenderTarget::Image(handle.into()));
    }
    let camera = camera.id();

    let button = app
        .world_mut()
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(100),
                top: px(100),
                width: px(200),
                height: px(100),
                ..default()
            },
            UiTargetCamera(camera),
        ))
        .observe(|e: On<PointerClick>, mut s: ResMut<Seen>| s.clicks.push(e.entity))
        .observe(|_: On<PointerOver>, mut s: ResMut<Seen>| s.overs += 1)
        .observe(|_: On<PointerOut>, mut s: ResMut<Seen>| s.outs += 1)
        .observe(|_: On<PointerDragStart>, mut s: ResMut<Seen>| s.drag_start += 1)
        .observe(|_: On<PointerDrag>, mut s: ResMut<Seen>| s.drags += 1)
        .observe(|_: On<PointerDragEnd>, mut s: ResMut<Seen>| s.drag_end += 1)
        .id();

    app.finish();
    app.cleanup();
    // Layout and the window's own size settle over a few frames.
    for _ in 0..4 {
        app.update();
    }
    (app, button)
}

fn send(app: &mut App, params: Value) {
    let reply = app
        .world_mut()
        .run_system_once_with(handle_input, Some(params))
        .unwrap();
    reply.expect("input accepted");
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

fn cursor(app: &mut App, at: Vec2) {
    send(app, json!({"kind": "Cursor", "x": at.x, "y": at.y}));
    frames(app, 4);
}

fn mouse(app: &mut App, action: &str) {
    send(app, json!({"kind": "Mouse", "button": "Left", "action": action}));
    frames(app, 4);
}

fn seen(app: &App) -> &Seen {
    app.world().resource::<Seen>()
}

fn click_lands(offscreen: bool) {
    let (mut app, button) = app(offscreen);
    cursor(&mut app, BUTTON_CENTRE);
    mouse(&mut app, "Press");
    mouse(&mut app, "Release");
    assert_eq!(seen(&app).clicks, vec![button], "{:?}", seen(&app));
}

#[test]
fn a_click_on_a_window_camera_node_fires_its_observer_exactly_once() {
    click_lands(false);
}

#[test]
fn a_click_on_an_offscreen_camera_node_fires_its_observer_exactly_once() {
    click_lands(true);
}

#[test]
fn a_cursor_alone_hovers_and_moving_off_unhovers() {
    let (mut app, _) = app(false);
    cursor(&mut app, BUTTON_CENTRE);
    assert_eq!((seen(&app).overs, seen(&app).outs), (1, 0), "{:?}", seen(&app));
    cursor(&mut app, ELSEWHERE);
    assert_eq!((seen(&app).overs, seen(&app).outs), (1, 1), "{:?}", seen(&app));
    assert!(seen(&app).clicks.is_empty());
}

#[test]
fn a_press_a_move_and_a_release_is_a_drag() {
    let (mut app, _) = app(false);
    cursor(&mut app, BUTTON_CENTRE);
    mouse(&mut app, "Press");
    cursor(&mut app, BUTTON_CENTRE + Vec2::new(40.0, 10.0));
    cursor(&mut app, ELSEWHERE);
    mouse(&mut app, "Release");
    let s = seen(&app);
    assert_eq!((s.drag_start, s.drag_end), (1, 1), "{s:?}");
    assert!(s.drags >= 2, "{s:?}");
}

#[test]
fn a_click_with_no_cursor_says_where_it_will_land() {
    let (mut app, _) = app(false);
    let reply = app
        .world_mut()
        .run_system_once_with(
            handle_input,
            Some(json!({"kind": "Mouse", "button": "Left", "action": "Press"})),
        )
        .unwrap()
        .unwrap();
    assert!(reply["note"].as_str().unwrap().contains("no injected cursor"), "{reply}");
    let _ = AppExit::Success;
}
