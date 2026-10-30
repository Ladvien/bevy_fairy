/// Debug command processor infrastructure for extensible debugging operations
use crate::server::brp_messages::{
    DebugCommand, DebugResponse,
};
use crate::server::error::{Error, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// Default timeout for debug commands (30 seconds as per requirements)
pub const DEBUG_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum priority level for debug commands
pub const MAX_PRIORITY: u8 = 10;

/// Maximum number of commands that can be queued
pub const MAX_QUEUE_SIZE: usize = 100;

/// Maximum number of concurrent debug operations
pub const MAX_CONCURRENT_OPERATIONS: usize = 10;

/// History size for processing time metrics
pub const METRICS_HISTORY_SIZE: usize = 1000;

/// Trait for processing debug commands
#[async_trait]
pub trait DebugCommandProcessor: Send + Sync {
    /// Process a debug command and return a response
    async fn process(&self, command: DebugCommand) -> Result<DebugResponse>;
    
    /// Validate if the command can be processed
    async fn validate(&self, command: &DebugCommand) -> Result<()>;
    
    /// Get the estimated processing time for a command
    fn estimate_processing_time(&self, command: &DebugCommand) -> Duration;
    
    /// Check if the processor supports a specific command type
    fn supports_command(&self, command: &DebugCommand) -> bool;
}

/// Debug command with metadata for processing
#[derive(Debug, Clone)]
pub struct DebugCommandRequest {
    /// Unique command ID
    pub id: String,
    /// The debug command to execute
    pub command: DebugCommand,
    /// Correlation ID for response tracking
    pub correlation_id: String,
    /// Priority level (0-10, higher = more priority)
    pub priority: u8,
    /// Timestamp when command was received
    pub received_at: Instant,
    /// Timeout duration for this command
    pub timeout: Duration,
    /// TTL for response correlation
    pub response_ttl: Duration,
}

impl DebugCommandRequest {
    /// Create a new debug command request
    pub fn new(command: DebugCommand, correlation_id: String, priority: Option<u8>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            command,
            correlation_id,
            priority: priority.unwrap_or(5).min(MAX_PRIORITY),
            received_at: Instant::now(),
            timeout: DEBUG_COMMAND_TIMEOUT,
            response_ttl: Duration::from_secs(60), // 1 minute TTL for responses
        }
    }
    
    /// Check if the command has timed out
    pub fn is_timed_out(&self) -> bool {
        self.received_at.elapsed() > self.timeout
    }
    
    /// Get remaining time before timeout
    pub fn remaining_time(&self) -> Option<Duration> {
        let elapsed = self.received_at.elapsed();
        if elapsed < self.timeout {
            Some(self.timeout - elapsed)
        } else {
            None
        }
    }
}

/// Debug command router that delegates to specific processors
pub struct DebugCommandRouter {
    /// Map of command processors
    processors: Arc<RwLock<HashMap<String, Arc<dyn DebugCommandProcessor>>>>,
    /// Command queue with priority support
    command_queue: Arc<RwLock<PriorityQueue<DebugCommandRequest>>>,
    /// Response correlation map with TTL
    response_map: Arc<RwLock<ResponseCorrelationMap>>,
    /// Metrics collector
    metrics: Arc<RwLock<DebugMetrics>>,
    /// Cleanup task handle
    _cleanup_handle: tokio::task::JoinHandle<()>,
}

impl DebugCommandRouter {
    /// Create a new debug command router
    pub fn new() -> Self {
        let response_map = Arc::new(RwLock::new(ResponseCorrelationMap::new()));
        
        // Start automatic cleanup task
        let response_map_clone = response_map.clone();
        let cleanup_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60)); // Cleanup every minute
            loop {
                interval.tick().await;
                response_map_clone.write().await.cleanup_expired();
            }
        });
        
        Self {
            processors: Arc::new(RwLock::new(HashMap::new())),
            command_queue: Arc::new(RwLock::new(PriorityQueue::new())),
            response_map,
            metrics: Arc::new(RwLock::new(DebugMetrics::new())),
            _cleanup_handle: cleanup_handle,
        }
    }
    
    /// Register a command processor
    pub async fn register_processor(
        &self,
        name: String,
        processor: Arc<dyn DebugCommandProcessor>,
    ) {
        let mut processors = self.processors.write().await;
        processors.insert(name, processor);
    }
    
    /// Queue a debug command for processing
    pub async fn queue_command(&self, request: DebugCommandRequest) -> Result<()> {
        // Check queue size limit
        {
            let queue = self.command_queue.read().await;
            if queue.len() >= MAX_QUEUE_SIZE {
                return Err(Error::DebugError(format!(
                    "Command queue full (max: {})", MAX_QUEUE_SIZE
                )));
            }
        }
        
        // Validate the command first
        if let Some(processor) = self.find_processor(&request.command).await {
            processor.validate(&request.command).await?;
        } else {
            return Err(Error::DebugError(
                "No processor found for command".to_string(),
            ));
        }
        
        // Add comprehensive validation
        self.validate_command_args(&request.command)?;
        
        // Add to priority queue
        let mut queue = self.command_queue.write().await;
        queue.push(request);
        
        Ok(())
    }
    
    /// Validate command arguments to prevent system crashes
    fn validate_command_args(&self, command: &DebugCommand) -> Result<()> {
        match command {
            DebugCommand::InspectEntity { entity_id, .. } => {
                if *entity_id == 0 {
                    return Err(Error::DebugError("Invalid entity ID: 0".to_string()));
                }
                // Add reasonable upper bound to prevent abuse
                if *entity_id > 0xFFFF_FFFF {
                    return Err(Error::DebugError("Entity ID too large".to_string()));
                }
            }
            DebugCommand::ProfileSystem { system_name, duration_ms, .. } => {
                if system_name.is_empty() {
                    return Err(Error::DebugError("System name cannot be empty".to_string()));
                }
                if let Some(duration) = duration_ms {
                    if *duration > 300_000 {  // 5 minutes max
                        return Err(Error::DebugError("Profile duration too long (max 5 minutes)".to_string()));
                    }
                }
            }
            DebugCommand::ExecuteQuery { query, limit, .. } => {
                if query.filter.with.as_ref().is_some_and(|w| w.len() > 20) {
                    return Err(Error::DebugError("Too many components in query (max 20)".to_string()));
                }
                if let Some(l) = limit {
                    if *l > 10_000 {
                        return Err(Error::DebugError("Query limit too high (max 10,000)".to_string()));
                    }
                }
            }
            DebugCommand::ProfileMemory { target_systems, duration_seconds, .. } => {
                if let Some(systems) = target_systems {
                    if systems.len() > 100 {
                        return Err(Error::DebugError("Too many target systems (max 100)".to_string()));
                    }
                }
                if let Some(duration) = duration_seconds {
                    if *duration > 86400 { // 24 hours max
                        return Err(Error::DebugError("Duration too long (max 24 hours)".to_string()));
                    }
                }
            }
            DebugCommand::DetectMemoryLeaks { target_systems } |
            DebugCommand::AnalyzeMemoryTrends { target_systems } => {
                if let Some(systems) = target_systems {
                    if systems.len() > 100 {
                        return Err(Error::DebugError("Too many target systems (max 100)".to_string()));
                    }
                }
            }
            DebugCommand::StopMemoryProfiling { session_id } => {
                if let Some(id) = session_id {
                    if id.len() > 100 {
                        return Err(Error::DebugError("Session ID too long (max 100 chars)".to_string()));
                    }
                }
            }
            DebugCommand::Custom { name, .. } => {
                if name.len() > 100 {
                    return Err(Error::DebugError("Custom command name too long (max 100 chars)".to_string()));
                }
            }
            DebugCommand::InspectBatch { entity_ids, limit, .. } => {
                if entity_ids.is_empty() {
                    return Err(Error::DebugError("Entity IDs list cannot be empty".to_string()));
                }
                if entity_ids.len() > crate::server::entity_inspector::MAX_BATCH_SIZE {
                    return Err(Error::DebugError(format!(
                        "Too many entities in batch (max: {})", 
                        crate::server::entity_inspector::MAX_BATCH_SIZE
                    )));
                }
                if entity_ids.contains(&0) {
                    return Err(Error::DebugError("Invalid entity ID: 0".to_string()));
                }
                // Check for unreasonable entity IDs
                if entity_ids.iter().any(|&id| id > 0xFFFF_FFFF) {
                    return Err(Error::DebugError("Entity ID too large".to_string()));
                }
                if let Some(l) = limit {
                    if *l > crate::server::entity_inspector::MAX_BATCH_SIZE {
                        return Err(Error::DebugError(format!(
                            "Batch limit too high (max: {})", 
                            crate::server::entity_inspector::MAX_BATCH_SIZE
                        )));
                    }
                }
            }
            _ => {} // Other commands are safe
        }
        Ok(())
    }
    
    /// Process the next command in the queue
    pub async fn process_next(&self) -> Option<Result<(String, DebugResponse)>> {
        // Get next command from priority queue
        let request = {
            let mut queue = self.command_queue.write().await;
            queue.pop()
        }?;
        
        // Check for timeout
        if request.is_timed_out() {
            return Some(Err(Error::Timeout(
                "Debug command timed out".to_string(),
            )));
        }
        
        // Record start time for metrics
        let start_time = Instant::now();
        
        // Find and execute processor
        let result = if let Some(processor) = self.find_processor(&request.command).await {
            processor.process(request.command.clone()).await
        } else {
            Err(Error::DebugError(
                "No processor found for command".to_string(),
            ))
        };
        
        // Update metrics
        let duration = start_time.elapsed();
        self.update_metrics(duration, result.is_ok()).await;
        
        // Store response with TTL
        if let Ok(response) = &result {
            self.response_map.write().await.store(
                request.correlation_id.clone(),
                response.clone(),
                request.response_ttl,
            );
        }
        
        Some(result.map(|r| (request.correlation_id, r)))
    }
    
    /// Get a response by correlation ID
    pub async fn get_response(&self, correlation_id: &str) -> Option<DebugResponse> {
        self.response_map.read().await.get(correlation_id)
    }
    
    /// Clean up expired responses
    pub async fn cleanup_expired_responses(&self) {
        self.response_map.write().await.cleanup_expired();
    }
    
    /// Get current metrics
    pub async fn get_metrics(&self) -> DebugMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Find a processor for a command
    async fn find_processor(&self, command: &DebugCommand) -> Option<Arc<dyn DebugCommandProcessor>> {
        let processors = self.processors.read().await;
        for processor in processors.values() {
            if processor.supports_command(command) {
                return Some(Arc::clone(processor));
            }
        }
        None
    }
    
    /// Route a debug command request for processing
    pub async fn route(&self, request: DebugCommandRequest) -> Result<DebugResponse> {
        // Queue the command
        self.queue_command(request.clone()).await?;
        
        // Process immediately and return result
        if let Some(result) = self.process_next().await {
            match result {
                Ok((correlation_id, response)) => {
                    if correlation_id == request.correlation_id {
                        Ok(response)
                    } else {
                        // Store and retrieve by correlation ID
                        self.get_response(&request.correlation_id).await
                            .ok_or_else(|| Error::DebugError("Response not found".to_string()))
                    }
                }
                Err(e) => Err(e),
            }
        } else {
            Err(Error::DebugError("No commands to process".to_string()))
        }
    }

    /// Validate a debug command
    pub async fn validate_command(&self, command: &DebugCommand) -> Result<()> {
        if let Some(processor) = self.find_processor(command).await {
            processor.validate(command).await
        } else {
            Err(Error::DebugError("No processor found for command".to_string()))
        }
    }
    
    /// Update metrics after command processing
    async fn update_metrics(&self, duration: Duration, success: bool) {
        let mut metrics = self.metrics.write().await;
        metrics.record_command(duration, success);
    }
}

/// Priority queue for debug commands
struct PriorityQueue<T> {
    items: std::collections::BinaryHeap<T>,
}

impl<T> PriorityQueue<T>
where
    T: Clone + Ord,
{
    fn new() -> Self {
        Self { 
            items: std::collections::BinaryHeap::new() 
        }
    }
    
    fn push(&mut self, item: T) {
        self.items.push(item);
    }
    
    fn pop(&mut self) -> Option<T> {
        self.items.pop()
    }
    
    fn len(&self) -> usize {
        self.items.len()
    }
}

impl std::cmp::Ord for DebugCommandRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // First compare by priority, then by received time
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.received_at.cmp(&self.received_at))
    }
}

impl std::cmp::PartialOrd for DebugCommandRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::cmp::PartialEq for DebugCommandRequest {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl std::cmp::Eq for DebugCommandRequest {}

/// Response correlation map with TTL support
struct ResponseCorrelationMap {
    responses: HashMap<String, (DebugResponse, Instant, Duration)>,
}

impl ResponseCorrelationMap {
    fn new() -> Self {
        Self {
            responses: HashMap::new(),
        }
    }
    
    fn store(&mut self, correlation_id: String, response: DebugResponse, ttl: Duration) {
        self.responses.insert(
            correlation_id,
            (response, Instant::now(), ttl),
        );
    }
    
    fn get(&self, correlation_id: &str) -> Option<DebugResponse> {
        self.responses.get(correlation_id).and_then(|(response, stored_at, ttl)| {
            if stored_at.elapsed() <= *ttl {
                Some(response.clone())
            } else {
                None
            }
        })
    }
    
    fn cleanup_expired(&mut self) {
        let now = Instant::now();
        self.responses.retain(|_, (_, stored_at, ttl)| {
            now.duration_since(*stored_at) <= *ttl
        });
    }
}

/// Debug command processing metrics
#[derive(Debug, Clone)]
pub struct DebugMetrics {
    /// Total commands processed
    pub total_commands: usize,
    /// Successful commands
    pub successful_commands: usize,
    /// Failed commands
    pub failed_commands: usize,
    /// Average processing time in microseconds
    pub avg_processing_time_us: u64,
    /// Minimum processing time in microseconds
    pub min_processing_time_us: u64,
    /// Maximum processing time in microseconds
    pub max_processing_time_us: u64,
    /// Last N processing times for calculating percentiles
    processing_times: Vec<u64>,
}

impl DebugMetrics {
    fn new() -> Self {
        Self {
            total_commands: 0,
            successful_commands: 0,
            failed_commands: 0,
            avg_processing_time_us: 0,
            min_processing_time_us: u64::MAX,
            max_processing_time_us: 0,
            processing_times: Vec::with_capacity(METRICS_HISTORY_SIZE),
        }
    }
    
    fn record_command(&mut self, duration: Duration, success: bool) {
        let duration_us = duration.as_micros() as u64;
        
        self.total_commands += 1;
        if success {
            self.successful_commands += 1;
        } else {
            self.failed_commands += 1;
        }
        
        // Update min/max
        self.min_processing_time_us = self.min_processing_time_us.min(duration_us);
        self.max_processing_time_us = self.max_processing_time_us.max(duration_us);
        
        // Update average
        self.processing_times.push(duration_us);
        if self.processing_times.len() > METRICS_HISTORY_SIZE {
            self.processing_times.remove(0);
        }
        
        let sum: u64 = self.processing_times.iter().sum();
        self.avg_processing_time_us = sum / self.processing_times.len() as u64;
    }
    
    /// Get percentile processing time
    pub fn get_percentile(&self, percentile: f32) -> u64 {
        if self.processing_times.is_empty() {
            return 0;
        }
        
        let mut sorted = self.processing_times.clone();
        sorted.sort_unstable();
        
        let index = ((percentile / 100.0) * sorted.len() as f32) as usize;
        sorted[index.min(sorted.len() - 1)]
    }
    
    /// Check if performance is within target (<1ms per command average)
    pub fn is_within_performance_target(&self) -> bool {
        self.avg_processing_time_us < 1000 // 1ms = 1000us
    }
}

/// Entity inspection processor with comprehensive capabilities
pub struct EntityInspectionProcessor {
    /// Entity inspector with advanced features
    inspector: Arc<crate::server::entity_inspector::EntityInspector>,
}

impl EntityInspectionProcessor {
    /// Create new entity inspection processor
    pub fn new(inspector: Arc<crate::server::entity_inspector::EntityInspector>) -> Self {
        Self { inspector }
    }
}

#[async_trait]
impl DebugCommandProcessor for EntityInspectionProcessor {
    async fn process(&self, command: DebugCommand) -> Result<DebugResponse> {
        match command {
            DebugCommand::InspectEntity { entity_id, include_metadata, include_relationships } => {
                self.inspector.inspect_entity(
                    entity_id,
                    include_metadata.unwrap_or(false),
                    include_relationships.unwrap_or(false),
                ).await
            }
            DebugCommand::InspectBatch { entity_ids, include_metadata, include_relationships, limit } => {
                self.inspector.inspect_batch(
                    entity_ids,
                    include_metadata.unwrap_or(false),
                    include_relationships.unwrap_or(false),
                    limit,
                ).await
            }
            _ => Err(Error::DebugError("Unsupported command".to_string())),
        }
    }
    
    async fn validate(&self, command: &DebugCommand) -> Result<()> {
        match command {
            DebugCommand::InspectEntity { entity_id, .. } => {
                if *entity_id == 0 {
                    Err(Error::DebugError("Invalid entity ID: 0".to_string()))
                } else if *entity_id > 0xFFFF_FFFF {
                    Err(Error::DebugError("Entity ID too large".to_string()))
                } else {
                    Ok(())
                }
            }
            DebugCommand::InspectBatch { entity_ids, limit, .. } => {
                if entity_ids.is_empty() {
                    return Err(Error::DebugError("Entity IDs list cannot be empty".to_string()));
                }
                if entity_ids.contains(&0) {
                    return Err(Error::DebugError("Invalid entity ID: 0".to_string()));
                }
                if entity_ids.iter().any(|&id| id > 0xFFFF_FFFF) {
                    return Err(Error::DebugError("Entity ID too large".to_string()));
                }
                let actual_limit = limit.unwrap_or(crate::server::entity_inspector::MAX_BATCH_SIZE);
                if actual_limit > crate::server::entity_inspector::MAX_BATCH_SIZE {
                    return Err(Error::DebugError(format!(
                        "Batch limit too high (max: {})", 
                        crate::server::entity_inspector::MAX_BATCH_SIZE
                    )));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    
    fn estimate_processing_time(&self, _command: &DebugCommand) -> Duration {
        Duration::from_millis(10) // Entity inspection is typically fast
    }
    
    fn supports_command(&self, command: &DebugCommand) -> bool {
        matches!(command, DebugCommand::InspectEntity { .. } | DebugCommand::InspectBatch { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_debug_command_request_creation() {
        let command = DebugCommand::GetStatus;
        let request = DebugCommandRequest::new(
            command.clone(),
            "test-correlation".to_string(),
            Some(8),
        );
        
        assert_eq!(request.correlation_id, "test-correlation");
        assert_eq!(request.priority, 8);
        assert!(!request.is_timed_out());
        assert!(request.remaining_time().is_some());
    }
    
    #[tokio::test]
    async fn test_priority_queue_ordering() {
        let mut queue = PriorityQueue::new();
        
        let low_priority = DebugCommandRequest::new(
            DebugCommand::GetStatus,
            "low".to_string(),
            Some(1),
        );
        
        let high_priority = DebugCommandRequest::new(
            DebugCommand::GetStatus,
            "high".to_string(),
            Some(9),
        );
        
        queue.push(low_priority.clone());
        queue.push(high_priority.clone());
        
        // High priority should come first
        let first = queue.pop().unwrap();
        assert_eq!(first.correlation_id, "high");
        
        let second = queue.pop().unwrap();
        assert_eq!(second.correlation_id, "low");
    }
    
    #[tokio::test]
    async fn test_response_correlation_ttl() {
        let mut map = ResponseCorrelationMap::new();
        
        let response = DebugResponse::Status {
            version: "1.0.0".to_string(),
            active_sessions: 0,
            command_queue_size: 0,
            performance_overhead_percent: 0.0,
        };
        
        map.store(
            "test-id".to_string(),
            response.clone(),
            Duration::from_millis(100),
        );
        
        // Should be available immediately
        assert!(map.get("test-id").is_some());
        
        // Should expire after TTL
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(map.get("test-id").is_none());
    }
    
    #[test]
    fn test_metrics_recording() {
        let mut metrics = DebugMetrics::new();
        
        metrics.record_command(Duration::from_micros(500), true);
        metrics.record_command(Duration::from_micros(1500), false);
        metrics.record_command(Duration::from_micros(800), true);
        
        assert_eq!(metrics.total_commands, 3);
        assert_eq!(metrics.successful_commands, 2);
        assert_eq!(metrics.failed_commands, 1);
        assert_eq!(metrics.min_processing_time_us, 500);
        assert_eq!(metrics.max_processing_time_us, 1500);
        
        // Should be within performance target (avg < 1ms)
        assert!(metrics.is_within_performance_target()); // avg is ~933us which is < 1000us
    }
}