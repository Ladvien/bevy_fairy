use serde_json::{json, Value};
/// Anomaly detection tool for automatic game state monitoring
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::server::anomaly_detector::{
    Anomaly, AnomalyConfig, AnomalyDetectionSystem, DetectorCoverage,
};
use crate::server::brp_client::BrpClient;
use crate::server::brp_messages::{
    builtin_methods, entity_data_from_query_result, BrpPayload, BrpRequest,
};
use crate::server::error::Result;

/// What `settle` found about the game the detectors' samples came from.
enum Samples {
    /// The samples held are of another game process: they were dropped.
    Dropped,
    /// Neither read could name its game, so samples may be another game's. The text says why.
    Unverified(String),
}

/// Shared state for anomaly detection
pub struct AnomalyState {
    detection_system: AnomalyDetectionSystem,
    /// The game the detectors' samples were read from (`Err`: why it could not say); `None` before
    /// the first `detect`.
    last_game: Option<std::result::Result<String, String>>,
}

impl AnomalyState {
    /// Create new anomaly detection state
    #[must_use]
    pub fn new() -> Self {
        let config = AnomalyConfig::default();
        Self {
            detection_system: AnomalyDetectionSystem::new(config),
            last_game: None,
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub fn with_config(config: AnomalyConfig) -> Self {
        Self {
            detection_system: AnomalyDetectionSystem::new(config),
            last_game: None,
        }
    }

    /// Record the game the next samples come from. The detectors score an entity against its own
    /// recent values by entity id, and a restarted game gives the same ids to other entities, so
    /// the samples of another game process are dropped (the configuration stays).
    fn settle(&mut self, game: &std::result::Result<String, String>) -> Option<Samples> {
        let found = match (&self.last_game, game) {
            (None, _) => None,
            (Some(Ok(then)), Ok(now)) if then != now => {
                self.detection_system = AnomalyDetectionSystem::new(self.detection_system.config().clone());
                Some(Samples::Dropped)
            }
            (Some(Ok(_)), Ok(_)) => None,
            (Some(Err(why)), _) | (Some(_), Err(why)) => Some(Samples::Unverified(why.clone())),
        };
        self.last_game = Some(game.clone());
        found
    }
}

impl Default for AnomalyState {
    fn default() -> Self {
        Self::new()
    }
}

// Global anomaly state
static ANOMALY_STATE: std::sync::OnceLock<Arc<RwLock<AnomalyState>>> = std::sync::OnceLock::new();

fn get_anomaly_state() -> Arc<RwLock<AnomalyState>> {
    ANOMALY_STATE
        .get_or_init(|| Arc::new(RwLock::new(AnomalyState::new())))
        .clone()
}

/// Handle anomaly detection tool requests
///
/// # Errors
/// Returns error if BRP communication fails, configuration is invalid, or analysis fails
pub async fn handle(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    debug!("Anomaly tool called with arguments: {}", arguments);

    let action = arguments
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("detect");

    match action {
        "detect" => handle_detect(arguments, brp_client).await,
        "configure" => handle_configure(arguments).await,
        "status" => handle_status().await,
        _ => Ok(json!({
            "error": "Invalid action",
            "message": format!("Unknown action: {}. Available actions: detect, configure, status", action),
            "available_actions": ["detect", "configure", "status"]
        })),
    }
}

/// Detect anomalies in current game state
async fn handle_detect(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    info!("Starting anomaly detection");

    // Get current game state
    let client_connected = {
        let client = brp_client.read().await;
        client.is_connected()
    };

    if !client_connected {
        warn!("BRP client not connected");
        return Ok(json!({
            "error": "BRP client not connected",
            "message": "Cannot detect anomalies - not connected to Bevy game"
        }));
    }

    // Which game process this is: the detectors keep samples per entity id across calls.
    let game = brp_client.write().await.game_instance().await;

    // Query all entities
    let brp_request = BrpRequest {
        method: builtin_methods::BRP_QUERY_METHOD.to_string(),
        id: None,
        params: Some(crate::server::brp_messages::query_params(None, true)),
    };
    let brp_response = {
        let mut client = brp_client.write().await;
        match client.send_request(&brp_request).await {
            Ok(response) => response,
            Err(e) => {
                error!("BRP request failed: {}", e);
                return Ok(json!({
                    "error": "BRP request failed",
                    "message": e.to_string()
                }));
            }
        }
    };

    let entities = match brp_response.payload {
        BrpPayload::Result(value) => {
            match entity_data_from_query_result(&value) {
                Some(entities) => entities,
                None => {
                    return Ok(json!({
                        "error": "Unexpected response type",
                        "message": "Expected entity list from world.query"
                    }))
                }
            }
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

    // Run anomaly detection
    let state = get_anomaly_state();
    let mut state_guard = state.write().await;

    let samples = state_guard.settle(&game);
    let anomalies = match state_guard.detection_system.detect_anomalies(&entities) {
        Ok(anomalies) => anomalies,
        Err(e) => {
            error!("Anomaly detection failed: {}", e);
            return Ok(json!({
                "error": "Anomaly detection failed",
                "message": e.to_string()
            }));
        }
    };

    // Filter by severity if requested
    let min_severity = arguments
        .get("min_severity")
        .and_then(|s| s.as_f64())
        .unwrap_or(0.0) as f32;

    let filtered_anomalies: Vec<&Anomaly> = anomalies
        .iter()
        .filter(|a| a.severity >= min_severity)
        .collect();

    // Limit results if requested
    let limit = arguments
        .get("limit")
        .and_then(|l| l.as_u64())
        .unwrap_or(50) as usize;

    let limited_anomalies: Vec<&Anomaly> = filtered_anomalies.into_iter().take(limit).collect();

    info!(
        "Detected {} anomalies (showing {} after filtering)",
        anomalies.len(),
        limited_anomalies.len()
    );

    let coverage = state_guard.detection_system.coverage();
    let mut reply = json!({
        "anomalies": limited_anomalies,
        "summary": {
            "total_detected": anomalies.len(),
            "after_filtering": limited_anomalies.len(),
            "entities_read": entities.len(),
            "min_severity_filter": min_severity,
            "limit_applied": limit,
            "timestamp": chrono::Utc::now().to_rfc3339()
        },
        "detectors": coverage,
        "severity_breakdown": calculate_severity_breakdown(&anomalies),
        "type_breakdown": calculate_type_breakdown(&anomalies)
    });
    if let Some(note) = coverage_note(&coverage) {
        reply["note"] = json!(note);
    }
    match samples {
        Some(Samples::Dropped) => {
            reply["history_discarded"] = json!({
                "reason": "the samples the detectors held were read from another game (the game restarted since): \
                           a restarted game gives the same entity ids to other entities, so they were dropped and \
                           these readings start the windows again",
            });
        }
        Some(Samples::Unverified(why)) => {
            reply["game_instance_unverified"] = json!(format!(
                "the samples held may be another game's: {why}"
            ));
        }
        None => {}
    }
    Ok(reply)
}

/// Say so when a detector read nothing: an empty `anomalies` list from it is not a clean result.
/// `entities_read` counts what the query returned, not what any detector could use of it.
fn coverage_note(coverage: &[DetectorCoverage]) -> Option<String> {
    let unevaluated: Vec<&str> = coverage.iter().filter(|c| c.evaluated == 0).map(|c| c.detector).collect();
    if unevaluated.is_empty() {
        None
    } else if unevaluated.len() == coverage.len() {
        Some("No detector could judge anything in this game (see `detectors`), so an empty `anomalies` list says nothing about the game.".to_string())
    } else {
        Some(format!(
            "{} did not evaluate (see `detectors`), so the list says nothing about what they read.",
            unevaluated.join(", ")
        ))
    }
}

/// Read an optional numeric argument. Absent is `Ok(None)`; present but not a positive finite
/// number is an error naming the field, so a typo'd value is refused rather than ignored.
fn positive_arg(arguments: &Value, key: &str) -> std::result::Result<Option<f64>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_f64() {
            Some(n) if n.is_finite() && n > 0.0 => Ok(Some(n)),
            _ => Err(format!("{key} must be a positive number, got {v}")),
        },
    }
}

/// A detector keeps `window_size` samples and needs `min_samples` of them, and at least 2 (growth
/// and a standard deviation are changes between samples), before it can report anything. A
/// window below that is a detector that never reports while `status` says it is configured, so
/// `configure` refuses it instead of accepting it.
fn check_window_holds_min_samples(config: &AnomalyConfig) -> std::result::Result<(), String> {
    let needed = config.min_samples.max(2);
    if config.window_size < needed {
        return Err(format!(
            "window_size ({}) is smaller than the {needed} samples a detector needs before it can report (min_samples is {}, and at least 2 are needed to measure a change), so it would never report. Raise window_size, or lower min_samples in the same call.",
            config.window_size, config.min_samples
        ));
    }
    Ok(())
}

/// Configure anomaly detection system: fields not named keep their current value.
async fn handle_configure(arguments: Value) -> Result<Value> {
    info!("Configuring anomaly detection system");

    let state = get_anomaly_state();
    let mut state_guard = state.write().await;
    let mut config = state_guard.detection_system.config().clone();

    let parsed = (|| -> std::result::Result<(), String> {
        if let Some(n) = positive_arg(&arguments, "window_size")? {
            config.window_size = n as usize;
        }
        if let Some(n) = positive_arg(&arguments, "z_score_threshold")? {
            config.z_score_threshold = n as f32;
        }
        if let Some(n) = positive_arg(&arguments, "min_samples")? {
            config.min_samples = n as usize;
        }
        if let Some(n) = positive_arg(&arguments, "entity_growth_threshold")? {
            config.entity_growth_threshold = n as f32;
        }
        check_window_holds_min_samples(&config)
    })();
    if let Err(message) = parsed {
        return Ok(json!({
            "error": "Invalid configuration",
            "message": message,
            "example": {"action": "configure", "window_size": 50, "z_score_threshold": 2.5}
        }));
    }

    state_guard.detection_system.update_config(config.clone());

    info!("Anomaly detection configuration updated");

    Ok(json!({
        "message": "Configuration updated successfully",
        "config": config_json(&config)
    }))
}

fn config_json(config: &AnomalyConfig) -> Value {
    json!({
        "window_size": config.window_size,
        "z_score_threshold": config.z_score_threshold,
        "min_samples": config.min_samples,
        "entity_growth_threshold": config.entity_growth_threshold,
        "whitelist_count": config.whitelist.len()
    })
}

/// List the detectors and the anomaly types they can report
async fn handle_status() -> Result<Value> {
    let state = get_anomaly_state();
    let state_guard = state.read().await;
    let system = &state_guard.detection_system;
    let detectors: Vec<Value> = system
        .coverage()
        .iter()
        .map(|c| json!({"detector": c.detector, "reads": c.reads}))
        .collect();
    let supported: Vec<String> = system
        .supported_anomaly_types()
        .iter()
        .map(|t| format!("{t:?}"))
        .collect();
    Ok(json!({
        "config": config_json(system.config()),
        "detectors": detectors,
        "supported_anomaly_types": supported
    }))
}

/// Calculate severity breakdown for anomalies
fn calculate_severity_breakdown(anomalies: &[Anomaly]) -> Value {
    let mut high = 0;
    let mut medium = 0;
    let mut low = 0;

    for anomaly in anomalies {
        if anomaly.severity >= 0.7 {
            high += 1;
        } else if anomaly.severity >= 0.4 {
            medium += 1;
        } else {
            low += 1;
        }
    }

    json!({
        "high_severity": high,
        "medium_severity": medium,
        "low_severity": low
    })
}

/// Calculate type breakdown for anomalies
fn calculate_type_breakdown(anomalies: &[Anomaly]) -> Value {
    let mut breakdown = std::collections::HashMap::new();

    for anomaly in anomalies {
        let type_name = format!("{:?}", anomaly.anomaly_type);
        *breakdown.entry(type_name).or_insert(0) += 1;
    }

    json!(breakdown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_anomaly_configure() {
        let args = json!({
            "action": "configure",
            "window_size": 50,
            "z_score_threshold": 2.5
        });

        let result = handle_configure(args).await.unwrap();
        assert_eq!(result["config"]["window_size"], 50);
        assert_eq!(result["config"]["z_score_threshold"], 2.5);
    }

    /// Two configure calls, each naming one field: the second must not reset the first.
    /// (Global state is shared with other tests, which only touch other fields.)
    #[tokio::test]
    async fn configure_keeps_fields_it_was_not_given() {
        handle_configure(json!({"min_samples": 37})).await.unwrap();
        let second = handle_configure(json!({"entity_growth_threshold": 42.0}))
            .await
            .unwrap();
        assert_eq!(second["config"]["entity_growth_threshold"], 42.0, "{second}");
        assert_eq!(second["config"]["min_samples"], 37, "reset to default: {second}");
        let status = handle_status().await.unwrap();
        assert_eq!(status["config"]["min_samples"], 37, "{status}");
    }

    #[tokio::test]
    async fn configure_refuses_a_value_it_cannot_apply() {
        for bad in [json!("50"), json!(-1), json!(0), json!(true)] {
            let r = handle_configure(json!({"window_size": bad})).await.unwrap();
            assert_eq!(r["error"], "Invalid configuration", "{bad}: {r}");
            assert!(r["message"].as_str().unwrap().contains("window_size"), "{r}");
        }
    }

    /// B-088: `status` listed six anomaly types, three of which (`PotentialMemoryLeak`,
    /// `PerformanceSpike`, `RapidValueChange`) no detector has ever emitted. What `status` says the
    /// tool can report is what the detectors can: drive each one and compare.
    #[tokio::test]
    async fn status_lists_the_anomaly_types_the_detectors_emit_and_no_others() {
        use crate::server::brp_messages::EntityData;
        use std::collections::BTreeSet;

        let mut system = AnomalyDetectionSystem::new(AnomalyConfig {
            min_samples: 3,
            window_size: 20,
            z_score_threshold: 1.0,
            entity_growth_threshold: 1.0,
            ..Default::default()
        });
        let mut emitted = BTreeSet::new();
        for round in 0..8u64 {
            // Entities grow every round; one moves at a constant speed until the last round, when
            // it spikes; one has health 0 and is marked alive.
            let speed = if round == 7 { 1000.0 } else { 10.0 };
            let mut entities: Vec<EntityData> = (0..round * 5)
                .map(|id| EntityData { id: 100 + id, components: Default::default() })
                .collect();
            entities.push(EntityData {
                id: 1,
                components: [("Velocity".to_string(), json!({"linear": {"x": speed, "y": 0.0, "z": 0.0}}))]
                    .into_iter()
                    .collect(),
            });
            entities.push(EntityData {
                id: 2,
                components: [("Health".to_string(), json!({"current": 0.0})), ("Alive".to_string(), json!(true))]
                    .into_iter()
                    .collect(),
            });
            for anomaly in system.detect_anomalies(&entities).unwrap() {
                emitted.insert(format!("{:?}", anomaly.anomaly_type));
            }
        }
        assert_eq!(emitted.len(), 3, "the driven data should trigger every detector: {emitted:?}");

        let status = handle_status().await.unwrap();
        let listed: BTreeSet<String> = status["supported_anomaly_types"]
            .as_array()
            .expect("status lists the types")
            .iter()
            .map(|t| t.as_str().expect("a type is a string").to_string())
            .collect();
        assert_eq!(listed, emitted, "status must list what the detectors can report");
    }

    #[tokio::test]
    async fn test_anomaly_status() {
        let result = handle_status().await.unwrap();
        assert!(result["detectors"].is_array());
        assert!(result["supported_anomaly_types"].is_array());
    }

    /// The window a detector keeps has to hold the samples it needs: `window_size` 5 under the
    /// default `min_samples` of 10 is a detector that can never report, and `configure` used to
    /// answer "Configuration updated successfully" for it.
    #[test]
    fn a_window_smaller_than_the_samples_a_detector_needs_is_refused() {
        let config = |window_size, min_samples| AnomalyConfig { window_size, min_samples, ..Default::default() };
        assert!(check_window_holds_min_samples(&config(10, 10)).is_ok());
        assert!(check_window_holds_min_samples(&config(2, 1)).is_ok());
        for (window, min) in [(5, 10), (9, 10), (1, 1), (0, 1)] {
            let message = check_window_holds_min_samples(&config(window, min)).expect_err("a window that cannot hold the samples");
            assert!(message.contains("window_size") && message.contains("min_samples"), "{message}");
        }
    }

    /// Through the tool: the refusal names both fields, and does not depend on what an earlier
    /// call configured because the same call names both.
    #[tokio::test]
    async fn configure_refuses_a_window_below_min_samples() {
        let r = handle_configure(json!({"window_size": 5, "min_samples": 10})).await.unwrap();
        assert_eq!(r["error"], "Invalid configuration", "{r}");
        let message = r["message"].as_str().unwrap();
        assert!(message.contains("window_size (5)") && message.contains("min_samples is 10"), "{r}");
    }

    /// `start_monitoring` used to set a flag, answer "Continuous monitoring started" and run
    /// nothing: no task ever sampled the game. The actions are gone, so asking for one is an error
    /// naming the valid actions, and `status` has no `is_monitoring` to claim.
    #[tokio::test]
    async fn monitoring_actions_are_refused_not_pretended() {
        for action in ["start_monitoring", "stop_monitoring"] {
            let result = handle(json!({ "action": action }), create_test_brp_client())
                .await
                .unwrap();
            assert_eq!(result["error"], "Invalid action", "{action}: {result}");
            assert_eq!(
                result["available_actions"],
                json!(["detect", "configure", "status"]),
                "{action}"
            );
            assert!(result.get("is_monitoring").is_none(), "{action}: {result}");
        }
        let status = handle_status().await.unwrap();
        assert!(status.get("is_monitoring").is_none(), "{status}");
    }

    #[tokio::test]
    async fn test_anomaly_detect_no_connection() {
        let config = Config::default();
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({"action": "detect"});
        let result = handle_detect(args, brp_client).await.unwrap();

        assert!(result.get("error").is_some());
        assert_eq!(result["error"], "BRP client not connected");
    }

    fn create_test_brp_client() -> Arc<RwLock<BrpClient>> {
        let config = Config::default();
        Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)))
    }
}
