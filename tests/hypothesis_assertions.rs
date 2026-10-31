//! `Assertion::evaluate` must answer from the game.
//!
//! `EntityExists`, `ComponentEquals` and `ComponentMatches` used to log "not fully implemented"
//! and return `true` without asking the game, so the `hypothesis` tool reported every such
//! hypothesis as confirmed. The fake game below has two entities: entity 1 holds `Health = 100`
//! and a `Transform`, entity 2 holds only a `Transform`.

use std::sync::Arc;

use axum::{routing::post, Json, Router};
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::experiment_system::Restoration;
use bevy_fairy::server::hypothesis_system::{Assertion, Predicate};
use serde_json::{json, Value};
use tokio::net::TcpListener;

async fn answer(body: String) -> Json<Value> {
    let request: Value = serde_json::from_str(&body).expect("the client sends JSON");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let result = match method {
        "rpc.discover" => Ok(json!({})),
        "world.query" => Ok(json!([
            {"entity": 1, "components": {"game::Transform": {"x": 1.0}, "game::Health": 100}},
            {"entity": 2, "components": {"game::Transform": {"x": 2.0}}}
        ])),
        other => Err(format!("method not found: {other}")),
    };
    Json(match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(m) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": m}}),
    })
}

async fn client() -> BrpClient {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let app = Router::new().route("/", post(answer));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let config = Config {
        bevy_brp_host: "127.0.0.1".to_string(),
        bevy_brp_port: port,
        ..Config::default()
    };
    BrpClient::new(&config)
}

async fn eval(a: Assertion) -> Result<bool, String> {
    let mut c = client().await;
    a.evaluate(&mut c, &mut Restoration::default()).await.map_err(|e| e.to_string())
}

fn exists(types: &[&str], count: Option<usize>) -> Assertion {
    Assertion::EntityExists {
        component_types: types.iter().map(|s| s.to_string()).collect(),
        count,
    }
}

fn equals(entity_id: Option<u64>, component: &str, value: Value) -> Assertion {
    Assertion::ComponentEquals {
        entity_id,
        component_type: component.to_string(),
        expected_value: value,
    }
}

#[tokio::test]
async fn entity_exists_is_false_for_a_component_nobody_has() {
    assert_eq!(eval(exists(&["Nope"], None)).await, Ok(false));
}

#[tokio::test]
async fn entity_exists_is_true_for_a_short_name_the_game_has() {
    assert_eq!(eval(exists(&["Health"], None)).await, Ok(true));
    assert_eq!(eval(exists(&["Transform", "Health"], None)).await, Ok(true));
}

#[tokio::test]
async fn entity_exists_count_is_the_exact_number_of_matching_entities() {
    assert_eq!(eval(exists(&["Transform"], Some(2))).await, Ok(true));
    assert_eq!(eval(exists(&["Transform"], Some(5))).await, Ok(false));
    assert_eq!(eval(exists(&["Transform", "Health"], Some(1))).await, Ok(true));
}

#[tokio::test]
async fn component_equals_compares_with_the_games_value() {
    assert_eq!(eval(equals(Some(1), "Health", json!(100))).await, Ok(true));
    assert_eq!(eval(equals(Some(1), "Health", json!(7))).await, Ok(false));
    // entity 2 has no Health at all
    assert_eq!(eval(equals(Some(2), "Health", json!(100))).await, Ok(false));
    // no entity named: some entity holding the value is enough
    assert_eq!(eval(equals(None, "Health", json!(100))).await, Ok(true));
    assert_eq!(eval(equals(None, "Health", json!(7))).await, Ok(false));
}

#[tokio::test]
async fn component_matches_is_refused_not_assumed_true() {
    let a = Assertion::ComponentMatches {
        entity_id: Some(1),
        component_type: "Health".to_string(),
        predicate: Predicate::GreaterThan { value: json!(1000) },
    };
    let err = eval(a).await.expect_err("an assertion the server cannot evaluate is refused");
    assert!(err.contains("component_matches"), "names the assertion: {err}");
    let _ = Arc::new(());
}
