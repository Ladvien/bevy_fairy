/// Experiment system for modifying game state
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::collections::VecDeque;
use tracing::{debug, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::brp_messages::{
    builtin_methods, entity_data_from_get_result, BrpPayload, BrpRequest, ComponentTypeId,
    ComponentValue, EntityData, EntityId,
};
use crate::server::error::{Error, Result};

/// Action types for state manipulation
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Spawn a new entity with specified components
    Spawn { components: Vec<ComponentSpec> },
    /// Modify an existing entity's components
    Modify {
        entity_id: EntityId,
        components: Vec<ComponentSpec>,
    },
    /// Delete an entity
    Delete { entity_id: EntityId },
    /// Batch multiple actions as a transaction
    Batch {
        actions: Vec<Action>,
        #[serde(default)]
        atomic: bool,
    },
}

impl Action {
    /// The name an action's result carries as its `action_type`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Spawn { .. } => "spawn",
            Self::Modify { .. } => "modify",
            Self::Delete { .. } => "delete",
            Self::Batch { .. } => "batch",
        }
    }

    /// The component type paths this action names, looking inside batches.
    pub fn component_types(&self) -> Vec<&str> {
        match self {
            Self::Spawn { components } | Self::Modify { components, .. } => {
                components.iter().map(|c| c.type_id.as_str()).collect()
            }
            Self::Delete { .. } => Vec::new(),
            Self::Batch { actions, .. } => actions.iter().flat_map(Action::component_types).collect(),
        }
    }

    /// Point every action on entity `from` at entity `to`, looking inside batches.
    fn retarget(&mut self, from: EntityId, to: EntityId) {
        match self {
            Self::Modify { entity_id, .. } | Self::Delete { entity_id } if *entity_id == from => {
                *entity_id = to;
            }
            Self::Batch { actions, .. } => {
                for action in actions {
                    action.retarget(from, to);
                }
            }
            _ => {}
        }
    }
}

/// Component specification for spawn/modify actions
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComponentSpec {
    pub type_id: ComponentTypeId,
    pub value: ComponentValue,
}

/// Result of executing an action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub success: bool,
    pub action_type: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<EntityId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_available: Option<bool>,
    /// What each action of a batch did, in order. Only a batch has this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<ActionResult>>,
    /// The action ran, and was then reversed because its atomic batch failed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rolled_back: bool,
    /// For `modify`: what the game held after the write, read back from the game.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_back: Option<ReadBack>,
    /// For `undo` and `redo`: the kind of the recorded action that was undone or redone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub of_action: Option<String>,
    /// How to undo this action; kept for `undo`, never sent to the caller.
    #[serde(skip)]
    reverse: Option<Action>,
}

impl ActionResult {
    fn success(action_type: &str, message: String) -> Self {
        Self {
            success: true,
            action_type: action_type.to_string(),
            message,
            entity_id: None,
            error: None,
            rollback_available: None,
            results: None,
            rolled_back: false,
            read_back: None,
            of_action: None,
            reverse: None,
        }
    }

    fn success_with_entity(action_type: &str, message: String, entity_id: EntityId) -> Self {
        Self {
            entity_id: Some(entity_id),
            ..Self::success(action_type, message)
        }
    }

    fn failure(action_type: &str, error: String) -> Self {
        Self {
            success: false,
            message: format!("Action failed: {error}"),
            error: Some(error),
            ..Self::success(action_type, String::new())
        }
    }

    /// A batch's outcome, with what each of its actions did.
    fn batch(success: bool, message: String, results: Vec<ActionResult>) -> Self {
        let mut batch = if success {
            Self::success("batch", message)
        } else {
            Self::failure("batch", message)
        };
        batch.results = Some(results);
        batch
    }
}

/// What a game held after a `modify`, read back from it once the write had been accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadBack {
    /// Every written value is what the game holds now (numbers compared at the precision of a
    /// 32-bit float, the type most Bevy fields use).
    pub matches: bool,
    /// The written components as the game holds them.
    pub components: serde_json::Map<String, serde_json::Value>,
}

/// What reversing everything an executor recorded did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restoration {
    /// Actions put back: spawned entities despawned, modified components set to the value they had.
    pub reversed: usize,
    /// Actions that stay applied: a delete cannot be put back, a modify has no reverse when one of
    /// its components had no readable original, and a reversal can be refused by the game.
    pub left_applied: usize,
}

impl Restoration {
    /// Add what another reversal did.
    pub fn add(&mut self, other: Restoration) {
        self.reversed += other.reversed;
        self.left_applied += other.left_applied;
    }
}

/// Whether the game's value holds everything `written` says: an object holds each key written to
/// it, an array has the same items, a number is the same number. Numbers that are both integers
/// are compared exactly; any other pair at 32-bit float precision, which is what a `f32` field
/// keeps of an `f64` written to it.
fn holds(game: &serde_json::Value, written: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (game, written) {
        (Value::Number(game), Value::Number(written)) => {
            let integer = |n: &serde_json::Number| n.is_i64() || n.is_u64();
            if integer(game) && integer(written) {
                game == written
            } else {
                match (game.as_f64(), written.as_f64()) {
                    (Some(game), Some(written)) => (game as f32) == (written as f32),
                    _ => false,
                }
            }
        }
        (Value::Array(game), Value::Array(written)) => {
            game.len() == written.len() && game.iter().zip(written).all(|(g, w)| holds(g, w))
        }
        (Value::Object(game), Value::Object(written)) => {
            written.iter().all(|(key, w)| game.get(key).is_some_and(|g| holds(g, w)))
        }
        _ => game == written,
    }
}

/// Transaction for atomic action groups
#[derive(Debug, Clone)]
pub struct Transaction {
    pub actions: Vec<Action>,
    pub results: Vec<ActionResult>,
    pub rolled_back: bool,
}

impl Transaction {
    fn new(actions: Vec<Action>) -> Self {
        Self {
            actions,
            results: Vec::new(),
            rolled_back: false,
        }
    }

    fn add_result(&mut self, result: ActionResult) {
        self.results.push(result);
    }

    fn should_rollback(&self) -> bool {
        self.results.iter().any(|r| !r.success)
    }
}

/// History entry for undo/redo
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub action: Action,
    pub result: ActionResult,
    pub reverse_action: Option<Action>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl HistoryEntry {
    /// Point this entry at entity `to` wherever it names entity `from`.
    fn retarget(&mut self, from: EntityId, to: EntityId) {
        self.action.retarget(from, to);
        if let Some(reverse) = &mut self.reverse_action {
            reverse.retarget(from, to);
        }
        if let Some(reverse) = &mut self.result.reverse {
            reverse.retarget(from, to);
        }
        if self.result.entity_id == Some(from) {
            self.result.entity_id = Some(to);
        }
    }
}

/// Action executor with rollback and history
pub struct ActionExecutor {
    undo_stack: VecDeque<HistoryEntry>,
    redo_stack: VecDeque<HistoryEntry>,
    max_history_size: usize,
    enable_rollback: bool,
    transaction_stack: Vec<Transaction>,
}

impl ActionExecutor {
    /// Create a new action executor
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_history_size: 100,
            enable_rollback: true,
            transaction_stack: Vec::new(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(max_history_size: usize, enable_rollback: bool) -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: VecDeque::new(),
            max_history_size,
            enable_rollback,
            transaction_stack: Vec::new(),
        }
    }

    /// Execute a single action
    pub fn execute_action<'a>(
        &'a mut self,
        action: &'a Action,
        brp_client: &'a mut BrpClient,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ActionResult>> + Send + 'a>>
    {
        Box::pin(async move {
            let result = self.dispatch(action, brp_client, true).await;

            // Add to history if successful and not in a transaction. A batch is not an entry: it
            // has no reverse of its own, and an entry `undo` cannot step past would block every
            // earlier one. The batch's actions are recorded instead (see `record_committed`).
            if let Ok(action_result) = &result {
                if action_result.success
                    && self.transaction_stack.is_empty()
                    && !matches!(action, Action::Batch { .. })
                {
                    self.add_to_history(action.clone(), action_result.clone());
                }
            }

            result
        })
    }

    /// Run one action and report whether it can be reversed, without touching history.
    fn dispatch<'a>(
        &'a mut self,
        action: &'a Action,
        brp_client: &'a mut BrpClient,
        read_back: bool,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<ActionResult>> + Send + 'a>>
    {
        Box::pin(async move {
            debug!("Executing action: {:?}", action);

            let result = match action {
                Action::Spawn { components } => self.execute_spawn(components, brp_client).await,
                Action::Modify {
                    entity_id,
                    components,
                } => {
                    self.execute_modify(*entity_id, components, brp_client, read_back)
                        .await
                }
                Action::Delete { entity_id } => self.execute_delete(*entity_id, brp_client).await,
                Action::Batch { actions, atomic } => {
                    self.execute_batch(actions, *atomic, brp_client).await
                }
            };

            // An action the server refuses (an empty component type) is a failed action, reported
            // with the rest. As an `Err` it escaped the caller's loop: the results so far were
            // lost, and a batch's transaction was left open.
            let mut result =
                result.unwrap_or_else(|e| ActionResult::failure(action.kind(), e.to_string()));

            // `rollback_available` is derived from what `undo` will actually run.
            if result.success && !matches!(action, Action::Batch { .. }) {
                result.rollback_available =
                    Some(self.create_reverse_action(action, &result).is_some());
            }
            Ok(result)
        })
    }

    /// Execute spawn action
    async fn execute_spawn(
        &mut self,
        components: &[ComponentSpec],
        brp_client: &mut BrpClient,
    ) -> Result<ActionResult> {
        info!("Spawning entity with {} components", components.len());

        let final_components = components.to_vec();

        // Validate component types
        for component in &final_components {
            self.validate_component_type(&component.type_id)?;
        }

        // Create BRP spawn request (world.spawn_entity)
        let request = BrpRequest {
            method: builtin_methods::BRP_SPAWN_ENTITY_METHOD.to_string(),
            id: None,
            params: Some(serde_json::json!({
                "components": final_components
                    .into_iter()
                    .map(|c| (c.type_id, c.value))
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
            })),
        };

        // Send request
        match brp_client.send_request(&request).await {
            Ok(response) => match response.payload {
                BrpPayload::Result(value) => {
                    if let Some(entity_id) = value.get("entity").and_then(|e| e.as_u64()) {
                        Ok(ActionResult::success_with_entity(
                            "spawn",
                            format!("Successfully spawned entity {entity_id}"),
                            entity_id,
                        ))
                    } else {
                        Err(Error::Validation(
                            "Expected entity id in world.spawn_entity result".to_string(),
                        ))
                    }
                }
                BrpPayload::Error(err) => Ok(ActionResult::failure("spawn", err.message)),
            },
            Err(e) => Ok(ActionResult::failure("spawn", e.to_string())),
        }
    }

    /// Execute modify action
    async fn execute_modify(
        &mut self,
        entity_id: EntityId,
        components: &[ComponentSpec],
        brp_client: &mut BrpClient,
        read_back: bool,
    ) -> Result<ActionResult> {
        info!(
            "Modifying entity {} with {} component changes",
            entity_id,
            components.len()
        );

        // Validate component types
        for component in components {
            self.validate_component_type(&component.type_id)?;
        }

        // Store original values for rollback if enabled
        let original_values = if self.enable_rollback {
            self.get_original_component_values(entity_id, components, brp_client)
                .await?
        } else {
            None
        };

        // Create BRP modify request (world.insert_components)
        let request = BrpRequest {
            method: builtin_methods::BRP_INSERT_COMPONENTS_METHOD.to_string(),
            id: None,
            params: Some(serde_json::json!({
                "entity": entity_id,
                "components": components
                    .iter()
                    .map(|c| (c.type_id.clone(), c.value.clone()))
                    .collect::<serde_json::Map<String, serde_json::Value>>(),
            })),
        };

        // Send request
        match brp_client.send_request(&request).await {
            Ok(response) => match response.payload {
                BrpPayload::Result(_) => {
                    let mut result = ActionResult::success_with_entity(
                        "modify",
                        format!("Successfully modified entity {entity_id}"),
                        entity_id,
                    );

                    // Only claim a reverse when every component this set had an original value.
                    if let Some(originals) = original_values {
                        if originals.len() == components.len() {
                            result.reverse = Some(Action::Modify {
                                entity_id,
                                components: originals,
                            });
                        }
                    }

                    // The game's answer to the write is "accepted". What it holds a frame later
                    // is another matter: a system can set the value again.
                    if read_back {
                        self.note_read_back(&mut result, entity_id, components, brp_client).await;
                    }

                    Ok(result)
                }
                BrpPayload::Error(err) => Ok(ActionResult::failure("modify", err.message)),
            },
            Err(e) => Ok(ActionResult::failure("modify", e.to_string())),
        }
    }

    /// Read the written components back and say, in the result, whether the game holds them.
    async fn note_read_back(
        &self,
        result: &mut ActionResult,
        entity_id: EntityId,
        components: &[ComponentSpec],
        brp_client: &mut BrpClient,
    ) {
        let names = components.iter().map(|c| c.type_id.as_str()).collect();
        let Ok(Some(data)) = self.get_entity_data(entity_id, names, brp_client).await else {
            result.message = format!(
                "The game accepted the write to entity {entity_id}; it could not be read back to confirm it"
            );
            return;
        };
        let held: serde_json::Map<String, serde_json::Value> = components
            .iter()
            .filter_map(|c| data.components.get(&c.type_id).map(|value| (c.type_id.clone(), value.clone())))
            .collect();
        let differing: Vec<&str> = components
            .iter()
            .filter(|c| !held.get(&c.type_id).is_some_and(|game| holds(game, &c.value)))
            .map(|c| c.type_id.as_str())
            .collect();
        if !differing.is_empty() {
            result.message = format!(
                "The game accepted the write to entity {entity_id}, but a read back after it shows the entity does not hold what was written for {}: something in the game sets it, or the game stores it differently",
                differing.join(", ")
            );
        }
        result.read_back = Some(ReadBack { matches: differing.is_empty(), components: held });
    }

    /// Execute delete action
    async fn execute_delete(
        &mut self,
        entity_id: EntityId,
        brp_client: &mut BrpClient,
    ) -> Result<ActionResult> {
        info!("Deleting entity {}", entity_id);

        // A despawn cannot be undone: the entity returns as a different one, so nothing is recorded.

        // Create BRP delete request (world.despawn_entity)
        let request = BrpRequest {
            method: builtin_methods::BRP_DESPAWN_COMPONENTS_METHOD.to_string(),
            id: None,
            params: Some(serde_json::json!({ "entity": entity_id })),
        };

        // Send request
        match brp_client.send_request(&request).await {
            Ok(response) => match response.payload {
                BrpPayload::Result(_) => {
                    Ok(ActionResult::success(
                        "delete",
                        format!("Successfully deleted entity {entity_id}"),
                    ))
                }
                BrpPayload::Error(err) => Ok(ActionResult::failure("delete", err.message)),
            },
            Err(e) => Ok(ActionResult::failure("delete", e.to_string())),
        }
    }

    /// Execute batch of actions
    async fn execute_batch(
        &mut self,
        actions: &[Action],
        atomic: bool,
        brp_client: &mut BrpClient,
    ) -> Result<ActionResult> {
        info!(
            "Executing batch of {} actions (atomic: {})",
            actions.len(),
            atomic
        );

        // Reversing a batch that holds a batch would have to look inside it, and a half-reversed
        // rollback that says "rolled back" is worse than a refusal. Nothing has run yet.
        if actions.iter().any(|action| matches!(action, Action::Batch { .. })) {
            return Ok(ActionResult::batch(
                false,
                "a batch cannot contain a batch: put all the actions in one batch. Nothing was run".to_string(),
                Vec::new(),
            ));
        }

        if atomic {
            // Start transaction. The stack entry marks that one is open; `transaction` is the record.
            let mut transaction = Transaction::new(actions.to_vec());
            self.transaction_stack.push(transaction.clone());

            // An atomic batch stops at its first failure: what follows would only be undone.
            for action in actions {
                let result = self.execute_action(action, brp_client).await?;
                let failed = !result.success;
                transaction.add_result(result);
                if failed {
                    break;
                }
            }

            // Check if rollback needed
            if transaction.should_rollback() {
                warn!("Transaction failed, rolling back {} actions", actions.len());
                let not_reversed = self.rollback_transaction(&mut transaction, brp_client).await?;

                self.transaction_stack.pop();
                let reason = transaction
                    .results
                    .iter()
                    .position(|r| !r.success)
                    .map(|i| {
                        let error = transaction.results[i].error.as_deref().unwrap_or("no reason given");
                        format!("action {i} failed: {error}")
                    })
                    .unwrap_or_default();
                let message = if not_reversed == 0 {
                    format!("Transaction rolled back ({reason})")
                } else {
                    format!(
                        "Transaction failed ({reason}); {not_reversed} successful action(s) could not be reversed and are still applied in the game"
                    )
                };
                return Ok(ActionResult::batch(false, message, transaction.results));
            }

            self.transaction_stack.pop();
            // The batch committed: its actions become history entries, so `undo` steps back through them.
            if self.transaction_stack.is_empty() {
                self.record_committed(actions, &transaction.results);
            }

            Ok(ActionResult::batch(
                true,
                format!(
                    "Successfully executed {}/{} actions",
                    transaction.results.len(),
                    actions.len()
                ),
                transaction.results,
            ))
        } else {
            // Non-atomic batch - execute all regardless of failures
            let mut all_results = Vec::new();

            for action in actions {
                let result = self.execute_action(action, brp_client).await?;
                all_results.push(result);
            }

            let success_count = all_results.iter().filter(|r| r.success).count();
            let all_success = success_count == actions.len();

            let message = if all_success {
                format!("Successfully executed all {} actions", actions.len())
            } else {
                format!(
                    "Executed {}/{} actions successfully",
                    success_count,
                    actions.len()
                )
            };
            Ok(ActionResult::batch(all_success, message, all_results))
        }
    }

    /// Record the actions of a committed atomic batch, in order. Actions that ran inside the
    /// transaction were kept out of history while it was open, so a rollback could not leave
    /// entries behind for actions it had reversed.
    fn record_committed(&mut self, actions: &[Action], results: &[ActionResult]) {
        for (action, result) in actions.iter().zip(results) {
            if result.success {
                self.add_to_history(action.clone(), result.clone());
            }
        }
    }

    /// Rollback a transaction, marking each result that was reversed. Returns how many
    /// successful actions could not be.
    async fn rollback_transaction(
        &mut self,
        transaction: &mut Transaction,
        brp_client: &mut BrpClient,
    ) -> Result<usize> {
        let mut not_reversed = 0;
        for i in (0..transaction.results.len()).rev() {
            if !transaction.results[i].success {
                continue;
            }
            match self.create_reverse_action(&transaction.actions[i], &transaction.results[i]) {
                Some(reverse_action) => {
                    let reversed = self.dispatch(&reverse_action, brp_client, false).await;
                    if matches!(&reversed, Ok(r) if r.success) {
                        transaction.results[i].rolled_back = true;
                    } else {
                        not_reversed += 1;
                    }
                }
                None => not_reversed += 1,
            }
        }
        transaction.rolled_back = not_reversed == 0;
        Ok(not_reversed)
    }

    /// The action that undoes a successful `action`, recorded by the executor when it ran.
    fn create_reverse_action(&self, action: &Action, result: &ActionResult) -> Option<Action> {
        match action {
            // The reverse of a spawn is deleting the entity the game reported.
            Action::Spawn { .. } => result.entity_id.map(|id| Action::Delete { entity_id: id }),
            // A modify records the original values it overwrote, when it could read them all.
            Action::Modify { .. } => result.reverse.clone(),
            // A despawned entity comes back as a different entity; nothing here can restore it.
            Action::Delete { .. } | Action::Batch { .. } => None,
        }
    }

    /// Add action to history
    fn add_to_history(&mut self, action: Action, result: ActionResult) {
        let entry = HistoryEntry {
            action: action.clone(),
            result: result.clone(),
            reverse_action: self.create_reverse_action(&action, &result),
            timestamp: chrono::Utc::now(),
        };

        self.undo_stack.push_back(entry);

        // Limit history size
        while self.undo_stack.len() > self.max_history_size {
            self.undo_stack.pop_front();
        }

        // Clear redo stack on new action
        self.redo_stack.clear();
    }

    /// Undo last action
    pub async fn undo(&mut self, brp_client: &mut BrpClient) -> Result<ActionResult> {
        let Some(entry) = self.undo_stack.pop_back() else {
            return Ok(ActionResult::failure("undo", "Nothing to undo".to_string()));
        };
        let Some(reverse_action) = entry.reverse_action.clone() else {
            // The entry stays in history: a refused undo must not erase what happened.
            let what = entry.result.action_type.clone();
            self.undo_stack.push_back(entry);
            return Ok(ActionResult::failure(
                "undo",
                format!("`{what}` cannot be undone: nothing recorded how to reverse it (a delete cannot be put back, a modify can only be reversed when every component it set existed before)"),
            ));
        };
        let result = self.dispatch(&reverse_action, brp_client, true).await?;
        let result = Self::answer_as("undo", "Undid", entry.action.kind(), result);
        if result.success {
            self.redo_stack.push_back(entry);
        } else {
            self.undo_stack.push_back(entry);
        }
        Ok(result)
    }

    /// Redo last undone action
    pub async fn redo(&mut self, brp_client: &mut BrpClient) -> Result<ActionResult> {
        let Some(entry) = self.redo_stack.pop_back() else {
            return Ok(ActionResult::failure("redo", "Nothing to redo".to_string()));
        };
        let result = self.dispatch(&entry.action, brp_client, true).await?;
        if result.success {
            // Redoing a spawn makes a different entity, so the entry is rebuilt from this run, and
            // the entries still waiting to be redone follow the new entity: they name the old one.
            if let (Action::Spawn { .. }, Some(old), Some(new)) =
                (&entry.action, entry.result.entity_id, result.entity_id)
            {
                for waiting in &mut self.redo_stack {
                    waiting.retarget(old, new);
                }
            }
            let reverse_action = self.create_reverse_action(&entry.action, &result);
            let kind = entry.action.kind();
            self.undo_stack.push_back(HistoryEntry {
                action: entry.action,
                result: result.clone(),
                reverse_action,
                timestamp: chrono::Utc::now(),
            });
            Ok(Self::answer_as("redo", "Redid", kind, result))
        } else {
            let kind = entry.action.kind();
            self.redo_stack.push_back(entry);
            Ok(Self::answer_as("redo", "Redo of", kind, result))
        }
    }

    /// The reply to an `undo` or `redo`: the result of the action that ran, named for what was
    /// asked, not for the reverse operation that carried it out.
    fn answer_as(asked: &str, verb: &str, of_action: &'static str, mut result: ActionResult) -> ActionResult {
        result.action_type = asked.to_string();
        result.of_action = Some(of_action.to_string());
        result.rollback_available = None;
        result.message = if result.success {
            format!("{verb} `{of_action}`: {}", result.message)
        } else {
            format!("{verb} `{of_action}` failed: {}", result.message)
        };
        result
    }

    /// Reverse every action this executor recorded, newest first, and forget them. What cannot be
    /// reversed is counted, not hidden.
    pub async fn restore_all(&mut self, brp_client: &mut BrpClient) -> Restoration {
        let mut restoration = Restoration::default();
        self.redo_stack.clear();
        while let Some(entry) = self.undo_stack.pop_back() {
            let reversed = match &entry.reverse_action {
                Some(reverse) => matches!(self.dispatch(reverse, brp_client, false).await, Ok(r) if r.success),
                None => false,
            };
            if reversed {
                restoration.reversed += 1;
            } else {
                restoration.left_applied += 1;
            }
        }
        restoration
    }

    /// Get history
    pub fn get_history(&self) -> Vec<HistoryEntry> {
        self.undo_stack.iter().cloned().collect()
    }

    /// Clear history
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Validate component type
    fn validate_component_type(&self, type_id: &ComponentTypeId) -> Result<()> {
        if type_id.is_empty() {
            return Err(Error::Validation(
                "Component type ID cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Get original component values for rollback
    async fn get_original_component_values(
        &self,
        entity_id: EntityId,
        components: &[ComponentSpec],
        brp_client: &mut BrpClient,
    ) -> Result<Option<Vec<ComponentSpec>>> {
        // Query entity to get current values
        let names = components.iter().map(|c| c.type_id.as_str()).collect();
        let entity_data = self.get_entity_data(entity_id, names, brp_client).await?;

        Ok(entity_data.map(|entity_data| {
            let mut original_values = Vec::new();

            for component in components {
                if let Some(value) = entity_data.components.get(&component.type_id) {
                    original_values.push(ComponentSpec {
                        type_id: component.type_id.clone(),
                        value: value.clone(),
                    });
                }
            }

            original_values
        }))
    }

    /// Read the named components of an entity; a component it does not have is simply absent.
    async fn get_entity_data(
        &self,
        entity_id: EntityId,
        component_names: Vec<&str>,
        brp_client: &mut BrpClient,
    ) -> Result<Option<EntityData>> {
        let request = BrpRequest {
            method: builtin_methods::BRP_GET_COMPONENTS_METHOD.to_string(),
            id: None,
            params: Some(serde_json::json!({
                "entity": entity_id,
                "components": component_names,
                "strict": false
            })),
        };

        match brp_client.send_request(&request).await {
            Ok(response) => match response.payload {
                BrpPayload::Result(value) => Ok(entity_data_from_get_result(entity_id, &value)),
                BrpPayload::Error(_) => Ok(None),
            },
            _ => Ok(None),
        }
    }
}

impl Default for ActionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_result_creation() {
        let success = ActionResult::success("test", "Test succeeded".to_string());
        assert!(success.success);
        assert_eq!(success.action_type, "test");

        let failure = ActionResult::failure("test", "Test failed".to_string());
        assert!(!failure.success);
        assert!(failure.error.is_some());
    }

    #[test]
    fn test_component_spec_creation() {
        let spec = ComponentSpec {
            type_id: "Transform".to_string(),
            value: serde_json::json!({"x": 0.0, "y": 0.0, "z": 0.0}),
        };
        assert_eq!(spec.type_id, "Transform");
    }

    #[test]
    fn test_transaction_creation() {
        let actions = vec![
            Action::Delete { entity_id: 1 },
            Action::Delete { entity_id: 2 },
        ];

        let mut transaction = Transaction::new(actions.clone());
        assert_eq!(transaction.actions.len(), 2);
        assert!(!transaction.rolled_back);

        transaction.add_result(ActionResult::success("delete", "Deleted".to_string()));
        assert!(!transaction.should_rollback());

        transaction.add_result(ActionResult::failure("delete", "Failed".to_string()));
        assert!(transaction.should_rollback());
    }

    #[test]
    fn test_action_executor_creation() {
        let executor = ActionExecutor::new();
        assert_eq!(executor.max_history_size, 100);
        assert!(executor.enable_rollback);

        let custom_executor = ActionExecutor::with_config(50, false);
        assert_eq!(custom_executor.max_history_size, 50);
        assert!(!custom_executor.enable_rollback);
    }

    #[tokio::test]
    async fn test_validate_component_type() {
        let executor = ActionExecutor::new();

        // Valid component type
        assert!(executor
            .validate_component_type(&"Transform".to_string())
            .is_ok());

        // Empty component type
        assert!(executor.validate_component_type(&"".to_string()).is_err());

        // Unknown but valid component type
        assert!(executor
            .validate_component_type(&"CustomComponent".to_string())
            .is_ok());
    }

    #[test]
    fn test_history_management() {
        let mut executor = ActionExecutor::with_config(2, true); // Small history for testing

        // Add to history
        executor.add_to_history(
            Action::Delete { entity_id: 1 },
            ActionResult::success("delete", "Deleted".to_string()),
        );

        assert_eq!(executor.undo_stack.len(), 1);
        assert_eq!(executor.redo_stack.len(), 0);

        // Add another
        executor.add_to_history(
            Action::Delete { entity_id: 2 },
            ActionResult::success("delete", "Deleted".to_string()),
        );

        assert_eq!(executor.undo_stack.len(), 2);

        // Add one more - should remove oldest
        executor.add_to_history(
            Action::Delete { entity_id: 3 },
            ActionResult::success("delete", "Deleted".to_string()),
        );

        assert_eq!(executor.undo_stack.len(), 2); // Limited to max_history_size

        // Clear history
        executor.clear_history();
        assert_eq!(executor.undo_stack.len(), 0);
        assert_eq!(executor.redo_stack.len(), 0);
    }
}
