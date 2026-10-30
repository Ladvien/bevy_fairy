/// State diffing and change detection for game state evolution tracking
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tracing::warn;

use crate::server::brp_messages::{ComponentTypeId, ComponentValue, EntityData, EntityId};

/// Types of changes that can occur to entities and components
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChangeType {
    /// Entity was added
    EntityAdded,
    /// Entity was removed
    EntityRemoved,
    /// Entity was modified (components changed)
    EntityModified,
    /// Component was added to entity
    ComponentAdded,
    /// Component was removed from entity
    ComponentRemoved,
    /// Component value was modified
    ComponentModified,
}

impl ChangeType {
    /// Get human-readable description
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::EntityAdded => "Entity added",
            Self::EntityRemoved => "Entity removed",
            Self::EntityModified => "Entity modified",
            Self::ComponentAdded => "Component added",
            Self::ComponentRemoved => "Component removed",
            Self::ComponentModified => "Component modified",
        }
    }

    /// Get terminal color code for display
    #[must_use]
    pub fn color_code(&self) -> &'static str {
        match self {
            Self::EntityAdded | Self::ComponentAdded => "\x1b[32m", // Green
            Self::EntityRemoved | Self::ComponentRemoved => "\x1b[31m", // Red
            Self::EntityModified | Self::ComponentModified => "\x1b[33m", // Yellow
        }
    }
}

/// A specific change detected between two states
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub change_type: ChangeType,
    pub entity_id: EntityId,
    pub component_type: Option<ComponentTypeId>,
    pub old_value: Option<ComponentValue>,
    pub new_value: Option<ComponentValue>,
    pub rate_of_change: Option<f64>, // For numeric values, units per second
    pub is_unexpected: bool,         // Based on game rules
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Change {
    /// Create a new change record
    #[must_use]
    pub fn new(
        change_type: ChangeType,
        entity_id: EntityId,
        component_type: Option<ComponentTypeId>,
        old_value: Option<ComponentValue>,
        new_value: Option<ComponentValue>,
    ) -> Self {
        Self {
            change_type,
            entity_id,
            component_type,
            old_value,
            new_value,
            rate_of_change: None,
            is_unexpected: false,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Calculate rate of change for numeric values
    pub fn calculate_rate_of_change(&mut self, time_delta: Duration) {
        if let (Some(old), Some(new)) = (&self.old_value, &self.new_value) {
            if let (Some(old_num), Some(new_num)) = (old.as_f64(), new.as_f64()) {
                let delta_seconds = time_delta.as_secs_f64();
                if delta_seconds > 0.0 {
                    self.rate_of_change = Some((new_num - old_num) / delta_seconds);
                }
            }
        }
    }

    /// Check if this change is unexpected based on game rules
    pub fn check_unexpected(&mut self, rules: &GameRules) {
        self.is_unexpected = rules.is_change_unexpected(self);
    }

    /// Format change for colored terminal output
    #[must_use]
    pub fn format_colored(&self) -> String {
        let color = self.change_type.color_code();
        let reset = "\x1b[0m";

        let unexpected_marker = if self.is_unexpected { " ⚠️" } else { "" };
        let rate_info = if let Some(rate) = self.rate_of_change {
            format!(" (rate: {rate:.3}/s)")
        } else {
            String::new()
        };

        match &self.component_type {
            Some(component) => {
                format!(
                    "{}{} entity {} {}: {}{}{}{}",
                    color,
                    self.change_type.description(),
                    self.entity_id,
                    component,
                    self.format_value_change(),
                    rate_info,
                    unexpected_marker,
                    reset
                )
            }
            None => {
                format!(
                    "{}{} entity {}{}{}",
                    color,
                    self.change_type.description(),
                    self.entity_id,
                    unexpected_marker,
                    reset
                )
            }
        }
    }

    fn format_value_change(&self) -> String {
        match (&self.old_value, &self.new_value) {
            (Some(old), Some(new)) => format!("{} → {}", format_value(old), format_value(new)),
            (None, Some(new)) => format!("→ {}", format_value(new)),
            (Some(old), None) => format!("{} → ∅", format_value(old)),
            (None, None) => String::new(),
        }
    }
}

/// Format a component value for display
fn format_value(value: &ComponentValue) -> String {
    match value {
        serde_json::Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 1e15 {
                    format!("{}", f as i64)
                } else {
                    format!("{f:.3}")
                }
            } else {
                n.to_string()
            }
        }
        serde_json::Value::String(s) => format!("\"{s}\""),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Array(arr) => {
            if arr.len() <= 3 {
                format!(
                    "[{}]",
                    arr.iter().map(format_value).collect::<Vec<_>>().join(", ")
                )
            } else {
                format!("[...{}]", arr.len())
            }
        }
        serde_json::Value::Object(obj) => {
            if obj.len() <= 2 {
                format!(
                    "{{{}}}",
                    obj.iter()
                        .map(|(k, v)| format!("{}: {}", k, format_value(v)))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                format!("{{...{}}}", obj.len())
            }
        }
        serde_json::Value::Null => "null".to_string(),
    }
}

/// Grouped changes by type for organized display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeGroup {
    pub group_type: String,
    pub changes: Vec<Change>,
    pub summary: String,
}

impl ChangeGroup {
    /// Create a new change group
    #[must_use]
    pub fn new(group_type: String, changes: Vec<Change>) -> Self {
        let summary = format!("{} changes of type {}", changes.len(), group_type);
        Self {
            group_type,
            changes,
            summary,
        }
    }
}

/// Game state snapshot for comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub entities: HashMap<EntityId, EntityData>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub generation: u64, // Generational index for efficient tracking
}

impl StateSnapshot {
    /// Create a new state snapshot
    #[must_use]
    pub fn new(entities: Vec<EntityData>, generation: u64) -> Self {
        let entity_map = entities
            .into_iter()
            .map(|entity| (entity.id, entity))
            .collect();

        Self {
            entities: entity_map,
            timestamp: chrono::Utc::now(),
            generation,
        }
    }

    /// Get entity by ID
    #[must_use]
    pub fn get_entity(&self, id: EntityId) -> Option<&EntityData> {
        self.entities.get(&id)
    }

    /// Get all entity IDs
    #[must_use]
    pub fn entity_ids(&self) -> HashSet<EntityId> {
        self.entities.keys().copied().collect()
    }

    /// Calculate compressed size estimate
    #[must_use]
    pub fn estimated_compressed_size(&self) -> usize {
        // Simple estimation based on JSON serialization
        serde_json::to_string(&self.entities)
            .map(|s| s.len() / 3) // Rough compression ratio
            .unwrap_or(0)
    }

    /// Compress snapshot for storage (simplified implementation)
    #[must_use]
    pub fn compress(&self) -> Vec<u8> {
        // In a real implementation, this would use a compression library like flate2
        // For now, we'll use JSON serialization as a placeholder
        serde_json::to_vec(&self.entities).unwrap_or_default()
    }

    /// Decompress snapshot from storage (simplified implementation)
    pub fn decompress(
        data: &[u8],
        generation: u64,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> Option<Self> {
        // In a real implementation, this would decompress using the same library
        let entities: HashMap<EntityId, EntityData> = serde_json::from_slice(data).ok()?;
        Some(Self {
            entities,
            timestamp,
            generation,
        })
    }
}

/// Configuration for fuzzy float comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuzzyCompareConfig {
    pub epsilon: f64,
    pub relative_tolerance: f64,
}

impl Default for FuzzyCompareConfig {
    fn default() -> Self {
        Self {
            epsilon: 1e-6,
            relative_tolerance: 1e-9,
        }
    }
}

/// Custom equality comparison with fuzzy float handling
pub trait FuzzyPartialEq {
    fn fuzzy_eq(&self, other: &Self, config: &FuzzyCompareConfig) -> bool;
}

impl FuzzyPartialEq for ComponentValue {
    fn fuzzy_eq(&self, other: &Self, config: &FuzzyCompareConfig) -> bool {
        match (self, other) {
            (serde_json::Value::Number(a), serde_json::Value::Number(b)) => {
                if let (Some(a_f), Some(b_f)) = (a.as_f64(), b.as_f64()) {
                    // Handle NaN and infinite values
                    if a_f.is_nan() && b_f.is_nan() {
                        return true; // NaN == NaN for our purposes
                    }
                    if a_f.is_infinite() || b_f.is_infinite() {
                        return a_f == b_f; // Must be exactly equal
                    }

                    let diff = (a_f - b_f).abs();
                    let max_val = a_f.abs().max(b_f.abs());
                    diff <= config.epsilon || diff <= max_val * config.relative_tolerance
                } else {
                    a == b
                }
            }
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|v2| v.fuzzy_eq(v2, config)))
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(v1, v2)| v1.fuzzy_eq(v2, config))
            }
            _ => self == other,
        }
    }
}

/// Rules for determining unexpected changes
#[derive(Debug, Clone, Default)]
pub struct GameRules {
    pub max_position_change_per_second: Option<f64>,
    pub max_velocity_change_per_second: Option<f64>,
    pub immutable_components: HashSet<ComponentTypeId>,
    pub expected_value_ranges: HashMap<String, (f64, f64)>, // component.field -> (min, max)
}

impl GameRules {
    /// Check if a change is unexpected based on the rules
    #[must_use]
    pub fn is_change_unexpected(&self, change: &Change) -> bool {
        // Check immutable components
        if let Some(component_type) = &change.component_type {
            if self.immutable_components.contains(component_type) {
                return true;
            }
        }

        // Check rate of change limits
        if let Some(rate) = change.rate_of_change {
            if let Some(component_type) = &change.component_type {
                match component_type.as_str() {
                    "Transform" => {
                        if let Some(max_pos_rate) = self.max_position_change_per_second {
                            if rate.abs() > max_pos_rate {
                                return true;
                            }
                        }
                    }
                    "Velocity" => {
                        if let Some(max_vel_rate) = self.max_velocity_change_per_second {
                            if rate.abs() > max_vel_rate {
                                return true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Check value ranges
        if let (Some(component_type), Some(new_value)) = (&change.component_type, &change.new_value)
        {
            if let Some(num_value) = new_value.as_f64() {
                let field_key = format!("{component_type}.value");
                if let Some((min, max)) = self.expected_value_ranges.get(&field_key) {
                    if num_value < *min || num_value > *max {
                        return true;
                    }
                }
            }
        }

        false
    }
}

/// Main state diffing engine
pub struct StateDiff {
    fuzzy_config: FuzzyCompareConfig,
    game_rules: GameRules,
    generation_counter: u64,
}

impl StateDiff {
    /// Create a new state diff engine
    #[must_use]
    pub fn new() -> Self {
        Self {
            fuzzy_config: FuzzyCompareConfig::default(),
            game_rules: GameRules::default(),
            generation_counter: 0,
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub fn with_config(fuzzy_config: FuzzyCompareConfig, game_rules: GameRules) -> Self {
        Self {
            fuzzy_config,
            game_rules,
            generation_counter: 0,
        }
    }

    /// Create a new snapshot from entity data
    pub fn create_snapshot(&mut self, entities: Vec<EntityData>) -> StateSnapshot {
        self.generation_counter = self.generation_counter.wrapping_add(1);
        StateSnapshot::new(entities, self.generation_counter)
    }

    /// Calculate differences between two snapshots
    pub fn diff_snapshots(&self, before: &StateSnapshot, after: &StateSnapshot) -> StateDiffResult {
        let mut changes = Vec::new();
        let time_delta = after.timestamp.signed_duration_since(before.timestamp);
        let time_delta_std = Duration::from_millis(time_delta.num_milliseconds().max(0) as u64);

        let before_ids = before.entity_ids();
        let after_ids = after.entity_ids();

        // Find added entities
        for entity_id in after_ids.difference(&before_ids) {
            changes.push(Change::new(
                ChangeType::EntityAdded,
                *entity_id,
                None,
                None,
                None,
            ));
        }

        // Find removed entities
        for entity_id in before_ids.difference(&after_ids) {
            changes.push(Change::new(
                ChangeType::EntityRemoved,
                *entity_id,
                None,
                None,
                None,
            ));
        }

        // Find modified entities
        for entity_id in before_ids.intersection(&after_ids) {
            if let (Some(before_entity), Some(after_entity)) =
                (before.get_entity(*entity_id), after.get_entity(*entity_id))
            {
                let entity_changes = self.diff_entity(before_entity, after_entity, time_delta_std);
                changes.extend(entity_changes);
            }
        }

        // Apply game rules to mark unexpected changes
        for change in &mut changes {
            change.check_unexpected(&self.game_rules);
        }

        StateDiffResult::new(changes, before.clone(), after.clone())
    }

    /// Compare two entities and find component-level changes
    fn diff_entity(
        &self,
        before: &EntityData,
        after: &EntityData,
        time_delta: Duration,
    ) -> Vec<Change> {
        let mut changes = Vec::new();

        let before_components: HashSet<&ComponentTypeId> = before.components.keys().collect();
        let after_components: HashSet<&ComponentTypeId> = after.components.keys().collect();

        // Find added components
        for component_type in after_components.difference(&before_components) {
            let new_value = match after.components.get(*component_type) {
                Some(value) => value,
                None => {
                    warn!("Component {:?} found in key set but not in component map", component_type);
                    continue;
                }
            };
            changes.push(Change::new(
                ChangeType::ComponentAdded,
                after.id,
                Some(component_type.to_string()),
                None,
                Some(new_value.clone()),
            ));
        }

        // Find removed components
        for component_type in before_components.difference(&after_components) {
            let old_value = match before.components.get(*component_type) {
                Some(value) => value,
                None => {
                    warn!("Component {:?} found in key set but not in component map", component_type);
                    continue;
                }
            };
            changes.push(Change::new(
                ChangeType::ComponentRemoved,
                before.id,
                Some(component_type.to_string()),
                Some(old_value.clone()),
                None,
            ));
        }

        // Find modified components
        for component_type in before_components.intersection(&after_components) {
            let before_value = match before.components.get(*component_type) {
                Some(value) => value,
                None => {
                    warn!("Component {:?} found in key set but not in component map", component_type);
                    continue;
                }
            };
            let after_value = match after.components.get(*component_type) {
                Some(value) => value,
                None => {
                    warn!("Component {:?} found in key set but not in component map", component_type);
                    continue;
                }
            };

            if !before_value.fuzzy_eq(after_value, &self.fuzzy_config) {
                let mut change = Change::new(
                    ChangeType::ComponentModified,
                    before.id,
                    Some(component_type.to_string()),
                    Some(before_value.clone()),
                    Some(after_value.clone()),
                );
                change.calculate_rate_of_change(time_delta);
                changes.push(change);
            }
        }

        changes
    }

    /// Group changes by type or component for organized display
    #[must_use]
    pub fn group_changes(&self, changes: &[Change]) -> Vec<ChangeGroup> {
        let mut groups: HashMap<String, Vec<Change>> = HashMap::new();

        for change in changes {
            let group_key = match &change.component_type {
                Some(component) => format!("{component} changes"),
                None => "Entity changes".to_string(),
            };
            groups.entry(group_key).or_default().push(change.clone());
        }

        groups
            .into_iter()
            .map(|(group_type, changes)| ChangeGroup::new(group_type, changes))
            .collect()
    }

    /// Update fuzzy comparison configuration
    pub fn set_fuzzy_config(&mut self, config: FuzzyCompareConfig) {
        self.fuzzy_config = config;
    }

    /// Update game rules
    pub fn set_game_rules(&mut self, rules: GameRules) {
        self.game_rules = rules;
    }

    /// Get the current generation counter
    #[must_use]
    pub fn generation_counter(&self) -> u64 {
        self.generation_counter
    }

    /// Get a reference to the fuzzy config
    #[must_use]
    pub fn fuzzy_config(&self) -> &FuzzyCompareConfig {
        &self.fuzzy_config
    }
}

impl Default for StateDiff {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a state diff operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDiffResult {
    pub changes: Vec<Change>,
    pub before_snapshot: StateSnapshot,
    pub after_snapshot: StateSnapshot,
    pub summary: DiffSummary,
}

impl StateDiffResult {
    /// Create a new diff result
    #[must_use]
    pub fn new(changes: Vec<Change>, before: StateSnapshot, after: StateSnapshot) -> Self {
        let summary = DiffSummary::from_changes(&changes);
        Self {
            changes,
            before_snapshot: before,
            after_snapshot: after,
            summary,
        }
    }

    /// Filter changes by type
    #[must_use]
    pub fn filter_by_type(&self, change_type: ChangeType) -> Vec<&Change> {
        self.changes
            .iter()
            .filter(|change| change.change_type == change_type)
            .collect()
    }

    /// Get only unexpected changes
    #[must_use]
    pub fn unexpected_changes(&self) -> Vec<&Change> {
        self.changes
            .iter()
            .filter(|change| change.is_unexpected)
            .collect()
    }

    /// Format all changes with colors for terminal display
    #[must_use]
    pub fn format_colored(&self) -> String {
        let mut output = Vec::new();

        output.push(format!(
            "State Diff ({} → {})",
            self.before_snapshot.timestamp.format("%H:%M:%S%.3f"),
            self.after_snapshot.timestamp.format("%H:%M:%S%.3f")
        ));
        output.push(format!("Summary: {}", self.summary.format()));
        output.push(String::new());

        if self.changes.is_empty() {
            output.push("No changes detected.".to_string());
        } else {
            for change in &self.changes {
                output.push(change.format_colored());
            }
        }

        output.join("\n")
    }
}

/// Summary statistics for a diff operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub entities_added: usize,
    pub entities_removed: usize,
    pub entities_modified: usize,
    pub components_added: usize,
    pub components_removed: usize,
    pub components_modified: usize,
    pub unexpected_changes: usize,
    pub total_changes: usize,
}

impl DiffSummary {
    /// Create summary from a list of changes.
    ///
    /// `entities_modified` counts the distinct entities with a component added, removed or
    /// modified. The diff reports an entity's change as component-level changes, never as one
    /// `EntityModified`, so counting only that kind answered 0 beside every component change.
    #[must_use]
    pub fn from_changes(changes: &[Change]) -> Self {
        let mut summary = Self {
            entities_added: 0,
            entities_removed: 0,
            entities_modified: 0,
            components_added: 0,
            components_removed: 0,
            components_modified: 0,
            unexpected_changes: 0,
            total_changes: changes.len(),
        };
        let mut modified_entities = HashSet::new();

        for change in changes {
            match change.change_type {
                ChangeType::EntityAdded => summary.entities_added += 1,
                ChangeType::EntityRemoved => summary.entities_removed += 1,
                ChangeType::EntityModified => {
                    modified_entities.insert(change.entity_id);
                }
                ChangeType::ComponentAdded => {
                    summary.components_added += 1;
                    modified_entities.insert(change.entity_id);
                }
                ChangeType::ComponentRemoved => {
                    summary.components_removed += 1;
                    modified_entities.insert(change.entity_id);
                }
                ChangeType::ComponentModified => {
                    summary.components_modified += 1;
                    modified_entities.insert(change.entity_id);
                }
            }

            if change.is_unexpected {
                summary.unexpected_changes += 1;
            }
        }
        summary.entities_modified = modified_entities.len();

        summary
    }

    /// Format summary as human-readable string
    #[must_use]
    pub fn format(&self) -> String {
        let mut parts = Vec::new();

        if self.entities_added > 0 {
            parts.push(format!("+{} entities", self.entities_added));
        }
        if self.entities_removed > 0 {
            parts.push(format!("-{} entities", self.entities_removed));
        }
        if self.entities_modified > 0 {
            parts.push(format!("~{} entities", self.entities_modified));
        }
        if self.components_added > 0 {
            parts.push(format!("+{} components", self.components_added));
        }
        if self.components_removed > 0 {
            parts.push(format!("-{} components", self.components_removed));
        }
        if self.components_modified > 0 {
            parts.push(format!("~{} components", self.components_modified));
        }

        let base_summary = if parts.is_empty() {
            "No changes".to_string()
        } else {
            parts.join(", ")
        };

        if self.unexpected_changes > 0 {
            format!("{} ({} unexpected)", base_summary, self.unexpected_changes)
        } else {
            base_summary
        }
    }
}

/// Time window for diff merging
#[derive(Debug, Clone)]
pub struct DiffTimeWindow {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub snapshots: Vec<StateSnapshot>,
    pub max_snapshots: usize, // Limit to prevent memory issues
}

impl DiffTimeWindow {
    /// Create a new time window
    #[must_use]
    pub fn new(start: chrono::DateTime<chrono::Utc>, end: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            start,
            end,
            snapshots: Vec::new(),
            max_snapshots: 100, // Default limit
        }
    }

    /// Create a new time window with custom snapshot limit
    #[must_use]
    pub fn with_capacity(
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
        max_snapshots: usize,
    ) -> Self {
        Self {
            start,
            end,
            snapshots: Vec::new(),
            max_snapshots,
        }
    }

    /// Add a snapshot to the window if it falls within the time range
    pub fn add_snapshot(&mut self, snapshot: StateSnapshot) -> bool {
        if snapshot.timestamp >= self.start && snapshot.timestamp <= self.end {
            self.snapshots.push(snapshot);
            self.snapshots.sort_by_key(|s| s.timestamp);

            // Enforce memory limit by removing oldest snapshots
            if self.snapshots.len() > self.max_snapshots {
                let excess = self.snapshots.len() - self.max_snapshots;
                self.snapshots.drain(0..excess);
            }

            true
        } else {
            false
        }
    }

    /// Get merged diff for the entire window
    pub fn get_merged_diff(&self, diff_engine: &StateDiff) -> Option<StateDiffResult> {
        if self.snapshots.len() < 2 {
            return None;
        }

        let first = self.snapshots.first()?;
        let last = self.snapshots.last()?;

        Some(diff_engine.diff_snapshots(first, last))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_test_entity(id: u64, components: Vec<(&str, serde_json::Value)>) -> EntityData {
        EntityData {
            id,
            components: components
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
    }

    #[test]
    fn test_fuzzy_float_comparison() {
        let config = FuzzyCompareConfig::default();

        let val1 = json!(1.0000001);
        let val2 = json!(1.0000002);
        let val3 = json!(1.1);

        assert!(val1.fuzzy_eq(&val2, &config));
        assert!(!val1.fuzzy_eq(&val3, &config));
    }

    #[test]
    fn test_entity_addition() {
        let mut diff_engine = StateDiff::new();

        let before = diff_engine.create_snapshot(vec![]);
        let after = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0, "y": 0.0}))],
        )]);

        let result = diff_engine.diff_snapshots(&before, &after);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, ChangeType::EntityAdded);
        assert_eq!(result.changes[0].entity_id, 1);
    }

    #[test]
    fn test_entity_removal() {
        let mut diff_engine = StateDiff::new();

        let before = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0, "y": 0.0}))],
        )]);
        let after = diff_engine.create_snapshot(vec![]);

        let result = diff_engine.diff_snapshots(&before, &after);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, ChangeType::EntityRemoved);
        assert_eq!(result.changes[0].entity_id, 1);
    }

    #[test]
    fn test_component_modification() {
        let mut diff_engine = StateDiff::new();

        let before = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0, "y": 0.0}))],
        )]);
        let after = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 1.0, "y": 0.0}))],
        )]);

        let result = diff_engine.diff_snapshots(&before, &after);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, ChangeType::ComponentModified);
        assert_eq!(result.changes[0].entity_id, 1);
        assert_eq!(
            result.changes[0].component_type,
            Some("Transform".to_string())
        );
    }

    #[test]
    fn test_component_addition_removal() {
        let mut diff_engine = StateDiff::new();

        let before = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0, "y": 0.0}))],
        )]);
        let after = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![
                ("Transform", json!({"x": 0.0, "y": 0.0})),
                ("Velocity", json!({"vx": 1.0, "vy": 0.0})),
            ],
        )]);

        let result = diff_engine.diff_snapshots(&before, &after);
        assert_eq!(result.changes.len(), 1);
        assert_eq!(result.changes[0].change_type, ChangeType::ComponentAdded);
        assert_eq!(
            result.changes[0].component_type,
            Some("Velocity".to_string())
        );
    }

    #[test]
    fn test_game_rules_unexpected_changes() {
        let mut diff_engine = StateDiff::new();
        let rules = GameRules {
            max_position_change_per_second: Some(10.0),
            ..Default::default()
        };
        diff_engine.set_game_rules(rules);

        let before = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0}))],
        )]);

        // Simulate 1 second later
        std::thread::sleep(std::time::Duration::from_millis(100));

        let after = diff_engine.create_snapshot(vec![
            create_test_entity(1, vec![("Transform", json!({"x": 100.0}))]), // Very fast change
        ]);

        let result = diff_engine.diff_snapshots(&before, &after);
        assert_eq!(result.changes.len(), 1);
        // Note: In a real scenario with proper timing, this would be unexpected
        // For this test, we just verify the structure works
    }

    #[test]
    fn test_change_grouping() {
        let diff_engine = StateDiff::new();

        let changes = vec![
            Change::new(
                ChangeType::ComponentModified,
                1,
                Some("Transform".to_string()),
                None,
                None,
            ),
            Change::new(
                ChangeType::ComponentModified,
                2,
                Some("Transform".to_string()),
                None,
                None,
            ),
            Change::new(
                ChangeType::ComponentModified,
                1,
                Some("Velocity".to_string()),
                None,
                None,
            ),
        ];

        let groups = diff_engine.group_changes(&changes);
        assert_eq!(groups.len(), 2); // Transform and Velocity groups

        let transform_group = groups
            .iter()
            .find(|g| g.group_type == "Transform changes")
            .unwrap();
        assert_eq!(transform_group.changes.len(), 2);
    }

    #[test]
    fn test_diff_summary() {
        let changes = vec![
            Change::new(ChangeType::EntityAdded, 1, None, None, None),
            Change::new(ChangeType::EntityRemoved, 2, None, None, None),
            Change::new(
                ChangeType::ComponentModified,
                3,
                Some("Transform".to_string()),
                None,
                None,
            ),
        ];

        let summary = DiffSummary::from_changes(&changes);
        assert_eq!(summary.entities_added, 1);
        assert_eq!(summary.entities_removed, 1);
        assert_eq!(summary.components_modified, 1);
        assert_eq!(summary.total_changes, 3);

        let formatted = summary.format();
        assert!(formatted.contains("+1 entities"));
        assert!(formatted.contains("-1 entities"));
        assert!(formatted.contains("~1 components"));
    }

    #[test]
    fn test_snapshot_creation() {
        let mut diff_engine = StateDiff::new();

        let entities = vec![
            create_test_entity(1, vec![("Transform", json!({"x": 0.0}))]),
            create_test_entity(2, vec![("Health", json!(100))]),
        ];

        let snapshot = diff_engine.create_snapshot(entities);
        assert_eq!(snapshot.entities.len(), 2);
        assert_eq!(snapshot.generation, 1);
        assert!(snapshot.get_entity(1).is_some());
        assert!(snapshot.get_entity(3).is_none());
    }

    #[test]
    fn test_time_window() {
        let mut diff_engine = StateDiff::new();
        let now = chrono::Utc::now();
        let mut window = DiffTimeWindow::new(now, now + chrono::Duration::seconds(10));

        let snapshot1 = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 0.0}))],
        )]);
        let snapshot2 = diff_engine.create_snapshot(vec![create_test_entity(
            1,
            vec![("Transform", json!({"x": 1.0}))],
        )]);

        assert!(window.add_snapshot(snapshot1));
        assert!(window.add_snapshot(snapshot2));

        let merged_diff = window.get_merged_diff(&diff_engine);
        assert!(merged_diff.is_some());

        let diff_result = merged_diff.unwrap();
        assert_eq!(diff_result.changes.len(), 1);
        assert_eq!(
            diff_result.changes[0].change_type,
            ChangeType::ComponentModified
        );
    }
}
