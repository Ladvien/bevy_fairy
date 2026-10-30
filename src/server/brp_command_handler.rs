/*
 * bevy-fairy - BRP Command Handler Interface
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::server::brp::{builtin_methods, BrpRequest, BrpResponse};
use crate::server::brp_validation::BrpValidator;
use crate::server::error::{Error, Result};

/// Version information for command handlers
#[derive(Debug, Clone)]
pub struct CommandVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl CommandVersion {
    pub fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }

    pub fn is_compatible_with(&self, other: &CommandVersion) -> bool {
        // Major version must match, minor/patch can differ
        self.major == other.major
    }
}

impl Default for CommandVersion {
    fn default() -> Self {
        Self::new(1, 0, 0)
    }
}

/// Metadata for command handlers
#[derive(Debug, Clone)]
pub struct CommandHandlerMetadata {
    pub name: String,
    pub version: CommandVersion,
    pub description: String,
    pub supported_commands: Vec<String>,
}

/// Trait for handling BRP commands in an extensible way.
/// 
/// Implementors of this trait can process specific types of BRP requests
/// and return appropriate responses. Handlers are registered with a
/// `CommandHandlerRegistry` and selected based on their ability to handle
/// a request and their priority.
/// 
/// # Example
/// ```ignore
/// struct MyHandler;
/// 
/// #[async_trait]
/// impl BrpCommandHandler for MyHandler {
///     fn metadata(&self) -> CommandHandlerMetadata { ... }
///     fn can_handle(&self, request: &BrpRequest) -> bool { ... }
///     async fn handle(&self, request: BrpRequest) -> Result<BrpResponse> { ... }
/// }
/// ```
#[async_trait]
pub trait BrpCommandHandler: Send + Sync {
    /// Get metadata about this handler including name, version, and supported commands
    fn metadata(&self) -> CommandHandlerMetadata;

    /// Check if this handler can process the given request.
    /// Returns true if the handler supports the request type.
    fn can_handle(&self, request: &BrpRequest) -> bool;

    /// Process a BRP request and return a response.
    /// This is called after validation passes.
    async fn handle(&self, request: BrpRequest) -> Result<BrpResponse>;

    /// Validate a request before processing.
    /// Override this to add custom validation logic.
    /// Default implementation uses comprehensive BRP validation.
    async fn validate(&self, request: &BrpRequest) -> Result<()> {
        // The request must name one of the real Bevy 0.19 BRP methods.
        crate::server::brp_validation::validate_method_name(&request.method)
            .map_err(Error::Validation)
    }

    /// Get the handler's priority (higher = processed first).
    /// Handlers with higher priority are checked first when finding
    /// a handler for a request. Default priority is 0.
    fn priority(&self) -> i32 {
        0
    }
}

/// Registry for command handlers with versioning support
pub struct CommandHandlerRegistry {
    handlers: Arc<RwLock<Vec<Arc<dyn BrpCommandHandler>>>>,
    version_map: Arc<RwLock<HashMap<String, CommandVersion>>>,
    validator: Arc<BrpValidator>,
}

impl Default for CommandHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Vec::new())),
            version_map: Arc::new(RwLock::new(HashMap::new())),
            validator: Arc::new(BrpValidator::new()),
        }
    }
    
    /// Create a new registry with custom validation configuration
    pub fn with_validator(validator: BrpValidator) -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Vec::new())),
            version_map: Arc::new(RwLock::new(HashMap::new())),
            validator: Arc::new(validator),
        }
    }

    /// Register a new command handler
    pub async fn register(&self, handler: Arc<dyn BrpCommandHandler>) {
        let metadata = handler.metadata();
        
        // Update version map
        let mut version_map = self.version_map.write().await;
        version_map.insert(metadata.name.clone(), metadata.version.clone());
        
        // Add handler sorted by priority
        let mut handlers = self.handlers.write().await;
        handlers.push(handler);
        handlers.sort_by_key(|h| -h.priority()); // Sort descending by priority
    }

    /// Find a handler for the given request
    pub async fn find_handler(&self, request: &BrpRequest) -> Option<Arc<dyn BrpCommandHandler>> {
        let handlers = self.handlers.read().await;
        
        for handler in handlers.iter() {
            if handler.can_handle(request) {
                return Some(handler.clone());
            }
        }
        
        None
    }

    /// Process a request using the appropriate handler with comprehensive validation
    pub async fn process(&self, request: BrpRequest) -> Result<BrpResponse> {
        self.process_with_session(request, "default_session").await
    }
    
    /// Process a request with session-specific validation
    pub async fn process_with_session(&self, request: BrpRequest, session_id: &str) -> Result<BrpResponse> {
        if let Some(handler) = self.find_handler(&request).await {
            // First run comprehensive validation
            let request_size = serde_json::to_vec(&request)
                .map_err(crate::server::error::Error::Json)?
                .len();
            
            self.validator.validate_request(&request, session_id, request_size).await?;
            
            // Then run handler-specific validation
            handler.validate(&request).await?;
            
            // Finally handle the request
            handler.handle(request).await
        } else {
            Err(crate::server::error::Error::Validation(format!(
                "No handler found for request type: {:?}",
                request
            )))
        }
    }

    /// Get version information for all registered handlers
    pub async fn get_versions(&self) -> HashMap<String, CommandVersion> {
        self.version_map.read().await.clone()
    }

    /// Check if a specific version is supported
    pub async fn is_version_supported(&self, handler_name: &str, version: &CommandVersion) -> bool {
        let version_map = self.version_map.read().await;
        
        if let Some(registered_version) = version_map.get(handler_name) {
            registered_version.is_compatible_with(version)
        } else {
            false
        }
    }
    
    /// Get the validator for configuration
    pub fn get_validator(&self) -> Arc<BrpValidator> {
        self.validator.clone()
    }
}

/// Default handler for core BRP methods
pub struct CoreBrpHandler;

#[async_trait]
impl BrpCommandHandler for CoreBrpHandler {
    fn metadata(&self) -> CommandHandlerMetadata {
        CommandHandlerMetadata {
            name: "core".to_string(),
            version: CommandVersion::new(1, 0, 0),
            description: "Handler for core BRP methods".to_string(),
            supported_commands: vec![
                builtin_methods::BRP_QUERY_METHOD.to_string(),
                builtin_methods::BRP_GET_COMPONENTS_METHOD.to_string(),
                builtin_methods::BRP_SPAWN_ENTITY_METHOD.to_string(),
                builtin_methods::BRP_DESPAWN_COMPONENTS_METHOD.to_string(),
                builtin_methods::BRP_INSERT_COMPONENTS_METHOD.to_string(),
                builtin_methods::BRP_REMOVE_COMPONENTS_METHOD.to_string(),
                builtin_methods::BRP_REPARENT_ENTITIES_METHOD.to_string(),
                builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string(),
            ],
        }
    }

    fn can_handle(&self, request: &BrpRequest) -> bool {
        matches!(
            request.method.as_str(),
            builtin_methods::BRP_QUERY_METHOD
                | builtin_methods::BRP_GET_COMPONENTS_METHOD
                | builtin_methods::BRP_SPAWN_ENTITY_METHOD
                | builtin_methods::BRP_DESPAWN_COMPONENTS_METHOD
                | builtin_methods::BRP_INSERT_COMPONENTS_METHOD
                | builtin_methods::BRP_REMOVE_COMPONENTS_METHOD
                | builtin_methods::BRP_REPARENT_ENTITIES_METHOD
                | builtin_methods::BRP_LIST_COMPONENTS_METHOD
        )
    }

    async fn handle(&self, _request: BrpRequest) -> Result<BrpResponse> {
        // Requests are transported by the HTTP BRP client, not this handler;
        // a handler only intercepts when it has domain knowledge to add.
        // No local intercept: report that the handler cannot produce a result.
        Err(Error::Validation(
            "CoreBrpHandler does not execute requests locally".to_string(),
        ))
    }

    fn priority(&self) -> i32 {
        -100 // Low priority, let specialized handlers go first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_compatibility() {
        let v1 = CommandVersion::new(1, 0, 0);
        let v2 = CommandVersion::new(1, 1, 0);
        let v3 = CommandVersion::new(2, 0, 0);

        assert!(v1.is_compatible_with(&v2));
        assert!(!v1.is_compatible_with(&v3));
    }

    #[tokio::test]
    async fn test_handler_registry() {
        let registry = CommandHandlerRegistry::new();
        let handler = Arc::new(CoreBrpHandler);

        registry.register(handler.clone()).await;

        let request = BrpRequest {
            method: builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string(),
            id: None,
            params: None,
        };
        let found = registry.find_handler(&request).await;

        assert!(found.is_some());
    }

    #[tokio::test]
    async fn test_handler_priority() {
        struct HighPriorityHandler;

        #[async_trait]
        impl BrpCommandHandler for HighPriorityHandler {
            fn metadata(&self) -> CommandHandlerMetadata {
                CommandHandlerMetadata {
                    name: "high_priority".to_string(),
                    version: CommandVersion::default(),
                    description: "High priority handler".to_string(),
                    supported_commands: vec![
                        builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string()
                    ],
                }
            }

            fn can_handle(&self, request: &BrpRequest) -> bool {
                request.method == builtin_methods::BRP_LIST_COMPONENTS_METHOD
            }

            async fn handle(&self, _request: BrpRequest) -> Result<BrpResponse> {
                Err(Error::Validation("test stub".to_string()))
            }

            fn priority(&self) -> i32 {
                100 // High priority
            }
        }

        let registry = CommandHandlerRegistry::new();

        // Register low priority first
        registry.register(Arc::new(CoreBrpHandler)).await;

        // Then high priority
        registry.register(Arc::new(HighPriorityHandler)).await;

        let request = BrpRequest {
            method: builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string(),
            id: None,
            params: None,
        };
        let handler = registry.find_handler(&request).await.unwrap();

        // Should get high priority handler
        assert_eq!(handler.priority(), 100);
    }
}