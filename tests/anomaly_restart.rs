//! `detect_anomaly` against a game that restarts. The detectors keep a window of past values per
//! entity id, the server outlives the game, and a restarted game gives the same ids to other
//! entities: samples of two games must not be scored against each other.
//!
//! The game here is an in-process fake of the BRP endpoint, because the fixture game has no
//! `Velocity`. This file holds one test on purpose: the detectors are process-wide, so a second
//! test in the same binary would read this one's samples.

use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{AnomalyRequest, BevyDebuggerTools, NoArgs};
use rmcp::handler::server::tool::Parameters;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

/// What the fake game currently is: which process it says it is, and how fast its one entity moves.
struct FakeGame {
    instance: Option<String>,
    speed: f64,
}

type SharedGame = Arc<Mutex<FakeGame>>;

async fn answer(State(game): State<SharedGame>, body: String) -> Json<Value> {
    let request: Value = serde_json::from_str(&body).expect("the client sends JSON");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let game = game.lock().expect("fake game lock");
    let result = match method {
        "bevy_fairy/status" => match &game.instance {
            Some(instance) => Ok(json!({"ready": true, "instance": instance, "frame": 1})),
            None => Err((-32601, "method not found: bevy_fairy/status".to_string())),
        },
        "world.query" => Ok(json!([
            {"entity": 7, "components": {"my_game::Velocity": {"linear": [game.speed, 0.0, 0.0]}}}
        ])),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Json(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

async fn detect(tools: &BevyDebuggerTools) -> Value {
    let request: AnomalyRequest = serde_json::from_value(json!({"action": "detect"})).expect("the request deserialises");
    let result = tools.detect_anomaly(Parameters(request)).await.expect("detect_anomaly returns a result");
    let text: String = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("detect_anomaly returned non-JSON ({e}): {text}"))
}

#[tokio::test]
async fn detect_anomaly_does_not_score_a_restarted_game_against_the_one_before_it() {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame { instance: Some("game-a".to_string()), speed: 1.0 }));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake game port");
    let port = listener.local_addr().expect("fake game address").port();
    let app = Router::new().route("/", post(answer)).with_state(game.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let config = Config { bevy_brp_host: "127.0.0.1".to_string(), bevy_brp_port: port, ..Config::default() };
    let tools = BevyDebuggerTools::new(Arc::new(RwLock::new(BrpClient::new(&config))));
    tools.status(Parameters(NoArgs {})).await.expect("status returns a result");

    // Fifteen readings of one steady speed: past min_samples (10) and the bound a z-score can reach.
    for _ in 0..15 {
        let reply = detect(&tools).await;
        assert_eq!(reply["anomalies"], json!([]), "a steady speed is not an anomaly: {reply}");
    }

    // The game restarts, and its entity 7 is a different entity that moves much faster. Against its
    // own history (none) there is nothing to call anomalous yet. Against the previous game's it is
    // 50 times the mean speed.
    {
        let mut game = game.lock().expect("fake game lock");
        game.instance = Some("game-b".to_string());
        game.speed = 50.0;
    }
    let reply = detect(&tools).await;
    assert_eq!(
        reply["anomalies"],
        json!([]),
        "entity 7 of the new game was scored against the speeds of the old one: {reply}"
    );
    assert!(
        reply["history_discarded"]["reason"].as_str().is_some_and(|r| r.contains("another game")),
        "the reply says the old samples were dropped, and why: {reply}"
    );

    // Within the new game the next reading is judged as usual, with the history of the new game.
    let reply = detect(&tools).await;
    assert!(reply.get("history_discarded").is_none(), "{reply}");
    assert!(reply.get("game_instance_unverified").is_none(), "{reply}");

    // A game that cannot name itself is still judged, and says it could not be told from another.
    game.lock().expect("fake game lock").instance = None;
    let reply = detect(&tools).await;
    assert!(
        reply["game_instance_unverified"].as_str().is_some_and(|why| why.contains("bevy_fairy/status")),
        "an anonymous game's samples cannot be told from another's, and the reply must say so: {reply}"
    );
}
