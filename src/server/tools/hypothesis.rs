use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::experiment_system::Action;
use crate::server::hypothesis_system::{
    registered_components, require_registered, Assertion, Hypothesis, TestRunner, VariationStrategy,
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
    // 0 is refused too: "held in at least 0% of iterations" confirms a claim that failed every time.
    if !(threshold > 0.0 && threshold <= 1.0) {
        return Err(Error::Validation(format!(
            "'threshold' is the share of iterations in which the condition must hold, above 0 and at most 1; got {threshold}. Example: \"threshold\": 0.95"
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

    // Before any action runs: a component name the game never registered would be judged false.
    let names = hypothesis.success_condition.component_names();
    if !names.is_empty() {
        let registered = registered_components(&mut client).await.map_err(|e| *e)?;
        require_registered(&names, &registered)?;
    }

    let result = runner.run(&hypothesis, &mut client).await?;

    let verdict = result.verdict(threshold);

    Ok(json!({
        "hypothesis": result.hypothesis,
        "verdict": verdict.as_str(),
        "success_rate": result.success_rate,
        "confidence_interval": result.confidence_interval.map(|(lower, upper)| json!({
            "lower": lower,
            "upper": upper,
            "level": 0.95,
            "method": "wilson"
        })),
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
    const SHAPES: &str = "a string (\"entity_exists:game::Crate,game::Health\" or \"component_equals:game::Score=100\") or an object whose \"type\" is one of entity_exists, component_equals, performance_within (entity_count or component_count), action_succeeds, all, any, not. Example: {\"type\": \"entity_exists\", \"component_types\": [\"game::Crate\"], \"count\": 6}";
    match arguments.get("success_condition") {
        Some(Value::String(condition)) => Assertion::parse(condition),
        Some(condition @ Value::Object(_)) => serde_json::from_value(condition.clone()).map_err(|e| {
            Error::Validation(format!("Invalid success_condition {condition}: {e}. A condition is {SHAPES}"))
        }),
        _ => Err(Error::Validation(format!(
            "Missing or invalid 'success_condition'. A condition is {SHAPES}"
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

            let bool_probability = probability(arguments, "bool_probability", 0.5)?;

            // `random_range(min..max)` panics on an empty range, and on NaN or an infinity.
            if !(min.is_finite() && max.is_finite() && min < max) {
                return Err(Error::Validation(format!(
                    "'numeric_max' must be a finite number above 'numeric_min'; got numeric_min {min} and numeric_max {max}. Example: \"numeric_min\": 0, \"numeric_max\": 100"
                )));
            }

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
        "fuzz" => {
            let max_mutations = arguments
                .get("max_mutations")
                .and_then(|m| m.as_u64())
                .unwrap_or(3);
            // A run applies between 1 and `max_mutations` mutations; 0 leaves no choice to make.
            if max_mutations == 0 {
                return Err(Error::Validation(
                    "'max_mutations' must be at least 1. Example: \"max_mutations\": 3".to_string(),
                ));
            }
            Ok(VariationStrategy::Fuzz {
                max_mutations: max_mutations as usize,
                mutation_probability: probability(arguments, "mutation_probability", 0.5)?,
            })
        }
        _ => Err(Error::Validation(format!(
            "Unknown variation type: {variation_type}"
        ))),
    }
}

/// A probability argument: `default` when absent, an error naming it when outside 0 to 1.
fn probability(arguments: &Value, name: &str, default: f64) -> Result<f64> {
    let value = arguments.get(name).and_then(|p| p.as_f64()).unwrap_or(default);
    if (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Validation(format!(
            "'{name}' is a probability, from 0 to 1; got {value}. Example: \"{name}\": 0.5"
        )))
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
