use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::experiment_system::Action;
use crate::server::hypothesis_system::{
    Assertion, Hypothesis, PerformanceMetric, TestRunner, VariationStrategy,
};

/// Handle hypothesis tool requests
pub async fn handle(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    debug!("Hypothesis tool called with arguments: {}", arguments);

    // Parse action parameter
    let action_str = arguments
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("test");

    match action_str {
        "test" => handle_test(arguments, brp_client).await,
        "quick_test" => handle_quick_test(arguments, brp_client).await,
        "stress_test" => handle_stress_test(arguments, brp_client).await,
        "validate" => handle_validate(arguments).await,
        _ => Ok(json!({
            "error": "Unknown action",
            "message": format!("Unknown action: {}", action_str),
            "available_actions": ["test", "quick_test", "stress_test", "validate"]
        })),
    }
}

/// Handle full hypothesis test
async fn handle_test(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    // Check connection
    let is_connected = {
        let client = brp_client.read().await;
        client.is_connected()
    };

    if !is_connected {
        warn!("BRP client not connected");
        return Ok(json!({
            "error": "BRP client not connected",
            "message": "Cannot run hypothesis test - not connected to Bevy game",
            "brp_connected": false
        }));
    }

    // Parse hypothesis parameters
    let description = arguments
        .get("description")
        .and_then(|d| d.as_str())
        .ok_or_else(|| Error::Validation("Missing 'description' parameter".to_string()))?;

    let success_condition = parse_success_condition(&arguments)?;

    // Parse actions if provided
    let actions = if let Some(actions_json) = arguments.get("actions") {
        parse_actions(actions_json)?
    } else {
        Vec::new()
    };

    // Parse variation strategy
    let variation = parse_variation_strategy(&arguments)?;

    // Build hypothesis
    let hypothesis = Hypothesis::builder(description.to_string())
        .with_condition(success_condition)
        .with_actions(actions)
        .with_variation(variation)
        .build()?;

    // Validated before anything runs: the actions change the live game.
    let threshold = arguments
        .get("threshold")
        .and_then(|t| t.as_f64())
        .unwrap_or(0.95);
    if !(0.0..=1.0).contains(&threshold) {
        return Err(Error::Validation(format!(
            "'threshold' is the share of iterations in which the condition must hold, from 0 to 1; got {threshold}. Example: \"threshold\": 0.95"
        )));
    }

    // Configure test runner
    let iterations = arguments
        .get("iterations")
        .and_then(|i| i.as_u64())
        .unwrap_or(100) as usize;

    let timeout_secs = arguments
        .get("timeout")
        .and_then(|t| t.as_u64())
        .unwrap_or(60);

    let seed = arguments.get("seed").and_then(|s| s.as_u64());

    let mut runner =
        TestRunner::with_config(iterations, std::time::Duration::from_secs(timeout_secs));

    if let Some(s) = seed {
        runner = runner.with_seed(s);
    }

    info!("Running hypothesis test: {}", description);

    // Run the test
    let mut client = brp_client.write().await;
    let result = runner.run(&hypothesis, &mut client).await?;

    let verdict = result.verdict(threshold);

    Ok(json!({
        "hypothesis": result.hypothesis,
        "verdict": verdict.as_str(),
        "success_rate": result.success_rate,
        "confidence_interval": result.confidence_interval.map(|(lower, upper)| json!({
            "lower": lower,
            "upper": upper
        })),
        "confidence_level": result.confidence_level().map(|level| format!("{level:.1}%")),
        "statistics": {
            "iterations_run": result.iterations_run,
            "held": result.held,
            "violated": result.violated,
            "not_evaluated": result.not_evaluated,
            "avg_execution_time_ms": result.avg_execution_time_ms
        },
        "edge_cases": result.edge_cases.len(),
        "examples": result.examples.iter().take(3).map(|f| {
            json!({
                "iteration": f.iteration,
                "kind": f.kind,
                "error": f.error
            })
        }).collect::<Vec<_>>(),
        "reproducible_seed": result.seed,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle quick hypothesis test (fewer iterations)
async fn handle_quick_test(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    // Override iterations for quick test
    let mut args = arguments.clone();
    if let Some(obj) = args.as_object_mut() {
        obj.insert("iterations".to_string(), json!(10));
        obj.insert("timeout".to_string(), json!(10));
    }

    handle_test(args, brp_client).await
}

/// Handle stress test (many iterations with fuzzing)
async fn handle_stress_test(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    // Stress defaults, each applied only where the caller did not choose one.
    let mut args = arguments.clone();
    if let Some(obj) = args.as_object_mut() {
        for (key, value) in [
            ("iterations", json!(1000)),
            ("variation_type", json!("fuzz")),
            ("max_mutations", json!(5)),
            ("mutation_probability", json!(0.8)),
        ] {
            if obj.get(key).is_none_or(Value::is_null) {
                obj.insert(key.to_string(), value);
            }
        }
    }

    handle_test(args, brp_client).await
}

/// Handle hypothesis validation (syntax check only)
async fn handle_validate(arguments: Value) -> Result<Value> {
    let description = arguments
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("Test hypothesis");

    // Try to parse success condition
    match parse_success_condition(&arguments) {
        Ok(condition) => {
            // Try to build hypothesis
            let builder = Hypothesis::builder(description.to_string()).with_condition(condition);

            match builder.build() {
                Ok(_) => Ok(json!({
                    "valid": true,
                    "message": "Hypothesis is valid",
                    "timestamp": chrono::Utc::now().to_rfc3339()
                })),
                Err(e) => Ok(json!({
                    "valid": false,
                    "error": e.to_string(),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                })),
            }
        }
        Err(e) => Ok(json!({
            "valid": false,
            "error": format!("Invalid success condition: {}", e),
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
    }
}

/// Parse success condition from arguments
fn parse_success_condition(arguments: &Value) -> Result<Assertion> {
    if let Some(condition_str) = arguments.get("success_condition").and_then(|c| c.as_str()) {
        // Try to parse as string format
        Assertion::parse(condition_str)
    } else if let Some(condition_obj) = arguments
        .get("success_condition")
        .and_then(|c| c.as_object())
    {
        // Parse as JSON object
        parse_assertion_from_json(condition_obj)
    } else {
        Err(Error::Validation(
            "Missing or invalid 'success_condition' parameter".to_string(),
        ))
    }
}

/// Parse assertion from JSON object
fn parse_assertion_from_json(obj: &serde_json::Map<String, Value>) -> Result<Assertion> {
    let assertion_type = obj
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or_else(|| Error::Validation("Assertion must have 'type' field".to_string()))?;

    match assertion_type {
        "entity_exists" => {
            let component_types = obj
                .get("component_types")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let count = obj
                .get("count")
                .and_then(|c| c.as_u64())
                .map(|c| c as usize);

            Ok(Assertion::EntityExists {
                component_types,
                count,
            })
        }
        "component_equals" => {
            let entity_id = obj.get("entity_id").and_then(|e| e.as_u64());
            let component_type = obj
                .get("component_type")
                .and_then(|c| c.as_str())
                .ok_or_else(|| {
                    Error::Validation("component_equals requires 'component_type'".to_string())
                })?;

            let expected_value = obj
                .get("expected_value")
                .ok_or_else(|| {
                    Error::Validation("component_equals requires 'expected_value'".to_string())
                })?
                .clone();

            Ok(Assertion::ComponentEquals {
                entity_id,
                component_type: component_type.to_string(),
                expected_value,
            })
        }
        "performance_within" => {
            let metric_str = obj.get("metric").and_then(|m| m.as_str()).ok_or_else(|| {
                Error::Validation("performance_within requires 'metric'".to_string())
            })?;

            let metric = match metric_str {
                "frame_time" => PerformanceMetric::FrameTime,
                "entity_count" => PerformanceMetric::EntityCount,
                "component_count" => PerformanceMetric::ComponentCount,
                "memory_usage" => PerformanceMetric::MemoryUsage,
                "cpu_usage" => PerformanceMetric::CpuUsage,
                _ => return Err(Error::Validation(format!("Unknown metric: {metric_str}"))),
            };

            let max_value = obj
                .get("max_value")
                .and_then(|v| v.as_f64())
                .ok_or_else(|| {
                    Error::Validation("performance_within requires 'max_value'".to_string())
                })?;

            Ok(Assertion::PerformanceWithin { metric, max_value })
        }
        "action_succeeds" => {
            let action_json = obj.get("action").ok_or_else(|| {
                Error::Validation("action_succeeds requires 'action'".to_string())
            })?;

            let action = serde_json::from_value(action_json.clone())
                .map_err(|e| Error::Validation(format!("Invalid action: {e}")))?;

            Ok(Assertion::ActionSucceeds { action })
        }
        "all" => {
            let assertions_json = obj
                .get("assertions")
                .and_then(|a| a.as_array())
                .ok_or_else(|| {
                    Error::Validation("'all' requires 'assertions' array".to_string())
                })?;

            let mut assertions = Vec::new();
            for assertion_json in assertions_json {
                if let Some(obj) = assertion_json.as_object() {
                    assertions.push(parse_assertion_from_json(obj)?);
                }
            }

            Ok(Assertion::All { assertions })
        }
        "any" => {
            let assertions_json = obj
                .get("assertions")
                .and_then(|a| a.as_array())
                .ok_or_else(|| {
                    Error::Validation("'any' requires 'assertions' array".to_string())
                })?;

            let mut assertions = Vec::new();
            for assertion_json in assertions_json {
                if let Some(obj) = assertion_json.as_object() {
                    assertions.push(parse_assertion_from_json(obj)?);
                }
            }

            Ok(Assertion::Any { assertions })
        }
        "not" => {
            let assertion_json = obj
                .get("assertion")
                .and_then(|a| a.as_object())
                .ok_or_else(|| {
                    Error::Validation("'not' requires 'assertion' object".to_string())
                })?;

            let assertion = parse_assertion_from_json(assertion_json)?;

            Ok(Assertion::Not {
                assertion: Box::new(assertion),
            })
        }
        _ => Err(Error::Validation(format!(
            "Unknown assertion type: {assertion_type}"
        ))),
    }
}

/// Parse actions from JSON
fn parse_actions(actions_json: &Value) -> Result<Vec<Action>> {
    if actions_json.is_array() {
        serde_json::from_value(actions_json.clone())
            .map_err(|e| Error::Validation(format!("Failed to parse actions: {e}")))
    } else {
        // Single action
        Ok(vec![serde_json::from_value(actions_json.clone()).map_err(
            |e| Error::Validation(format!("Failed to parse action: {e}")),
        )?])
    }
}

/// Parse variation strategy from arguments
fn parse_variation_strategy(arguments: &Value) -> Result<VariationStrategy> {
    let variation_type = arguments
        .get("variation_type")
        .and_then(|v| v.as_str())
        .unwrap_or("none");

    match variation_type {
        "none" => Ok(VariationStrategy::None),
        "random" => {
            let min = arguments
                .get("numeric_min")
                .and_then(|n| n.as_f64())
                .unwrap_or(0.0);
            let max = arguments
                .get("numeric_max")
                .and_then(|n| n.as_f64())
                .unwrap_or(100.0);

            let string_pool = arguments
                .get("string_pool")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let bool_probability = arguments
                .get("bool_probability")
                .and_then(|b| b.as_f64())
                .unwrap_or(0.5);

            Ok(VariationStrategy::Random {
                numeric_range: (min, max),
                string_pool,
                bool_probability,
            })
        }
        "grid" => {
            let numeric_values = arguments
                .get("numeric_values")
                .and_then(|n| n.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
                .unwrap_or_else(|| vec![0.0, 50.0, 100.0]);

            let string_values = arguments
                .get("string_values")
                .and_then(|s| s.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_else(|| vec!["default".to_string()]);

            let bool_values = arguments
                .get("bool_values")
                .and_then(|b| b.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_bool()).collect())
                .unwrap_or_else(|| vec![true, false]);

            Ok(VariationStrategy::Grid {
                numeric_values,
                string_values,
                bool_values,
            })
        }
        "boundary" => Ok(VariationStrategy::Boundary {
            include_zero: arguments
                .get("include_zero")
                .and_then(|z| z.as_bool())
                .unwrap_or(true),
            include_negative: arguments
                .get("include_negative")
                .and_then(|n| n.as_bool())
                .unwrap_or(true),
            include_max: arguments
                .get("include_max")
                .and_then(|m| m.as_bool())
                .unwrap_or(true),
            include_empty: arguments
                .get("include_empty")
                .and_then(|e| e.as_bool())
                .unwrap_or(true),
        }),
        "fuzz" => Ok(VariationStrategy::Fuzz {
            max_mutations: arguments
                .get("max_mutations")
                .and_then(|m| m.as_u64())
                .unwrap_or(3) as usize,
            mutation_probability: arguments
                .get("mutation_probability")
                .and_then(|p| p.as_f64())
                .unwrap_or(0.5),
        }),
        _ => Err(Error::Validation(format!(
            "Unknown variation type: {variation_type}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_handle_without_connection() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "test",
            "description": "Test hypothesis",
            "success_condition": "entity_exists:Transform"
        });

        let result = handle(args, brp_client).await.unwrap();
        assert_eq!(result.get("error").unwrap(), "BRP client not connected");
    }

    #[tokio::test]
    async fn test_handle_validate() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let args = json!({
            "action": "validate",
            "description": "Test hypothesis",
            "success_condition": "entity_exists:Transform,Health"
        });

        let result = handle(args, brp_client).await.unwrap();
        assert_eq!(result.get("valid").unwrap(), true);
    }

    #[test]
    fn test_parse_success_condition_string() {
        let args = json!({
            "success_condition": "entity_exists:Transform,Health"
        });

        let condition = parse_success_condition(&args).unwrap();
        match condition {
            Assertion::EntityExists {
                component_types, ..
            } => {
                assert_eq!(component_types.len(), 2);
                assert!(component_types.contains(&"Transform".to_string()));
            }
            _ => panic!("Wrong assertion type"),
        }
    }

    #[test]
    fn test_parse_success_condition_json() {
        let args = json!({
            "success_condition": {
                "type": "component_equals",
                "entity_id": 123,
                "component_type": "Health",
                "expected_value": 100
            }
        });

        let condition = parse_success_condition(&args).unwrap();
        match condition {
            Assertion::ComponentEquals {
                entity_id,
                component_type,
                expected_value,
            } => {
                assert_eq!(entity_id, Some(123));
                assert_eq!(component_type, "Health");
                assert_eq!(expected_value, json!(100));
            }
            _ => panic!("Wrong assertion type"),
        }
    }

    #[test]
    fn test_parse_variation_strategy() {
        // Test random variation
        let args = json!({
            "variation_type": "random",
            "numeric_min": 10.0,
            "numeric_max": 90.0,
            "bool_probability": 0.7
        });

        let variation = parse_variation_strategy(&args).unwrap();
        match variation {
            VariationStrategy::Random {
                numeric_range,
                bool_probability,
                ..
            } => {
                assert_eq!(numeric_range, (10.0, 90.0));
                assert_eq!(bool_probability, 0.7);
            }
            _ => panic!("Wrong variation type"),
        }

        // Test fuzz variation
        let args = json!({
            "variation_type": "fuzz",
            "max_mutations": 10,
            "mutation_probability": 0.9
        });

        let variation = parse_variation_strategy(&args).unwrap();
        match variation {
            VariationStrategy::Fuzz {
                max_mutations,
                mutation_probability,
            } => {
                assert_eq!(max_mutations, 10);
                assert_eq!(mutation_probability, 0.9);
            }
            _ => panic!("Wrong variation type"),
        }
    }
}
