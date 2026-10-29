//! BRP (Bevy Remote Protocol) types.
//!
//! The wire-level protocol types (`BrpRequest`, `BrpResponse`, `BrpError`,
//! `BrpResult`, ...) are re-exported from the pinned `bevy_remote` crate so
//! this client always speaks the exact protocol of the matching Bevy version.
//!
//! Bevy's BRP has been HTTP JSON-RPC 2.0 since Bevy 0.16 (see
//! `bevy_remote::http`); there is no WebSocket transport.
//!
//! All other (non-wire) debugging data types used across this crate are
//! defined locally here.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Real Bevy wire types (Bevy 0.19, JSON-RPC 2.0 over HTTP)
// ---------------------------------------------------------------------------

pub use bevy_remote::{BrpError, BrpRequest, BrpResponse};

/// The result payload of a BRP request: `Result<serde_json::Value, BrpError>`.
///
/// In real Bevy this is a type alias, not an enum. The response value is the
/// method-specific JSON document described in the `bevy_remote` crate docs
/// (e.g. an array of `{entity, components, has}` objects for `world.query`).
pub use bevy_remote::BrpResult;

/// The JSON-RPC result/error payload of a [`BrpResponse`].
pub use bevy_remote::BrpPayload;

pub use bevy_remote::builtin_methods;

/// The plugin feature that registers a `bevy_fairy/*` method, or `None` for a method the plugin
/// does not register at all (a typo gets the game's bare "not found", not a feature to turn on).
fn feature_registering(method: &str) -> Option<&'static str> {
    match method {
        "bevy_fairy/screenshot" | "bevy_fairy/capture" => Some("plugin-screenshot"),
        "bevy_fairy/guide" | "bevy_fairy/guide+watch" => Some("plugin-guide"),
        "bevy_fairy/ui_text" => Some("plugin-ui-text"),
        "bevy_fairy/input" | "bevy_fairy/status" | "bevy_fairy/commands" | "bevy_fairy/scene"
        | "bevy_fairy/quit" | "bevy_fairy/time" => Some("plugin"),
        _ => None,
    }
}

/// What an agent reads when the game refused `method`.
///
/// A bare "Method `bevy_fairy/screenshot` not found" names neither the cause nor the fix, and an
/// agent that reads it starts debugging its own JSON. There is exactly one cause for a missing
/// `bevy_fairy/*` method: the game did not add `DebuggerPlugin`, or built it without the feature
/// that registers that method. So name the feature.
pub fn refusal_text(method: &str, error: &BrpError) -> String {
    match (error.code, feature_registering(method)) {
        (bevy_remote::error_codes::METHOD_NOT_FOUND, Some(feature)) => format!(
            "{} — the game does not register `{method}`: add DebuggerPlugin, from bevy_fairy \
             built with the `{feature}` feature",
            error.message
        ),
        _ => error.message.clone(),
    }
}

// ---------------------------------------------------------------------------
// Wire response decoders/encoders
//
// On the wire, a BRP result is method-specific JSON. These functions decode
// the `serde_json::Value` payloads of `world.query`, `world.get_components`,
// and `world.list_components` into the shared `EntityData` / `ComponentTypeInfo`
// shapes used throughout this crate.
// ---------------------------------------------------------------------------

/// Decode a `world.query` result into `Vec<EntityData>`.
///
/// Wire form: `[{ "entity": N, "components": {type: value, ...}, "has": {...}? }, ...]`
pub fn entity_data_from_query_result(value: &serde_json::Value) -> Option<Vec<EntityData>> {
    let rows = value.as_array()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id = row.get("entity")?.as_u64()?;
        let mut components: HashMap<ComponentTypeId, ComponentValue> = HashMap::new();
        if let Some(map) = row.get("components").and_then(|c| c.as_object()) {
            for (k, v) in map {
                components.insert(k.clone(), v.clone());
            }
        }
        for (k, v) in row.get("has").and_then(|c| c.as_object()).into_iter().flatten() {
            components.insert(k.clone(), v.clone());
        }
        out.push(EntityData { id, components });
    }
    Some(out)
}

/// Decode a `world.get_components` result into a single `EntityData`.
///
/// Wire form (non-strict): `{ "components": {...}, "errors": {...} }`;
/// wire form (strict): the component map directly.
pub fn entity_data_from_get_result(
    entity: EntityId,
    value: &serde_json::Value,
) -> Option<EntityData> {
    let map = value
        .get("components")
        .and_then(|c| c.as_object())
        .or_else(|| value.as_object())?;
    let components = map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    Some(EntityData { id: entity, components })
}

/// Decode a `world.list_components` result (`["a::b::C", ...]`) into
/// `Vec<ComponentTypeInfo>`.
pub fn component_type_info_from_list_result(value: &serde_json::Value) -> Vec<ComponentTypeInfo> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|name| ComponentTypeInfo {
                    id: name.to_string(),
                    name: name.to_string(),
                    schema: None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Serialize an optional `QueryFilter` to the `world.query` params
/// (`data`/`filter` envelope documented in `bevy_remote`).
pub fn query_params(filter: Option<&QueryFilter>, components_all: bool) -> serde_json::Value {
    let with: Vec<&str> = filter
        .and_then(|f| f.with.as_deref())
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let without: Vec<&str> = filter
        .and_then(|f| f.without.as_deref())
        .map(|v| v.iter().map(String::as_str).collect())
        .unwrap_or_default();
    let option = if components_all {
        serde_json::Value::from("all")
    } else {
        serde_json::Value::Array(vec![])
    };
    serde_json::json!({
        "data": {
            "components": [],
            "option": option,
            "has": []
        },
        "filter": {
            "with": with,
            "without": without
        },
        "strict": false
    })
}

// ---------------------------------------------------------------------------
// Common identifier aliases
// ---------------------------------------------------------------------------

/// Unique identifier for an entity in the Bevy ECS world.
///
/// On the wire this is Bevy's 64-bit entity serialization
/// (`(generation << 32) | index`), as used by `world.query`,
/// `world.get_components`, etc.
pub type EntityId = u64;

/// Unique identifier for a component type.
///
/// On the wire these are fully-qualified type paths,
/// e.g. `bevy_transform::components::transform::Transform`.
pub type ComponentTypeId = String;

/// Raw JSON value for flexible component data.
pub type ComponentValue = serde_json::Value;

// ---------------------------------------------------------------------------
// Debug overlay types
// ---------------------------------------------------------------------------

/// Debug overlay types for visual debugging
#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DebugOverlayType {
    /// Entity highlight overlay
    EntityHighlight,
    /// Physics collider visualization
    Colliders,
    /// Physics collider visualization (alternative name for compatibility)
    ColliderVisualization,
    /// Transform gizmos
    Transforms,
    /// Transform gizmos (alternative name for compatibility)
    TransformGizmos,
    /// System execution flow
    SystemFlow,
    /// Performance metrics
    PerformanceMetrics,
    /// Debug markers
    DebugMarkers,
    /// Custom overlay
    Custom(String),
}

// ---------------------------------------------------------------------------
// Query types
// ---------------------------------------------------------------------------

/// Query filter for selecting entities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QueryFilter {
    /// Entities must have all of these components
    pub with: Option<Vec<ComponentTypeId>>,
    /// Entities must not have any of these components
    pub without: Option<Vec<ComponentTypeId>>,
    /// Component value filters
    pub where_clause: Option<Vec<ComponentFilter>>,
}

/// Filter for component values
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentFilter {
    /// Component type to filter on
    pub component: ComponentTypeId,
    /// Field path within the component (e.g., "position.x")
    pub field: Option<String>,
    /// Filter operation
    pub op: FilterOp,
    /// Value to compare against
    pub value: ComponentValue,
}

/// Filter operations for component values
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FilterOp {
    #[serde(rename = "eq")]
    Equal,
    #[serde(rename = "ne")]
    NotEqual,
    #[serde(rename = "gt")]
    GreaterThan,
    #[serde(rename = "gte")]
    GreaterThanOrEqual,
    #[serde(rename = "lt")]
    LessThan,
    #[serde(rename = "lte")]
    LessThanOrEqual,
    #[serde(rename = "contains")]
    Contains,
    #[serde(rename = "regex")]
    Regex,
}

/// Validated query structure for safe ECS queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedQuery {
    /// Query ID for caching
    pub id: String,
    /// Validated filter
    pub filter: QueryFilter,
    /// Estimated cost
    pub estimated_cost: QueryCost,
    /// Optimization hints
    pub hints: Vec<String>,
}

/// Query cost estimation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCost {
    /// Estimated entities to scan
    pub estimated_entities: usize,
    /// Estimated time in microseconds
    pub estimated_time_us: u64,
    /// Memory usage estimate in bytes
    pub estimated_memory: usize,
}

// ---------------------------------------------------------------------------
// Debug command types
// ---------------------------------------------------------------------------

/// Debug command types for extensible debugging operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "params")]
#[non_exhaustive]
pub enum DebugCommand {
    /// Inspect entity with detailed component information
    InspectEntity {
        entity_id: EntityId,
        /// Include component metadata
        include_metadata: Option<bool>,
        /// Include parent/child relationships
        include_relationships: Option<bool>,
    },

    /// Inspect multiple entities in a single batch operation
    InspectBatch {
        entity_ids: Vec<EntityId>,
        /// Include component metadata for all entities
        include_metadata: Option<bool>,
        /// Include parent/child relationships for all entities
        include_relationships: Option<bool>,
        /// Maximum number of entities to inspect (default: 100)
        limit: Option<usize>,
    },

    /// Profile system performance
    ProfileSystem {
        /// System name to profile
        system_name: String,
        /// Duration in milliseconds
        duration_ms: Option<u64>,
        /// Include memory allocations
        track_allocations: Option<bool>,
    },

    /// Enable/disable visual debugging overlays
    SetVisualDebug {
        /// Type of overlay
        overlay_type: DebugOverlayType,
        /// Enable or disable
        enabled: bool,
        /// Optional configuration
        config: Option<serde_json::Value>,
    },

    /// Execute a validated ECS query
    ExecuteQuery {
        /// Validated query structure
        query: ValidatedQuery,
        /// Pagination offset
        offset: Option<usize>,
        /// Results limit
        limit: Option<usize>,
    },

    /// Validate a query without executing it
    ValidateQuery {
        /// Query parameters as JSON
        params: serde_json::Value,
    },

    /// Estimate the cost of executing a query
    EstimateCost {
        /// Query parameters as JSON
        params: serde_json::Value,
    },

    /// Get optimization suggestions for a query
    GetQuerySuggestions {
        /// Query parameters as JSON
        params: serde_json::Value,
    },

    /// Build and execute a query using the query builder
    BuildAndExecuteQuery {
        /// Query parameters as JSON
        params: serde_json::Value,
    },

    /// Memory profiling command
    ProfileMemory {
        /// Capture allocation backtraces
        capture_backtraces: Option<bool>,
        /// Track specific systems
        target_systems: Option<Vec<String>>,
        /// Duration in seconds for profiling session
        duration_seconds: Option<u64>,
    },

    /// Stop memory profiling session
    StopMemoryProfiling {
        /// Session ID to stop (None for default session)
        session_id: Option<String>,
    },

    /// Get current memory profile
    GetMemoryProfile,

    /// Detect memory leaks
    DetectMemoryLeaks {
        /// Target systems to check for leaks
        target_systems: Option<Vec<String>>,
    },

    /// Analyze memory usage trends
    AnalyzeMemoryTrends {
        /// Target systems to analyze
        target_systems: Option<Vec<String>>,
    },

    /// Take a memory snapshot
    TakeMemorySnapshot,

    /// Get memory profiler statistics
    GetMemoryStatistics,

    /// Session management
    SessionControl {
        /// Session operation
        operation: SessionOperation,
        /// Session ID
        session_id: Option<String>,
    },

    /// Get debug system status
    GetStatus,

    /// Get entity hierarchy information
    GetHierarchy {
        /// Optional root entity to start from
        root_entity: Option<EntityId>,
        /// Maximum depth to traverse
        max_depth: Option<usize>,
    },

    /// Get system information and metadata
    GetSystemInfo {
        /// Optional system name filter
        system_name: Option<String>,
        /// Include scheduling information
        include_scheduling: Option<bool>,
    },

    /// Start automated issue detection monitoring
    StartIssueDetection,

    /// Stop automated issue detection monitoring
    StopIssueDetection,

    /// Get detected issues/alerts
    GetDetectedIssues {
        /// Maximum number of issues to return
        limit: Option<usize>,
    },

    /// Acknowledge an issue alert
    AcknowledgeIssue {
        /// Alert ID to acknowledge
        alert_id: String,
    },

    /// Report an alert as a false positive
    ReportFalsePositive {
        /// Alert ID to mark as false positive
        alert_id: String,
    },

    /// Get issue detection statistics
    GetIssueDetectionStats,

    /// Update a detection rule configuration
    UpdateDetectionRule {
        /// Rule name to update
        name: String,
        /// Enable/disable the rule
        enabled: Option<bool>,
        /// Sensitivity level (0.0 to 1.0)
        sensitivity: Option<f32>,
    },

    /// Clear issue alert history
    ClearIssueHistory,

    /// Start performance budget monitoring
    StartBudgetMonitoring,

    /// Stop performance budget monitoring
    StopBudgetMonitoring,

    /// Set performance budget configuration
    SetPerformanceBudget {
        /// Budget configuration as JSON
        config: serde_json::Value,
    },

    /// Get current performance budget configuration
    GetPerformanceBudget,

    /// Check for current budget violations
    CheckBudgetViolations,

    /// Get budget violation history
    GetBudgetViolationHistory {
        /// Maximum number of violations to return
        limit: Option<usize>,
    },

    /// Generate compliance report for specified duration
    GenerateComplianceReport {
        /// Duration in seconds (default: 3600 = 1 hour)
        duration_seconds: Option<u64>,
    },

    /// Get budget recommendations based on historical data
    GetBudgetRecommendations,

    /// Clear budget violation history
    ClearBudgetHistory,

    /// Get budget monitoring statistics
    GetBudgetStatistics,

    /// Take a screenshot of the primary window
    Screenshot {
        /// Path where to save the screenshot (optional)
        path: Option<String>,
        /// Time in milliseconds to wait before capture (game warmup)
        warmup_duration: Option<u64>,
        /// Additional delay in milliseconds before capture
        capture_delay: Option<u64>,
        /// Whether to wait for at least one frame to render
        wait_for_render: Option<bool>,
        /// Optional description for logging/debugging
        description: Option<String>,
    },

    /// Custom debug command for extensions
    Custom {
        /// Command name
        name: String,
        /// Command parameters
        params: serde_json::Value,
    },
}

// ---------------------------------------------------------------------------
// Session types
// ---------------------------------------------------------------------------

/// Session operations for debug session management
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SessionOperation {
    /// Create new session
    Create,
    /// Resume existing session
    Resume,
    /// Checkpoint current state
    Checkpoint,
    /// Restore from checkpoint
    Restore { checkpoint_id: String },
    /// End session
    End,
}

/// Debug session state
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SessionState {
    /// Session is active
    Active,
    /// Session is paused
    Paused,
    /// Session is replaying commands
    Replaying,
    /// Session ended
    Ended,
}

/// Checkpoint information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointInfo {
    /// Checkpoint ID
    pub id: String,
    /// Creation timestamp
    pub timestamp: u64,
    /// Description
    pub description: Option<String>,
    /// Size in bytes
    pub size: usize,
}

// ---------------------------------------------------------------------------
// Debug response types
// ---------------------------------------------------------------------------

/// Debug response types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum DebugResponse {
    /// Entity inspection result
    EntityInspection {
        entity: EntityData,
        metadata: Option<EntityMetadata>,
        relationships: Option<EntityRelationships>,
    },

    /// Batch entity inspection result
    BatchEntityInspection {
        entities: Vec<EntityInspectionResult>,
        /// Total entities requested
        requested_count: usize,
        /// Successfully inspected entities
        found_count: usize,
        /// Entities that were not found (despawned)
        missing_entities: Vec<EntityId>,
        /// Total inspection time in microseconds
        inspection_time_us: u64,
    },

    /// System profiling result
    SystemProfile(SystemProfile),

    /// Profiling started response
    ProfilingStarted {
        system_name: String,
        duration_ms: Option<u64>,
    },

    /// Profiling history response
    ProfileHistory {
        system_name: String,
        samples: Vec<ProfileSample>,
        frame_count: usize,
    },

    /// Performance anomalies response
    PerformanceAnomalies {
        count: usize,
        anomalies: Vec<serde_json::Value>,
    },

    /// Visual debug status
    VisualDebugStatus {
        overlay_type: DebugOverlayType,
        enabled: bool,
        config: Option<serde_json::Value>,
    },

    /// Query execution result
    QueryResult {
        entities: Vec<EntityData>,
        total_count: usize,
        execution_time_us: u64,
        has_more: bool,
    },

    /// Memory profile result
    MemoryProfile {
        total_allocated: usize,
        allocations_per_system: HashMap<String, usize>,
        top_allocations: Vec<AllocationInfo>,
    },

    /// Session control result
    SessionStatus {
        session_id: String,
        state: SessionState,
        command_count: usize,
        checkpoints: Vec<CheckpointInfo>,
    },

    /// Debug system status
    Status {
        version: String,
        active_sessions: usize,
        command_queue_size: usize,
        performance_overhead_percent: f32,
    },

    /// Query validation result
    QueryValidation {
        valid: bool,
        query: Option<ValidatedQuery>,
        errors: Vec<String>,
        suggestions: Vec<String>,
    },

    /// Query cost estimation result
    QueryCost {
        cost: QueryCost,
        performance_budget_exceeded: bool,
        suggestions: Vec<String>,
    },

    /// Query optimization suggestions
    QuerySuggestions {
        suggestions: Vec<String>,
        query_complexity: u32,
    },

    /// Query execution result
    QueryExecution {
        success: bool,
        result: Option<serde_json::Value>,
        execution_time_us: u64,
        entities_processed: Option<usize>,
    },

    /// Generic success response
    Success {
        message: String,
        data: Option<serde_json::Value>,
    },

    /// Custom debug response
    Custom(serde_json::Value),
}

// ---------------------------------------------------------------------------
// Entity data types
// ---------------------------------------------------------------------------

/// Entity data with components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    /// Entity identifier
    pub id: EntityId,
    /// Component data by type
    pub components: HashMap<ComponentTypeId, ComponentValue>,
}

/// Information about a component type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentTypeInfo {
    /// Component type identifier
    pub id: ComponentTypeId,
    /// Human-readable name
    pub name: String,
    /// JSON schema for the component structure
    pub schema: Option<serde_json::Value>,
}

/// Entity metadata for inspection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMetadata {
    /// Component count
    pub component_count: usize,
    /// Total memory size in bytes
    pub memory_size: usize,
    /// Last modified timestamp
    pub last_modified: Option<u64>,
    /// Entity generation (Bevy 0.16 compatibility)
    pub generation: u32,
    /// Entity index (Bevy 0.16 compatibility)
    pub index: u32,
    /// Component type information
    pub component_types: Vec<DetailedComponentTypeInfo>,
    /// Which components have been modified
    pub modified_components: Vec<String>,
    /// Entity archetype information
    pub archetype_id: Option<u32>,
    /// Entity location in world storage
    pub location_info: Option<EntityLocationInfo>,
}

/// Detailed component type information with reflection data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedComponentTypeInfo {
    /// Component type identifier
    pub type_id: String,
    /// Human-readable type name
    pub type_name: String,
    /// Size in bytes
    pub size_bytes: usize,
    /// Whether component has reflection data
    pub is_reflected: bool,
    /// Type schema if available
    pub schema: Option<serde_json::Value>,
    /// Whether component was modified this frame
    pub is_modified: bool,
}

/// Entity location information in Bevy's storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityLocationInfo {
    /// Archetype ID
    pub archetype_id: u32,
    /// Index within archetype
    pub index: u32,
    /// Table ID
    pub table_id: Option<u32>,
    /// Row in table
    pub table_row: Option<u32>,
}

/// Entity relationships
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRelationships {
    /// Parent entity if exists
    pub parent: Option<EntityId>,
    /// Child entities
    pub children: Vec<EntityId>,
    /// Related entities (custom relationships)
    pub related: HashMap<String, Vec<EntityId>>,
}

/// Individual entity inspection result for batch operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityInspectionResult {
    /// Entity ID
    pub entity_id: EntityId,
    /// Whether the entity was found
    pub found: bool,
    /// Entity data if found
    pub entity: Option<EntityData>,
    /// Entity metadata if requested and found
    pub metadata: Option<EntityMetadata>,
    /// Entity relationships if requested and found
    pub relationships: Option<EntityRelationships>,
    /// Error message if inspection failed
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Profiling types
// ---------------------------------------------------------------------------

/// System performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMetrics {
    /// Total execution time in microseconds
    pub total_time_us: u64,
    /// Minimum execution time in microseconds
    pub min_time_us: u64,
    /// Maximum execution time in microseconds
    pub max_time_us: u64,
    /// Average execution time in microseconds
    pub avg_time_us: u64,
    /// Median execution time in microseconds
    pub median_time_us: u64,
    /// 95th percentile time
    pub p95_time_us: u64,
    /// 99th percentile time
    pub p99_time_us: u64,
    /// Total memory allocations
    pub total_allocations: usize,
    /// Allocation rate per invocation
    pub allocation_rate: f32,
    /// Overhead percentage
    pub overhead_percent: f32,
}

/// Profile sample point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSample {
    /// Timestamp in microseconds
    pub timestamp: u64,
    /// Execution time in microseconds
    pub duration_us: u64,
    /// Memory allocations during execution
    pub allocations: Option<usize>,
}

/// System profile data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemProfile {
    /// System name
    pub system_name: String,
    /// Performance metrics
    pub metrics: SystemMetrics,
    /// Sample timeline
    pub samples: Vec<ProfileSample>,
    /// System dependencies
    pub dependencies: Vec<String>,
}

/// Memory allocation information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationInfo {
    /// Size in bytes
    pub size: usize,
    /// Allocation site (function name)
    pub location: String,
    /// Backtrace if available
    pub backtrace: Option<Vec<String>>,
    /// Allocation count
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Error display
// ---------------------------------------------------------------------------

/// Formats a [`BrpError`] the way the old hand-rolled type did, for log lines.
///
/// A free function because `Display` cannot be implemented for the
/// re-exported `bevy_remote::BrpError` (orphan rule).
pub fn format_brp_error(error: &BrpError) -> String {
    format!("BRP error {}: {}", error.code, error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_query_filter_serialization() {
        let filter = QueryFilter {
            with: Some(vec!["Transform".to_string(), "Velocity".to_string()]),
            without: None,
            where_clause: None,
        };
        let json = serde_json::to_string(&filter).unwrap();
        let deserialized: QueryFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.with.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_debug_command_serialization() {
        let command = DebugCommand::InspectEntity {
            entity_id: 123,
            include_metadata: Some(true),
            include_relationships: None,
        };
        let json = serde_json::to_string(&command).unwrap();
        let _deserialized: DebugCommand = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_wire_request_round_trip() {
        // The real bevy_remote::BrpRequest is a JSON-RPC 2.0 envelope.
        let request = BrpRequest {
            method: "world.query".to_string(),
            id: Some(json!(7)),
            params: Some(json!({
                "data": { "components": ["bevy_transform::components::transform::Transform"] },
                "filter": { "with": [], "without": [] }
            })),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains(r#""jsonrpc":"2.0""#));
        assert!(json.contains(r#""method":"world.query""#));
        let deserialized: BrpRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.method, "world.query");
        assert_eq!(deserialized.id, Some(json!(7)));
    }
}
