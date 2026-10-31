//! `experiment` and `observe` against a game that cannot name its process and then restarts as one
//! that can. A history or a diff baseline names entities by id, and a restarted game gives the same
//! ids to other entities. A game that answered `bevy_fairy/status` without an `instance` (no plugin,
//! or an older one) cannot become a game that names itself: a process does not gain a plugin. So the
//! state recorded against the anonymous game is another game's, not "unverified".
//!
//! The game here is an in-process fake of the BRP endpoint, because the fixture game always has the
//! plugin. Each test uses its own fake and its own tool, and the two tools keep separate
//! process-wide state, so they can run side by side.

use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{BevyDebuggerTools, ExperimentRequest, NoArgs, ObserveRequest};
use rmcp::handler::server::tool::Parameters;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

/// How the fake game answers `bevy_fairy/status`.
#[derive(Clone)]
enum Status {
    /// A game with the plugin: it names the process it is.
    Named(&'static str),
    /// A game with no plugin: "method not found".
    NoPlugin,
    /// A game whose plugin is older than the server: a status with no `instance`.
    OlderPlugin,
    /// A game that answers, with an internal error: it says nothing about which process it is.
    Failing,
}

/// What the fake game currently is, and what has been asked of it.
struct FakeGame {
    status: Status,
    /// The `n` of the one `my_game::Thing` it holds, as entity 7.
    n: u64,
    /// Every method received, in order.
    calls: Vec<String>,
}

type SharedGame = Arc<Mutex<FakeGame>>;

async fn answer(State(game): State<SharedGame>, body: String) -> Json<Value> {
    let request: Value = serde_json::from_str(&body).expect("the client sends JSON");
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut game = game.lock().expect("fake game lock");
    game.calls.push(method.clone());
    let result = match method.as_str() {
        "bevy_fairy/status" => match &game.status {
            Status::Named(instance) => Ok(json!({"ready": true, "instance": instance, "frame": 1})),
            Status::NoPlugin => Err((-32601, "method not found: bevy_fairy/status".to_string())),
            Status::OlderPlugin => Ok(json!({"ready": true, "frame": 1})),
            Status::Failing => Err((-32603, "internal error".to_string())),
        },
        "world.query" => Ok(json!([{"entity": 7, "components": {"my_game::Thing": {"n": game.n}}}])),
        "world.spawn_entity" => Ok(json!({"entity": 7})),
        "world.list_components" => Ok(json!(["my_game::Thing"])),
        "world.despawn_entity" => Ok(Value::Null),
        other => Err((-32601, format!("method not found: {other}"))),
    };
    Json(match result {
        Ok(value) => json!({"jsonrpc": "2.0", "id": id, "result": value}),
        Err((code, message)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    })
}

/// Start a fake game that answers `status` as given, and a connected tool set.
async fn game_answering(status: Status) -> (SharedGame, BevyDebuggerTools) {
    let game: SharedGame = Arc::new(Mutex::new(FakeGame { status, n: 1, calls: Vec::new() }));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake game port");
    let port = listener.local_addr().expect("fake game address").port();
    let app = Router::new().route("/", post(answer)).with_state(game.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let config = Config { bevy_brp_host: "127.0.0.1".to_string(), bevy_brp_port: port, ..Config::default() };
    let tools = BevyDebuggerTools::new(Arc::new(RwLock::new(BrpClient::new(&config))));
    tools.status(Parameters(NoArgs {})).await.expect("status returns a result");
    (game, tools)
}

fn parse(result: rmcp::model::CallToolResult) -> Value {
    let text: String = result.content.iter().filter_map(|c| c.as_text().map(|t| t.text.clone())).collect();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("the tool returned non-JSON ({e}): {text}"))
}

async fn experiment(tools: &BevyDebuggerTools, request: Value) -> Value {
    let request: ExperimentRequest = serde_json::from_value(request).expect("the request deserialises");
    parse(tools.experiment(Parameters(request)).await.expect("experiment returns a result"))
}

async fn observe_diff(tools: &BevyDebuggerTools, query: &str) -> Value {
    let request = ObserveRequest { query: query.to_string(), diff: true };
    parse(tools.observe(Parameters(request)).await.expect("observe returns a result"))
}

fn calls_of(game: &SharedGame, method: &str) -> usize {
    game.lock().expect("fake game lock").calls.iter().filter(|c| c.as_str() == method).count()
}

/// Spawn one thing against `game`, restart it as the game `game-b`, and return what `history` and
/// `undo` then say, with the number of despawns the game received because of the undo.
async fn spawn_restart_and_undo(status: Status) -> (Value, Value, Value, usize) {
    let (game, tools) = game_answering(status).await;
    experiment(&tools, json!({"action": "clear_history"})).await;

    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{"type": "spawn",
            "components": [{"type_id": "my_game::Thing", "value": {"n": 1}}]}]}),
    )
    .await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");

    // The game restarts, now with a plugin that names itself. Its entity 7 is not the one spawned.
    game.lock().expect("fake game lock").status = Status::Named("game-b");

    let history = experiment(&tools, json!({"action": "history"})).await;
    let despawns_before = calls_of(&game, "world.despawn_entity");
    let undo = experiment(&tools, json!({"action": "undo"})).await;
    let despawns = calls_of(&game, "world.despawn_entity") - despawns_before;
    (run, history, undo, despawns)
}

#[tokio::test(flavor = "multi_thread")]
async fn experiment_refuses_undo_of_a_history_recorded_against_a_game_that_could_not_name_itself() {
    // A game that answered without naming itself: no plugin, or a plugin older than the server. A
    // process does not gain a plugin, so the game that names itself now is another one.
    for status in [Status::NoPlugin, Status::OlderPlugin] {
        let (run, history, undo, despawns) = spawn_restart_and_undo(status.clone()).await;
        let which = match status {
            Status::NoPlugin => "no plugin",
            _ => "older plugin",
        };
        assert!(
            run["game_instance_unverified"].is_string(),
            "{which}: an anonymous game's history cannot be told from another's, and says so: {run}"
        );
        assert_eq!(history["stale"], json!(true), "{which}: the history is of another game: {history}");
        assert!(undo["error"].is_string(), "{which}: undo of another game's history must be refused: {undo}");
        assert!(
            undo["message"].as_str().is_some_and(|m| m.contains("another game") && m.contains("discarded")),
            "{which}: the refusal says why and what happened to the history: {undo}"
        );
        assert_eq!(
            despawns, 0,
            "{which}: nothing may be written into the new game: its entity 7 is not the one the old game spawned"
        );
    }

    // A game that was only not asked successfully says nothing about itself: the game that names
    // itself now may be the same one, so the history stays unverified and the action goes ahead.
    let (run, history, undo, despawns) = spawn_restart_and_undo(Status::Failing).await;
    assert!(run["game_instance_unverified"].is_string(), "{run}");
    assert_eq!(history["stale"], json!(null), "a game that gave no answer is not known to be another: {history}");
    assert!(undo.get("error").is_none(), "{undo}");
    assert!(undo["game_instance_unverified"].is_string(), "the undo says it could not tell: {undo}");
    assert_eq!(despawns, 1, "the undo ran: nothing says the game is another one");
}

#[tokio::test(flavor = "multi_thread")]
async fn observe_diff_compares_a_read_with_one_of_another_game_only_when_it_cannot_be_the_same() {
    let query = "find entities with component my_game::Thing";
    let (game, tools) = game_answering(Status::NoPlugin).await;
    let restart = |status: Status, n: u64| {
        let mut game = game.lock().expect("fake game lock");
        game.status = status;
        game.n = n;
    };

    let first = observe_diff(&tools, query).await;
    assert_eq!(first["diff"]["baseline"], true, "{first}");

    // The game restarts, now with a plugin that names itself, and its entity 7 is another entity.
    restart(Status::Named("game-b"), 2);
    let read = observe_diff(&tools, query).await;
    assert_eq!(read["diff"]["baseline"], true, "the earlier read was of another game: {read}");
    assert!(read["diff"].get("changes").is_none(), "nothing is compared across games: {read}");
    assert!(
        read["diff"]["message"].as_str().is_some_and(|m| m.contains("another game")),
        "the reply says why it is a baseline: {read}"
    );

    // A game that answers without an `instance` is still diffed, and the reply says the two reads
    // could not be confirmed as one game's.
    restart(Status::OlderPlugin, 3);
    let read = observe_diff(&tools, query).await;
    assert!(read["diff"]["changes"].is_array(), "{read}");
    assert!(read["diff"]["game_instance_unverified"].is_string(), "{read}");

    // The game that answered without an `instance` was not the one that names itself now.
    restart(Status::Named("game-c"), 4);
    let read = observe_diff(&tools, query).await;
    assert_eq!(read["diff"]["baseline"], true, "an older plugin cannot become a newer one: {read}");
    assert!(read["diff"].get("changes").is_none(), "{read}");

    // A game that gave an internal error says nothing: the next game may be the same one.
    restart(Status::Failing, 4);
    let read = observe_diff(&tools, query).await;
    assert!(read["diff"]["game_instance_unverified"].is_string(), "{read}");
    restart(Status::Named("game-d"), 5);
    let read = observe_diff(&tools, query).await;
    assert!(read["diff"].get("baseline").is_none(), "a game that gave no answer is not known to be another: {read}");
    assert!(read["diff"]["changes"].is_array(), "{read}");
    assert!(read["diff"]["game_instance_unverified"].is_string(), "{read}");
}
