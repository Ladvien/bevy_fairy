/*
 * bevy-fairy - Reflection-based Queries
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

//! Reflection-based Query System
//!
//! This module provides an enhanced query system that leverages Bevy's reflection
//! capabilities to enable more sophisticated entity and component queries.

#[cfg(feature = "bevy-reflection")]
use ::serde::{Deserialize, Serialize};
#[cfg(not(feature = "bevy-reflection"))]
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::bevy_reflection::inspector::{BevyReflectionInspector, ReflectionMetadata};
use crate::server::brp_messages::{BrpRequest, EntityData, ComponentValue};
use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};

/// Enhanced query system using reflection
#[derive(Clone)]
pub struct ReflectionQueryEngine {
    /// Reflection inspector for component analysis
    reflection_inspector: BevyReflectionInspector,
    /// Query cache for performance
    query_cache: Arc<RwLock<HashMap<String, CachedQueryResult>>>,
    /// Query statistics
    stats: Arc<RwLock<QueryStats>>,
}

/// Cached query result
#[derive(Debug, Clone)]
struct CachedQueryResult {
    result: ReflectionQueryResult,
    cached_at: u64,
    ttl_seconds: u64,
    hit_count: u64,
}

/// Statistics about query operations
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct QueryStats {
    pub total_queries: u64,
    pub cache_hits: u64,
    pub reflection_queries: u64,
    pub type_discovery_queries: u64,
    pub avg_execution_time_ms: f64,
    pub complex_queries: u64,
}

/// Reflection-enhanced query result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionQueryResult {
    /// Original query string
    pub query: String,
    /// Matching entities with reflected component data
    pub entities: Vec<ReflectedEntityData>,
    /// Query execution metadata
    pub metadata: QueryExecutionMetadata,
    /// Type information for discovered components
    pub discovered_types: HashMap<String, ReflectionMetadata>,
    /// Suggestions for query improvement
    pub suggestions: Vec<String>,
}

/// Entity data enhanced with reflection information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedEntityData {
    /// Base entity data
    pub entity: EntityData,
    /// Reflected component inspections
    pub reflected_components: HashMap<String, ReflectedComponentData>,
    /// Entity metadata derived from reflection
    pub entity_metadata: ReflectedEntityMetadata,
}

/// Component data enhanced with reflection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedComponentData {
    /// Component type name
    pub type_name: String,
    /// Raw component value
    pub raw_value: ComponentValue,
    /// Reflection inspection result
    pub inspection: crate::server::bevy_reflection::ReflectionInspectionResult,
    /// Component relationships (if any)
    pub relationships: Option<ComponentRelationships>,
    /// Performance metrics
    pub metrics: ComponentMetrics,
}

/// Metadata about an entity derived from reflection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectedEntityMetadata {
    /// Archetype signature (component types)
    pub archetype_signature: Vec<String>,
    /// Estimated memory usage
    pub memory_usage_bytes: usize,
    /// Component count
    pub component_count: usize,
    /// Whether entity has hierarchical relationships
    pub has_hierarchy: bool,
    /// Whether entity has spatial components
    pub has_spatial: bool,
    /// Whether entity has rendering components
    pub has_rendering: bool,
    /// Custom entity classifications
    pub classifications: Vec<String>,
}

/// Component relationship information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentRelationships {
    /// Components that depend on this one
    pub dependents: Vec<String>,
    /// Components this one depends on
    pub dependencies: Vec<String>,
    /// Mutually exclusive components
    pub conflicts: Vec<String>,
}

/// Performance metrics for components
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentMetrics {
    /// Serialization size in bytes
    pub serialized_size: usize,
    /// Inspection time in microseconds
    pub inspection_time_us: u64,
    /// Complexity score (0.0 to 1.0)
    pub complexity_score: f64,
}

/// Metadata about query execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExecutionMetadata {
    /// Execution time in milliseconds
    pub execution_time_ms: u64,
    /// Whether result was cached
    pub from_cache: bool,
    /// Number of entities processed
    pub entities_processed: usize,
    /// Number of components inspected
    pub components_inspected: usize,
    /// Types discovered during query
    pub types_discovered: usize,
    /// Query complexity score
    pub complexity_score: f64,
}

/// Enhanced query parameters
#[derive(Debug, Clone)]
pub struct ReflectionQuery {
    /// Base entity query (similar to existing system)
    pub base_query: String,
    /// Reflection-specific parameters
    pub reflection_params: ReflectionQueryParams,
    /// Performance limits
    pub limits: QueryLimits,
}

/// Reflection-specific query parameters
#[derive(Debug, Clone)]
pub struct ReflectionQueryParams {
    /// Include reflection inspection for all components
    pub include_reflection: bool,
    /// Perform deep inspection of complex types
    pub deep_inspection: bool,
    /// Include type discovery information
    pub include_type_discovery: bool,
    /// Include component relationships
    pub include_relationships: bool,
    /// Include performance metrics
    pub include_metrics: bool,
    /// Filter by component type patterns
    pub component_type_filters: Vec<String>,
    /// Filter by field value patterns
    pub field_value_filters: Vec<FieldValueFilter>,
}

/// Field value filter for reflection queries
#[derive(Debug, Clone)]
pub struct FieldValueFilter {
    /// Component type to filter
    pub component_type: String,
    /// Field path (e.g., "translation.x")
    pub field_path: String,
    /// Filter operation
    pub operation: FilterOperation,
    /// Expected value
    pub value: Value,
}

/// Filter operations for field values
#[derive(Debug, Clone)]
pub enum FilterOperation {
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
    StartsWith,
    EndsWith,
    Exists,
    NotExists,
}

/// Query performance limits
#[derive(Debug, Clone)]
pub struct QueryLimits {
    /// Maximum entities to process
    pub max_entities: usize,
    /// Maximum execution time in milliseconds
    pub max_execution_time_ms: u64,
    /// Maximum components to inspect per entity
    pub max_components_per_entity: usize,
    /// Maximum memory usage in bytes
    pub max_memory_usage_bytes: usize,
}

impl ReflectionQueryEngine {
    /// Create a new reflection query engine
    pub fn new() -> Self {
        Self {
            reflection_inspector: BevyReflectionInspector::new(),
            query_cache: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(QueryStats::default())),
        }
    }

    /// Execute a reflection-enhanced query
    pub async fn execute_query(
        &self,
        query: ReflectionQuery,
        brp_client: Arc<RwLock<BrpClient>>,
    ) -> Result<ReflectionQueryResult> {
        let start_time = std::time::Instant::now();
        debug!("Executing reflection query: {}", query.base_query);

        // Check cache first
        if let Some(cached_result) = self.get_cached_result(&query.base_query).await {
            info!("Query cache hit for: {}", query.base_query);
            self.update_stats(true, start_time.elapsed().as_millis() as u64).await;
            return Ok(cached_result);
        }

        // Execute base query through BRP
        let base_entities = self.execute_base_query(&query.base_query, brp_client).await?;
        
        // Enhance with reflection data
        let reflected_entities = self.enhance_entities_with_reflection(
            base_entities,
            &query.reflection_params,
            &query.limits,
        ).await?;

        // Discover types if requested
        let discovered_types = if query.reflection_params.include_type_discovery {
            self.discover_types_from_entities(&reflected_entities).await?
        } else {
            HashMap::new()
        };

        // Generate suggestions
        let suggestions = self.generate_query_suggestions(&query, &reflected_entities).await;

        let execution_time = start_time.elapsed().as_millis() as u64;
        let result = ReflectionQueryResult {
            query: query.base_query.clone(),
            entities: reflected_entities.clone(),
            metadata: QueryExecutionMetadata {
                execution_time_ms: execution_time,
                from_cache: false,
                entities_processed: reflected_entities.len(),
                components_inspected: reflected_entities.iter()
                    .map(|e| e.reflected_components.len())
                    .sum(),
                types_discovered: discovered_types.len(),
                complexity_score: self.calculate_query_complexity(&query).await,
            },
            discovered_types,
            suggestions,
        };

        // Cache the result
        self.cache_result(&query.base_query, &result).await;

        // Update statistics
        self.update_stats(false, execution_time).await;

        Ok(result)
    }

    /// Execute base query through BRP client
    async fn execute_base_query(
        &self,
        query: &str,
        brp_client: Arc<RwLock<BrpClient>>,
    ) -> Result<Vec<EntityData>> {
        // This would integrate with the existing query parser
        // For now, we'll simulate a basic query execution
        let mut client = brp_client.write().await;
        
        // Parse the query and convert to BRP request
        let brp_request = self.parse_query_to_brp(query)?;
        
        let response = client.send_request(&brp_request).await?;
        
        // Extract entities from response
        match response.payload {
            crate::server::brp_messages::BrpPayload::Result(value) => {
                Ok(crate::server::brp_messages::entity_data_from_query_result(&value)
                    .unwrap_or_default())
            }
            crate::server::brp_messages::BrpPayload::Error(err) => {
                Err(Error::Brp(format!("BRP query failed: {}", err.message)))
            }
        }
    }

    /// Parse query string to BRP request (simplified)
    fn parse_query_to_brp(&self, query: &str) -> Result<BrpRequest> {
        // This is a simplified implementation; all natural-language variants
        // fall to a full world.query with no filter.
        let _ = query;
        Ok(BrpRequest {
            method: crate::server::brp_messages::builtin_methods::BRP_QUERY_METHOD.to_string(),
            id: None,
            params: Some(crate::server::brp_messages::query_params(None, true)),
        })
    }

    /// Enhance entities with reflection data
    async fn enhance_entities_with_reflection(
        &self,
        entities: Vec<EntityData>,
        params: &ReflectionQueryParams,
        limits: &QueryLimits,
    ) -> Result<Vec<ReflectedEntityData>> {
        let mut reflected_entities = Vec::new();

        for (processed_count, entity) in entities.into_iter().enumerate() {
            if processed_count >= limits.max_entities {
                warn!("Query limit reached: max_entities = {}", limits.max_entities);
                break;
            }

            let reflected_entity = self.enhance_single_entity(entity, params, limits).await?;
            reflected_entities.push(reflected_entity);
        }

        Ok(reflected_entities)
    }

    /// Enhance a single entity with reflection data
    async fn enhance_single_entity(
        &self,
        entity: EntityData,
        params: &ReflectionQueryParams,
        limits: &QueryLimits,
    ) -> Result<ReflectedEntityData> {
        let mut reflected_components = HashMap::new();
        let mut component_count = 0;

        for (component_type, component_value) in &entity.components {
            if component_count >= limits.max_components_per_entity {
                break;
            }

            // Apply component type filters
            if !params.component_type_filters.is_empty() {
                let matches_filter = params.component_type_filters.iter()
                    .any(|filter| component_type.contains(filter));
                if !matches_filter {
                    continue;
                }
            }

            let start_inspection = std::time::Instant::now();
            
            // Perform reflection inspection if requested
            let inspection = if params.include_reflection {
                Some(self.reflection_inspector.inspect_component(component_type, component_value).await?)
            } else {
                None
            };

            let inspection_time = start_inspection.elapsed().as_micros() as u64;

            // Calculate metrics
            let metrics = ComponentMetrics {
                serialized_size: serde_json::to_string(component_value)
                    .map(|s| s.len())
                    .unwrap_or(0),
                inspection_time_us: inspection_time,
                complexity_score: self.calculate_component_complexity(component_value).await,
            };

            // Build relationships if requested
            let relationships = if params.include_relationships {
                Some(self.analyze_component_relationships(component_type, &entity).await)
            } else {
                None
            };

            if let Some(inspection) = inspection {
                reflected_components.insert(component_type.clone(), ReflectedComponentData {
                    type_name: component_type.clone(),
                    raw_value: component_value.clone(),
                    inspection,
                    relationships,
                    metrics,
                });
            }

            component_count += 1;
        }

        // Build entity metadata
        let entity_metadata = self.build_entity_metadata(&entity, &reflected_components).await;

        Ok(ReflectedEntityData {
            entity,
            reflected_components,
            entity_metadata,
        })
    }

    /// Analyze component relationships
    async fn analyze_component_relationships(
        &self,
        component_type: &str,
        entity: &EntityData,
    ) -> ComponentRelationships {
        let mut dependents = Vec::new();
        let dependencies = Vec::new();
        let conflicts = Vec::new();

        // Analyze common Bevy component relationships
        match component_type {
            "bevy_transform::components::transform::Transform" => {
                // GlobalTransform depends on Transform
                if entity.components.contains_key("bevy_transform::components::global_transform::GlobalTransform") {
                    dependents.push("bevy_transform::components::global_transform::GlobalTransform".to_string());
                }
                // Many rendering components depend on Transform
                for comp_type in entity.components.keys() {
                    if comp_type.contains("sprite") || comp_type.contains("mesh") {
                        dependents.push(comp_type.clone());
                    }
                }
            }
            "bevy_hierarchy::components::parent::Parent" => {}
            _ => {}
        }

        ComponentRelationships {
            dependents,
            dependencies,
            conflicts,
        }
    }

    /// Build entity metadata from reflection analysis
    async fn build_entity_metadata(
        &self,
        entity: &EntityData,
        reflected_components: &HashMap<String, ReflectedComponentData>,
    ) -> ReflectedEntityMetadata {
        let archetype_signature: Vec<String> = entity.components.keys().cloned().collect();
        
        let memory_usage_bytes = reflected_components.values()
            .map(|comp| comp.metrics.serialized_size)
            .sum();

        let has_hierarchy = entity.components.keys()
            .any(|k| k.contains("parent") || k.contains("children") || k.contains("Parent") || k.contains("Children"));

        let has_spatial = entity.components.keys()
            .any(|k| k.contains("transform") || k.contains("Transform"));

        let has_rendering = entity.components.keys()
            .any(|k| k.contains("sprite") || k.contains("mesh") || k.contains("material"));

        let mut classifications = Vec::new();
        if has_hierarchy { classifications.push("Hierarchical".to_string()); }
        if has_spatial { classifications.push("Spatial".to_string()); }
        if has_rendering { classifications.push("Renderable".to_string()); }

        ReflectedEntityMetadata {
            archetype_signature,
            memory_usage_bytes,
            component_count: entity.components.len(),
            has_hierarchy,
            has_spatial,
            has_rendering,
            classifications,
        }
    }

    /// Calculate component complexity score
    async fn calculate_component_complexity(&self, component_value: &ComponentValue) -> f64 {
        let mut complexity;

        match component_value {
            Value::Null => complexity = 0.1,
            Value::Bool(_) => complexity = 0.2,
            Value::Number(_) => complexity = 0.3,
            Value::String(s) => complexity = 0.4 + (s.len() as f64 / 1000.0).min(0.3),
            Value::Array(arr) => {
                complexity = 0.6 + (arr.len() as f64 / 100.0).min(0.3);
                // Add complexity for nested structures
                for item in arr.iter().take(10) { // Sample first 10 items
                    complexity += Box::pin(self.calculate_component_complexity(item)).await * 0.1;
                }
            }
            Value::Object(obj) => {
                complexity = 0.7 + (obj.len() as f64 / 50.0).min(0.2);
                // Add complexity for nested structures
                for value in obj.values().take(10) { // Sample first 10 fields
                    complexity += Box::pin(self.calculate_component_complexity(value)).await * 0.1;
                }
            }
        }

        complexity.clamp(0.0_f64, 1.0_f64)
    }

    /// Discover types from entities
    async fn discover_types_from_entities(
        &self,
        entities: &[ReflectedEntityData],
    ) -> Result<HashMap<String, ReflectionMetadata>> {
        let mut discovered_types = HashMap::new();

        for entity in entities {
            for comp_data in entity.reflected_components.values() {
                if !discovered_types.contains_key(&comp_data.type_name) {
                    discovered_types.insert(
                        comp_data.type_name.clone(),
                        comp_data.inspection.metadata.clone(),
                    );
                }
            }
        }

        Ok(discovered_types)
    }

    /// Generate query improvement suggestions
    async fn generate_query_suggestions(
        &self,
        query: &ReflectionQuery,
        entities: &[ReflectedEntityData],
    ) -> Vec<String> {
        let mut suggestions = Vec::new();

        // Analyze query patterns
        if entities.is_empty() {
            suggestions.push("No entities matched your query. Try broadening the search criteria.".to_string());
        } else if entities.len() > 100 {
            suggestions.push("Large result set returned. Consider adding filters to narrow results.".to_string());
        }

        // Suggest reflection enhancements
        if !query.reflection_params.include_reflection {
            suggestions.push("Enable reflection inspection to get detailed component analysis.".to_string());
        }

        // Suggest type discovery
        if !query.reflection_params.include_type_discovery {
            suggestions.push("Enable type discovery to learn about available component types.".to_string());
        }

        suggestions
    }

    /// Calculate query complexity score
    async fn calculate_query_complexity(&self, query: &ReflectionQuery) -> f64 {
        let mut complexity = 0.3_f64; // Base complexity

        if query.reflection_params.include_reflection {
            complexity += 0.3;
        }
        if query.reflection_params.deep_inspection {
            complexity += 0.2;
        }
        if query.reflection_params.include_relationships {
            complexity += 0.2;
        }

        complexity.clamp(0.0_f64, 1.0_f64)
    }

    /// Get cached query result
    async fn get_cached_result(&self, query: &str) -> Option<ReflectionQueryResult> {
        let mut cache = self.query_cache.write().await;
        if let Some(cached) = cache.get_mut(query) {
            let now = chrono::Utc::now().timestamp_micros() as u64;
            if now - cached.cached_at < cached.ttl_seconds * 1_000_000 {
                cached.hit_count += 1;
                return Some(cached.result.clone());
            } else {
                cache.remove(query);
            }
        }
        None
    }

    /// Cache query result
    async fn cache_result(&self, query: &str, result: &ReflectionQueryResult) {
        let mut cache = self.query_cache.write().await;
        
        // Clean old entries if cache is getting large
        if cache.len() > 1000 {
            let cutoff = chrono::Utc::now().timestamp_micros() as u64 - 3600 * 1_000_000; // 1 hour
            cache.retain(|_, cached| cached.cached_at > cutoff);
        }

        cache.insert(query.to_string(), CachedQueryResult {
            result: result.clone(),
            cached_at: chrono::Utc::now().timestamp_micros() as u64,
            ttl_seconds: 300, // 5 minutes
            hit_count: 0,
        });
    }

    /// Update query statistics
    async fn update_stats(&self, cache_hit: bool, execution_time_ms: u64) {
        let mut stats = self.stats.write().await;
        stats.total_queries += 1;
        
        if cache_hit {
            stats.cache_hits += 1;
        }

        // Update rolling average execution time
        let total_time = stats.avg_execution_time_ms * (stats.total_queries - 1) as f64;
        stats.avg_execution_time_ms = (total_time + execution_time_ms as f64) / stats.total_queries as f64;
    }

    /// Get query statistics
    pub async fn get_stats(&self) -> QueryStats {
        let stats = self.stats.read().await;
        (*stats).clone()
    }

    /// Clear query cache
    pub async fn clear_cache(&self) {
        let mut cache = self.query_cache.write().await;
        cache.clear();
        info!("Cleared reflection query cache");
    }
}

impl Default for ReflectionQueryEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ReflectionQueryParams {
    fn default() -> Self {
        Self {
            include_reflection: true,
            deep_inspection: false,
            include_type_discovery: false,
            include_relationships: false,
            include_metrics: true,
            component_type_filters: Vec::new(),
            field_value_filters: Vec::new(),
        }
    }
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_entities: 1000,
            max_execution_time_ms: 30000, // 30 seconds
            max_components_per_entity: 50,
            max_memory_usage_bytes: 100 * 1024 * 1024, // 100MB
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_reflection_query_engine_creation() {
        let engine = ReflectionQueryEngine::new();
        let stats = engine.get_stats().await;
        
        assert_eq!(stats.total_queries, 0);
        assert_eq!(stats.cache_hits, 0);
    }

    #[tokio::test]
    async fn test_component_complexity_calculation() {
        let engine = ReflectionQueryEngine::new();
        
        // Simple values
        assert!(engine.calculate_component_complexity(&json!(42)).await < 0.5);
        assert!(engine.calculate_component_complexity(&json!("hello")).await < 0.5);
        
        // Complex object
        let complex_obj = json!({
            "field1": "value1",
            "field2": {"nested": "value"},
            "field3": [1, 2, 3, 4, 5]
        });
        assert!(engine.calculate_component_complexity(&complex_obj).await > 0.7);
    }

    #[test]
    fn test_filter_operation() {
        // Test that filter operations can be created
        let filter = FieldValueFilter {
            component_type: "Transform".to_string(),
            field_path: "translation.x".to_string(),
            operation: FilterOperation::GreaterThan,
            value: json!(10.0),
        };
        
        assert_eq!(filter.component_type, "Transform");
        assert_eq!(filter.field_path, "translation.x");
    }

    #[test]
    fn test_query_limits() {
        let limits = QueryLimits::default();
        
        assert_eq!(limits.max_entities, 1000);
        assert_eq!(limits.max_execution_time_ms, 30000);
    }
}