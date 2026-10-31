use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::server::brp_client::{certainly_another_game, BrpClient, GameName, Unnamed};
use crate::server::brp_messages::{
    entity_data_from_query_result, BrpPayload, BrpRequest,
    EntityData,
};
use crate::server::error::Result;
use crate::server::mcp_tools::ObserveRequest;
use crate::server::query_parser::{QueryParser, RegexQueryParser};
use crate::server::state_diff::{StateDiff, StateDiffResult, StateSnapshot};

/// How a `diff` read relates to the one before it.
pub enum Baseline {
    /// There is no earlier read of this query.
    None,
    /// The earlier read of this query was of another game process: entity ids are not comparable.
    AnotherGame,
    /// The earlier read of this query, compared with. The text says why the two reads could not be
    /// confirmed as one game's, when one of them could not name its game.
    Earlier(Box<StateDiffResult>, Option<String>),
}

/// Shared state for the observe tool
pub struct ObserveState {
    parser: RegexQueryParser,
    diff_engine: StateDiff,
    last_snapshot: Option<StateSnapshot>,
    /// The query `last_snapshot` answered. A read of another query is not comparable with it.
    last_query: Option<String>,
    /// The game process `last_snapshot` was read from (`Err`: why it could not say).
    last_game: GameName,
}

impl ObserveState {
    /// Create new observe state
    ///
    /// # Errors
    /// Returns error if query parser initialization fails
    pub fn new() -> Result<Self> {
        Ok(Self {
            parser: RegexQueryParser::new()?,
            diff_engine: StateDiff::new(),
            last_snapshot: None,
            last_query: None,
            last_game: Err(Unnamed { why: "no read yet".to_string(), answered_without_a_name: false }),
        })
    }

    /// Keep `snapshot`, the answer to `query` read from `game`, as the read the next `diff` read of
    /// the same query is compared with.
    pub fn add_snapshot(&mut self, query: &str, snapshot: StateSnapshot, game: GameName) {
        self.last_snapshot = Some(snapshot);
        self.last_query = Some(query.to_string());
        self.last_game = game;
    }

    /// Diff against the last snapshot only when it answered the same `query` and came from the
    /// same game. Two queries cover different entities and components, so comparing them would
    /// report what the queries left out as entities and components the game added or removed. Two
    /// games (a restart) give the same entity ids to other entities, so comparing them would
    /// report the difference between the games as a change.
    pub fn diff_against_last_read_of(
        &self,
        query: &str,
        current_snapshot: &StateSnapshot,
        game: &GameName,
    ) -> Baseline {
        if self.last_query.as_deref() != Some(query) {
            return Baseline::None;
        }
        let Some(last) = self.last_snapshot.as_ref() else {
            return Baseline::None;
        };
        if certainly_another_game(&self.last_game, game) {
            return Baseline::AnotherGame;
        }
        let unverified = match (&self.last_game, game) {
            (Ok(_), Ok(_)) => None,
            (Err(Unnamed { why, .. }), _) | (_, Err(Unnamed { why, .. })) => Some(why.clone()),
        };
        Baseline::Earlier(Box::new(self.diff_engine.diff_snapshots(last, current_snapshot)), unverified)
    }

    /// Number `entities` as the diff engine's next snapshot, without keeping it
    #[must_use]
    pub fn create_snapshot(&mut self, entities: Vec<EntityData>) -> StateSnapshot {
        self.diff_engine.create_snapshot(entities)
    }
}

impl Default for ObserveState {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            // Fallback to basic state if initialization fails
            // This should rarely happen in practice
            panic!("Failed to create default ObserveState - this indicates a serious configuration issue");
        })
    }
}

// Global observe state - in a real implementation this would be injected
static OBSERVE_STATE: std::sync::OnceLock<Arc<RwLock<ObserveState>>> = std::sync::OnceLock::new();

fn get_observe_state() -> Arc<RwLock<ObserveState>> {
    OBSERVE_STATE
        .get_or_init(|| Arc::new(RwLock::new(
            ObserveState::new().expect("Default observe state should initialize successfully")
        )))
        .clone()
}

/// Handle observe tool requests
///
/// # Errors
/// Returns error if query parsing fails, BRP communication fails, or response formatting fails
pub async fn handle(request: &ObserveRequest, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let query = request.query.as_str();
    let diff_mode = request.diff;

    info!("Processing observe query: {} (diff_mode: {})", query, diff_mode);

    let start_time = Instant::now();
    let state = get_observe_state();
    let state_guard = state.read().await;

    let (brp_request, limit) = match state_guard.parser.parse(query) {
        Ok(parsed) => (parsed.request, parsed.limit),
        Err(e) => {
            warn!("Query parsing failed: {}", e);
            return Ok(json!({
                "error": "Query parsing failed",
                "message": e.to_string(),
                "help": state_guard.parser.help()
            }));
        }
    };

    drop(state_guard); // Release the lock before async operations

    // Execute BRP request
    let client_connected = {
        let client = brp_client.read().await;
        client.is_connected()
    };

    if !client_connected {
        warn!("BRP client not connected");
        return Ok(json!({
            "error": "BRP client not connected",
            "message": "Cannot execute query - not connected to Bevy game",
            "brp_connected": false
        }));
    }

    // A filter that names a component the game does not register matches nothing, and the
    // game answers `[]` for it, which reads as "no such entities". Check the names first.
    let mut brp_request = brp_request;
    let named = filter_names(&brp_request);
    if !named.is_empty() {
        let registered = match registered_components(&brp_client).await {
            Ok(registered) => registered,
            Err(message) => {
                return Ok(json!({
                    "error": "could not check the component names in the query",
                    "message": message,
                    "query": query
                }));
            }
        };
        let unknown: Vec<Value> = named
            .iter()
            .filter(|name| !registered.contains(name))
            .map(|name| {
                json!({
                    "name": name,
                    "registered_paths_ending_with_it": registered_paths_ending_with(name, &registered),
                })
            })
            .collect();
        if !unknown.is_empty() {
            return Ok(json!({
                "error": "component not registered in the game",
                "message": "the game answers [] to a filter naming a component it does not register, \
                            which would read as 'no such entities'. Use the full type path.",
                "unknown": unknown,
                "query": query,
                "example": "find entities with component bevy_ecs::name::Name",
            }));
        }
    }

    // `show entity N` names no components, and `world.get_components` answers an empty list with
    // an empty map, so every entity read as having none. Ask for every component the game can
    // read instead. `world.list_components` for one entity is no help here: it answers Rust type
    // names, which are not the reflected paths `world.get_components` takes when a type sets its
    // own `type_path`. The ones the entity lacks come back as "not present" and are dropped below.
    let asks_for_every_component = names_no_components(&brp_request);
    if asks_for_every_component {
        let registered = match registered_components(&brp_client).await {
            Ok(registered) => registered,
            Err(message) => {
                return Ok(json!({
                    "error": "could not list the game's components",
                    "message": message,
                    "query": query
                }));
            }
        };
        if let Some(params) = brp_request.params.as_mut() {
            params["components"] = json!(registered);
        }
    }

    let brp_response = {
        let mut client = brp_client.write().await;
        match client.send_request(&brp_request).await {
            Ok(response) => response,
            Err(e) => {
                error!("BRP request failed: {}", e);
                return Ok(json!({
                    "error": "BRP request failed",
                    "message": e.to_string(),
                    "query": query
                }));
            }
        }
    };

    // Process response and handle diff mode
    let (result_json, count, matched, diff_result) = match brp_response.payload {
        BrpPayload::Result(mut value) => {
            if asks_for_every_component {
                drop_not_present_errors(&mut value);
            }
            // Decode the method-specific payload into EntityData where possible
            let decoded_entities = entity_data_from_query_result(&value)
                .or_else(|| {
                    // world.get_components result: {"components": {...}}; the reply does not name
                    // the entity, the request did. Without an id there is nothing honest to key a
                    // diff snapshot on, so decode nothing rather than call it entity 0.
                    let entity = brp_request
                        .params
                        .as_ref()
                        .and_then(|p| p.get("entity"))
                        .and_then(|e| e.as_u64())?;
                    value
                        .get("components")
                        .and_then(|c| c.as_object())
                        .map(|map| {
                            vec![EntityData {
                                id: entity,
                                components: map
                                    .iter()
                                    .map(|(k, v)| (k.clone(), v.clone()))
                                    .collect(),
                            }]
                        })
                })
                .unwrap_or_default();

            // `world.query` has no limit, so `find N entities` cuts the returned rows. The diff
            // below still runs on every match: the game answers in table order, a despawn moves
            // another row into the freed slot, and a diff of the cut alone reported an entity that
            // only moved into the cut as spawned. `matched` says how many there were, so a cut
            // answer never reads as the whole set.
            let matched = match (limit, value.as_array_mut()) {
                (Some(limit), Some(rows)) => {
                    let matched = rows.len();
                    rows.truncate(limit);
                    Some((limit, matched))
                }
                _ => None,
            };
            let returned = matched.map_or(decoded_entities.len(), |(limit, _)| decoded_entities.len().min(limit));
            let count = count_of(&value, returned);

            // Compare with the previous `diff` read of the same query first, then keep this read as
            // the next one's baseline. Keeping it first compared every read with itself: "no
            // changes", always.
            let diff_result = if diff_mode {
                let game = brp_client.write().await.game_instance().await;
                let mut state_guard = state.write().await;
                let current_snapshot = state_guard.create_snapshot(decoded_entities);
                let baseline = state_guard.diff_against_last_read_of(query, &current_snapshot, &game);
                state_guard.add_snapshot(query, current_snapshot, game);
                baseline
            } else {
                Baseline::None
            };

            (value, count, matched, diff_result)
        }
        BrpPayload::Error(error) => {
            warn!("BRP returned error: {}", error.message);
            return Ok(json!({
                "error": "BRP error",
                "code": error.code,
                "message": error.message,
                "details": error.data
            }));
        }
    };

    let execution_time = start_time.elapsed().as_millis() as u64;

    info!(
        "Query '{}' completed in {}ms, {} {} (diff_mode: {})",
        query, execution_time, count.1, count.0, diff_mode
    );

    let mut response = json!({
        "result": result_json,
        "metadata": {
            "query": query,
            "execution_time_ms": execution_time,
            "read_at": chrono::Utc::now().to_rfc3339(),
            "diff_mode": diff_mode,
        }
    });
    response["metadata"][count.0] = json!(count.1);
    if let Some((limit, matched)) = matched {
        response["metadata"]["limit"] = json!(limit);
        response["metadata"]["matched"] = json!(matched);
    }

    // Add diff information if available
    match diff_result {
        Baseline::Earlier(diff_result, unverified) => {
            let grouped_changes = {
                let state_guard = state.read().await;
                state_guard.diff_engine.group_changes(&diff_result.changes)
            };

            response["diff"] = json!({
                "summary": diff_result.summary,
                "changes": diff_result.changes,
                "grouped_changes": grouped_changes,
                "before_timestamp": diff_result.before_snapshot.timestamp.to_rfc3339(),
                "after_timestamp": diff_result.after_snapshot.timestamp.to_rfc3339(),
                "colored_output": diff_result.format_colored(),
                "unexpected_changes_count": diff_result.unexpected_changes().len(),
            });
            if let Some(why) = unverified {
                response["diff"]["game_instance_unverified"] = json!(format!(
                    "the two reads could not be confirmed as one game's: {why}"
                ));
            }
        }
        Baseline::AnotherGame => {
            response["diff"] = json!({
                "baseline": true,
                "message": "The earlier `diff` read of this query was of another game (the game restarted \
                            since): entity ids name other entities there, so the two are not compared. \
                            This read is the new baseline: repeat the same query with `diff: true` to see \
                            what changed.",
            });
        }
        Baseline::None if diff_mode => {
            response["diff"] = json!({
                "baseline": true,
                "message": "No earlier `diff` read of this exact query to compare with. This read is the \
                            baseline: repeat the same query with `diff: true` to see what changed.",
            });
        }
        Baseline::None => {}
    }
    if let (Some((_, matched)), Some(diff)) = (matched, response.get_mut("diff")) {
        diff["compares"] = json!(format!(
            "all {matched} matched entities, not only the rows in `result`"
        ));
    }

    Ok(response)
}

/// How many of what `result` holds. `world.list_components` answers an array of type names,
/// and calling its length an entity count told an agent a 323-type registry was 323 entities.
fn count_of(value: &Value, decoded_entities: usize) -> (&'static str, usize) {
    if decoded_entities > 0 {
        return ("entity_count", decoded_entities);
    }
    match value {
        Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_string) => {
            ("component_type_count", items.len())
        }
        Value::Array(items) => ("entity_count", items.len()),
        // world.get_components for a single entity
        Value::Object(_) => ("entity_count", 1),
        _ => ("entity_count", 0),
    }
}

/// The component names a request's `filter` mentions, `with` first then `without`.
fn filter_names(request: &BrpRequest) -> Vec<String> {
    let filter = request.params.as_ref().and_then(|params| params.get("filter"));
    ["with", "without"]
        .iter()
        .filter_map(|key| filter?.get(*key)?.as_array())
        .flatten()
        .filter_map(|name| name.as_str().map(String::from))
        .collect()
}

/// Registered component paths that are `name`, or end in `::name`.
fn registered_paths_ending_with(name: &str, registered: &[String]) -> Vec<String> {
    let suffix = format!("::{name}");
    registered
        .iter()
        .filter(|path| path.as_str() == name || path.ends_with(&suffix))
        .cloned()
        .collect()
}

/// Whether `request` is a `world.get_components` that names no components.
fn names_no_components(request: &BrpRequest) -> bool {
    request.method == crate::server::brp::builtin_methods::BRP_GET_COMPONENTS_METHOD
        && request
            .params
            .as_ref()
            .and_then(|params| params.get("components"))
            .and_then(Value::as_array)
            .is_none_or(|names| names.is_empty())
}

/// Drop the "not present" errors `world.get_components` answers for each component it was asked
/// for that the entity does not have. Asked for every registered component, they say nothing
/// about the entity; any other error (a component that could not be serialised) is the game's
/// answer and stays.
fn drop_not_present_errors(value: &mut Value) {
    let not_present = i64::from(bevy_remote::error_codes::COMPONENT_NOT_PRESENT);
    if let Some(errors) = value.get_mut("errors").and_then(Value::as_object_mut) {
        errors.retain(|_, error| error.get("code").and_then(Value::as_i64) != Some(not_present));
    }
}

/// Every component type path the game has registered.
async fn registered_components(brp_client: &Arc<RwLock<BrpClient>>) -> std::result::Result<Vec<String>, String> {
    let request = crate::server::query_parser::make_list_components_request();
    let response = brp_client
        .write()
        .await
        .send_request(&request)
        .await
        .map_err(|e| e.to_string())?;
    match response.payload {
        BrpPayload::Result(Value::Array(names)) => {
            Ok(names.iter().filter_map(|n| n.as_str().map(String::from)).collect())
        }
        BrpPayload::Result(other) => Err(format!("world.list_components answered {other}")),
        BrpPayload::Error(error) => Err(error.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_observe_query_parsing() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let request = ObserveRequest { query: "list all entities".to_string(), diff: false };
        let result = handle(&request, brp_client).await.unwrap();

        // Should return error since BRP client is not connected
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn test_invalid_query() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let request = ObserveRequest { query: "invalid query syntax".to_string(), diff: false };
        let result = handle(&request, brp_client).await.unwrap();

        assert_eq!(result.get("error").unwrap(), "Query parsing failed");
        assert!(result.get("help").is_some());
    }

    #[test]
    fn a_list_of_type_names_is_counted_as_types_not_entities() {
        let names = json!(["bevy_ecs::name::Name", "fixture_game::Crate"]);
        assert_eq!(count_of(&names, 0), ("component_type_count", 2));
        let rows = json!([{"entity": 1, "components": {}}]);
        assert_eq!(count_of(&rows, 1), ("entity_count", 1));
        assert_eq!(count_of(&json!([]), 0), ("entity_count", 0));
    }

    #[test]
    fn filter_names_are_collected_from_with_and_without() {
        let request = BrpRequest {
            method: "world.query".to_string(),
            id: None,
            params: Some(json!({"data": {}, "filter": {"with": ["A", "B"], "without": ["C"]}})),
        };
        assert_eq!(filter_names(&request), vec!["A", "B", "C"]);
    }

    #[test]
    fn a_short_name_is_matched_to_the_registered_path_it_ends() {
        let registered = vec!["bevy_ecs::name::Name".to_string(), "fixture_game::Crate".to_string()];
        assert_eq!(registered_paths_ending_with("Crate", &registered), vec!["fixture_game::Crate"]);
        assert!(registered_paths_ending_with("Cr", &registered).is_empty());
    }
}
