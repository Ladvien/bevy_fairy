use serde_json::{json, Value};
/// Anomaly detection tool for automatic game state monitoring
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::server::anomaly_detector::{Anomaly, AnomalyConfig, AnomalyDetectionSystem};
use crate::server::brp_client::BrpClient;
use crate::server::brp_messages::{
    builtin_methods, entity_data_from_query_result, BrpPayload, BrpRequest,
};
use crate::server::error::Result;

/// Shared state for anomaly detection
pub struct AnomalyState {
    detection_system: AnomalyDetectionSystem,
}

impl AnomalyState {
    /// Create new anomaly detection state
    #[must_use]
    pub fn new() -> Self {
        let config = AnomalyConfig::default();
        Self {
            detection_system: AnomalyDetectionSystem::new(config),
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub fn with_config(config: AnomalyConfig) -> Self {
        Self {
            detection_system: AnomalyDetectionSystem::new(config),
        }
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

    Ok(json!({
        "anomalies": limited_anomalies,
        "summary": {
            "total_detected": anomalies.len(),
            "after_filtering": limited_anomalies.len(),
            "entities_analyzed": entities.len(),
            "min_severity_filter": min_severity,
            "limit_applied": limit,
            "timestamp": chrono::Utc::now().to_rfc3339()
        },
        "severity_breakdown": calculate_severity_breakdown(&anomalies),
        "type_breakdown": calculate_type_breakdown(&anomalies)
    }))
}

/// Configure anomaly detection system
async fn handle_configure(arguments: Value) -> Result<Value> {
    info!("Configuring anomaly detection system");

    let mut config = AnomalyConfig::default();

    // Update configuration from arguments
    if let Some(window_size) = arguments.get("window_size").and_then(|w| w.as_u64()) {
        config.window_size = window_size as usize;
    }

    if let Some(z_threshold) = arguments.get("z_score_threshold").and_then(|z| z.as_f64()) {
        config.z_score_threshold = z_threshold as f32;
    }

    if let Some(iqr_multiplier) = arguments.get("iqr_multiplier").and_then(|i| i.as_f64()) {
        config.iqr_multiplier = iqr_multiplier as f32;
    }

    if let Some(min_samples) = arguments.get("min_samples").and_then(|m| m.as_u64()) {
        config.min_samples = min_samples as usize;
    }

    if let Some(perf_threshold) = arguments
        .get("performance_threshold")
        .and_then(|p| p.as_f64())
    {
        config.performance_threshold = perf_threshold as f32;
    }

    if let Some(entity_growth) = arguments
        .get("entity_growth_threshold")
        .and_then(|e| e.as_f64())
    {
        config.entity_growth_threshold = entity_growth as f32;
    }

    // Apply configuration
    let state = get_anomaly_state();
    let mut state_guard = state.write().await;
    state_guard.detection_system.update_config(config.clone());

    info!("Anomaly detection configuration updated");

    Ok(json!({
        "message": "Configuration updated successfully",
        "config": {
            "window_size": config.window_size,
            "z_score_threshold": config.z_score_threshold,
            "iqr_multiplier": config.iqr_multiplier,
            "min_samples": config.min_samples,
            "performance_threshold": config.performance_threshold,
            "entity_growth_threshold": config.entity_growth_threshold,
            "whitelist_count": config.whitelist.len()
        }
    }))
}

/// List the detectors and the anomaly types they can report
async fn handle_status() -> Result<Value> {
    Ok(json!({
        "detectors": [
            "PhysicsDetector",
            "PerformanceDetector",
            "ConsistencyDetector"
        ],
        "supported_anomaly_types": [
            "PhysicsViolation",
            "PotentialMemoryLeak",
            "StateInconsistency",
            "PerformanceSpike",
            "EntityCountSpike",
            "RapidValueChange"
        ]
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

    #[tokio::test]
    async fn test_anomaly_status() {
        let result = handle_status().await.unwrap();
        assert!(result["detectors"].is_array());
        assert!(result["supported_anomaly_types"].is_array());
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
