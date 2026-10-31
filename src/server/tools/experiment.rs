use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::experiment_system::{Action, ActionExecutor};

/// Global experiment state
pub struct ExperimentState {
    executor: ActionExecutor,
}

impl ExperimentState {
    /// Create new experiment state
    pub fn new() -> Self {
        Self {
            executor: ActionExecutor::new(),
        }
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
        "history" => return handle_history().await,
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

    Ok(json!({
        "results": results,
        "summary": {
            "total_actions": actions.len(),
            "successful": success_count,
            "failed": failure_count,
            "success_rate": if actions.is_empty() { 0.0 } else { (success_count as f64) / (actions.len() as f64) }
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle undo action
async fn handle_undo(brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    info!("Performing undo");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;
    let mut client = brp_client.write().await;

    let result = state_guard.executor.undo(&mut client).await?;

    Ok(json!({
        "result": result,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle redo action
async fn handle_redo(brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    info!("Performing redo");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;
    let mut client = brp_client.write().await;

    let result = state_guard.executor.redo(&mut client).await?;

    Ok(json!({
        "result": result,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle history request
async fn handle_history() -> Result<Value> {
    let state = get_experiment_state();
    let state_guard = state.read().await;

    let history = state_guard.executor.get_history();

    Ok(json!({
        "history": history,
        "count": history.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle clear history
async fn handle_clear_history() -> Result<Value> {
    info!("Clearing action history");

    let state = get_experiment_state();
    let mut state_guard = state.write().await;

    state_guard.executor.clear_history();

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
}
