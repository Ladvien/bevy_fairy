//! The agent-facing tools (`status`, `commands`, `run`, `screenshot`, `scene`, `quit`, `brp`),
//! driven against an in-process fake of the game's BRP endpoint, so no Bevy game is needed.
//!
//! The fake answers the JSON-RPC shapes that the companion `bevy_fairy` serves. Each test
//! builds its own `Config` and client, so nothing reads or writes the process environment.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{extract::State, routing::post, Json, Router};
use base64::Engine as _;
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{
    BevyDebuggerTools, BrpCallRequest, BurstRequest, NoArgs, RunRequest, SceneAction, SceneRequest, ScreenshotRequest,
};
use rmcp::handler::server::tool::Parameters;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

/// A 1x1 PNG, base64. The tests check that the bytes round-trip and carry the PNG signature.
const PNG_1X1_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

#[derive(Default)]
struct FakeGame {
    ready: bool,
    /// How many `bevy_fairy/capture` polls answer `pending` before the capture is `done`.
    pending_polls: u32,
    polls_seen: u32,
    /// Captures delivered so far; each one is on a later game frame than the last.
    delivered: u64,
    /// The commands the game has recorded, as `bevy_fairy/scene get` returns them.
    steps: Vec<Value>,
}

type SharedGame = Arc<Mutex<FakeGame>>;

fn game_with(ready: bool, pending_polls: u32) -> SharedGame {
    Arc::new(Mutex::new(FakeGame { ready, pending_polls, ..FakeGame::default() }))
}

/// Answers one JSON-RPC request. The body is read as text, so no `Content-Type` is needed.
async fn answer(State(game): State<SharedGame>, body: String) -> Json<Value> {
    let request: Value = match serde_json::from_str(&body) {
        Ok(request) => request,
        Err(e) => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("parse error: {e}")}
            }))
        }
    };
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let outcome = {
        let mut game = game.lock().expect("fake game lock");
        dispatch(&mut game, &method, &params)
    };
    Json(match outcome {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

fn dispatch(game: &mut FakeGame, method: &str, params: &Value) -> Result<Value, (i16, String)> {
    match method {
        "bevy_fairy/status" => Ok(json!({
            "ready": game.ready,
            "frame": 1,
            "commands": 2,
            "capture": "offscreen",
            "replay": null
        })),
        "bevy_fairy/commands" => Ok(json!({"commands": [
            {
                "name": "sanctuary/light",
                "description": "Set the palette.",
                "params": "palette: str",
                "example": {"palette": "moonlit_night"}
            },
            {
                "name": "sanctuary/clear",
                "description": "Despawn inspect props.",
                "params": "",
                "example": {}
            }
        ]})),
        "sanctuary/light" | "sanctuary/clear" => {
            let recorded = if params.is_null() { json!({}) } else { params.clone() };
            game.steps.push(json!({"method": method, "params": recorded}));
            Ok(json!({"ok": true}))
        }
        "bevy_fairy/screenshot" => Ok(json!({"id": 7, "path": null})),
        "bevy_fairy/capture" => {
            if game.polls_seen < game.pending_polls {
                game.polls_seen += 1;
                Ok(json!({"status": "pending"}))
            } else {
                let frame = 40 + 10 * game.delivered;
                game.delivered += 1;
                Ok(json!({"status": "done", "width": 1, "height": 1, "png_base64": PNG_1X1_BASE64,
                    "source": "window", "frame_requested": frame, "frame_captured": frame + 3}))
            }
        }
        "bevy_fairy/scene" => match params.get("action").and_then(Value::as_str) {
            Some("get") => Ok(json!({"format": "bevy_fairy_scene", "version": 1, "steps": game.steps})),
            Some("clear") => {
                let cleared = game.steps.len();
                game.steps.clear();
                Ok(json!({"cleared": cleared}))
            }
            _ => Err((-32602, "bevy_fairy/scene takes action get or clear".to_string())),
        },
        "world.get_components" => Ok(json!({"components": {"bevy_transform::components::transform::Transform": {"translation": [1.0, 2.0, 3.0]}}})),
        "bevy_fairy/quit" => Ok(json!({"quitting": true})),
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

/// Serve the fake game on a free loopback port and return that port.
async fn start_fake_game(game: SharedGame) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake game port");
    let port = listener.local_addr().expect("fake game address").port();
    let app = Router::new().route("/", post(answer)).with_state(game);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    port
}

fn tools_for(port: u16) -> BevyDebuggerTools {
    let config = Config { bevy_brp_host: "127.0.0.1".to_string(), bevy_brp_port: port, ..Config::default() };
    BevyDebuggerTools::new(Arc::new(RwLock::new(BrpClient::new(&config))))
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|item| item.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn unknown_command_is_refused_with_the_closest_known_name() {
    let port = start_fake_game(game_with(true, 0)).await;
    let tools = tools_for(port);

    let result = tools
        .run(Parameters(RunRequest { command: "sanctuary/lite".to_string(), params: None }))
        .await
        .expect("run returns a result");

    assert_eq!(result.is_error, Some(true));
    let text = text_of(&result);
    assert!(text.starts_with("unknown command `sanctuary/lite`; did you mean `sanctuary/light`?"), "{text}");
    assert!(text.contains("known: sanctuary/clear, sanctuary/light, bevy_fairy/time"), "{text}");
}

#[tokio::test]
async fn run_waits_until_the_game_is_ready_then_runs() {
    let game = game_with(false, 0);
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);

    let flip = {
        let game = game.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            game.lock().expect("fake game lock").ready = true;
        })
    };

    let started = Instant::now();
    let result = tools
        .run(Parameters(RunRequest {
            command: "sanctuary/light".to_string(),
            params: Some(json!({"palette": "moonlit_night"})),
        }))
        .await
        .expect("run returns a result");
    let waited = started.elapsed();
    flip.await.expect("the flip task finishes");

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert_eq!(text_of(&result), r#"{"ok":true}"#);
    assert!(waited >= Duration::from_millis(500), "run returned after {waited:?}, before the game was ready");
}

#[tokio::test]
async fn screenshot_writes_a_png_after_one_pending_reply() {
    let game = game_with(true, 1);
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    let dir = tempfile::tempdir().expect("temp dir");
    let save_to = dir.path().join("nested").join("shot.png");

    let result = tools
        .screenshot(Parameters(ScreenshotRequest {
            save_to: Some(save_to.to_string_lossy().into_owned()),
            source: None,
            region: None,
            zoom: None,
            inline: None,
        }))
        .await
        .expect("screenshot returns a result");

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let text = text_of(&result);
    assert!(text.starts_with("saved ") && text.contains("(1x1)"), "{text}");
    assert!(
        text.ends_with("source window, requested at frame 40, delivered at frame 43"),
        "the reply must say which source was read and on which frames: {text}"
    );
    assert_eq!(game.lock().expect("fake game lock").polls_seen, 1, "the first poll must have been pending");

    let written = std::fs::read(&save_to).expect("the PNG was written, creating its directory");
    let expected = base64::engine::general_purpose::STANDARD.decode(PNG_1X1_BASE64).expect("fixture decodes");
    assert_eq!(written, expected);
    assert_eq!(&written[..8], &PNG_SIGNATURE);
}

#[tokio::test]
async fn scene_save_writes_the_exact_file_format() {
    let port = start_fake_game(game_with(true, 0)).await;
    let tools = tools_for(port);

    let ran = tools
        .run(Parameters(RunRequest {
            command: "sanctuary/light".to_string(),
            params: Some(json!({"palette": "moonlit_night"})),
        }))
        .await
        .expect("run returns a result");
    assert_ne!(ran.is_error, Some(true), "{}", text_of(&ran));

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("scenes").join("flame.json");
    let result = tools
        .scene(Parameters(SceneRequest {
            action: SceneAction::Save,
            path: Some(path.to_string_lossy().into_owned()),
        }))
        .await
        .expect("scene returns a result");

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    assert!(text_of(&result).ends_with("(1 steps)"), "{}", text_of(&result));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("scene file written"))
        .expect("scene file is JSON");
    assert_eq!(
        written,
        json!({
            "format": "bevy_fairy_scene",
            "version": 1,
            "steps": [{"method": "sanctuary/light", "params": {"palette": "moonlit_night"}}]
        })
    );

    let refused = tools
        .scene(Parameters(SceneRequest { action: SceneAction::Save, path: None }))
        .await
        .expect("scene returns a result");
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(text_of(&refused), "scene save needs a path");
}

#[tokio::test]
async fn commands_lists_the_game_commands_then_the_built_in_time_last() {
    let port = start_fake_game(game_with(true, 0)).await;
    let tools = tools_for(port);

    let result = tools.commands(Parameters(NoArgs {})).await.expect("commands returns a result");

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let text = text_of(&result);
    let blocks: Vec<&str> = text.split("\n\n").collect();
    assert_eq!(blocks.len(), 3, "{text}");
    assert!(
        blocks[0].starts_with("sanctuary/clear — Despawn inspect props.\n  params: (none)\n  example: {}"),
        "{text}"
    );
    assert!(
        blocks[1].starts_with(
            "sanctuary/light — Set the palette.\n  params: palette: str\n  example: {\"palette\":\"moonlit_night\"}"
        ),
        "{text}"
    );
    assert!(blocks[2].starts_with("bevy_fairy/time — "), "{text}");
}

#[tokio::test]
async fn status_reports_the_game_and_the_url_it_talks_to() {
    let port = start_fake_game(game_with(true, 0)).await;
    let tools = tools_for(port);

    let result = tools.status(Parameters(NoArgs {})).await.expect("status returns a result");

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let status: Value = serde_json::from_str(&text_of(&result)).expect("status is JSON");
    assert_eq!(status.get("ready"), Some(&Value::Bool(true)));
    assert_eq!(status.get("url"), Some(&Value::from(format!("http://127.0.0.1:{port}"))));
}

#[tokio::test]
async fn every_tool_says_no_game_answers_when_nothing_listens() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind a port to free");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let tools = tools_for(port);
    let expected = format!(
        "bevy-fairy: no game answering at http://127.0.0.1:{port} — start the game with DebuggerPlugin and RemoteHttpPlugin, or set BEVY_BRP_HOST/BEVY_BRP_PORT"
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let shot_path = dir.path().join("shot.png");
    let scene_path = dir.path().join("scene.json");

    let outcomes = vec![
        ("status", tools.status(Parameters(NoArgs {})).await),
        ("commands", tools.commands(Parameters(NoArgs {})).await),
        (
            "run",
            tools
                .run(Parameters(RunRequest { command: "sanctuary/light".to_string(), params: None }))
                .await,
        ),
        (
            "brp",
            tools
                .brp(Parameters(BrpCallRequest { method: "world.query".to_string(), params: None }))
                .await,
        ),
        (
            "screenshot",
            tools
                .screenshot(Parameters(ScreenshotRequest {
                    save_to: Some(shot_path.to_string_lossy().into_owned()),
                    source: None,
                    region: None,
                    zoom: None,
                    inline: None,
                }))
                .await,
        ),
        (
            "scene save",
            tools
                .scene(Parameters(SceneRequest {
                    action: SceneAction::Save,
                    path: Some(scene_path.to_string_lossy().into_owned()),
                }))
                .await,
        ),
        ("scene show", tools.scene(Parameters(SceneRequest { action: SceneAction::Show, path: None })).await),
        ("scene clear", tools.scene(Parameters(SceneRequest { action: SceneAction::Clear, path: None })).await),
        ("quit", tools.quit(Parameters(NoArgs {})).await),
    ];

    for (name, outcome) in outcomes {
        let result = outcome.expect("tool returns a result");
        assert_eq!(result.is_error, Some(true), "{name} should report an error");
        assert_eq!(text_of(&result), expected, "{name}");
    }
    assert!(!shot_path.exists(), "screenshot must not write a file when no game answers");
    assert!(!scene_path.exists(), "scene save must not write a file when no game answers");
}

#[tokio::test]
async fn burst_labels_every_cell_with_the_frame_the_game_reported() {
    let game = game_with(true, 0);
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    let dir = tempfile::tempdir().expect("temp dir");

    let started = Instant::now();
    let result = tools
        .burst(Parameters(BurstRequest {
            frames: Some(3),
            fps: Some(10.0),
            source: None,
            save_dir: Some(dir.path().join("b").to_string_lossy().into_owned()),
            full_frames: Some(true),
            inline: Some(true),
        }))
        .await
        .expect("burst returns a result");
    let elapsed = started.elapsed();

    assert_ne!(result.is_error, Some(true), "{}", text_of(&result));
    let text = text_of(&result);
    // The fake delivers on frames 43, 53, 63, 73: the reference and three frames.
    assert!(text.contains(" ms, source window, requested at frame 40, delivered at frame 43"), "{text}");
    assert!(text.contains("frame 3: +"), "{text}");
    assert!(text.contains("delivered at frame 73"), "{text}");
    assert!(text.ends_with("4 distinct game frames across 4 captures"), "{text}");
    assert!(elapsed >= Duration::from_millis(300), "3 frames at 10 fps take at least 300 ms, took {elapsed:?}");
    assert_eq!(game.lock().expect("fake game lock").delivered, 4, "one reference and three frames");
    assert_eq!(result.content.len(), 2, "the sheet is returned inline when asked");

    // 4 cells of 1x1 frames: one row of 4, each cell a label strip over a 1x1 picture.
    let sheet = image::open(dir.path().join("b").join("sheet.png")).expect("sheet.png is a PNG");
    assert_eq!(sheet.width(), 4);
    assert!(dir.path().join("b").join("ref.png").exists());
    for n in 1..=3 {
        assert!(dir.path().join("b").join(format!("frame-{n}.png")).exists(), "frame-{n}.png with full_frames");
    }
}

#[tokio::test]
async fn burst_refuses_out_of_range_arguments_before_capturing() {
    let game = game_with(true, 0);
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    for (frames, fps) in [(Some(8), None), (Some(0), None), (None, Some(11.0)), (None, Some(0.1))] {
        let result = tools
            .burst(Parameters(BurstRequest { frames, fps, source: None, save_dir: None, full_frames: None, inline: None }))
            .await
            .expect("burst returns a result");
        assert_eq!(result.is_error, Some(true), "{frames:?} {fps:?}");
        assert!(text_of(&result).contains("Example:"), "{}", text_of(&result));
    }
    assert_eq!(game.lock().expect("fake game lock").delivered, 0, "nothing was captured");
}
