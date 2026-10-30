/*
 * bevy-fairy - Bevy Reflection Inspector
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

//! # Bevy Reflection Integration
//!
//! This module integrates Bevy's reflection system for dynamic component inspection,
//! type discovery, and advanced debugging capabilities. It provides runtime introspection
//! of component types using Bevy's TypeRegistry and Reflect trait.
//!
//! ## Key Features
//!
//! - **TypeRegistry Integration**: Discover all registered component types dynamically
//! - **Reflection-based Queries**: Query components using reflection metadata
//! - **Complex Type Handling**: Support for `Option<T>`, `Vec<T>`, `HashMap<K,V>`, etc.
//! - **Custom Inspectors**: Extensible inspector system for complex types
//! - **Reflection-based Diffing**: Compare component states using reflection
//! - **Bevy 0.16 Compatible**: Uses latest Bevy reflection APIs

#[cfg(feature = "bevy-reflection")]
use bevy::prelude::*;
#[cfg(feature = "bevy-reflection")]
use bevy::reflect::*;
#[cfg(feature = "bevy-reflection")]
use ::serde::{Deserialize, Serialize};
#[cfg(not(feature = "bevy-reflection"))]
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::server::error::{Error, Result};

fn default_type_id() -> TypeId {
    TypeId::of::<()>()
}
use crate::server::brp_messages::ComponentValue;

/// Reflection-based component inspector for Bevy components
#[derive(Clone)]
pub struct BevyReflectionInspector {
    /// Cache of reflection metadata by type name
    reflection_cache: Arc<RwLock<HashMap<String, ReflectionMetadata>>>,
    /// Custom inspectors for complex types
    custom_inspectors: Arc<RwLock<HashMap<String, Box<dyn CustomInspector + Send + Sync>>>>,
}

/// Metadata about a reflected component type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionMetadata {
    /// Type name as registered in TypeRegistry
    pub type_name: String,
    /// Type ID for efficient lookup
    #[serde(skip, default = "default_type_id")]
    pub type_id: TypeId,
    /// Whether this type supports reflection
    pub is_reflected: bool,
    /// Field information for struct types
    pub fields: Vec<FieldMetadata>,
    /// Type category (struct, enum, tuple, etc.)
    pub type_category: TypeCategory,
    /// Custom type information
    pub type_info: Option<Value>,
    /// Last updated timestamp
    pub last_updated: u64,
}

/// Information about a field in a reflected struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldMetadata {
    /// Field name
    pub name: String,
    /// Field type name
    pub type_name: String,
    /// Field index in the struct
    pub index: usize,
    /// Whether field supports reflection
    pub is_reflected: bool,
    /// Custom inspector for this field
    pub inspector_name: Option<String>,
}

/// Category of reflected type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeCategory {
    Struct,
    TupleStruct,
    Enum,
    Array,
    List,
    Map,
    Value,
}

/// Result of reflection-based component inspection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionInspectionResult {
    /// Component type name
    pub component_type: String,
    /// Reflection metadata
    pub metadata: ReflectionMetadata,
    /// Inspected field values
    pub field_values: HashMap<String, InspectedValue>,
    /// Any errors encountered during inspection
    pub errors: Vec<String>,
    /// Inspection timestamp
    pub timestamp: u64,
}

/// Value inspected using reflection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectedValue {
    /// Field name
    pub name: String,
    /// Raw JSON value
    pub raw_value: Value,
    /// Reflected type information
    pub type_info: String,
    /// Human-readable display value
    pub display_value: String,
    /// Whether this value supports further inspection
    pub inspectable: bool,
    /// Child values for nested structures
    pub children: Option<Vec<InspectedValue>>,
}

/// Trait for custom component inspectors
pub trait CustomInspector: Any + Send + Sync {
    /// Inspect a component value using custom logic
    fn inspect(&self, value: &ComponentValue, type_name: &str) -> Result<InspectedValue>;
    
    /// Get the inspector name
    fn name(&self) -> &str;
    
    /// Get supported type names
    fn supported_types(&self) -> Vec<String>;
}

/// Result of reflection-based diffing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionDiffResult {
    /// Type being compared
    pub type_name: String,
    /// Field-level differences
    pub field_diffs: HashMap<String, FieldDiff>,
    /// Summary statistics
    pub summary: DiffSummary,
    /// Detailed change descriptions
    pub change_descriptions: Vec<String>,
}

/// Difference in a specific field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDiff {
    /// Field name
    pub field_name: String,
    /// Field type
    pub field_type: String,
    /// Old value (if available)
    pub old_value: Option<InspectedValue>,
    /// New value (if available)
    pub new_value: Option<InspectedValue>,
    /// Type of change
    pub change_type: ChangeType,
    /// Semantic significance of this change
    pub significance: ChangeSeverity,
}

/// Type of change detected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeType {
    Added,
    Removed,
    Modified,
    TypeChanged,
}

/// Severity of a detected change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChangeSeverity {
    Trivial,   // Small numerical changes, formatting
    Minor,     // Field value changes within expected ranges
    Major,     // Significant state changes
    Critical,  // Type changes, structural modifications
}

/// Summary of diff results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffSummary {
    pub total_fields: usize,
    pub changed_fields: usize,
    pub added_fields: usize,
    pub removed_fields: usize,
    pub critical_changes: usize,
    pub major_changes: usize,
    pub minor_changes: usize,
    pub trivial_changes: usize,
}

impl BevyReflectionInspector {
    /// Create a new reflection inspector
    pub fn new() -> Self {
        let mut inspector = Self {
            reflection_cache: Arc::new(RwLock::new(HashMap::new())),
            custom_inspectors: Arc::new(RwLock::new(HashMap::new())),
        };
        
        // Register default custom inspectors
        inspector.register_default_inspectors();
        inspector
    }

    /// Register default custom inspectors for common Bevy types
    fn register_default_inspectors(&mut self) {
        // Note: Implementation would register inspectors here
        // This is a placeholder for the structure
        debug!("Registering default custom inspectors");
    }

    /// Register a custom inspector for specific types
    pub async fn register_custom_inspector(&self, inspector: Box<dyn CustomInspector + Send + Sync>) {
        let inspector_name = inspector.name().to_string();
        let mut inspectors = self.custom_inspectors.write().await;
        inspectors.insert(inspector_name.clone(), inspector);
        info!("Registered custom inspector: {}", inspector_name);
    }

    /// Discover component types using TypeRegistry (when Bevy feature is enabled)
    #[cfg(feature = "bevy-reflection")]
    pub async fn discover_component_types(&self, type_registry: &TypeRegistry) -> Result<Vec<ReflectionMetadata>> {
        debug!("Discovering component types from TypeRegistry");
        let mut discovered_types = Vec::new();
        
        for registration in type_registry.iter() {
            let type_info = registration.type_info();
            let metadata = self.build_reflection_metadata(registration).await?;
            discovered_types.push(metadata);
        }
        
        info!("Discovered {} reflected component types", discovered_types.len());
        Ok(discovered_types)
    }

    /// Build reflection metadata from type registration
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
                        is_reflected: true, // If it's in the registry, it should be reflected
                        inspector_name: self.find_inspector_for_type(field.type_path()).await,
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
                        inspector_name: self.find_inspector_for_type(field.type_path()).await,
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
            type_info: Some(self.serialize_type_info(&type_info).await),
            last_updated: chrono::Utc::now().timestamp_micros() as u64,
        })
    }

    /// Serialize type info to JSON for storage
    #[cfg(feature = "bevy-reflection")]
    async fn serialize_type_info(&self, type_info: &TypeInfo) -> Value {
        match type_info {
            TypeInfo::Struct(struct_info) => json!({
                "type": "struct",
                "name": struct_info.type_path(),
                "field_count": struct_info.field_len(),
                "fields": struct_info.iter().map(|field| {
                    json!({
                        "name": field.name(),
                        "type": field.type_path(),
                    })
                }).collect::<Vec<_>>()
            }),
            TypeInfo::Enum(enum_info) => json!({
                "type": "enum",
                "name": enum_info.type_path(),
                "variant_count": enum_info.variant_len(),
            }),
            _ => json!({
                "type": "other",
                "name": type_info.type_path(),
            }),
        }
    }

    /// Find custom inspector for a type
    #[cfg(feature = "bevy-reflection")]
    async fn find_inspector_for_type(&self, type_path: &str) -> Option<String> {
        let inspectors = self.custom_inspectors.read().await;
        for inspector in inspectors.values() {
            if inspector.supported_types().contains(&type_path.to_string()) {
                return Some(inspector.name().to_string());
            }
        }
        None
    }

    /// Inspect a component value using reflection
    pub async fn inspect_component(&self, component_type: &str, component_value: &ComponentValue) -> Result<ReflectionInspectionResult> {
        debug!("Inspecting component: {}", component_type);
        
        let metadata = self.get_or_create_metadata(component_type).await?;
        let mut field_values = HashMap::new();
        let mut errors = Vec::new();

        // Try custom inspector first
        if let Some(inspector_name) = &metadata.fields.first().and_then(|f| f.inspector_name.as_ref()) {
            let inspectors = self.custom_inspectors.read().await;
            if let Some(inspector) = inspectors.get(inspector_name.as_str()) {
                match inspector.inspect(component_value, component_type) {
                    Ok(inspected_value) => {
                        field_values.insert("root".to_string(), inspected_value);
                    }
                    Err(e) => {
                        errors.push(format!("Custom inspector error: {}", e));
                    }
                }
            }
        }

        // Fallback to generic inspection
        if field_values.is_empty() {
            match self.inspect_value_generic(component_value, component_type).await {
                Ok(inspected_value) => {
                    field_values.insert("root".to_string(), inspected_value);
                }
                Err(e) => {
                    errors.push(format!("Generic inspection error: {}", e));
                }
            }
        }

        Ok(ReflectionInspectionResult {
            component_type: component_type.to_string(),
            metadata,
            field_values,
            errors,
            timestamp: chrono::Utc::now().timestamp_micros() as u64,
        })
    }

    /// Generic value inspection for fallback
    pub async fn inspect_value_generic(&self, value: &ComponentValue, type_name: &str) -> Result<InspectedValue> {
        let inspected = match value {
            Value::Object(obj) => {
                let mut children = Vec::new();
                for (key, val) in obj {
                    let _child = Box::pin(self.inspect_value_generic(val, "unknown")).await?;
                    children.push(InspectedValue {
                        name: key.clone(),
                        raw_value: val.clone(),
                        type_info: self.infer_type_from_value(val),
                        display_value: self.format_display_value(val),
                        inspectable: self.is_value_inspectable(val),
                        children: None,
                    });
                }
                
                InspectedValue {
                    name: "root".to_string(),
                    raw_value: value.clone(),
                    type_info: type_name.to_string(),
                    display_value: format!("{} {{ {} fields }}", type_name, obj.len()),
                    inspectable: true,
                    children: Some(children),
                }
            }
            Value::Array(arr) => {
                let mut children = Vec::new();
                for (i, val) in arr.iter().enumerate() {
                    let _child = Box::pin(self.inspect_value_generic(val, "unknown")).await?;
                    children.push(InspectedValue {
                        name: format!("[{}]", i),
                        raw_value: val.clone(),
                        type_info: self.infer_type_from_value(val),
                        display_value: self.format_display_value(val),
                        inspectable: self.is_value_inspectable(val),
                        children: None,
                    });
                }
                
                InspectedValue {
                    name: "root".to_string(),
                    raw_value: value.clone(),
                    type_info: format!("Vec<T> (length: {})", arr.len()),
                    display_value: format!("Array[{}]", arr.len()),
                    inspectable: true,
                    children: Some(children),
                }
            }
            _ => {
                InspectedValue {
                    name: "root".to_string(),
                    raw_value: value.clone(),
                    type_info: self.infer_type_from_value(value),
                    display_value: self.format_display_value(value),
                    inspectable: false,
                    children: None,
                }
            }
        };

        Ok(inspected)
    }

    /// Infer type from JSON value
    fn infer_type_from_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "Option<T> (None)".to_string(),
            Value::Bool(_) => "bool".to_string(),
            Value::Number(n) => {
                if n.is_i64() {
                    "i64".to_string()
                } else if n.is_u64() {
                    "u64".to_string()
                } else {
                    "f64".to_string()
                }
            }
            Value::String(_) => "String".to_string(),
            Value::Array(_) => "Vec<T>".to_string(),
            Value::Object(_) => "Struct".to_string(),
        }
    }

    /// Format display value for UI
    fn format_display_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "None".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("\"{}\"", s),
            Value::Array(arr) => format!("[{} items]", arr.len()),
            Value::Object(obj) => format!("{{ {} fields }}", obj.len()),
        }
    }

    /// Check if value can be further inspected
    fn is_value_inspectable(&self, value: &Value) -> bool {
        matches!(value, Value::Object(_) | Value::Array(_))
    }

    /// Get or create metadata for a component type
    async fn get_or_create_metadata(&self, component_type: &str) -> Result<ReflectionMetadata> {
        let mut cache = self.reflection_cache.write().await;
        
        if let Some(metadata) = cache.get(component_type) {
            return Ok(metadata.clone());
        }

        // Create basic metadata for unknown types
        let metadata = ReflectionMetadata {
            type_name: component_type.to_string(),
            type_id: TypeId::of::<()>(), // Placeholder
            is_reflected: false,
            fields: Vec::new(),
            type_category: TypeCategory::Value,
            type_info: None,
            last_updated: chrono::Utc::now().timestamp_micros() as u64,
        };

        cache.insert(component_type.to_string(), metadata.clone());
        Ok(metadata)
    }

    /// The fields of a component value, keyed by name — the unit `diff_components` compares.
    ///
    /// A component's fields *are* its JSON object keys; a component that is not an object (a newtype
    /// over a scalar, say) has exactly one implicit field. That is a definition, not a fallback: both
    /// shapes produce one map with the same meaning, and the caller has no second code path.
    async fn field_map(
        &self,
        value: &ComponentValue,
    ) -> Result<HashMap<String, InspectedValue>> {
        let mut fields = HashMap::new();
        match value {
            Value::Object(obj) => {
                for (name, field_value) in obj {
                    fields.insert(
                        name.clone(),
                        self.inspect_value_generic(field_value, name).await?,
                    );
                }
            }
            other => {
                fields.insert(
                    "root".to_string(),
                    self.inspect_value_generic(other, "root").await?,
                );
            }
        }
        Ok(fields)
    }

    /// Perform reflection-based diffing of component states
    pub async fn diff_components(
        &self,
        component_type: &str,
        old_value: &ComponentValue,
        new_value: &ComponentValue,
    ) -> Result<ReflectionDiffResult> {
        debug!("Diffing components of type: {}", component_type);

        // **Diff by the component's actual fields.**
        //
        // This used to read `inspect_component(..).field_values`, which inserts exactly one entry —
        // `"root"` — holding the whole component (see `inspect_component`). Both sides therefore always
        // had the same single key, so the `(None, Some(_))` / `(Some(_), None)` arms below were
        // unreachable and `added_fields`/`removed_fields` could never be anything but zero. A diff that
        // structurally cannot report an added field is worse than no diff, because the summary looks
        // authoritative.
        let old_fields = self.field_map(old_value).await?;
        let new_fields = self.field_map(new_value).await?;

        let mut field_diffs = HashMap::new();
        let mut summary = DiffSummary {
            total_fields: 0,
            changed_fields: 0,
            added_fields: 0,
            removed_fields: 0,
            critical_changes: 0,
            major_changes: 0,
            minor_changes: 0,
            trivial_changes: 0,
        };

        // Compare field values
        let all_field_names: std::collections::HashSet<String> = old_fields
            .keys()
            .chain(new_fields.keys())
            .cloned()
            .collect();

        summary.total_fields = all_field_names.len();

        for field_name in all_field_names {
            let old_field = old_fields.get(&field_name);
            let new_field = new_fields.get(&field_name);

            let (change_type, severity) = match (old_field, new_field) {
                (None, Some(_)) => (ChangeType::Added, ChangeSeverity::Minor),
                (Some(_), None) => (ChangeType::Removed, ChangeSeverity::Major),
                (Some(old), Some(new)) => {
                    if old.type_info != new.type_info {
                        (ChangeType::TypeChanged, ChangeSeverity::Critical)
                    } else if old.raw_value != new.raw_value {
                        let severity = self.assess_change_severity(&old.raw_value, &new.raw_value);
                        (ChangeType::Modified, severity)
                    } else {
                        continue; // No change
                    }
                }
                (None, None) => continue, // Should not happen
            };

            // Update summary counters
            match change_type {
                ChangeType::Added => summary.added_fields += 1,
                ChangeType::Removed => summary.removed_fields += 1,
                _ => {}
            }

            match severity {
                ChangeSeverity::Trivial => summary.trivial_changes += 1,
                ChangeSeverity::Minor => summary.minor_changes += 1,
                ChangeSeverity::Major => summary.major_changes += 1,
                ChangeSeverity::Critical => summary.critical_changes += 1,
            }

            summary.changed_fields += 1;

            let field_diff = FieldDiff {
                field_name: field_name.clone(),
                field_type: old_field
                    .or(new_field)
                    .map(|f| f.type_info.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                old_value: old_field.cloned(),
                new_value: new_field.cloned(),
                change_type,
                significance: severity,
            };

            field_diffs.insert(field_name, field_diff);
        }

        let change_descriptions = self.generate_change_descriptions(&field_diffs);

        Ok(ReflectionDiffResult {
            type_name: component_type.to_string(),
            field_diffs,
            summary,
            change_descriptions,
        })
    }

    /// Assess the severity of a value change
    pub fn assess_change_severity(&self, old_value: &Value, new_value: &Value) -> ChangeSeverity {
        match (old_value, new_value) {
            (Value::Number(old_num), Value::Number(new_num)) => {
                // Assess numerical changes
                if let (Some(old_f), Some(new_f)) = (old_num.as_f64(), new_num.as_f64()) {
                    let diff = (old_f - new_f).abs();
                    let relative_change = diff / old_f.abs().max(1.0);
                    
                    if relative_change < 0.001 {
                        ChangeSeverity::Trivial
                    } else if relative_change < 0.1 {
                        ChangeSeverity::Minor
                    } else {
                        ChangeSeverity::Major
                    }
                } else {
                    ChangeSeverity::Minor
                }
            }
            (Value::String(old_s), Value::String(new_s)) => {
                if old_s.len() != new_s.len() || old_s != new_s {
                    ChangeSeverity::Minor
                } else {
                    ChangeSeverity::Trivial
                }
            }
            (Value::Bool(_), Value::Bool(_)) => ChangeSeverity::Minor,
            (Value::Array(old_arr), Value::Array(new_arr)) => {
                if old_arr.len() != new_arr.len() {
                    ChangeSeverity::Major
                } else {
                    ChangeSeverity::Minor
                }
            }
            (Value::Object(old_obj), Value::Object(new_obj)) => {
                if old_obj.len() != new_obj.len() {
                    ChangeSeverity::Major
                } else {
                    ChangeSeverity::Minor
                }
            }
            _ => ChangeSeverity::Major, // Type mismatch
        }
    }

    /// Generate human-readable change descriptions
    fn generate_change_descriptions(&self, field_diffs: &HashMap<String, FieldDiff>) -> Vec<String> {
        let mut descriptions = Vec::new();

        for (field_name, diff) in field_diffs {
            let description = match &diff.change_type {
                ChangeType::Added => {
                    format!("Added field '{}' with value: {}", 
                        field_name, 
                        diff.new_value.as_ref()
                            .map(|v| v.display_value.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
                ChangeType::Removed => {
                    format!("Removed field '{}' (was: {})", 
                        field_name, 
                        diff.old_value.as_ref()
                            .map(|v| v.display_value.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
                ChangeType::Modified => {
                    format!("Modified field '{}': {} -> {}", 
                        field_name,
                        diff.old_value.as_ref()
                            .map(|v| v.display_value.clone())
                            .unwrap_or_else(|| "unknown".to_string()),
                        diff.new_value.as_ref()
                            .map(|v| v.display_value.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
                ChangeType::TypeChanged => {
                    format!("Type changed for field '{}': {} -> {}", 
                        field_name,
                        diff.old_value.as_ref()
                            .map(|v| v.type_info.clone())
                            .unwrap_or_else(|| "unknown".to_string()),
                        diff.new_value.as_ref()
                            .map(|v| v.type_info.clone())
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
            };
            descriptions.push(description);
        }

        descriptions
    }

    /// Get cached reflection metadata for all known types
    pub async fn get_all_reflection_metadata(&self) -> HashMap<String, ReflectionMetadata> {
        self.reflection_cache.read().await.clone()
    }

    /// Clear the reflection cache
    pub async fn clear_cache(&self) {
        let mut cache = self.reflection_cache.write().await;
        cache.clear();
        info!("Cleared reflection cache");
    }

    /// Get statistics about the reflection system
    pub async fn get_reflection_stats(&self) -> Value {
        let cache = self.reflection_cache.read().await;
        let inspectors = self.custom_inspectors.read().await;

        json!({
            "cached_types": cache.len(),
            "custom_inspectors": inspectors.len(),
            "reflected_types": cache.values().filter(|m| m.is_reflected).count(),
            "total_fields": cache.values().map(|m| m.fields.len()).sum::<usize>(),
        })
    }
}

impl Default for BevyReflectionInspector {
    fn default() -> Self {
        Self::new()
    }
}

/// Transform component inspector with 3D math understanding
pub struct TransformInspector;

impl CustomInspector for TransformInspector {
    fn inspect(&self, value: &ComponentValue, _type_name: &str) -> Result<InspectedValue> {
        if let Value::Object(obj) = value {
            let mut children = Vec::new();

            // Extract translation
            if let Some(translation) = obj.get("translation") {
                children.push(InspectedValue {
                    name: "translation".to_string(),
                    raw_value: translation.clone(),
                    type_info: "Vec3".to_string(),
                    display_value: self.format_vec3(translation),
                    inspectable: true,
                    children: None,
                });
            }

            // Extract rotation
            if let Some(rotation) = obj.get("rotation") {
                children.push(InspectedValue {
                    name: "rotation".to_string(),
                    raw_value: rotation.clone(),
                    type_info: "Quat".to_string(),
                    display_value: self.format_quat(rotation),
                    inspectable: true,
                    children: None,
                });
            }

            // Extract scale
            if let Some(scale) = obj.get("scale") {
                children.push(InspectedValue {
                    name: "scale".to_string(),
                    raw_value: scale.clone(),
                    type_info: "Vec3".to_string(),
                    display_value: self.format_vec3(scale),
                    inspectable: true,
                    children: None,
                });
            }

            Ok(InspectedValue {
                name: "Transform".to_string(),
                raw_value: value.clone(),
                type_info: "bevy_transform::components::transform::Transform".to_string(),
                display_value: "Transform Component".to_string(),
                inspectable: true,
                children: Some(children),
            })
        } else {
            Err(Error::DebugError("Transform component should be an object".to_string()))
        }
    }

    fn name(&self) -> &str {
        "TransformInspector"
    }

    fn supported_types(&self) -> Vec<String> {
        vec!["bevy_transform::components::transform::Transform".to_string()]
    }
}

impl TransformInspector {
    fn format_vec3(&self, value: &Value) -> String {
        if let Value::Object(obj) = value {
            let x = obj.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = obj.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = obj.get("z").and_then(|v| v.as_f64()).unwrap_or(0.0);
            format!("({:.2}, {:.2}, {:.2})", x, y, z)
        } else {
            "Invalid Vec3".to_string()
        }
    }

    fn format_quat(&self, value: &Value) -> String {
        if let Value::Object(obj) = value {
            let x = obj.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let y = obj.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let z = obj.get("z").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let w = obj.get("w").and_then(|v| v.as_f64()).unwrap_or(1.0);
            
            // Convert to euler angles for better readability
            let euler = self.quat_to_euler(x, y, z, w);
            format!("Quat(x:{:.2}, y:{:.2}, z:{:.2}, w:{:.2}) ≈ ({:.1}°, {:.1}°, {:.1}°)", 
                x, y, z, w, euler.0, euler.1, euler.2)
        } else {
            "Invalid Quat".to_string()
        }
    }

    fn quat_to_euler(&self, x: f64, y: f64, z: f64, w: f64) -> (f64, f64, f64) {
        // Simple quaternion to euler conversion (yaw, pitch, roll)
        let yaw = (2.0 * (w * y + z * x)).atan2(1.0 - 2.0 * (y * y + z * z));
        let pitch = (2.0 * (w * z - x * y)).asin();
        let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + z * z));
        
        (yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reflection_inspector_creation() {
        let inspector = BevyReflectionInspector::new();
        let stats = inspector.get_reflection_stats().await;
        
        assert!(stats.get("cached_types").is_some());
        assert!(stats.get("custom_inspectors").is_some());
    }

    #[tokio::test]
    async fn test_generic_inspection() {
        let inspector = BevyReflectionInspector::new();
        
        let test_value = json!({
            "x": 1.0,
            "y": 2.0,
            "z": 3.0
        });
        
        let result = inspector.inspect_value_generic(&test_value, "TestType").await.unwrap();
        assert_eq!(result.type_info, "TestType");
        assert!(result.inspectable);
        assert!(result.children.is_some());
    }

    #[tokio::test]
    async fn test_transform_inspector() {
        let inspector = TransformInspector;
        
        let transform_value = json!({
            "translation": {"x": 1.0, "y": 2.0, "z": 3.0},
            "rotation": {"x": 0.0, "y": 0.0, "z": 0.0, "w": 1.0},
            "scale": {"x": 1.0, "y": 1.0, "z": 1.0}
        });
        
        let result = inspector.inspect(&transform_value, "Transform").unwrap();
        assert_eq!(result.name, "Transform");
        assert!(result.children.is_some());
        assert_eq!(result.children.as_ref().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_diff_components() {
        let inspector = BevyReflectionInspector::new();
        
        let old_value = json!({"x": 1.0, "y": 2.0});
        let new_value = json!({"x": 1.5, "y": 2.0, "z": 3.0});
        
        let diff_result = inspector.diff_components("TestComponent", &old_value, &new_value).await.unwrap();
        
        assert_eq!(diff_result.type_name, "TestComponent");
        assert!(diff_result.summary.changed_fields > 0);
        assert!(diff_result.summary.added_fields > 0);
    }

    #[test]
    fn test_change_severity_assessment() {
        let inspector = BevyReflectionInspector::new();
        
        // Test numerical change severity
        let old_num = json!(10.0);
        let new_num_trivial = json!(10.001);
        let new_num_minor = json!(10.5);
        let new_num_major = json!(15.0);
        
        assert!(matches!(inspector.assess_change_severity(&old_num, &new_num_trivial), ChangeSeverity::Trivial));
        assert!(matches!(inspector.assess_change_severity(&old_num, &new_num_minor), ChangeSeverity::Minor));
        assert!(matches!(inspector.assess_change_severity(&old_num, &new_num_major), ChangeSeverity::Major));
    }
}