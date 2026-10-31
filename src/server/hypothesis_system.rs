use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info};
// use rand_distr::{Distribution, Uniform}; // Currently unused, kept for future use

use crate::server::brp_client::BrpClient;
use crate::server::error::{BoxedResult, Error, Result};
use crate::server::experiment_system::{Action, ActionExecutor, Restoration};

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
        success_condition.check()?;

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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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
            // A string carries no action, and running a made-up one answers a different question.
            Err(Error::Validation(
                "action_succeeds needs the action it should run, which a string cannot carry. \
                 Use the object form: {\"type\": \"action_succeeds\", \"action\": {\"type\": \"delete\", \"entity_id\": 4294967297}}"
                    .to_string(),
            ))
        } else {
            Err(Error::Validation(format!(
                "Cannot parse assertion: {condition}. A string is entity_exists:<type>[,<type>...] or component_equals:<type>=<json value>; anything else takes the object form"
            )))
        }
    }

    /// Refuse a condition that could not be checked, or that every game satisfies, before any
    /// action runs: the actions change the live game, so a bad condition must not cost a mutation.
    pub fn check(&self) -> Result<()> {
        match self {
            Self::EntityExists { component_types, .. } => {
                if component_types.is_empty() || component_types.iter().any(|name| name.trim().is_empty()) {
                    return Err(Error::Validation(
                        "entity_exists needs at least one component type name in component_types (as a list), none of them empty: with no component to require, any entity satisfies it. Example: entity_exists:game::Crate".to_string(),
                    ));
                }
            }
            Self::ComponentEquals { component_type, .. } => {
                if component_type.trim().is_empty() {
                    return Err(Error::Validation(
                        "component_equals needs a component_type, the registered path or its short name".to_string(),
                    ));
                }
            }
            Self::ComponentMatches { .. } => {
                return Err(Error::Validation(
                    "component_matches cannot be evaluated: the server has no predicate evaluator, \
                     and answering true would confirm every hypothesis. Use component_equals, or \
                     assert on entity_exists."
                        .to_string(),
                ));
            }
            Self::PerformanceWithin { metric, .. } => {
                if let Some(reason) = metric.unmeasurable() {
                    return Err(Error::Validation(reason.to_string()));
                }
            }
            Self::ActionSucceeds { action } => {
                if action.component_types().iter().any(|name| name.trim().is_empty()) {
                    return Err(Error::Validation(
                        "action_succeeds names a component with an empty type_id; the server would refuse the action every time, which reads as the action failing".to_string(),
                    ));
                }
            }
            Self::All { assertions } | Self::Any { assertions } => {
                if assertions.is_empty() {
                    return Err(Error::Validation(
                        "all and any need at least one entry in assertions: an empty list is true for all and false for any, whatever the game does".to_string(),
                    ));
                }
                for assertion in assertions {
                    assertion.check()?;
                }
            }
            Self::Not { assertion } => assertion.check()?,
        }
        Ok(())
    }

    /// Every component name the condition reads from the game.
    pub fn component_names(&self) -> Vec<&str> {
        match self {
            Self::EntityExists { component_types, .. } => {
                component_types.iter().map(String::as_str).collect()
            }
            Self::ComponentEquals { component_type, .. }
            | Self::ComponentMatches { component_type, .. } => vec![component_type.as_str()],
            Self::PerformanceWithin { .. } => Vec::new(),
            Self::ActionSucceeds { action } => action.component_types(),
            Self::All { assertions } | Self::Any { assertions } => {
                assertions.iter().flat_map(Assertion::component_names).collect()
            }
            Self::Not { assertion } => assertion.component_names(),
        }
    }

    /// Evaluate assertion against current game state. An `action_succeeds` runs its action to see
    /// whether the game accepts it, then reverses it; what that reversal did is added to
    /// `restoration`, so the caller can say what stayed applied.
    pub fn evaluate<'a>(
        &'a self,
        brp_client: &'a mut BrpClient,
        restoration: &'a mut Restoration,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async move {
            match self {
                Self::EntityExists {
                    component_types,
                    count,
                } => {
                    let entities = query_entities(brp_client).await.map_err(|e| *e)?;
                    let matching = entities
                        .iter()
                        .filter(|entity| {
                            component_types
                                .iter()
                                .all(|name| component_value(entity, name).is_some())
                        })
                        .count();
                    Ok(match count {
                        Some(exact) => matching == *exact,
                        None => matching > 0,
                    })
                }
                Self::ComponentEquals {
                    entity_id,
                    component_type,
                    expected_value,
                } => {
                    let entities = query_entities(brp_client).await.map_err(|e| *e)?;
                    Ok(entities
                        .iter()
                        .filter(|entity| entity_id.is_none_or(|id| entity.id == id))
                        .any(|entity| {
                            component_value(entity, component_type)
                                .is_some_and(|value| same_value(value, expected_value))
                        }))
                }
                Self::ComponentMatches { .. } => Err(Error::Validation(
                    "component_matches cannot be evaluated yet: the server has no predicate \
                     evaluator, and answering true would confirm every hypothesis. Use \
                     component_equals, or assert on entity_exists."
                        .to_string(),
                )),
                Self::PerformanceWithin { metric, max_value } => {
                    let current = metric.measure(brp_client).await.map_err(Error::from)?;
                    Ok(current <= *max_value)
                }
                Self::ActionSucceeds { action } => {
                    let mut executor = ActionExecutor::new();
                    let result = executor.execute_action(action, brp_client).await;
                    // The probe asks whether the game accepts the action; it is not a change the
                    // caller asked for, so it is put back before the answer is given.
                    restoration.add(executor.restore_all(brp_client).await);
                    Ok(result?.success)
                }
                Self::All { assertions } => {
                    for assertion in assertions {
                        if !assertion.evaluate(brp_client, restoration).await? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                }
                Self::Any { assertions } => {
                    for assertion in assertions {
                        if assertion.evaluate(brp_client, restoration).await? {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                }
                Self::Not { assertion } => Ok(!assertion.evaluate(brp_client, restoration).await?),
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
    /// Why this metric cannot be measured over BRP, or `None` when it can.
    ///
    /// Only the two metrics the BRP surface can actually report are measured. The rest have no
    /// way to be observed from here, so they are refused with a reason rather than answered with
    /// a number. A number that was made up looks exactly like a measurement to the agent reading
    /// it, and the agent has no way to tell the difference.
    pub fn unmeasurable(&self) -> Option<&'static str> {
        match self {
            Self::EntityCount | Self::ComponentCount => None,
            Self::FrameTime => Some(
                "frame time cannot be measured over BRP: the protocol reports no timing. Use the \
                 game's own timing, or assert on a component value instead.",
            ),
            Self::MemoryUsage => Some(
                "memory usage cannot be measured over BRP: the protocol reports no process \
                 statistics. Use the game's own resource, or assert on a component value instead.",
            ),
            Self::CpuUsage => Some(
                "CPU usage cannot be measured over BRP: the protocol reports no process \
                 statistics. Use the game's own resource, or assert on a component value instead.",
            ),
        }
    }

    /// Measure the current value of this metric, by asking the game.
    pub async fn measure(&self, brp_client: &mut BrpClient) -> BoxedResult<f64> {
        if let Some(reason) = self.unmeasurable() {
            return Err(Box::new(Error::Validation(reason.to_string())));
        }
        let entities = query_entities(brp_client).await?;
        Ok(match self {
            Self::EntityCount => entities.len() as f64,
            Self::ComponentCount => entities
                .iter()
                .map(|entity| entity.components.len())
                .sum::<usize>() as f64,
            _ => unreachable!("`unmeasurable` refused every other metric"),
        })
    }
}

/// The value of `name` on `entity`. `name` is either the registered path or the short type name
/// at the end of one (`Health` finds `game::components::Health`).
fn component_value<'e>(
    entity: &'e crate::server::brp::EntityData,
    name: &str,
) -> Option<&'e serde_json::Value> {
    entity.components.iter().find_map(|(path, value)| {
        let short = path.rsplit("::").next().unwrap_or(path);
        (path == name || short == name).then_some(value)
    })
}

/// Whether two JSON values are the same value. Numbers are compared as numbers: the game writes
/// a float field as `3.0` and an agent writes `3`, and `serde_json::Value` equality tells them apart.
fn same_value(game: &serde_json::Value, expected: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (game, expected) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, a)| b.get(key).is_some_and(|b| same_value(a, b)))
        }
        _ => game == expected,
    }
}

/// The component type paths the game has registered.
pub async fn registered_components(brp_client: &mut BrpClient) -> BoxedResult<Vec<String>> {
    let request = crate::server::brp::BrpRequest {
        method: crate::server::brp::builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string(),
        id: None,
        params: None,
    };
    let response = brp_client.send_request(&request).await.map_err(Box::new)?;
    match response.payload {
        crate::server::brp::BrpPayload::Result(serde_json::Value::Array(paths)) => Ok(paths
            .into_iter()
            .filter_map(|path| path.as_str().map(String::from))
            .collect()),
        crate::server::brp::BrpPayload::Result(_) => Err(Box::new(Error::Validation(
            "the game answered world.list_components with something other than a list".to_string(),
        ))),
        crate::server::brp::BrpPayload::Error(error) => Err(Box::new(Error::Brp(error.message))),
    }
}

/// Refuse a component name the game has not registered, or one that names two registered
/// components. A name nothing can hold makes a claim false whatever the game does (and its
/// negation true), which reads as a measurement but is a typo.
pub fn require_registered(names: &[&str], registered: &[String]) -> Result<()> {
    let short = |path: &str| path.rsplit("::").next().unwrap_or(path).to_string();
    for name in names {
        if registered.iter().any(|path| path == name) {
            continue;
        }
        let sharing: Vec<&String> = registered.iter().filter(|path| short(path) == *name).collect();
        match sharing.as_slice() {
            [_] => {}
            [] => {
                let wanted = short(name).to_lowercase();
                let mut close: Vec<&str> = registered
                    .iter()
                    .filter(|path| {
                        let candidate = short(path).to_lowercase();
                        !wanted.is_empty() && (candidate.contains(&wanted) || wanted.contains(&candidate))
                    })
                    .map(String::as_str)
                    .collect();
                close.sort_unstable();
                close.truncate(5);
                let hint = if close.is_empty() {
                    "call world.list_components to see what is registered".to_string()
                } else {
                    format!("close registered names: {}", close.join(", "))
                };
                return Err(Error::Validation(format!(
                    "component {name} is not registered in the game, so no entity can hold it and a claim about it is false whatever the game does; {hint}"
                )));
            }
            many => {
                let paths: Vec<&str> = many.iter().map(|path| path.as_str()).collect();
                return Err(Error::Validation(format!(
                    "component name {name} is ambiguous: it is the short name of {}. Use the full path",
                    paths.join(" and ")
                )));
            }
        }
    }
    Ok(())
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


/// Apply `f` to every number, string and boolean inside `value`, however deeply it is nested in
/// objects and arrays. A Bevy component is nearly always a struct, so what a variation has to
/// change sits one or more levels below the component's value.
fn for_each_leaf(value: &mut serde_json::Value, f: &mut dyn FnMut(&mut serde_json::Value)) {
    use serde_json::Value;
    match value {
        Value::Object(map) => map.values_mut().for_each(|v| for_each_leaf(v, f)),
        Value::Array(items) => items.iter_mut().for_each(|v| for_each_leaf(v, f)),
        Value::Null => {}
        leaf => f(leaf),
    }
}

/// Apply `f` to every leaf of every value an action writes: the components of a spawn or a
/// modify, and those of the actions inside a batch. A delete writes none.
fn for_each_written_leaf(action: &mut Action, f: &mut dyn FnMut(&mut serde_json::Value)) {
    match action {
        Action::Spawn { components } | Action::Modify { components, .. } => {
            for component in components {
                for_each_leaf(&mut component.value, f);
            }
        }
        Action::Batch { actions, .. } => {
            for action in actions {
                for_each_written_leaf(action, f);
            }
        }
        Action::Delete { .. } => {}
    }
}

/// A copy of `actions` with `f` applied to every leaf they write.
fn varied(actions: &[Action], f: &mut dyn FnMut(&mut serde_json::Value)) -> Vec<Action> {
    let mut variant = actions.to_vec();
    for action in &mut variant {
        for_each_written_leaf(action, f);
    }
    variant
}

/// `value` as a JSON number of the kind `like` is. A number written over an integer is rounded to
/// an integer (`3`, where `3.0` is refused by a `u32` field); one written over a float stays a
/// float. A value that is not finite leaves `like` as it was: JSON has no infinity, and `null` is
/// not a number.
fn number_like(like: &serde_json::Number, value: f64) -> serde_json::Value {
    if !value.is_finite() {
        return serde_json::Value::Number(like.clone());
    }
    if like.is_i64() || like.is_u64() {
        let whole = value.round();
        if (0.0..u64::MAX as f64).contains(&whole) {
            return serde_json::json!(whole as u64);
        }
        if whole < 0.0 && whole >= i64::MIN as f64 {
            return serde_json::json!(whole as i64);
        }
    }
    serde_json::json!(value)
}

/// Which kinds of value a list of actions writes.
#[derive(Debug, Default, Clone, Copy)]
struct Kinds {
    numbers: bool,
    strings: bool,
    booleans: bool,
}

impl Kinds {
    fn of(actions: &[Action]) -> Self {
        let mut kinds = Self::default();
        let mut copy = actions.to_vec();
        for action in &mut copy {
            for_each_written_leaf(action, &mut |leaf| match leaf {
                serde_json::Value::Number(_) => kinds.numbers = true,
                serde_json::Value::String(_) => kinds.strings = true,
                serde_json::Value::Bool(_) => kinds.booleans = true,
                _ => {}
            });
        }
        kinds
    }
}

impl VariationStrategy {
    /// The name the caller gave this strategy in `variation_type`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Random { .. } => "random",
            Self::Grid { .. } => "grid",
            Self::Boundary { .. } => "boundary",
            Self::Fuzz { .. } => "fuzz",
        }
    }

    /// Generate variations of base actions. A variation changes the numbers, strings and booleans
    /// the actions write, at any depth of the component values; it never changes which entity an
    /// action targets or what kind of action it is.
    pub fn generate_variations(
        &self,
        base_actions: &[Action],
        count: usize,
        seed: u64,
    ) -> Vec<Vec<Action>> {
        use serde_json::Value;
        let mut rng = StdRng::seed_from_u64(seed);
        let mut variations = Vec::new();
        let kinds = Kinds::of(base_actions);

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
                    variations.push(varied(base_actions, &mut |leaf| match leaf {
                        Value::Number(like) => {
                            let like = like.clone();
                            let drawn = rng.random_range(numeric_range.0..numeric_range.1);
                            *leaf = number_like(&like, drawn);
                        }
                        Value::String(_) if !string_pool.is_empty() => {
                            let pick = rng.random_range(0..string_pool.len());
                            *leaf = Value::String(string_pool[pick].clone());
                        }
                        Value::Bool(_) => *leaf = Value::Bool(rng.random::<f64>() < *bool_probability),
                        _ => {}
                    }));
                }
            }
            Self::Grid {
                numeric_values,
                string_values,
                bool_values,
            } => {
                // Combinations of the kinds of value the actions write. A kind they do not write
                // multiplies nothing: it would only repeat identical iterations.
                let n = if kinds.numbers { numeric_values.len() } else { 1 };
                let s = if kinds.strings { string_values.len() } else { 1 };
                let b = if kinds.booleans { bool_values.len() } else { 1 };
                let iterations = count.min(n * s * b);

                for i in 0..iterations {
                    let (num_idx, str_idx, bool_idx) = (i % n, (i / n) % s, (i / (n * s)) % b);
                    variations.push(varied(base_actions, &mut |leaf| match leaf {
                        Value::Number(like) => {
                            let like = like.clone();
                            *leaf = number_like(&like, numeric_values[num_idx]);
                        }
                        Value::String(_) => *leaf = Value::String(string_values[str_idx].clone()),
                        Value::Bool(_) => *leaf = Value::Bool(bool_values[bool_idx]),
                        _ => {}
                    }));
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
                // One iteration per boundary number. Actions that write no number have only the
                // empty string to try, once.
                let numbers: Vec<Option<f64>> = if kinds.numbers {
                    boundary_values.iter().take(count).copied().map(Some).collect()
                } else {
                    vec![None; count.min(1)]
                };
                for number in numbers {
                    variations.push(varied(base_actions, &mut |leaf| match leaf {
                        Value::Number(like) => {
                            if let Some(boundary) = number {
                                let like = like.clone();
                                *leaf = number_like(&like, boundary);
                            }
                        }
                        Value::String(_) if *include_empty => *leaf = Value::String(String::new()),
                        _ => {}
                    }));
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
                                for_each_written_leaf(action, &mut |leaf| Self::fuzz_leaf(leaf, &mut rng));
                            }
                        }
                    }
                    variations.push(variant);
                }
            }
        }

        variations
    }

    /// Perturb one number, string or boolean.
    fn fuzz_leaf(leaf: &mut serde_json::Value, rng: &mut StdRng) {
        use serde_json::Value;
        match leaf {
            Value::Number(like) => {
                if let Some(f) = like.as_f64() {
                    let fuzzed = f * (0.5 + rng.random::<f64>() * 1.5);
                    // An integer field is fuzzed to another integer (`number_like` rounds it).
                    let like = like.clone();
                    *leaf = number_like(&like, fuzzed);
                }
            }
            Value::String(s) => {
                // Add random characters
                let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()"
                    .chars()
                    .collect();
                for _ in 0..rng.random_range(0..3) {
                    let idx = rng.random_range(0..chars.len());
                    s.push(chars[idx]);
                }
            }
            Value::Bool(_) => *leaf = Value::Bool(rng.random::<bool>()),
            _ => {}
        }
    }

    /// The numbers `boundary` tries. All finite: JSON has no infinity, and one written as `null`
    /// is not a boundary of anything. Each flag adds its own values and nothing else does.
    fn get_boundary_values(
        &self,
        include_zero: bool,
        include_negative: bool,
        include_max: bool,
    ) -> Vec<f64> {
        let mut values = Vec::new();

        if include_zero {
            values.extend([0.0, 0.0001, -0.0001]);
        }

        if include_negative {
            values.extend([-1.0, -100.0, -1_000_000.0]);
        }

        if include_max {
            values.extend([f64::MAX, f64::MIN]);
        }

        // Common boundaries
        values.extend([1.0, 255.0, 65535.0]);

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

    /// Run hypothesis test.
    ///
    /// An iteration holds the client from its first action to its last reversal and not a moment
    /// longer, so another tool is answered between iterations; it runs the iteration's actions,
    /// reads the condition, and then reverses what the actions spawned or modified, so the next
    /// iteration starts from the game the last one found. `timeout` bounds the whole run: when it
    /// passes the run stops, and the iterations that never ran count against a verdict of
    /// confirmed the way an iteration that was not evaluated does.
    pub async fn run(
        &self,
        hypothesis: &Hypothesis,
        brp_client: &RwLock<BrpClient>,
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

        // A variation that changes nothing is N copies of one test. Counting them as N trials
        // narrows the interval without testing anything, so it is refused before anything runs.
        let as_json = |actions: &Vec<Action>| {
            serde_json::to_value(actions).map_err(|e| Error::Validation(e.to_string()))
        };
        let base = as_json(&hypothesis.base_actions)?;
        let changed = variations
            .iter()
            .map(|variant| Ok(as_json(variant)? != base))
            .collect::<Result<Vec<bool>>>()?;
        let strategy = hypothesis.variation_strategy.name();
        if strategy != "none" && !variations.is_empty() && !changed.contains(&true) {
            let why = if hypothesis.base_actions.is_empty() {
                "there are no `actions` to vary"
            } else {
                "the `actions` write no value it changes, or it chose the values they already hold"
            };
            return Err(Error::Validation(format!(
                "variation_type `{strategy}` changes the numbers, strings and booleans inside `actions`, and {why}: none of its {} variations differs from the actions as given, so running them would repeat one test and count each repeat as a trial. Pass `actions` that write values (a spawn or modify with component values), or use variation_type none. Nothing was run",
                variations.len()
            )));
        }

        let planned = variations.len();
        let deadline = tokio::time::Instant::now() + self.timeout;
        let mut stopped_early: Option<String> = None;
        let mut restoration = Restoration::default();
        let mut held = 0;
        let mut violated = 0;
        let mut not_evaluated = 0;
        let mut varied_iterations = 0;
        let mut edge_cases = Vec::new();
        let mut examples = Vec::new();
        let mut execution_times = Vec::new();

        // Execute test variations
        for (i, variant) in variations.iter().enumerate() {
            if tokio::time::Instant::now() >= deadline {
                stopped_early = Some(format!(
                    "the {} s timeout passed after {i} of {planned} iterations",
                    self.timeout.as_secs()
                ));
                break;
            }
            debug!("Running iteration {} of {}", i + 1, planned);

            let mut client = brp_client.write().await;
            let start = std::time::Instant::now();

            // Execute actions
            let mut executor = ActionExecutor::new();
            // The first reason an action of this iteration did not run, in the game's own words.
            let mut action_error: Option<String> = None;
            let mut timed_out = false;

            for action in variant {
                let result =
                    tokio::time::timeout_at(deadline, executor.execute_action(action, &mut client))
                        .await;

                match result {
                    Ok(Ok(action_result)) => {
                        if !action_result.success {
                            action_error = Some(action_result.error.unwrap_or(action_result.message));
                            break;
                        }
                    }
                    Ok(Err(e)) => {
                        action_error = Some(e.to_string());
                        break;
                    }
                    Err(_) => {
                        timed_out = true;
                        action_error = Some(format!(
                            "the {} s timeout passed while an action was running; whether the game applied it is unknown",
                            self.timeout.as_secs()
                        ));
                        break;
                    }
                }
            }

            // Evaluate the success condition, unless an action failed first.
            let outcome = match action_error {
                Some(error) => Err(error),
                None => Ok(hypothesis.success_condition.evaluate(&mut client, &mut restoration).await),
            };
            let elapsed = start.elapsed();

            // Put back what this iteration spawned or modified, whatever the condition said, so
            // that the next one starts from the game this one found.
            restoration.add(executor.restore_all(&mut client).await);
            drop(client);
            varied_iterations += usize::from(changed[i]);

            let condition_met = match outcome {
                Ok(condition) => condition?,
                // The condition was not read, so this iteration neither held nor violated the claim.
                Err(error) => {
                    not_evaluated += 1;
                    if examples.len() < 5 {
                        examples.push(FailureExample {
                            iteration: i,
                            kind: ExampleKind::NotEvaluated,
                            actions: variant.clone(),
                            error,
                        });
                    }
                    if timed_out {
                        stopped_early = Some(format!(
                            "the {} s timeout passed during iteration {} of {planned}",
                            self.timeout.as_secs(),
                            i + 1
                        ));
                        break;
                    }
                    continue;
                }
            };

            execution_times.push(elapsed.as_millis() as f64);

            if condition_met {
                held += 1;
            } else {
                violated += 1;
                if examples.len() < 5 {
                    examples.push(FailureExample {
                        iteration: i,
                        kind: ExampleKind::Violated,
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

        // Rates and intervals are over the iterations that read the condition. With none, there is
        // no rate: 0 of 0 is not a result.
        let evaluated = held + violated;
        let success_rate = (evaluated > 0).then(|| (held as f64) / (evaluated as f64));
        let confidence_interval =
            (evaluated > 0).then(|| self.calculate_confidence_interval(held, evaluated));

        let avg_execution_time_ms = (!execution_times.is_empty())
            .then(|| execution_times.iter().sum::<f64>() / (execution_times.len() as f64));

        let result = TestResult {
            hypothesis: hypothesis.description.clone(),
            iterations_planned: planned,
            iterations_run: held + violated + not_evaluated,
            varied_iterations,
            held,
            violated,
            not_evaluated,
            success_rate,
            confidence_interval,
            edge_cases,
            examples,
            avg_execution_time_ms,
            seed,
            restoration,
            stopped_early,
        };

        info!(
            "Test completed: {held} held, {violated} violated, {not_evaluated} not evaluated"
        );

        Ok(result)
    }

    /// The 95% Wilson score interval for the share of iterations that held.
    ///
    /// Not the Wald interval, which has zero width at a rate of 0 or 1 whatever the number of
    /// trials and so reports certainty after one.
    fn calculate_confidence_interval(&self, successes: usize, total: usize) -> (f64, f64) {
        if total == 0 {
            // Nothing was read: any share is possible.
            return (0.0, 1.0);
        }

        let n = total as f64;
        let p = (successes as f64) / n;
        let z2 = 1.96_f64 * 1.96;
        let denominator = 1.0 + z2 / n;
        let centre = (p + z2 / (2.0 * n)) / denominator;
        let half = 1.96 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / denominator;

        // Exactly 0 and 1 at the ends: the formula gives them up to rounding.
        let lower = if successes == 0 { 0.0 } else { (centre - half).max(0.0) };
        let upper = if successes == total { 1.0 } else { (centre + half).min(1.0) };
        (lower, upper)
    }
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of running a hypothesis test.
///
/// Every iteration that ran is exactly one of `held`, `violated` or `not_evaluated`. An iteration
/// whose actions the game refused never read the condition, so it is `not_evaluated`: counting it
/// as `violated` told an agent its claim was false when the claim had not been looked at. An
/// iteration that was planned and never ran (the timeout passed) is none of the three.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub hypothesis: String,
    /// How many iterations the variation strategy produced; `iterations_run` is how many ran.
    pub iterations_planned: usize,
    pub iterations_run: usize,
    /// Of the iterations that ran, how many ran actions that differ from the actions as given.
    pub varied_iterations: usize,
    pub held: usize,
    pub violated: usize,
    pub not_evaluated: usize,
    /// `held` over the iterations that read the condition; `None` when none did.
    pub success_rate: Option<f64>,
    pub confidence_interval: Option<(f64, f64)>,
    pub edge_cases: Vec<EdgeCase>,
    pub examples: Vec<FailureExample>,
    /// Over the iterations that read the condition; `None` when none did.
    pub avg_execution_time_ms: Option<f64>,
    pub seed: u64,
    /// What reversing the iterations' changes did, summed over the run.
    pub restoration: Restoration,
    /// Why the run ended before every planned iteration ran; `None` when they all did.
    pub stopped_early: Option<String>,
}

/// What a test says about its claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Confirmed,
    Rejected,
    Inconclusive,
    /// No iteration read the condition, so the test says nothing about the claim.
    NotEvaluated,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirmed => "CONFIRMED",
            Self::Rejected => "REJECTED",
            Self::Inconclusive => "INCONCLUSIVE",
            Self::NotEvaluated => "NOT_EVALUATED",
        }
    }
}

/// Fewer than this share of iterations holding is a rejection.
pub const REJECTED_BELOW: f64 = 0.05;

impl TestResult {
    /// The iterations that read the condition.
    pub fn evaluated(&self) -> usize {
        self.held + self.violated
    }

    /// The verdict, bounding the share of planned iterations that held by what is known. An
    /// iteration that was not evaluated, or never ran, may or may not have held: it counts
    /// against `Confirmed` (only iterations known to hold count toward `threshold`) and for
    /// `Rejected` (a rejection needs the claim to fail even if every such iteration had held).
    pub fn verdict(&self, threshold: f64) -> Verdict {
        if self.evaluated() == 0 {
            return Verdict::NotEvaluated;
        }
        let total = self.iterations_planned as f64;
        let lowest = self.held as f64 / total;
        let highest = (self.iterations_planned - self.violated) as f64 / total;
        if lowest >= threshold {
            Verdict::Confirmed
        } else if highest < REJECTED_BELOW {
            Verdict::Rejected
        } else {
            Verdict::Inconclusive
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

/// Why an iteration did not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExampleKind {
    /// The condition was read and was false.
    Violated,
    /// An action failed first, so the condition was never read.
    NotEvaluated,
}

/// An iteration that did not hold, and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureExample {
    pub iteration: usize,
    pub kind: ExampleKind,
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

    fn counted(held: usize, violated: usize, not_evaluated: usize) -> TestResult {
        TestResult {
            hypothesis: "Test".to_string(),
            iterations_planned: held + violated + not_evaluated,
            iterations_run: held + violated + not_evaluated,
            varied_iterations: 0,
            held,
            violated,
            not_evaluated,
            success_rate: (held + violated > 0).then(|| held as f64 / (held + violated) as f64),
            confidence_interval: None,
            edge_cases: vec![],
            examples: vec![],
            avg_execution_time_ms: None,
            seed: 42,
            restoration: Restoration::default(),
            stopped_early: None,
        }
    }

    #[test]
    fn iterations_that_never_ran_neither_confirm_nor_reject() {
        // 850 of 1000 planned ran and held: at best 85% of the iterations are known to hold.
        let mut stopped = counted(850, 0, 0);
        stopped.iterations_planned = 1000;
        assert_eq!(stopped.verdict(0.95), Verdict::Inconclusive);
        // 20 of 1000 ran and all were violated: up to 98% of the plan could still have held.
        let mut stopped = counted(0, 20, 0);
        stopped.iterations_planned = 1000;
        assert_eq!(stopped.verdict(0.95), Verdict::Inconclusive);
    }

    fn modify(value: serde_json::Value) -> Action {
        Action::Modify {
            entity_id: 7,
            components: vec![ComponentSpec { type_id: "game::C".to_string(), value }],
        }
    }

    fn written(action: &Action) -> &serde_json::Value {
        match action {
            Action::Modify { components, .. } => &components[0].value,
            _ => panic!("not a modify"),
        }
    }

    #[test]
    fn every_strategy_varies_the_fields_inside_objects_and_arrays_and_keeps_integers_whole() {
        let base = vec![modify(serde_json::json!({"weight": 5.0, "count": 3, "tags": [1.5, "a"], "on": true}))];
        let strategies = [
            VariationStrategy::Random { numeric_range: (10.0, 20.0), string_pool: vec!["x".into()], bool_probability: 0.5 },
            VariationStrategy::Grid { numeric_values: vec![10.0], string_values: vec!["x".into()], bool_values: vec![false] },
            VariationStrategy::Boundary { include_zero: true, include_negative: true, include_max: false, include_empty: true },
            VariationStrategy::Fuzz { max_mutations: 3, mutation_probability: 1.0 },
        ];
        for strategy in strategies {
            let variations = strategy.generate_variations(&base, 3, 42);
            assert!(!variations.is_empty(), "{}", strategy.name());
            for variant in &variations {
                let value = written(&variant[0]);
                assert_ne!(value, written(&base[0]), "{} left the component as given", strategy.name());
                assert!(value["count"].is_i64() || value["count"].is_u64(), "{}: an integer field stayed whole: {value}", strategy.name());
                assert!(value["weight"].is_f64(), "{}: {value}", strategy.name());
                assert!(value["tags"][0].is_number() && value["tags"][1].is_string(), "{}: arrays are varied too: {value}", strategy.name());
            }
        }
    }

    #[test]
    fn grid_only_multiplies_the_kinds_of_value_the_actions_write() {
        // Numbers only: two numbers are two iterations, not two times the two default booleans.
        let grid = VariationStrategy::Grid {
            numeric_values: vec![10.0, 20.0],
            string_values: vec!["default".into()],
            bool_values: vec![true, false],
        };
        let variations = grid.generate_variations(&[modify(serde_json::json!({"weight": 5.0}))], 100, 1);
        assert_eq!(variations.len(), 2);
    }

    #[test]
    fn boundary_has_no_infinity_and_each_flag_adds_only_its_own_values() {
        let strategy = VariationStrategy::Boundary { include_zero: false, include_negative: false, include_max: false, include_empty: false };
        let values = strategy.get_boundary_values(false, false, false);
        assert_eq!(values, vec![1.0, 255.0, 65535.0], "no zero, no negative, no maximum was asked for");
        let all = strategy.get_boundary_values(true, true, true);
        assert!(all.iter().all(|v| v.is_finite()), "JSON cannot carry an infinity: {all:?}");
        assert!(all.contains(&-1.0) && all.contains(&0.0) && all.contains(&f64::MAX));
    }

    #[test]
    fn fuzz_never_changes_which_entity_a_delete_targets() {
        let strategy = VariationStrategy::Fuzz { max_mutations: 5, mutation_probability: 1.0 };
        for variant in strategy.generate_variations(&[Action::Delete { entity_id: 4294967297 }], 20, 9) {
            assert!(matches!(variant[0], Action::Delete { entity_id: 4294967297 }), "{:?}", variant[0]);
        }
    }

    #[test]
    fn verdict_is_decided_by_the_iterations_that_read_the_condition() {
        assert_eq!(counted(95, 5, 0).verdict(0.9), Verdict::Confirmed);
        assert_eq!(counted(95, 5, 0).verdict(0.95), Verdict::Confirmed);
        assert_eq!(counted(95, 5, 0).verdict(0.96), Verdict::Inconclusive);
        assert_eq!(counted(1, 99, 0).verdict(0.95), Verdict::Rejected);
        assert_eq!(counted(0, 0, 0).verdict(0.0), Verdict::NotEvaluated);
        assert_eq!(counted(0, 0, 7).verdict(0.0), Verdict::NotEvaluated);
    }

    #[test]
    fn an_iteration_that_was_not_evaluated_neither_confirms_nor_rejects() {
        // 50 held, 50 never read: at best the claim held in half the iterations.
        assert_eq!(counted(50, 0, 50).verdict(0.95), Verdict::Inconclusive);
        // 100 violated, 900 never read: up to 90% of the iterations could still have held.
        assert_eq!(counted(0, 100, 900).verdict(0.95), Verdict::Inconclusive);
        // Even if all 3 unread iterations had held, 3 of 100 is below the rejection line.
        assert_eq!(counted(0, 97, 3).verdict(0.95), Verdict::Rejected);
    }

    #[test]
    fn numbers_are_the_same_value_however_they_are_written() {
        use serde_json::json;
        assert!(same_value(&json!(3.0), &json!(3)));
        assert!(same_value(&json!({"weight": 3.0, "tags": [1.0, "a"]}), &json!({"tags": [1, "a"], "weight": 3})));
        assert!(!same_value(&json!(3.5), &json!(3)));
        assert!(!same_value(&json!(3.0), &json!("3")));
        assert!(!same_value(&json!({"weight": 3.0}), &json!({"weight": 3, "extra": 1})));
    }

    #[test]
    fn a_component_name_must_be_one_registered_component() {
        let registered: Vec<String> = ["game::Crate", "game::Score", "a::Health", "b::Health"]
            .map(String::from)
            .to_vec();
        assert!(require_registered(&["game::Crate", "Score", "a::Health"], &registered).is_ok());
        let unknown = require_registered(&["Crat"], &registered).unwrap_err().to_string();
        assert!(unknown.contains("not registered") && unknown.contains("game::Crate"), "{unknown}");
        let ambiguous = require_registered(&["Health"], &registered).unwrap_err().to_string();
        assert!(ambiguous.contains("a::Health") && ambiguous.contains("b::Health"), "{ambiguous}");
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
