//! The tool contract: every tool's advertised arguments must reach the handler that reads them.
//!
//! This file is the failing-first check for the tools whose schemas and handlers disagreed.
//! Before the fix, `experiment`, `hypothesis`, `detect_anomaly` and `stress_test` each advertised
//! fields that no handler read, while the handlers dispatched on an `action` the tools never sent.
//! `stress_test` has since been removed from `tools/list`: every operation it performed failed
//! against a real game while it reported success, and its frame times were random numbers.
//!
//! The fake game below answers the BRP methods the handlers call, so the tests exercise the real
//! handler paths rather than a stub of them.

use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{
    AnomalyRequest, BevyDebuggerTools, BrpCallRequest, ExperimentRequest, HypothesisRequest,
};
use rmcp::handler::server::tool::Parameters;
use rmcp::model::CallToolResult;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[derive(Default)]
struct FakeGame {
    /// Every BRP method the server asked for, with its params, in order.
    calls: Vec<(String, Value)>,
}

type SharedGame = Arc<Mutex<FakeGame>>;

async fn answer(State(game): State<SharedGame>, body: String) -> Json<Value> {
    let request: Value = serde_json::from_str(&body).expect("the client sends JSON");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    {
        let mut game = game.lock().expect("fake game lock");
        game.calls.push((method.clone(), params.clone()));
    }

    // A game that answers, but not with a JSON-RPC reply this server can read.
    if method == "fake.unreadable" {
        return Json(json!({"not": "a json-rpc reply"}));
    }

    let result = match method.as_str() {
        // The real `world.query` result is an array of rows, each with an `entity` id and a
        // `components` map. The server's own parser (`entity_data_from_query_result`) reads that
        // shape, so the fake answers it.
        "world.query" => Ok(json!([
            {"entity": 1, "components": {"Transform": {"x": 1.0}}},
            {"entity": 2, "components": {"Health": 100}}
        ])),
        "world.get_components" => Ok(json!({"components": {}})),
        "world.list_components" => Ok(json!({"components": ["Transform", "Health"]})),
        "world.spawn_entity" | "world.despawn_entity" | "world.insert_components"
        | "world.remove_components" => Ok(json!({"entity": 1, "success": true})),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    Json(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    })
}

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
    let config = Config {
        bevy_brp_host: "127.0.0.1".to_string(),
        bevy_brp_port: port,
        ..Config::default()
    };
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

/// Build a request the way an agent's call arrives: as JSON, deserialised into the tool's struct.
/// Going through JSON means the test exercises the same path a real call takes, and it does not
/// need editing when a field is added.
fn request<T: DeserializeOwned>(value: Value) -> Parameters<T> {
    Parameters(serde_json::from_value(value).expect("the request deserialises"))
}

/// Bring the client up to the game, the way a session does: one call that reaches it.
///
/// The legacy tools gate on the client's `connected` flag, and that flag is set by a successful
/// request. A real session makes its first call against the game before it makes any gated one, so
/// the test does the same instead of starting from a disconnected client.
async fn connect(tools: &BevyDebuggerTools) {
    let result = tools
        .status(Parameters(bevy_fairy::server::mcp_tools::NoArgs {}))
        .await
        .expect("status returns a result");
    assert!(!text_of(&result).contains("\"error\""), "the fake game should answer status");
}

/// The action a caller asks for must reach the handler. Before the fix the tool never sent an
/// `action` at all, so every branch was unreachable.
#[tokio::test]
async fn experiment_runs_the_requested_action() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);

    // `history` needs no game connection and no actions, so it is the cleanest probe of dispatch.
    let result = tools
        .experiment(request::<ExperimentRequest>(json!({"action": "history"})))
        .await
        .expect("experiment returns a result");

    let text = text_of(&result);
    assert!(
        !text.contains("BRP client not connected"),
        "the history action needs no game, so it must not have asked for one; text: {text}"
    );
    let value: Value = serde_json::from_str(&text).expect("the history reply is JSON");
    assert!(
        value.get("history").is_some(),
        "the history action should have run; text: {text}"
    );
}

/// `detect_anomaly` must run the detection the caller asked for, against the live game. Before the
/// fix the tool sent `type`/`sensitivity`/`window`, none of which the handler reads, so it always
/// ran the default configuration and the caller's thresholds were dropped.
#[tokio::test]
async fn detect_anomaly_analyses_the_live_game_with_the_given_thresholds() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    connect(&tools).await;

    let result = tools
        .detect_anomaly(request::<AnomalyRequest>(json!({
            "action": "detect",
            "limit": 7,
            "min_severity": 0.25,
        })))
        .await
        .expect("detect_anomaly returns a result");

    let text = text_of(&result);
    assert!(
        !text.contains("BRP client not connected"),
        "the detection must reach the game; text: {text}"
    );
    assert!(
        !text.contains("Unknown action"),
        "the action was not understood at all; text: {text}"
    );

    let value: Value = serde_json::from_str(&text).expect("the detection reply is JSON");
    let summary = value.get("summary").expect("the reply carries a summary");
    assert_eq!(
        summary.get("entities_analyzed").and_then(Value::as_u64),
        Some(2),
        "the detection must have analysed the game it was pointed at; text: {text}"
    );
    assert_eq!(
        summary.get("limit_applied").and_then(Value::as_u64),
        Some(7),
        "the caller's limit must reach the handler; text: {text}"
    );
    assert_eq!(
        summary.get("min_severity_filter").and_then(Value::as_f64),
        Some(0.25),
        "the caller's severity floor must reach the handler; text: {text}"
    );
}

/// `hypothesis` must run the test the caller asked for. Before the fix it fell through to
/// `test`, which then failed on a missing `description` the schema never advertised.
#[tokio::test]
async fn hypothesis_runs_the_requested_action() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    connect(&tools).await;

    let result = tools
        .hypothesis(request::<HypothesisRequest>(json!({
            "action": "test",
            "description": "the cube keeps moving",
            "success_condition": {"type": "entity_exists"},
            "iterations": 1,
            "timeout": 5,
            "threshold": 0.5,
        })))
        .await
        .expect("hypothesis returns a result");

    let text = text_of(&result);
    assert!(
        !text.contains("Missing 'description' parameter"),
        "the handler read a field the schema never advertised; text: {text}"
    );
    assert!(
        text.contains("the cube keeps moving"),
        "the report should carry the hypothesis that was asked about; text: {text}"
    );
}

/// A tool must refuse an operation it does not have, naming the ones it does. Before the fix the
/// tool could not even name one, because it never sent an `action` at all.
#[tokio::test]
async fn an_unknown_action_is_refused_with_the_valid_ones() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    connect(&tools).await;

    let result = tools
        .hypothesis(request::<HypothesisRequest>(json!({"action": "teleport"})))
        .await
        .expect("hypothesis returns a result");

    let text = text_of(&result);
    assert!(text.contains("Unknown action"), "text: {text}");
    assert!(text.contains("quick_test"), "the error should list the valid actions; text: {text}");
    assert_eq!(
        result.is_error,
        Some(true),
        "a refusal is a failed call, and the client must be told so; text: {text}"
    );
}

/// A query the tool cannot parse is a failed call. `observe` answered it as a success whose body
/// was `{"error": "Query parsing failed", ...}`, so a client that branches on `isError` -- which
/// is what the field is for -- took the refusal for an answer.
#[tokio::test]
async fn an_unparseable_observe_query_comes_back_as_a_tool_error() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    connect(&tools).await;

    let result = tools
        .observe(request::<bevy_fairy::server::mcp_tools::ObserveRequest>(json!({
            "query": "teleport the moon to mars",
        })))
        .await
        .expect("observe returns a result");

    let text = text_of(&result);
    assert!(text.contains("Query parsing failed"), "text: {text}");
    assert_eq!(result.is_error, Some(true), "the refusal must set isError; text: {text}");
}

/// A game that answered did answer, even when this server cannot read the reply. `brp` used to
/// map every client error to "no game answering", so an agent concluded the game was down and
/// stopped looking (eval transcript, spinner_stops, 2026-10-23).
#[tokio::test]
async fn an_unreadable_answer_is_not_reported_as_no_game() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);

    let result = tools
        .brp(request::<BrpCallRequest>(json!({"method": "fake.unreadable"})))
        .await
        .expect("brp returns a result");

    let text = text_of(&result);
    assert_eq!(result.is_error, Some(true), "an unreadable answer is a failed call; text: {text}");
    assert!(
        !text.contains("no game answering"),
        "the fake game answered, so the error must not say nothing did; text: {text}"
    );
    assert!(text.contains("fake.unreadable"), "the error should name the method; text: {text}");
}

/// A handler that returns `Err` (here `experiment execute` with no `actions`) is a failed call the
/// model must see. It used to leave as a JSON-RPC protocol error, which MCP clients may hide from
/// the model, while an `isError` result is always shown (B-048).
#[tokio::test]
async fn a_handler_error_comes_back_as_a_tool_error_the_model_can_read() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame::default()));
    let port = start_fake_game(game.clone()).await;
    let tools = tools_for(port);
    connect(&tools).await;

    let result = tools
        .experiment(request::<bevy_fairy::server::mcp_tools::ExperimentRequest>(json!({"action": "execute"})))
        .await
        .expect("a handler error is a tool result, not a protocol error");

    let text = text_of(&result);
    assert_eq!(result.is_error, Some(true), "text: {text}");
    assert!(text.contains("actions"), "the error should name the missing parameter; text: {text}");
}
