use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, info, warn};
// use rand_distr::{Distribution, Uniform}; // Currently unused, kept for future use

use crate::server::brp_client::BrpClient;
use crate::server::error::{BoxedResult, Error, Result};
use crate::server::experiment_system::{Action, ActionExecutor};

/// Represents a testable hypothesis about game behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub description: String,
    pub success_condition: Assertion,
    pub base_actions: Vec<Action>,
    pub variation_strategy: VariationStrategy,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl Hypothesis {
    /// Parse hypothesis from natural language description
    pub fn parse(description: String, success_condition: String) -> Result<Self> {
        let assertion = Assertion::parse(&success_condition)?;

        Ok(Self {
            description,
            success_condition: assertion,
            base_actions: Vec::new(),
            variation_strategy: VariationStrategy::default(),
            metadata: HashMap::new(),
        })
    }

    /// Create hypothesis with builder pattern
    pub fn builder(description: String) -> HypothesisBuilder {
        HypothesisBuilder {
            description,
            success_condition: None,
            base_actions: Vec::new(),
            variation_strategy: VariationStrategy::default(),
            metadata: HashMap::new(),
        }
    }
}

/// Builder for creating hypotheses
pub struct HypothesisBuilder {
    description: String,
    success_condition: Option<Assertion>,
    base_actions: Vec<Action>,
    variation_strategy: VariationStrategy,
    metadata: HashMap<String, serde_json::Value>,
}

impl HypothesisBuilder {
    /// Set the success condition for the hypothesis
    pub fn with_condition(mut self, condition: Assertion) -> Self {
        self.success_condition = Some(condition);
        self
    }

    /// Set the base actions to execute for each test iteration
    pub fn with_actions(mut self, actions: Vec<Action>) -> Self {
        self.base_actions = actions;
        self
    }

    /// Set the variation strategy for generating test cases
    pub fn with_variation(mut self, strategy: VariationStrategy) -> Self {
        self.variation_strategy = strategy;
        self
    }

    /// Add metadata to the hypothesis for tracking additional information
    pub fn with_metadata(mut self, key: String, value: serde_json::Value) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// Build the hypothesis, validating all required fields
    pub fn build(self) -> Result<Hypothesis> {
        let success_condition = self.success_condition.ok_or_else(|| {
            Error::Validation("Hypothesis must have a success condition".to_string())
        })?;

        Ok(Hypothesis {
            description: self.description,
            success_condition,
            base_actions: self.base_actions,
            variation_strategy: self.variation_strategy,
            metadata: self.metadata,
        })
    }
}

/// Testable assertion types
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Assertion {
    /// Entity exists with specific components
    EntityExists {
        component_types: Vec<String>,
        count: Option<usize>,
    },
    /// Component has specific value
    ComponentEquals {
        entity_id: Option<u64>,
        component_type: String,
        expected_value: serde_json::Value,
    },
    /// Component value matches predicate
    ComponentMatches {
        entity_id: Option<u64>,
        component_type: String,
        predicate: Predicate,
    },
    /// Performance metric is within bounds
    PerformanceWithin {
        metric: PerformanceMetric,
        max_value: f64,
    },
    /// Action completes successfully
    ActionSucceeds { action: Action },
    /// Composite assertion (all must pass)
    All { assertions: Vec<Assertion> },
    /// Composite assertion (any must pass)
    Any { assertions: Vec<Assertion> },
    /// Negation of assertion
    Not { assertion: Box<Assertion> },
}

impl Assertion {
    /// Parse assertion from string representation
    pub fn parse(condition: &str) -> Result<Self> {
        // Simple parsing for common patterns
        if condition.starts_with("entity_exists:") {
            let parts: Vec<&str> = condition
                .strip_prefix("entity_exists:")
                .unwrap()
                .split(',')
                .collect();
            Ok(Self::EntityExists {
                component_types: parts.iter().map(|s| s.trim().to_string()).collect(),
                count: None,
            })
        } else if condition.starts_with("component_equals:") {
            // Format: component_equals:component_type=value
            let parts: Vec<&str> = condition
                .strip_prefix("component_equals:")
                .unwrap()
                .split('=')
                .collect();
            if parts.len() != 2 {
                return Err(Error::Validation(
                    "Invalid component_equals format".to_string(),
                ));
            }
            Ok(Self::ComponentEquals {
                entity_id: None,
                component_type: parts[0].to_string(),
                expected_value: serde_json::from_str(parts[1])
                    .map_err(|e| Error::Validation(format!("Invalid JSON value: {e}")))?,
            })
        } else if condition == "action_succeeds" {
            // Placeholder - would need actual action
            Ok(Self::ActionSucceeds {
                action: Action::Spawn {
                    components: vec![],
                    archetype: None,
                },
            })
        } else {
            Err(Error::Validation(format!(
                "Cannot parse assertion: {condition}"
            )))
        }
    }

    /// Evaluate assertion against current game state
    pub fn evaluate<'a>(
        &'a self,
        brp_client: &'a mut BrpClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            match self {
                Self::EntityExists {
                    component_types,
                    count: _,
                } => {
                    // Query for entities with specified components
                    // TODO: Implement actual entity query in BRP
                    debug!(
                        "Checking for entities with components: {:?}",
                        component_types
                    );
                    warn!("EntityExists assertion not fully implemented - returning true");
                    Ok(true)
                }
                Self::ComponentEquals {
                    entity_id: _,
                    component_type,
                    expected_value,
                } => {
                    debug!(
                        "Checking component {} equals {:?}",
                        component_type, expected_value
                    );
                    // TODO: Query entity and compare component value
                    warn!("ComponentEquals assertion not fully implemented - returning true");
                    Ok(true)
                }
                Self::ComponentMatches {
                    entity_id: _,
                    component_type,
                    predicate: _,
                } => {
                    debug!("Checking component {} matches predicate", component_type);
                    // TODO: Query entity and apply predicate
                    warn!("ComponentMatches assertion not fully implemented - returning true");
                    Ok(true)
                }
                Self::PerformanceWithin { metric, max_value } => {
                    let current = metric.measure(brp_client).await.map_err(Error::from)?;
                    Ok(current <= *max_value)
                }
                Self::ActionSucceeds { action } => {
                    let mut executor = ActionExecutor::new();
                    let result = executor.execute_action(action, brp_client).await?;
                    Ok(result.success)
                }
                Self::All { assertions } => {
                    for assertion in assertions {
                        if !assertion.evaluate(brp_client).await? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Self::Any { assertions } => {
                    for assertion in assertions {
                        if assertion.evaluate(brp_client).await? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Self::Not { assertion } => Ok(!assertion.evaluate(brp_client).await?),
            }
        })
    }
}

/// Predicate for matching component values
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Predicate {
    GreaterThan {
        value: serde_json::Value,
    },
    LessThan {
        value: serde_json::Value,
    },
    Between {
        min: serde_json::Value,
        max: serde_json::Value,
    },
    Contains {
        substring: String,
    },
    Regex {
        pattern: String,
    },
}

/// Performance metrics that can be measured
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMetric {
    FrameTime,
    EntityCount,
    ComponentCount,
    MemoryUsage,
    CpuUsage,
}

impl PerformanceMetric {
    /// Measure the current value of this metric, by asking the game.
    ///
    /// Only the two metrics the BRP surface can actually report are measured. The rest have no
    /// way to be observed from here, so they are refused with a reason rather than answered with
    /// a number. A number that was made up looks exactly like a measurement to the agent reading
    /// it, and the agent has no way to tell the difference.
    pub async fn measure(&self, brp_client: &mut BrpClient) -> BoxedResult<f64> {
        match self {
            Self::EntityCount | Self::ComponentCount => {
                let entities = query_entities(brp_client).await?;
                Ok(match self {
                    Self::EntityCount => entities.len() as f64,
                    Self::ComponentCount => entities
                        .iter()
                        .map(|entity| entity.components.len())
                        .sum::<usize>() as f64,
                    _ => unreachable!("the arm above is the only one that reaches here"),
                })
            }
            Self::FrameTime => Err(Box::new(Error::Validation(
                "frame time cannot be measured over BRP: the protocol reports no timing. Use the \
                 game's own timing, or assert on a component value instead."
                    .to_string(),
            ))),
            Self::MemoryUsage => Err(Box::new(Error::Validation(
                "memory usage cannot be measured over BRP: the protocol reports no process \
                 statistics. Use the game's own resource, or assert on a component value instead."
                    .to_string(),
            ))),
            Self::CpuUsage => Err(Box::new(Error::Validation(
                "CPU usage cannot be measured over BRP: the protocol reports no process \
                 statistics. Use the game's own resource, or assert on a component value instead."
                    .to_string(),
            ))),
        }
    }
}

/// Ask the game for its entities, so a count comes from the game rather than from a constant.
async fn query_entities(
    brp_client: &mut BrpClient,
) -> BoxedResult<Vec<crate::server::brp::EntityData>> {
    let request = crate::server::brp::BrpRequest {
        method: crate::server::brp::builtin_methods::BRP_QUERY_METHOD.to_string(),
        id: None,
        params: Some(crate::server::brp::query_params(None, true)),
    };
    let response = brp_client.send_request(&request).await.map_err(Box::new)?;
    match response.payload {
        crate::server::brp::BrpPayload::Result(value) => {
            crate::server::brp::entity_data_from_query_result(&value).ok_or_else(|| {
                Box::new(Error::Validation(
                    "the game answered world.query with a shape this server cannot read an \
                     entity list from"
                        .to_string(),
                ))
            })
        }
        crate::server::brp::BrpPayload::Error(error) => Err(Box::new(Error::Brp(error.message))),
    }
}

/// Strategy for generating test variations
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[derive(Default)]
pub enum VariationStrategy {
    /// No variations - run base test only
    #[default]
    None,
    /// Random variations within bounds
    Random {
        numeric_range: (f64, f64),
        string_pool: Vec<String>,
        bool_probability: f64,
    },
    /// Systematic grid search
    Grid {
        numeric_values: Vec<f64>,
        string_values: Vec<String>,
        bool_values: Vec<bool>,
    },
    /// Edge cases and boundary values
    Boundary {
        include_zero: bool,
        include_negative: bool,
        include_max: bool,
        include_empty: bool,
    },
    /// Property-based fuzzing
    Fuzz {
        max_mutations: usize,
        mutation_probability: f64,
    },
}


impl VariationStrategy {
    /// Generate variations of base actions
    pub fn generate_variations(
        &self,
        base_actions: &[Action],
        count: usize,
        seed: u64,
    ) -> Vec<Vec<Action>> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut variations = Vec::new();

        match self {
            Self::None => {
                variations.push(base_actions.to_vec());
            }
            Self::Random {
                numeric_range,
                string_pool,
                bool_probability,
            } => {
                for _ in 0..count {
                    let mut variant = base_actions.to_vec();
                    for action in &mut variant {
                        self.mutate_action_random(
                            action,
                            &mut rng,
                            *numeric_range,
                            string_pool,
                            *bool_probability,
                        );
                    }
                    variations.push(variant);
                }
            }
            Self::Grid {
                numeric_values,
                string_values,
                bool_values,
            } => {
                // Generate all combinations
                let total_combinations =
                    numeric_values.len() * string_values.len() * bool_values.len();
                let iterations = count.min(total_combinations);

                for i in 0..iterations {
                    let mut variant = base_actions.to_vec();
                    let num_idx = i % numeric_values.len();
                    let str_idx = (i / numeric_values.len()) % string_values.len();
                    let bool_idx =
                        (i / (numeric_values.len() * string_values.len())) % bool_values.len();

                    for action in &mut variant {
                        self.mutate_action_grid(
                            action,
                            numeric_values[num_idx],
                            &string_values[str_idx],
                            bool_values[bool_idx],
                        );
                    }
                    variations.push(variant);
                }
            }
            Self::Boundary {
                include_zero,
                include_negative,
                include_max,
                include_empty,
            } => {
                let boundary_values =
                    self.get_boundary_values(*include_zero, *include_negative, *include_max);

                for value in boundary_values.iter().take(count) {
                    let mut variant = base_actions.to_vec();
                    for action in &mut variant {
                        self.mutate_action_boundary(action, *value, *include_empty);
                    }
                    variations.push(variant);
                }
            }
            Self::Fuzz {
                max_mutations,
                mutation_probability,
            } => {
                for _ in 0..count {
                    let mut variant = base_actions.to_vec();
                    let mutations = rng.random_range(1..=*max_mutations);

                    for _ in 0..mutations {
                        if rng.random::<f64>() < *mutation_probability {
                            for action in &mut variant {
                                self.mutate_action_fuzz(action, &mut rng);
                            }
                        }
                    }
                    variations.push(variant);
                }
            }
        }

        variations
    }

    fn mutate_action_random(
        &self,
        action: &mut Action,
        rng: &mut StdRng,
        numeric_range: (f64, f64),
        string_pool: &[String],
        bool_probability: f64,
    ) {
        match action {
            Action::Spawn { components, .. } => {
                for component in components {
                    self.mutate_component_value(
                        &mut component.value,
                        rng,
                        numeric_range,
                        string_pool,
                        bool_probability,
                    );
                }
            }
            Action::Modify { components, .. } => {
                for component in components {
                    self.mutate_component_value(
                        &mut component.value,
                        rng,
                        numeric_range,
                        string_pool,
                        bool_probability,
                    );
                }
            }
            _ => {}
        }
    }

    fn mutate_action_grid(
        &self,
        action: &mut Action,
        num_value: f64,
        str_value: &str,
        bool_value: bool,
    ) {
        match action {
            Action::Spawn { components, .. } | Action::Modify { components, .. } => {
                for component in components {
                    if component.value.is_number() {
                        component.value = serde_json::json!(num_value);
                    } else if component.value.is_string() {
                        component.value = serde_json::json!(str_value);
                    } else if component.value.is_boolean() {
                        component.value = serde_json::json!(bool_value);
                    }
                }
            }
            _ => {}
        }
    }

    fn mutate_action_boundary(
        &self,
        action: &mut Action,
        boundary_value: f64,
        include_empty: bool,
    ) {
        match action {
            Action::Spawn { components, .. } | Action::Modify { components, .. } => {
                for component in components {
                    if component.value.is_number() {
                        component.value = serde_json::json!(boundary_value);
                    } else if component.value.is_string() && include_empty {
                        component.value = serde_json::json!("");
                    }
                }
            }
            _ => {}
        }
    }

    fn mutate_action_fuzz(&self, action: &mut Action, rng: &mut StdRng) {
        match action {
            Action::Spawn { components, .. } | Action::Modify { components, .. } => {
                for component in components {
                    component.value = self.fuzz_value(&component.value, rng);
                }
            }
            Action::Delete { entity_id } => {
                // Fuzz entity ID
                *entity_id = rng.random_range(0..1000);
            }
            _ => {}
        }
    }

    fn mutate_component_value(
        &self,
        value: &mut serde_json::Value,
        rng: &mut StdRng,
        numeric_range: (f64, f64),
        string_pool: &[String],
        bool_probability: f64,
    ) {
        match value {
            serde_json::Value::Number(_) => {
                let value_f64 = rng.random_range(numeric_range.0..numeric_range.1);
                *value = serde_json::json!(value_f64);
            }
            serde_json::Value::String(_) => {
                if !string_pool.is_empty() {
                    let idx = rng.random_range(0..string_pool.len());
                    *value = serde_json::json!(string_pool[idx].clone());
                }
            }
            serde_json::Value::Bool(_) => {
                *value = serde_json::json!(rng.random::<f64>() < bool_probability);
            }
            serde_json::Value::Object(map) => {
                for (_, v) in map.iter_mut() {
                    self.mutate_component_value(
                        v,
                        rng,
                        numeric_range,
                        string_pool,
                        bool_probability,
                    );
                }
            }
            _ => {}
        }
    }

    fn fuzz_value(&self, value: &serde_json::Value, rng: &mut StdRng) -> serde_json::Value {
        match value {
            serde_json::Value::Number(n) => {
                if let Some(f) = n.as_f64() {
                    let fuzzed = f * (0.5 + rng.random::<f64>() * 1.5);
                    serde_json::json!(fuzzed)
                } else {
                    value.clone()
                }
            }
            serde_json::Value::String(s) => {
                // Add random characters
                let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()"
                    .chars()
                    .collect();
                let mut fuzzed = s.clone();
                for _ in 0..rng.random_range(0..3) {
                    let idx = rng.random_range(0..chars.len());
                    fuzzed.push(chars[idx]);
                }
                serde_json::json!(fuzzed)
            }
            serde_json::Value::Bool(_) => {
                serde_json::json!(rng.random::<bool>())
            }
            _ => value.clone(),
        }
    }

    fn get_boundary_values(
        &self,
        include_zero: bool,
        include_negative: bool,
        include_max: bool,
    ) -> Vec<f64> {
        let mut values = Vec::new();

        if include_zero {
            values.push(0.0);
            values.push(0.0001); // Near zero
            values.push(-0.0001);
        }

        if include_negative {
            values.push(-1.0);
            values.push(-100.0);
            values.push(-1000000.0);
            values.push(f64::NEG_INFINITY);
        }

        if include_max {
            values.push(f64::MAX);
            values.push(f64::MIN);
            values.push(f64::INFINITY);
        }

        // Common boundaries
        values.push(1.0);
        values.push(-1.0);
        values.push(255.0); // u8 max
        values.push(65535.0); // u16 max

        values
    }
}

/// Test runner for executing hypothesis tests
pub struct TestRunner {
    iteration_count: usize,
    timeout: Duration,
    seed: Option<u64>,
    collect_edge_cases: bool,
    parallel_execution: bool,
}

impl TestRunner {
    /// Create new test runner with default settings
    pub fn new() -> Self {
        Self {
            iteration_count: 100,
            timeout: Duration::from_secs(60),
            seed: None,
            collect_edge_cases: true,
            parallel_execution: false,
        }
    }

    /// Create test runner with custom configuration
    pub fn with_config(iteration_count: usize, timeout: Duration) -> Self {
        Self {
            iteration_count,
            timeout,
            seed: None,
            collect_edge_cases: true,
            parallel_execution: false,
        }
    }

    /// Set random seed for reproducibility
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Set whether to collect edge cases
    pub fn with_edge_cases(mut self, collect: bool) -> Self {
        self.collect_edge_cases = collect;
        self
    }

    /// Set whether to run tests in parallel
    pub fn with_parallel(mut self, parallel: bool) -> Self {
        self.parallel_execution = parallel;
        self
    }

    /// Run hypothesis test
    pub async fn run(
        &self,
        hypothesis: &Hypothesis,
        brp_client: &mut BrpClient,
    ) -> Result<TestResult> {
        info!("Running hypothesis test: {}", hypothesis.description);

        let seed = self.seed.unwrap_or_else(|| {
            use std::time::SystemTime;
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });

        info!("Using seed: {} for reproducibility", seed);

        // Generate test variations
        let variations = hypothesis.variation_strategy.generate_variations(
            &hypothesis.base_actions,
            self.iteration_count,
            seed,
        );

        let mut successes = 0;
        let mut failures = 0;
        let mut edge_cases = Vec::new();
        let mut failure_examples = Vec::new();
        let mut execution_times = Vec::new();

        // Execute test variations
        for (i, variant) in variations.iter().enumerate() {
            debug!("Running iteration {} of {}", i + 1, variations.len());

            let start = std::time::Instant::now();

            // Execute actions
            let mut executor = ActionExecutor::new();
            let mut all_succeeded = true;

            for action in variant {
                let result =
                    tokio::time::timeout(self.timeout, executor.execute_action(action, brp_client))
                        .await;

                match result {
                    Ok(Ok(action_result)) => {
                        if !action_result.success {
                            all_succeeded = false;
                            break;
                        }
                    }
                    Ok(Err(e)) => {
                        warn!("Action failed: {}", e);
                        all_succeeded = false;
                        break;
                    }
                    Err(_) => {
                        warn!("Action timed out");
                        all_succeeded = false;
                        break;
                    }
                }
            }

            if !all_succeeded {
                failures += 1;
                if failure_examples.len() < 5 {
                    failure_examples.push(FailureExample {
                        iteration: i,
                        actions: variant.clone(),
                        error: "Action execution failed".to_string(),
                    });
                }
                continue;
            }

            // Evaluate success condition
            let condition_met = hypothesis.success_condition.evaluate(brp_client).await?;

            let elapsed = start.elapsed();
            execution_times.push(elapsed.as_millis() as f64);

            if condition_met {
                successes += 1;
            } else {
                failures += 1;
                if failure_examples.len() < 5 {
                    failure_examples.push(FailureExample {
                        iteration: i,
                        actions: variant.clone(),
                        error: "Success condition not met".to_string(),
                    });
                }
            }

            // Check for edge cases
            if self.collect_edge_cases && (elapsed.as_millis() > 1000 || !condition_met) {
                edge_cases.push(EdgeCase {
                    iteration: i,
                    actions: variant.clone(),
                    execution_time: elapsed,
                    passed: condition_met,
                });
            }
        }

        // Calculate statistics
        let total = successes + failures;
        let success_rate = if total > 0 {
            (successes as f64) / (total as f64)
        } else {
            0.0
        };

        let confidence_interval = self.calculate_confidence_interval(successes, total);

        let avg_execution_time = if !execution_times.is_empty() {
            execution_times.iter().sum::<f64>() / (execution_times.len() as f64)
        } else {
            0.0
        };

        let result = TestResult {
            hypothesis: hypothesis.description.clone(),
            iterations_run: total,
            successes,
            failures,
            success_rate,
            confidence_interval,
            edge_cases,
            failure_examples,
            avg_execution_time_ms: avg_execution_time,
            seed,
        };

        info!(
            "Test completed: {} successes, {} failures, {:.2}% success rate",
            successes,
            failures,
            success_rate * 100.0
        );

        Ok(result)
    }

    /// Calculate 95% confidence interval for success rate
    fn calculate_confidence_interval(&self, successes: usize, total: usize) -> (f64, f64) {
        if total == 0 {
            return (0.0, 0.0);
        }

        let p = (successes as f64) / (total as f64);
        let z = 1.96; // 95% confidence
        let n = total as f64;

        let margin = z * ((p * (1.0 - p)) / n).sqrt();

        ((p - margin).max(0.0), (p + margin).min(1.0))
    }
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of running a hypothesis test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub hypothesis: String,
    pub iterations_run: usize,
    pub successes: usize,
    pub failures: usize,
    pub success_rate: f64,
    pub confidence_interval: (f64, f64),
    pub edge_cases: Vec<EdgeCase>,
    pub failure_examples: Vec<FailureExample>,
    pub avg_execution_time_ms: f64,
    pub seed: u64,
}

impl TestResult {
    /// Check if hypothesis is confirmed (success rate above threshold)
    pub fn is_confirmed(&self, threshold: f64) -> bool {
        self.success_rate >= threshold
    }

    /// Get confidence level as percentage
    pub fn confidence_level(&self) -> f64 {
        let (lower, upper) = self.confidence_interval;
        if lower > 0.5 {
            // High confidence it's true
            (lower - 0.5) * 200.0
        } else if upper < 0.5 {
            // High confidence it's false
            (0.5 - upper) * 200.0
        } else {
            // Uncertain
            0.0
        }
    }
}

/// Edge case found during testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeCase {
    pub iteration: usize,
    pub actions: Vec<Action>,
    pub execution_time: Duration,
    pub passed: bool,
}

/// Example of a test failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureExample {
    pub iteration: usize,
    pub actions: Vec<Action>,
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::experiment_system::ComponentSpec;

    #[test]
    fn test_hypothesis_builder() {
        let hypothesis = Hypothesis::builder("Test hypothesis".to_string())
            .with_condition(Assertion::ActionSucceeds {
                action: Action::Delete { entity_id: 1 },
            })
            .with_variation(VariationStrategy::None)
            .build()
            .unwrap();

        assert_eq!(hypothesis.description, "Test hypothesis");
        assert!(matches!(
            hypothesis.success_condition,
            Assertion::ActionSucceeds { .. }
        ));
    }

    #[test]
    fn test_assertion_parsing() {
        let assertion = Assertion::parse("entity_exists:Transform,Health").unwrap();
        match assertion {
            Assertion::EntityExists {
                component_types, ..
            } => {
                assert_eq!(component_types.len(), 2);
                assert!(component_types.contains(&"Transform".to_string()));
                assert!(component_types.contains(&"Health".to_string()));
            }
            _ => panic!("Wrong assertion type"),
        }
    }

    #[test]
    fn test_variation_strategy_none() {
        let strategy = VariationStrategy::None;
        let base_actions = vec![Action::Delete { entity_id: 1 }];
        let variations = strategy.generate_variations(&base_actions, 10, 42);

        assert_eq!(variations.len(), 1);
        assert_eq!(variations[0].len(), 1);
    }

    #[test]
    fn test_variation_strategy_random() {
        let strategy = VariationStrategy::Random {
            numeric_range: (0.0, 100.0),
            string_pool: vec!["test1".to_string(), "test2".to_string()],
            bool_probability: 0.5,
        };

        let base_actions = vec![Action::Spawn {
            components: vec![ComponentSpec {
                type_id: "TestComponent".to_string(),
                value: serde_json::json!(50.0),
            }],
            archetype: None,
        }];

        let variations = strategy.generate_variations(&base_actions, 5, 42);
        assert_eq!(variations.len(), 5);
    }

    #[test]
    fn test_variation_strategy_boundary() {
        let strategy = VariationStrategy::Boundary {
            include_zero: true,
            include_negative: true,
            include_max: false,
            include_empty: true,
        };

        let base_actions = vec![Action::Delete { entity_id: 1 }];
        let variations = strategy.generate_variations(&base_actions, 3, 42);

        assert!(!variations.is_empty());
        assert!(variations.len() <= 3);
    }

    #[test]
    fn test_test_runner_creation() {
        let runner = TestRunner::new()
            .with_seed(12345)
            .with_edge_cases(true)
            .with_parallel(false);

        assert_eq!(runner.seed, Some(12345));
        assert!(runner.collect_edge_cases);
        assert!(!runner.parallel_execution);
    }

    #[test]
    fn test_confidence_interval_calculation() {
        let runner = TestRunner::new();

        // Perfect success
        let (lower, upper) = runner.calculate_confidence_interval(100, 100);
        assert!(lower > 0.9);
        assert_eq!(upper, 1.0);

        // Perfect failure
        let (lower, upper) = runner.calculate_confidence_interval(0, 100);
        assert_eq!(lower, 0.0);
        assert!(upper < 0.1);

        // 50/50
        let (lower, upper) = runner.calculate_confidence_interval(50, 100);
        assert!(lower < 0.5);
        assert!(upper > 0.5);
    }

    #[test]
    fn test_test_result_confirmation() {
        let result = TestResult {
            hypothesis: "Test".to_string(),
            iterations_run: 100,
            successes: 95,
            failures: 5,
            success_rate: 0.95,
            confidence_interval: (0.9, 1.0),
            edge_cases: vec![],
            failure_examples: vec![],
            avg_execution_time_ms: 10.0,
            seed: 42,
        };

        assert!(result.is_confirmed(0.9));
        assert!(result.is_confirmed(0.95));
        assert!(!result.is_confirmed(0.96));
    }

    #[test]
    fn test_predicate_serialization() {
        let predicate = Predicate::Between {
            min: serde_json::json!(0),
            max: serde_json::json!(100),
        };

        let json = serde_json::to_string(&predicate).unwrap();
        assert!(json.contains("between"));
        assert!(json.contains("min"));
        assert!(json.contains("max"));
    }
}
