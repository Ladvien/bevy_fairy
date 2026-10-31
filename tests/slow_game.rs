//! A game that is slow, or that answers with an HTTP failure, is not "no game answering".
//!
//! `run`, `brp`, `quit` and `status` told an agent "no game answering at <url> -- start the game"
//! for any request that timed out and for any non-2xx reply. A game in a long frame (a load, a
//! hitch, a window nobody is redrawing) is running and has queued the request, which it may carry
//! out after the server has given up: an agent that believed the game was gone and tried again
//! applied a command twice. The fake game here records what reached it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{extract::State, routing::post, Json, Router};
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{BevyDebuggerTools, BrpCallRequest};
use rmcp::handler::server::tool::Parameters;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[derive(Clone, Copy)]
enum Mode {
    /// Answers after this long, which is past the client's request timeout.
    Slow(Duration),
    /// Answers with an HTTP 500 and no JSON-RPC body.
    ServerError,
}

struct FakeGame {
    mode: Mode,
    /// Requests that reached the game, by method.
    received: Vec<String>,
}

type SharedGame = Arc<Mutex<FakeGame>>;

async fn answer(State(game): State<SharedGame>, body: String) -> Response {
    let request: Value = serde_json::from_str(&body).expect("the client sends JSON");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let mode = {
        let mut game = game.lock().expect("fake game lock");
        game.received.push(method);
        game.mode
    };
    match mode {
        Mode::Slow(delay) => {
            tokio::time::sleep(delay).await;
            Json(json!({"jsonrpc": "2.0", "id": id, "result": {"late": true}})).into_response()
        }
        Mode::ServerError => (StatusCode::INTERNAL_SERVER_ERROR, "the game is on fire").into_response(),
    }
}

async fn tools_for(mode: Mode) -> (SharedGame, BevyDebuggerTools) {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame { mode, received: Vec::new() }));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake game port");
    let port = listener.local_addr().expect("fake game address").port();
    let app = Router::new().route("/", post(answer)).with_state(game.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let config = Config { bevy_brp_host: "127.0.0.1".to_string(), bevy_brp_port: port, ..Config::default() };
    (game, BevyDebuggerTools::new(Arc::new(RwLock::new(BrpClient::new(&config)))))
}

/// What the `brp` tool says for `world.query`, and whether it was an error.
async fn brp_text(tools: &BevyDebuggerTools) -> (bool, String) {
    let request: BrpCallRequest =
        serde_json::from_value(json!({"method": "world.query", "params": {"data": {}}})).expect("request");
    let result = tools.brp(Parameters(request)).await.expect("brp returns a result");
    let text: String = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect();
    (result.is_error == Some(true), text)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_that_answers_with_an_http_failure_is_not_reported_as_absent() {
    let (game, tools) = tools_for(Mode::ServerError).await;
    let (failed, text) = brp_text(&tools).await;
    assert!(failed, "{text}");
    assert_eq!(game.lock().expect("fake game lock").received, ["world.query"], "the request reached the game");
    assert!(!text.contains("no game answering"), "a game that replied is not absent: {text}");
    assert!(text.contains("500"), "the status it answered with: {text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_that_does_not_answer_in_time_is_not_reported_as_absent_and_may_still_act() {
    let (game, tools) = tools_for(Mode::Slow(Duration::from_secs(7))).await;
    let (failed, text) = brp_text(&tools).await;
    assert!(failed, "{text}");
    assert_eq!(game.lock().expect("fake game lock").received, ["world.query"], "the request reached the game");
    assert!(!text.contains("no game answering"), "a game that took the request is not absent: {text}");
    assert!(text.contains("did not answer"), "it says what happened: {text}");
    assert!(
        text.contains("may still") && text.contains("status"),
        "and that the request may yet run, and how to check before sending it again: {text}"
    );
}

/// `watch` samples in one batch (status, the method, status), which is a different send path from
/// `brp`, and said the same wrong thing for the same two games.
async fn watch_text(tools: &BevyDebuggerTools) -> (bool, String) {
    let request: bevy_fairy::server::mcp_tools::WatchRequest =
        serde_json::from_value(json!({"method": "world.query", "params": {"data": {}}, "samples": 2, "interval_ms": 20}))
            .expect("request");
    let result = tools.watch(Parameters(request)).await.expect("watch returns a result");
    let text: String = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect();
    (result.is_error == Some(true), text)
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_does_not_report_a_game_that_failed_to_answer_as_absent_either() {
    let (_, tools) = tools_for(Mode::ServerError).await;
    let (failed, text) = watch_text(&tools).await;
    assert!(failed, "{text}");
    assert!(!text.contains("no game answering"), "a game that replied is not absent: {text}");
    assert!(text.contains("500"), "{text}");

    let (_, tools) = tools_for(Mode::Slow(Duration::from_secs(7))).await;
    let (failed, text) = watch_text(&tools).await;
    assert!(failed, "{text}");
    assert!(!text.contains("no game answering"), "a game that took the request is not absent: {text}");
    assert!(text.contains("may still") && text.contains("status"), "{text}");
}
