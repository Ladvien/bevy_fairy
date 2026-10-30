/*
 * bevy-fairy - TypeRegistry Tools
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

//! TypeRegistry Integration Tools
//!
//! This module provides utilities for working with Bevy's TypeRegistry
//! to enable dynamic component discovery and reflection-based operations.

#[cfg(feature = "bevy-reflection")]
use bevy::prelude::*;
#[cfg(feature = "bevy-reflection")]
use bevy::reflect::*;

#[cfg(feature = "bevy-reflection")]
use ::serde::{Deserialize, Serialize};
#[cfg(not(feature = "bevy-reflection"))]
use serde::{Deserialize, Serialize};
#[cfg(feature = "bevy-reflection")]
use serde_json::{json, Value};
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::server::bevy_reflection::inspector::{ReflectionMetadata, TypeCategory};
#[cfg(feature = "bevy-reflection")]
use crate::server::bevy_reflection::inspector::FieldMetadata;
use crate::server::error::Result;

/// TypeRegistry manager for dynamic component type discovery
#[derive(Clone)]
pub struct TypeRegistryManager {
    /// Cached type information indexed by type name
    type_cache: Arc<RwLock<HashMap<String, CachedTypeInfo>>>,
    /// Cached type information indexed by TypeId  
    type_id_cache: Arc<RwLock<HashMap<TypeId, String>>>,
    /// Statistics about type discovery
    stats: Arc<RwLock<TypeDiscoveryStats>>,
}

/// Cached information about a reflected type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTypeInfo {
    /// Basic reflection metadata
    pub metadata: ReflectionMetadata,
    /// Whether this type can be constructed via reflection
    pub constructible: bool,
    /// Whether this type supports serialization
    pub serializable: bool,
    /// Type aliases and alternative names
    pub aliases: Vec<String>,
    /// Parent types (if this extends another type)
    pub parent_types: Vec<String>,
    /// Child types that extend this type
    pub child_types: Vec<String>,
    /// Last time this type info was updated
    pub last_updated: u64,
    /// Cache hit count for performance monitoring
    pub hit_count: u64,
}

/// Statistics about type discovery operations
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TypeDiscoveryStats {
    /// Total types discovered
    pub total_types: usize,
    /// Types with reflection support
    pub reflected_types: usize,
    /// Types with custom inspectors
    pub custom_inspector_types: usize,
    /// Cache hit rate
    pub cache_hit_rate: f64,
    /// Last discovery time
    pub last_discovery: Option<u64>,
    /// Discovery time in milliseconds
    pub discovery_time_ms: u64,
}

/// Result of a type query operation
#[derive(Debug, Serialize, Deserialize)]
pub struct TypeQueryResult {
    /// Matching types
    pub types: Vec<TypeQueryMatch>,
    /// Query execution time
    pub execution_time_ms: u64,
    /// Total matches found
    pub total_matches: usize,
    /// Whether results were truncated
    pub truncated: bool,
}

/// A single type match from a query
#[derive(Debug, Serialize, Deserialize)]
pub struct TypeQueryMatch {
    /// Type name
    pub type_name: String,
    /// Type metadata
    pub metadata: ReflectionMetadata,
    /// Match score (0.0 to 1.0)
    pub relevance_score: f64,
    /// Reasons why this type matched
    pub match_reasons: Vec<String>,
}

impl TypeRegistryManager {
    /// Create a new TypeRegistry manager
    pub fn new() -> Self {
        Self {
            type_cache: Arc::new(RwLock::new(HashMap::new())),
            type_id_cache: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(TypeDiscoveryStats::default())),
        }
    }

    /// Discover types from Bevy's TypeRegistry
    #[cfg(feature = "bevy-reflection")]
    pub async fn discover_types_from_registry(&self, type_registry: &TypeRegistry) -> Result<usize> {
        let start_time = std::time::Instant::now();
        debug!("Starting type discovery from TypeRegistry");

        let mut discovered_count = 0;
        let mut reflected_count = 0;

        for registration in type_registry.iter() {
            let type_info = registration.type_info();
            let type_name = type_info.type_path().to_string();
            let type_id = registration.type_id();

            // Build cached type info
            let cached_info = self.build_cached_type_info(registration).await?;
            let is_reflected = cached_info.metadata.is_reflected;
            
            // Store in caches
            {
                let mut cache = self.type_cache.write().await;
                cache.insert(type_name.clone(), cached_info);
            }
            
            {
                let mut id_cache = self.type_id_cache.write().await;
                id_cache.insert(type_id, type_name.clone());
            }

            discovered_count += 1;
            if is_reflected {
                reflected_count += 1;
            }

            debug!("Discovered type: {} (reflected: {})", type_name, is_reflected);
        }

        // Update statistics
        let discovery_time = start_time.elapsed().as_millis() as u64;
        {
            let mut stats = self.stats.write().await;
            stats.total_types = discovered_count;
            stats.reflected_types = reflected_count;
            stats.last_discovery = Some(chrono::Utc::now().timestamp_micros() as u64);
            stats.discovery_time_ms = discovery_time;
        }

        info!(
            "Type discovery complete: {} types discovered ({} reflected) in {}ms",
            discovered_count, reflected_count, discovery_time
        );

        Ok(discovered_count)
    }

    /// Build cached type information from TypeRegistration
    #[cfg(feature = "bevy-reflection")]
    async fn build_cached_type_info(&self, registration: &TypeRegistration) -> Result<CachedTypeInfo> {
        let type_info = registration.type_info();
        let type_name = type_info.type_path().to_string();
        let type_id = registration.type_id();

        // Build reflection metadata
        let metadata = self.build_reflection_metadata(registration).await?;

        // Check capabilities
        let constructible = registration.data::<ReflectDefault>().is_some() 
            || registration.data::<ReflectFromPtr>().is_some();
        let serializable = registration.data::<ReflectSerialize>().is_some()
            || registration.data::<ReflectDeserialize>().is_some();

        // Find aliases (simplified - in real implementation would check type aliases)
        let aliases = self.find_type_aliases(&type_name).await;

        Ok(CachedTypeInfo {
            metadata,
            constructible,
            serializable,
            aliases,
            parent_types: Vec::new(), // Would be populated by analyzing inheritance
            child_types: Vec::new(),  // Would be populated by analyzing inheritance
            last_updated: chrono::Utc::now().timestamp_micros() as u64,
            hit_count: 0,
        })
    }

    /// Build reflection metadata from TypeRegistration
    #[cfg(feature = "bevy-reflection")]
    async fn build_reflection_metadata(&self, registration: &TypeRegistration) -> Result<ReflectionMetadata> {
        let type_info = registration.type_info();
        let type_name = type_info.type_path().to_string();
        let type_id = registration.type_id();

        let (type_category, fields) = match type_info {
            TypeInfo::Struct(struct_info) => {
                let mut field_metadata = Vec::new();
                for (index, field) in struct_info.iter().enumerate() {
                    field_metadata.push(FieldMetadata {
                        name: field.name().to_string(),
                        type_name: field.type_path().to_string(),
                        index,
                        is_reflected: true, // If it's in struct_info, it should be reflected
                        inspector_name: None, // Would be populated by checking custom inspectors
                    });
                }
                (TypeCategory::Struct, field_metadata)
            }
            TypeInfo::TupleStruct(tuple_info) => {
                let mut field_metadata = Vec::new();
                for (index, field) in tuple_info.iter().enumerate() {
                    field_metadata.push(FieldMetadata {
                        name: format!("field_{}", index),
                        type_name: field.type_path().to_string(),
                        index,
                        is_reflected: true,
                        inspector_name: None,
                    });
                }
                (TypeCategory::TupleStruct, field_metadata)
            }
            TypeInfo::Enum(_) => (TypeCategory::Enum, Vec::new()),
            TypeInfo::Array(_) => (TypeCategory::Array, Vec::new()),
            TypeInfo::List(_) => (TypeCategory::List, Vec::new()),
            TypeInfo::Map(_) => (TypeCategory::Map, Vec::new()),
            TypeInfo::Set(_) => (TypeCategory::Map, Vec::new()),
            TypeInfo::Tuple(_) => (TypeCategory::TupleStruct, Vec::new()),
            TypeInfo::Opaque(_) => (TypeCategory::Value, Vec::new()),
        };

        Ok(ReflectionMetadata {
            type_name: type_name.clone(),
            type_id,
            is_reflected: true,
            fields,
            type_category,
            type_info: Some(self.serialize_type_info(type_info).await),
            last_updated: chrono::Utc::now().timestamp_micros() as u64,
        })
    }

    /// Serialize TypeInfo to JSON
    #[cfg(feature = "bevy-reflection")]
    async fn serialize_type_info(&self, type_info: &TypeInfo) -> Value {
        match type_info {
            TypeInfo::Struct(struct_info) => {
                json!({
                    "type": "struct",
                    "name": struct_info.type_path(),
                    "field_count": struct_info.field_len(),
                    "fields": struct_info.iter().map(|field| {
                        json!({
                            "name": field.name(),
                            "type": field.type_path(),
                        })
                    }).collect::<Vec<_>>()
                })
            }
            TypeInfo::TupleStruct(tuple_info) => {
                json!({
                    "type": "tuple_struct",
                    "name": tuple_info.type_path(),
                    "field_count": tuple_info.field_len(),
                })
            }
            TypeInfo::Enum(enum_info) => {
                json!({
                    "type": "enum",
                    "name": enum_info.type_path(),
                    "variant_count": enum_info.variant_len(),
                    "variants": enum_info.iter().map(|variant| {
                        json!({
                            "name": variant.name(),
                        })
                    }).collect::<Vec<_>>()
                })
            }
            TypeInfo::Array(array_info) => {
                json!({
                    "type": "array",
                    "name": array_info.type_path(),
                    "capacity": array_info.capacity(),
                    "item_type": array_info.item_info().map(|i| i.type_path()),
                })
            }
            TypeInfo::List(list_info) => {
                json!({
                    "type": "list",
                    "name": list_info.type_path(),
                    "item_type": list_info.item_info().map(|i| i.type_path()),
                })
            }
            TypeInfo::Map(map_info) => {
                json!({
                    "type": "map",
                    "name": map_info.type_path(),
                    "key_type": map_info.key_info().map(|i| i.type_path()),
                    "value_type": map_info.value_info().map(|i| i.type_path()),
                })
            }
            TypeInfo::Set(set_info) => {
                json!({
                    "type": "set",
                    "name": set_info.type_path(),
                })
            }
            TypeInfo::Tuple(tuple_info) => {
                json!({
                    "type": "tuple",
                    "name": tuple_info.type_path(),
                    "field_count": tuple_info.field_len(),
                })
            }
            TypeInfo::Opaque(opaque_info) => {
                json!({
                    "type": "opaque",
                    "name": opaque_info.type_path(),
                })
            }
        }
    }

    /// Find aliases for a type name
    #[cfg(feature = "bevy-reflection")]
    async fn find_type_aliases(&self, type_name: &str) -> Vec<String> {
        let mut aliases = Vec::new();

        // Extract short name from full path
        if let Some(short_name) = type_name.split("::").last() {
            if short_name != type_name {
                aliases.push(short_name.to_string());
            }
        }

        // Add common Bevy type aliases
        match type_name {
            "bevy_transform::components::transform::Transform" => {
                aliases.extend(vec!["Transform".to_string(), "transform".to_string()]);
            }
            "bevy_transform::components::global_transform::GlobalTransform" => {
                aliases.extend(vec!["GlobalTransform".to_string(), "global_transform".to_string()]);
            }
            "bevy_core::name::Name" => {
                aliases.extend(vec!["Name".to_string(), "name".to_string()]);
            }
            _ => {}
        }

        aliases
    }

    /// Query types by name pattern or characteristics
    pub async fn query_types(&self, query: &TypeQuery) -> Result<TypeQueryResult> {
        let start_time = std::time::Instant::now();
        debug!("Executing type query: {:?}", query);

        let cache = self.type_cache.read().await;
        let mut matches = Vec::new();

        for (type_name, cached_info) in cache.iter() {
            let score = self.calculate_match_score(type_name, cached_info, query).await;
            if score > query.min_score {
                matches.push(TypeQueryMatch {
                    type_name: type_name.clone(),
                    metadata: cached_info.metadata.clone(),
                    relevance_score: score,
                    match_reasons: self.get_match_reasons(type_name, cached_info, query).await,
                });
            }
        }

        // Sort by relevance score (descending)
        matches.sort_by(|a, b| b.relevance_score.partial_cmp(&a.relevance_score).unwrap_or(std::cmp::Ordering::Equal));

        // Apply limit
        let truncated = matches.len() > query.limit;
        matches.truncate(query.limit);

        let execution_time = start_time.elapsed().as_millis() as u64;

        Ok(TypeQueryResult {
            total_matches: matches.len(),
            types: matches,
            execution_time_ms: execution_time,
            truncated,
        })
    }

    /// Calculate match score for a type against query
    async fn calculate_match_score(&self, type_name: &str, cached_info: &CachedTypeInfo, query: &TypeQuery) -> f64 {
        let mut score = 0.0;

        // Name matching
        if let Some(name_pattern) = &query.name_pattern {
            if type_name.to_lowercase().contains(&name_pattern.to_lowercase()) {
                score += 0.8;
                if type_name.to_lowercase() == name_pattern.to_lowercase() {
                    score += 0.2; // Exact match bonus
                }
            }
            
            // Check aliases
            for alias in &cached_info.aliases {
                if alias.to_lowercase().contains(&name_pattern.to_lowercase()) {
                    score += 0.6;
                }
            }
        }

        // Category matching
        if let Some(category) = &query.category {
            if std::mem::discriminant(&cached_info.metadata.type_category) == std::mem::discriminant(category) {
                score += 0.5;
            }
        }

        // Reflection requirement
        if query.requires_reflection && !cached_info.metadata.is_reflected {
            score = 0.0; // Hard requirement
        }

        // Field requirements
        if let Some(required_fields) = &query.required_fields {
            let field_names: Vec<String> = cached_info.metadata.fields.iter().map(|f| f.name.clone()).collect();
            let matching_fields = required_fields.iter().filter(|f| field_names.contains(f)).count();
            let field_score = matching_fields as f64 / required_fields.len() as f64;
            score += field_score * 0.3;
        }

        // Constructibility requirement
        if query.requires_constructible && !cached_info.constructible {
            score *= 0.5; // Soft penalty
        }

        score.clamp(0.0, 1.0)
    }

    /// Get reasons why a type matched the query
    async fn get_match_reasons(&self, type_name: &str, cached_info: &CachedTypeInfo, query: &TypeQuery) -> Vec<String> {
        let mut reasons = Vec::new();

        if let Some(name_pattern) = &query.name_pattern {
            if type_name.to_lowercase().contains(&name_pattern.to_lowercase()) {
                reasons.push(format!("Name contains '{}'", name_pattern));
            }
            
            for alias in &cached_info.aliases {
                if alias.to_lowercase().contains(&name_pattern.to_lowercase()) {
                    reasons.push(format!("Alias '{}' contains '{}'", alias, name_pattern));
                }
            }
        }

        if let Some(category) = &query.category {
            if std::mem::discriminant(&cached_info.metadata.type_category) == std::mem::discriminant(category) {
                reasons.push(format!("Type category matches: {:?}", category));
            }
        }

        if cached_info.metadata.is_reflected {
            reasons.push("Type supports reflection".to_string());
        }

        if cached_info.constructible {
            reasons.push("Type is constructible".to_string());
        }

        reasons
    }

    /// Get type information by name
    pub async fn get_type_info(&self, type_name: &str) -> Option<CachedTypeInfo> {
        let mut cache = self.type_cache.write().await;
        if let Some(info) = cache.get_mut(type_name) {
            info.hit_count += 1;
            Some(info.clone())
        } else {
            None
        }
    }

    /// Get type name by TypeId
    pub async fn get_type_name_by_id(&self, type_id: &TypeId) -> Option<String> {
        let cache = self.type_id_cache.read().await;
        cache.get(type_id).cloned()
    }

    /// Get all cached type names
    pub async fn get_all_type_names(&self) -> Vec<String> {
        let cache = self.type_cache.read().await;
        cache.keys().cloned().collect()
    }

    /// Get discovery statistics
    pub async fn get_stats(&self) -> TypeDiscoveryStats {
        let stats = self.stats.read().await;
        (*stats).clone()
    }

    /// Clear all caches
    pub async fn clear_cache(&self) {
        {
            let mut cache = self.type_cache.write().await;
            cache.clear();
        }
        {
            let mut id_cache = self.type_id_cache.write().await;
            id_cache.clear();
        }
        {
            let mut stats = self.stats.write().await;
            *stats = TypeDiscoveryStats::default();
        }
        info!("Cleared TypeRegistry caches");
    }
}

impl Default for TypeRegistryManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Query parameters for type discovery
#[derive(Debug, Clone)]
pub struct TypeQuery {
    /// Pattern to match in type names
    pub name_pattern: Option<String>,
    /// Required type category
    pub category: Option<TypeCategory>,
    /// Minimum match score (0.0 to 1.0)
    pub min_score: f64,
    /// Maximum number of results
    pub limit: usize,
    /// Require reflection support
    pub requires_reflection: bool,
    /// Require constructibility
    pub requires_constructible: bool,
    /// Required field names
    pub required_fields: Option<Vec<String>>,
}

impl Default for TypeQuery {
    fn default() -> Self {
        Self {
            name_pattern: None,
            category: None,
            min_score: 0.1,
            limit: 100,
            requires_reflection: false,
            requires_constructible: false,
            required_fields: None,
        }
    }
}

/// Utility functions for working with type paths
pub mod type_path_utils {
    /// Extract short name from full type path
    pub fn extract_short_name(type_path: &str) -> &str {
        type_path.split("::").last().unwrap_or(type_path)
    }

    /// Check if a type path is a Bevy core type
    pub fn is_bevy_core_type(type_path: &str) -> bool {
        type_path.starts_with("bevy_") || type_path.contains("::bevy_")
    }

    /// Check if a type path represents a generic type
    pub fn is_generic_type(type_path: &str) -> bool {
        type_path.contains('<') && type_path.contains('>')
    }

    /// Extract generic parameters from a type path
    pub fn extract_generic_params(type_path: &str) -> Vec<String> {
        if let Some(start) = type_path.find('<') {
            if let Some(end) = type_path.rfind('>') {
                let params_str = &type_path[start + 1..end];
                return params_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_type_registry_manager_creation() {
        let manager = TypeRegistryManager::new();
        let stats = manager.get_stats().await;
        
        assert_eq!(stats.total_types, 0);
        assert_eq!(stats.reflected_types, 0);
    }

    #[tokio::test]
    async fn test_type_query_default() {
        let query = TypeQuery::default();
        
        assert_eq!(query.min_score, 0.1);
        assert_eq!(query.limit, 100);
        assert!(!query.requires_reflection);
    }

    #[test]
    fn test_type_path_utils() {
        use type_path_utils::*;

        assert_eq!(extract_short_name("bevy_transform::components::transform::Transform"), "Transform");
        assert!(is_bevy_core_type("bevy_core::name::Name"));
        assert!(!is_bevy_core_type("my_game::MyComponent"));
        assert!(is_generic_type("Option<String>"));
        assert!(!is_generic_type("Transform"));
        
        let params = extract_generic_params("HashMap<String, i32>");
        assert_eq!(params, vec!["String", "i32"]);
    }
}