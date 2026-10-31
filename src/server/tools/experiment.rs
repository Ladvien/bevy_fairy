use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::brp_client::{BrpClient, GameName, Unnamed};
use crate::server::error::{Error, Result};
use crate::server::experiment_system::{Action, ActionExecutor};

/// The game process the history was recorded against.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Recorded {
    /// No action has been recorded.
    Nothing,
    /// A game that named itself with the `instance` of `bevy_fairy/status`.
    Game(String),
    /// A game that could not name itself, and why: its history cannot be told from another's, unless
    /// a game that names itself answers later (a process does not gain a plugin).
    Unverifiable(Unnamed),
}

/// What comparing the game that answers now with the one the history was recorded against found.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Standing {
    /// Nothing recorded, or the same game.
    Current,
    /// The history is of another game process: a restart reuses entity ids for other entities.
    Stale { entries: usize },
    /// The history is of a game that named itself, and the game now cannot be asked who it is.
    Unconfirmed(String),
    /// Neither game can name itself, so no comparison is possible. The reason says why.
    Unverified(String),
}

impl Standing {
    /// The reply that refuses an operation this standing forbids, or `None` when it may go ahead.
    /// A stale history refuses `undo` and `redo` but not `execute`, which starts a new one. A
    /// history recorded against a game that named itself refuses every operation, `execute`
    /// included, while that game cannot be asked who it is.
    fn refusal(&self, operation: &str, url: &str, stale_refuses: bool) -> Option<Value> {
        match self {
            Standing::Stale { entries } if stale_refuses => Some(json!({
                "error": "history belongs to another game",
                "message": format!(
                    "The {entries} recorded action(s) were taken against another game process than the one answering at {url} now; \
                     a restarted game gives its entities other ids, so `{operation}` would change the wrong ones. \
                     Nothing was changed in the game, and the stale history was discarded. `execute` starts a new history."
                ),
                "history_discarded": {"entries": entries},
            })),
            Standing::Unconfirmed(why) => Some(json!({
                "error": "cannot tell whether the history belongs to this game",
                "message": format!(
                    "The history was recorded against a game that named itself, and the game at {url} cannot now: {why}. \
                     It may be another game, so `{operation}` was not run. `clear_history` discards the history."
                ),
            })),
            _ => None,
        }
    }

    /// Put what the comparison found into the reply of an operation that went ahead.
    fn annotate(self, reply: &mut Value, url: &str) {
        let Some(fields) = reply.as_object_mut() else { return };
        match self {
            Standing::Stale { entries } => {
                fields.insert(
                    "history_discarded".to_string(),
                    json!({
                        "entries": entries,
                        "reason": format!("they were recorded against another game process than the one answering at {url} now (a restart reuses entity ids)"),
                    }),
                );
            }
            Standing::Unverified(why) => {
                fields.insert(
                    "game_instance_unverified".to_string(),
                    Value::from(format!(
                        "the game answering could not be confirmed as the one that ran the recorded actions: {why}"
                    )),
                );
            }
            Standing::Current | Standing::Unconfirmed(_) => {}
        }
    }
}

/// Global experiment state
pub struct ExperimentState {
    executor: ActionExecutor,
    /// Which game the history in `executor` was recorded against.
    recorded: Recorded,
}

impl ExperimentState {
    /// Create new experiment state
    pub fn new() -> Self {
        Self {
            executor: ActionExecutor::new(),
            recorded: Recorded::Nothing,
        }
    }

    /// Compare the game that answers now (`Err`: why it cannot name itself) with the one the
    /// history was recorded against. Changes nothing.
    fn standing(&self, now: &GameName) -> Standing {
        standing_of(&self.recorded, self.executor.entry_count(), now)
    }

    /// Settle on the game now answering before an operation runs: a stale history is dropped, and
    /// an empty one is recorded against the game the operation is about to act on.
    fn adopt(&mut self, now: &GameName) -> Standing {
        let standing = self.standing(now);
        let empty = self.executor.entry_count() == 0;
        if matches!(standing, Standing::Stale { .. }) {
            self.executor.clear_history();
        }
        if empty || matches!(standing, Standing::Stale { .. }) {
            self.recorded = match now {
                Ok(id) => Recorded::Game(id.clone()),
                Err(unnamed) => Recorded::Unverifiable(unnamed.clone()),
            };
        }
        standing
    }
}

/// The decision behind [`ExperimentState::standing`]: `entries` actions are on record, taken
/// against `recorded`, and `now` is the game that answers.
fn standing_of(recorded: &Recorded, entries: usize, now: &GameName) -> Standing {
    match (entries, recorded, now) {
        (0, _, Ok(_)) => Standing::Current,
        (0, _, Err(now)) => Standing::Unverified(now.why.clone()),
        (_, Recorded::Game(then), Ok(id)) if then == id => Standing::Current,
        (_, Recorded::Game(_), Ok(_)) => Standing::Stale { entries },
        (_, Recorded::Game(_), Err(now)) => Standing::Unconfirmed(now.why.clone()),
        // The game that ran the actions answered without naming itself and a game that names itself
        // answers now: a process does not gain a plugin, so it is another one.
        (_, Recorded::Unverifiable(then), Ok(_)) if then.answered_without_a_name => Standing::Stale { entries },
        (_, Recorded::Unverifiable(then), _) => Standing::Unverified(then.why.clone()),
        // Actions on record against no game cannot happen: `adopt` runs before every action.
        (_, Recorded::Nothing, _) => Standing::Unverified("the history records no game".to_string()),
    }
}

impl Default for ExperimentState {
    fn default() -> Self {
        Self::new()
    }
}

// Global experiment state
static EXPERIMENT_STATE: std::sync::OnceLock<Arc<RwLock<ExperimentState>>> =
    std::sync::OnceLock::new();

fn get_experiment_state() -> Arc<RwLock<ExperimentState>> {
    EXPERIMENT_STATE
        .get_or_init(|| Arc::new(RwLock::new(ExperimentState::new())))
        .clone()
}

/// Handle experiment tool requests
pub async fn handle(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    debug!("Experiment tool called with arguments: {}", arguments);

    // Parse action parameter
    let action_str = arguments
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("execute");

    // Actions that don't require BRP connection
    match action_str {
        "history" => return handle_history(brp_client).await,
        "clear_history" => return handle_clear_history().await,
        _ => {}
    }

    // Check connection for actions that need BRP
    let is_connected = {
        let client = brp_client.read().await;
        client.reachable().await
    };

    if !is_connected {
        warn!("BRP client not connected");
        return Ok(json!({
            "error": "BRP client not connected",
            "message": "Cannot execute experiment - not connected to Bevy game",
            "brp_connected": false
        }));
    }

    match action_str {
        "execute" => handle_execute(arguments, brp_client).await,
        "undo" => handle_undo(brp_client).await,
        "redo" => handle_redo(brp_client).await,
        _ => Ok(json!({
            "error": "Unknown action",
            "message": format!("Unknown action: {}", action_str),
            "available_actions": ["execute", "undo", "redo", "history", "clear_history"]
        })),
    }
}

/// Handle execute action
async fn handle_execute(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    // Parse actions array
    let actions_json = arguments
        .get("actions")
        .ok_or_else(|| Error::Validation("Missing 'actions' parameter".to_string()))?;

    let actions: Vec<Action> = if actions_json.is_array() {
        serde_json::from_value(actions_json.clone())
            .map_err(|e| Error::Validation(format!("Failed to parse actions: {e}")))?
    } else {
        // Single action
        vec![serde_json::from_value(actions_json.clone())
            .map_err(|e| Error::Validation(format!("Failed to parse action: {e}")))?]
    };

    if actions.is_empty() {
        return Err(Error::Validation(
            "`execute` needs a non-empty `actions` array, e.g. {\"action\":\"execute\",\"actions\":[...]}; \
             nothing was run"
                .to_string(),
        ));
    }

    info!("Executing {} actions", actions.len());

    let state = get_experiment_state();
    let mut state_guard = state.write().await;
    let mut client = brp_client.write().await;

    let standing = state_guard.adopt(&client.game_instance().await);
    if let Some(refusal) = standing.refusal("execute", client.url(), false) {
        return Ok(refusal);
    }

    let mut results = Vec::new();
    let mut success_count = 0;
    let mut failure_count = 0;

    for action in &actions {
        let result = state_guard
            .executor
            .execute_action(action, &mut client)
            .await?;

        if result.success {
            success_count += 1;
            info!("Action succeeded: {}", result.message);
        } else {
            failure_count += 1;
            warn!("Action failed: {}", result.message);
        }

        results.push(result);
    }

    let mut reply = json!({
        "results": results,
        "summary": {
            "total_actions": actions.len(),
            "successful": success_count,
            "failed": failure_count,
            "success_rate": if actions.is_empty() { 0.0 } else { (success_count as f64) / (actions.len() as f64) }
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    standing.annotate(&mut reply, client.url());
    Ok(reply)
}

/// Handle undo action
async fn handle_undo(brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    info!("Performing undo");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;
    let mut client = brp_client.write().await;

    let standing = state_guard.adopt(&client.game_instance().await);
    if let Some(refusal) = standing.refusal("undo", client.url(), true) {
        return Ok(refusal);
    }
    let result = state_guard.executor.undo(&mut client).await?;

    let mut reply = json!({
        "result": result,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    standing.annotate(&mut reply, client.url());
    Ok(reply)
}

/// Handle redo action
async fn handle_redo(brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    info!("Performing redo");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;
    let mut client = brp_client.write().await;

    let standing = state_guard.adopt(&client.game_instance().await);
    if let Some(refusal) = standing.refusal("redo", client.url(), true) {
        return Ok(refusal);
    }
    let result = state_guard.executor.redo(&mut client).await?;

    let mut reply = json!({
        "result": result,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    standing.annotate(&mut reply, client.url());
    Ok(reply)
}

/// Handle history request. It says which game the history was recorded against and whether the
/// game answering now is that one: `stale` is `true` or `false` when the game can be asked, and
/// `null` when it cannot, with the reason in `game_instance_unverified` while actions are on
/// record. `history` and `count` are the actions `undo` can step back over; `redoable` counts the
/// ones `redo` can step forward over, which a stale check also covers. Reading changes nothing.
async fn handle_history(brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let state = get_experiment_state();
    let state_guard = state.read().await;

    let now = {
        let mut client = brp_client.write().await;
        client.game_instance().await
    };
    let standing = state_guard.standing(&now);
    let history = state_guard.executor.get_history();

    let mut reply = json!({
        "history": history,
        "count": history.len(),
        "redoable": state_guard.executor.entry_count() - history.len(),
        "recorded_against": match &state_guard.recorded {
            Recorded::Game(id) => Value::from(id.as_str()),
            Recorded::Nothing | Recorded::Unverifiable(_) => Value::Null,
        },
        "stale": match &standing {
            Standing::Stale { .. } => Value::Bool(true),
            Standing::Current => Value::Bool(false),
            Standing::Unconfirmed(_) | Standing::Unverified(_) => Value::Null,
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    });
    if let (Standing::Unconfirmed(why) | Standing::Unverified(why), Some(fields)) = (&standing, reply.as_object_mut()) {
        if state_guard.executor.entry_count() > 0 {
            fields.insert("game_instance_unverified".to_string(), Value::from(why.as_str()));
        }
    }
    Ok(reply)
}

/// Handle clear history
async fn handle_clear_history() -> Result<Value> {
    info!("Clearing action history");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;

    state_guard.executor.clear_history();
    state_guard.recorded = Recorded::Nothing;

    Ok(json!({
        "message": "History cleared",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_handle_without_connection() {
        let config = Config { mcp_port: 3000, ..Config::default() };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "execute",
            "actions": []
        });

        let result = handle(args, brp_client).await.unwrap();
        assert_eq!(result.get("error").unwrap(), "BRP client not connected");
    }

    #[tokio::test]
    async fn test_handle_unknown_action() {
        let config = Config { mcp_port: 3000, ..Config::default() };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "unknown"
        });

        let result = handle(args, brp_client).await.unwrap();
        assert!(result.get("error").is_some());
    }

    #[tokio::test]
    async fn test_handle_history() {
        let config = Config { mcp_port: 3000, ..Config::default() };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "history"
        });

        let result = handle(args, brp_client).await.unwrap();
        assert!(result.get("history").is_some());
        assert!(result.get("count").is_some());
    }

    #[tokio::test]
    async fn test_handle_clear_history() {
        let config = Config { mcp_port: 3000, ..Config::default() };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "clear_history"
        });

        let result = handle(args, brp_client).await.unwrap();
        assert_eq!(result.get("message").unwrap(), "History cleared");
    }

    fn game(id: &str) -> Recorded {
        Recorded::Game(id.to_string())
    }

    fn unnamed(why: &str, answered_without_a_name: bool) -> Unnamed {
        Unnamed { why: why.to_string(), answered_without_a_name }
    }

    #[test]
    fn a_history_is_current_only_for_the_game_it_was_recorded_against() {
        let same = Ok("a".to_string());
        let other = Ok("b".to_string());
        assert_eq!(standing_of(&game("a"), 3, &same), Standing::Current);
        assert_eq!(standing_of(&game("a"), 3, &other), Standing::Stale { entries: 3 });
    }

    #[test]
    fn a_game_that_cannot_be_asked_leaves_a_named_history_unconfirmed_not_current() {
        let silent = Err(unnamed("no answer", false));
        assert_eq!(standing_of(&game("a"), 1, &silent), Standing::Unconfirmed("no answer".to_string()));
    }

    #[test]
    fn a_history_recorded_without_a_name_is_unverified_while_no_game_names_itself() {
        for answered in [true, false] {
            let recorded = Recorded::Unverifiable(unnamed("no plugin", answered));
            assert_eq!(
                standing_of(&recorded, 2, &Err(unnamed("still none", true))),
                Standing::Unverified("no plugin".to_string()),
                "answered: {answered}"
            );
        }
    }

    /// A process does not gain a plugin: the game that answered with no `instance` cannot be the
    /// one that names itself now.
    #[test]
    fn a_history_recorded_against_a_game_that_answered_without_a_name_is_stale_for_one_that_names_itself() {
        let recorded = Recorded::Unverifiable(unnamed("no plugin", true));
        assert_eq!(standing_of(&recorded, 2, &Ok("a".to_string())), Standing::Stale { entries: 2 });
    }

    /// A game that was not asked successfully says nothing about itself, so a game that names
    /// itself later may be the same one.
    #[test]
    fn a_history_recorded_while_the_game_could_not_be_asked_stays_unverified_for_one_that_names_itself() {
        let recorded = Recorded::Unverifiable(unnamed("no answer", false));
        assert_eq!(standing_of(&recorded, 2, &Ok("a".to_string())), Standing::Unverified("no answer".to_string()));
    }

    #[test]
    fn an_empty_history_has_nothing_to_be_stale_and_says_when_the_game_cannot_name_itself() {
        assert_eq!(standing_of(&game("a"), 0, &Ok("b".to_string())), Standing::Current);
        assert_eq!(standing_of(&Recorded::Nothing, 0, &Ok("b".to_string())), Standing::Current);
        assert_eq!(
            standing_of(&Recorded::Nothing, 0, &Err(unnamed("no plugin", true))),
            Standing::Unverified("no plugin".to_string())
        );
    }
}
