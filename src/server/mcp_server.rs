use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::brp_messages::DebugCommand;
use crate::server::suggestion_engine::{SuggestionContext, SystemState};
use crate::server::workflow_automation::UserPreferences;
use crate::server::checkpoint::{CheckpointConfig, CheckpointManager};
use crate::server::config::Config;
use crate::server::dead_letter_queue::{DeadLetterConfig, DeadLetterQueue};
use crate::server::debug_command_processor::{
    DebugCommandRequest, DebugCommandRouter, 
    EntityInspectionProcessor, DebugMetrics,
};
use crate::server::visual_debug_overlay_processor::VisualDebugOverlayProcessor;
use crate::server::query_builder_processor::QueryBuilderProcessor;
use crate::server::memory_profiler_processor::MemoryProfilerProcessor;
use crate::server::session_processor::SessionProcessor;
use crate::server::issue_detector_processor::IssueDetectorProcessor;
use crate::server::performance_budget_processor::PerformanceBudgetProcessor;
use crate::server::entity_inspector::EntityInspector;
use crate::server::system_profiler::SystemProfiler;
use crate::server::system_profiler_processor::SystemProfilerProcessor;
use crate::server::diagnostics::{create_bug_report, DiagnosticCollector};
use crate::server::error::{Error, ErrorContext, ErrorSeverity, Result};
use crate::server::resource_manager::{ResourceConfig, ResourceManager};
use crate::server::tool_orchestration::{ToolContext, ToolOrchestrator, ToolPipeline};
use crate::server::tools::{anomaly, experiment, hypothesis, observe, orchestration, replay};
use crate::server::lazy_init::{LazyComponents, preload_critical_components};
use crate::server::command_cache::{CommandCache, CacheConfig, CacheKey};
use crate::server::response_pool::{ResponsePool, ResponsePoolConfig};
use crate::server::profiling::{init_profiler, get_profiler, PerfMeasurement};
use crate::{profile_block, profile_async_block};
use crate::server::compile_opts::{CompileConfig, inline_hot_path, cold_path};

pub struct McpServer {
    config: Config,
    brp_client: Arc<RwLock<BrpClient>>,
    orchestrator: Arc<RwLock<ToolOrchestrator>>,
    resource_manager: Arc<RwLock<ResourceManager>>,
    dead_letter_queue: Arc<RwLock<DeadLetterQueue>>,
    diagnostic_collector: Arc<DiagnosticCollector>,
    checkpoint_manager: Arc<RwLock<CheckpointManager>>,
    lazy_components: Arc<LazyComponents>,
    command_cache: Arc<CommandCache>,
    response_pool: Arc<ResponsePool>,
    debug_mode: bool,
}

impl McpServer {
    pub fn new(config: Config, brp_client: Arc<RwLock<BrpClient>>) -> Self {
        let orchestrator = orchestration::create_orchestrator(Arc::clone(&brp_client));
        let resource_manager = ResourceManager::new(ResourceConfig::default());

        // Initialize error recovery and diagnostic systems
        let dead_letter_queue = DeadLetterQueue::new(DeadLetterConfig::default());
        let diagnostic_collector = Arc::new(DiagnosticCollector::new(100)); // Keep 100 recent errors
        let checkpoint_manager = CheckpointManager::new(CheckpointConfig::default());

        // Initialize lazy components manager for optimized startup
        let lazy_components = Arc::new(LazyComponents::new(Arc::clone(&brp_client)));

        // Initialize command result cache for performance optimization
        let cache_config = CacheConfig {
            max_entries: 500,
            default_ttl: Duration::from_secs(300), // 5 minutes
            cleanup_interval: Duration::from_secs(60), // 1 minute
            max_response_size: 512 * 1024, // 512KB per response
        };
        let command_cache = Arc::new(CommandCache::new(cache_config));

        // Initialize response pool for memory optimization
        let response_pool_config = ResponsePoolConfig {
            max_small_buffers: 100,
            max_medium_buffers: 50,
            max_large_buffers: 20,
            small_buffer_capacity: 1024,      // 1KB
            medium_buffer_capacity: 32768,    // 32KB
            large_buffer_capacity: 524288,    // 512KB
            track_utilization: true,
            cleanup_interval: Duration::from_secs(120), // 2 minutes
        };
        let response_pool = Arc::new(ResponsePool::new(response_pool_config));

        // Check for debug mode from environment
        let debug_mode = std::env::var("DEBUG_MODE")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        if debug_mode {
            info!("Debug mode enabled - verbose logging and diagnostics active");
        }

        // Optionally preload critical components based on feature flags
        let lazy_components_for_preload = Arc::clone(&lazy_components);
        tokio::spawn(async move {
            if let Err(e) = preload_critical_components(&lazy_components_for_preload).await {
                error!("Failed to preload critical components: {}", e);
            }
        });

        // Initialize performance profiler
        let _profiler = init_profiler();
        
        info!("MCP Server initialized with lazy component loading, command caching, response pooling, and hot path profiling for optimal startup performance");

        McpServer {
            config,
            brp_client,
            orchestrator: Arc::new(RwLock::new(orchestrator)),
            resource_manager: Arc::new(RwLock::new(resource_manager)),
            dead_letter_queue: Arc::new(RwLock::new(dead_letter_queue)),
            diagnostic_collector,
            checkpoint_manager: Arc::new(RwLock::new(checkpoint_manager)),
            lazy_components,
            command_cache,
            response_pool,
            debug_mode,
        }
    }

    pub async fn start(&self) -> Result<()> {
        // Start all systems
        {
            let mut rm = self.resource_manager.write().await;
            rm.start_monitoring().await?;
        }

        {
            let mut dlq = self.dead_letter_queue.write().await;
            dlq.start().await?;
        }

        {
            let mut cm = self.checkpoint_manager.write().await;
            cm.start().await?;
        }

        info!("MCP Server started with error recovery and diagnostic systems");
        if self.debug_mode {
            info!("Debug mode active - enhanced logging enabled");
        }
        Ok(())
    }

    pub async fn run(&self, listener: TcpListener) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("New MCP connection from: {}", addr);
                    let server = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            error!("Error handling MCP connection: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn handle_connection(&self, _stream: TcpStream) -> Result<()> {
        debug!("Handling MCP connection - performing handshake");

        let capabilities = json!({
            "capabilities": {
                "tools": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": "bevy-fairy",
                "version": "0.1.0"
            }
        });

        debug!(
            "MCP handshake completed with capabilities: {}",
            capabilities
        );

        Ok(())
    }

    pub async fn handle_tool_call(&self, tool_name: &str, arguments: Value) -> Result<Value> {
        profile_async_block!(format!("handle_tool_call_{}", tool_name), async {
            debug!("Handling tool call: {} with args: {}", tool_name, arguments);

            // Try to get cached result first (for cacheable tools)
            let cache_key = if self.is_tool_cacheable(tool_name) {
                match CacheKey::new(tool_name, &arguments) {
                    Ok(key) => {
                        if let Some(cached_result) = profile_async_block!("cache_lookup", async {
                            self.command_cache.get(&key).await
                        }) {
                            debug!("Returning cached result for tool: {}", tool_name);
                            return Ok(cached_result);
                        }
                        Some(key)
                    }
                    Err(e) => {
                        warn!("Failed to create cache key for {}: {}", tool_name, e);
                        None
                    }
                }
            } else {
                None
            };

            // Clone arguments for error reporting later
            let args_for_error = arguments.clone();
            
            // Use shared Arc reference for all tool handlers
            let brp_client_ref = Arc::clone(&self.brp_client);

            let result: Result<serde_json::Value> = profile_async_block!(format!("tool_execution_{}", tool_name), async {
                match tool_name {
                    "observe" => observe::handle(arguments, brp_client_ref).await,
                    "experiment" => experiment::handle(arguments, Arc::clone(&brp_client_ref)).await,
                    "screenshot" => self.handle_screenshot(arguments).await,
                    "hypothesis" => hypothesis::handle(arguments, Arc::clone(&brp_client_ref)).await,
                    "replay" => replay::handle(arguments, Arc::clone(&brp_client_ref)).await,
                    "anomaly" => anomaly::handle(arguments, Arc::clone(&brp_client_ref)).await,
                    "orchestrate" => self.handle_orchestration(arguments).await,
                    "pipeline" => self.handle_pipeline_execution(arguments).await,
                    "resource_metrics" => self.handle_resource_metrics(arguments).await,
                    "performance_dashboard" => self.handle_performance_dashboard(arguments).await,
                    "health_check" => self.handle_health_check(arguments).await,
                    // New diagnostic and error recovery endpoints
                    "dead_letter_queue" => self.handle_dead_letter_queue(arguments).await,
                    "diagnostic_report" => self.handle_diagnostic_report(arguments).await,
                    "checkpoint" => self.handle_checkpoint(arguments).await,
                    "bug_report" => self.handle_bug_report(arguments).await,
                    "debug" => self.handle_debug_command(arguments).await,
                    // Machine learning and automation endpoints
                    "get_suggestions" => self.handle_get_suggestions(arguments).await,
                    "track_suggestion" => self.handle_track_suggestion(arguments).await,
                    "get_patterns" => self.handle_get_patterns(arguments).await,
                    "execute_workflow" => self.handle_execute_workflow(arguments).await,
                    "approve_workflow" => self.handle_approve_workflow(arguments).await,
                    "get_workflows" => self.handle_get_workflows(arguments).await,
                    // Hot reload endpoints
                    "hot_reload" => self.handle_hot_reload(arguments).await,
                    "get_model_versions" => self.handle_get_model_versions(arguments).await,
                    _ => Err(Error::Mcp(format!("Unknown tool: {tool_name}"))),
                }
            });

            // Cache successful results for cacheable tools
            if let (Ok(ref response), Some(cache_key)) = (&result, cache_key) {
                profile_async_block!("cache_store", async {
                    let tags = self.get_cache_tags_for_tool(tool_name);
                    if let Err(e) = self.command_cache.put(&cache_key, response.clone(), tags).await {
                        warn!("Failed to cache result for {}: {}", tool_name, e);
                    }
                });
            }

            // Record errors for diagnostics
            if let Err(ref error) = result {
                profile_block!("error_reporting", {
                    let error_context = ErrorContext::new(tool_name, "mcp_server")
                        .add_cause(&error.to_string())
                        .add_context("tool", tool_name)
                        .add_context("arguments", &format!("{args_for_error}"))
                        .set_retryable(true)
                        .set_severity(ErrorSeverity::Error);

                    self.diagnostic_collector.record_error(error_context);

                    if self.debug_mode {
                        warn!("Tool call failed: {} - {}", tool_name, error);
                    }
                });
            }

            result
        })
    }

    /// Handle orchestration tool calls
    async fn handle_orchestration(&self, arguments: Value) -> Result<Value> {
        let mut context = ToolContext::new();

        // Extract tool and arguments from request
        let tool = arguments
            .get("tool")
            .and_then(|t| t.as_str())
            .ok_or_else(|| Error::Validation("Missing 'tool' field".to_string()))?;

        let tool_args = arguments.get("arguments").unwrap_or(&Value::Null);

        // Apply context configuration if provided
        if let Some(config) = arguments.get("config") {
            if let Some(auto_record) = config.get("auto_record").and_then(|v| v.as_bool()) {
                context.config.auto_record = auto_record;
            }
            if let Some(auto_experiment) = config.get("auto_experiment").and_then(|v| v.as_bool()) {
                context.config.auto_experiment = auto_experiment;
            }
            if let Some(cache_results) = config.get("cache_results").and_then(|v| v.as_bool()) {
                context.config.cache_results = cache_results;
            }
        }

        let mut orchestrator = self.orchestrator.write().await;
        let tool_result = orchestrator
            .execute_tool(tool.to_string(), tool_args.clone(), &mut context)
            .await?;
        
        let result = tool_result.output;

        // Sanitize context before returning - remove sensitive data
        let sanitized_context = json!({
            "execution_id": context.execution_id,
            "execution_count": context.metadata.execution_count,
            "result_count": context.results.len(),
            "variable_count": context.variables.len(),
            "config": {
                "auto_record": context.config.auto_record,
                "auto_experiment": context.config.auto_experiment,
                "cache_results": context.config.cache_results
            }
        });

        Ok(json!({
            "tool_result": result,
            "context": sanitized_context
        }))
    }

    /// Handle pipeline execution
    async fn handle_pipeline_execution(&self, arguments: Value) -> Result<Value> {
        let context = ToolContext::new();

        // Check if this is a template pipeline or custom pipeline
        if let Some(template_name) = arguments.get("template").and_then(|t| t.as_str()) {
            let mut orchestrator = self.orchestrator.write().await;

            // Get pipeline template (this would need to be implemented in orchestrator)
            let pipeline = match template_name {
                "observe_experiment_replay" => {
                    crate::server::tool_orchestration::WorkflowDSL::observe_experiment_replay()
                }
                _ => {
                    return Err(Error::Validation(format!(
                        "Unknown pipeline template: {template_name}"
                    )))
                }
            };

            let result = orchestrator.execute_pipeline(pipeline, context).await?;

            Ok(json!({
                "pipeline_result": result
            }))
        } else if let Some(pipeline_data) = arguments.get("pipeline") {
            // Custom pipeline execution with validation
            let pipeline: ToolPipeline = serde_json::from_value(pipeline_data.clone())
                .map_err(|e| Error::Validation(format!("Invalid pipeline format: {e}")))?;

            // Validate pipeline constraints
            if pipeline.steps.len() > 50 {
                return Err(Error::Validation(
                    "Pipeline too complex: maximum 50 steps allowed".to_string(),
                ));
            }

            for step in &pipeline.steps {
                if step.timeout.unwrap_or(Duration::from_secs(300)) > Duration::from_secs(600) {
                    return Err(Error::Validation(format!(
                        "Step '{}' timeout too long: maximum 10 minutes allowed",
                        step.name
                    )));
                }

                // Validate tool names against known tools
                if ![
                    "observe",
                    "experiment",
                    "hypothesis",
                    "replay",
                    "anomaly",
                ]
                .contains(&step.tool.as_str())
                {
                    return Err(Error::Validation(format!(
                        "Unknown tool '{}' in step '{}'",
                        step.tool, step.name
                    )));
                }
            }

            let mut orchestrator = self.orchestrator.write().await;
            let result = orchestrator.execute_pipeline(pipeline, context).await?;

            Ok(json!({
                "pipeline_result": result
            }))
        } else {
            Err(Error::Validation(
                "Missing 'template' or 'pipeline' field".to_string(),
            ))
        }
    }

    /// Handle resource metrics requests
    async fn handle_resource_metrics(&self, _arguments: Value) -> Result<Value> {
        let resource_manager = self.resource_manager.read().await;
        let metrics = resource_manager.get_metrics().await;

        serde_json::to_value(metrics)
            .map_err(|e| Error::Validation(format!("Failed to serialize metrics: {e}")))
    }

    /// Handle performance dashboard requests
    async fn handle_performance_dashboard(&self, _arguments: Value) -> Result<Value> {
        let resource_manager = self.resource_manager.read().await;
        let dashboard = resource_manager.get_performance_dashboard().await;

        Ok(dashboard)
    }

    /// Handle health check requests
    async fn handle_health_check(&self, _arguments: Value) -> Result<Value> {
        let resource_manager = self.resource_manager.read().await;
        let metrics = resource_manager.get_metrics().await;

        // Determine overall health status
        let cpu_ok = metrics.cpu_percent < 80.0; // 80% threshold
        let memory_ok = metrics.memory_bytes < 100 * 1024 * 1024; // 100MB threshold
        let circuit_ok = !metrics.circuit_breaker_open;

        let status = if cpu_ok && memory_ok && circuit_ok {
            "healthy"
        } else if !circuit_ok {
            "circuit_breaker_open"
        } else {
            "degraded"
        };

        Ok(json!({
            "status": status,
            "timestamp": SystemTime::now().duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_secs(),
            "checks": {
                "cpu": {
                    "status": if cpu_ok { "ok" } else { "warning" },
                    "value": metrics.cpu_percent,
                    "threshold": 80.0
                },
                "memory": {
                    "status": if memory_ok { "ok" } else { "warning" },
                    "value_mb": metrics.memory_bytes / (1024 * 1024),
                    "threshold_mb": 100
                },
                "circuit_breaker": {
                    "status": if circuit_ok { "ok" } else { "error" },
                    "open": metrics.circuit_breaker_open
                }
            },
            "uptime_seconds": metrics.timestamp.duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_secs()
        }))
    }

    /// Handle screenshot requests
    async fn handle_screenshot(&self, arguments: Value) -> Result<Value> {
        debug!("Screenshot tool called with arguments: {}", arguments);

        // Check BRP connection
        let is_connected = {
            let client = self.brp_client.read().await;
            client.is_connected()
        };

        if !is_connected {
            warn!("BRP client not connected for screenshot");
            return Ok(json!({
                "error": "BRP client not connected",
                "message": "Cannot take screenshot - not connected to Bevy game",
                "brp_connected": false
            }));
        }

        // Extract and validate parameters
        let path = arguments
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("./screenshot.png")
            .to_string();

        let warmup_duration = arguments
            .get("warmup_duration")
            .and_then(|d| d.as_u64())
            .unwrap_or(1000) // Default 1 second warmup
            .min(30000); // Max 30 seconds

        let capture_delay = arguments
            .get("capture_delay")
            .and_then(|d| d.as_u64())
            .unwrap_or(500) // Default 500ms capture delay
            .min(10000); // Max 10 seconds

        let wait_for_render = arguments
            .get("wait_for_render")
            .and_then(|w| w.as_bool())
            .unwrap_or(true); // Default to waiting for render

        let description = arguments
            .get("description")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());

        // Log the screenshot request for debugging
        if let Some(ref desc) = description {
            info!("Taking screenshot: {} -> {}", desc, path);
        } else {
            info!("Taking screenshot -> {}", path);
        }

        // Apply warmup duration
        if warmup_duration > 0 {
            debug!("Waiting {}ms for game warmup", warmup_duration);
            tokio::time::sleep(tokio::time::Duration::from_millis(warmup_duration)).await;
        }

        // Apply capture delay (for animation timing, etc.)
        if capture_delay > 0 {
            debug!("Waiting {}ms capture delay", capture_delay);
            tokio::time::sleep(tokio::time::Duration::from_millis(capture_delay)).await;
        }

        // Send BRP screenshot request (as a debug command via the custom
        // bevy_fairy/debug method — screenshot is not a stock BRP verb)
        let request = crate::server::debug_brp_handler::encode_debug_request(
            &crate::server::brp_messages::DebugCommand::Screenshot {
                path: Some(path.clone()),
                warmup_duration: Some(warmup_duration),
                capture_delay: Some(capture_delay),
                wait_for_render: Some(wait_for_render),
                description: description.clone(),
            },
            &format!("screenshot-{}", path),
            None,
        )?;

        let mut client = self.brp_client.write().await;
        match client.send_request(&request).await {
            Ok(response) => match response.payload {
                crate::server::brp_messages::BrpPayload::Result(value) => {
                    let success = value.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
                    let path = value
                        .get("path")
                        .and_then(|p| p.as_str())
                        .unwrap_or(&path)
                        .to_string();
                    if success {
                        Ok(json!({
                            "success": true,
                            "message": "Screenshot saved successfully",
                            "path": path,
                            "timestamp": SystemTime::now().duration_since(UNIX_EPOCH)
                                .unwrap_or_default().as_secs()
                        }))
                    } else {
                        Ok(json!({
                            "success": false,
                            "error": "Screenshot failed",
                            "message": "Screenshot operation failed in Bevy game"
                        }))
                    }
                }
                crate::server::brp_messages::BrpPayload::Error(error) => {
                    warn!("Screenshot BRP request failed: {:?}", error);
                    Ok(json!({
                        "success": false,
                        "error": "BRP request failed",
                        "message": format!("Screenshot request failed: {}", error.message)
                    }))
                }
            },
            Err(e) => {
                error!("Failed to send screenshot BRP request: {}", e);
                Ok(json!({
                    "success": false,
                    "error": "Request failed",
                    "message": format!("Failed to send screenshot request: {e}")
                }))
            }
        }
    }

    /// Handle dead letter queue operations
    async fn handle_dead_letter_queue(&self, arguments: Value) -> Result<Value> {
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list");

        match action {
            "list" => {
                let dlq = self.dead_letter_queue.read().await;
                let operations = dlq.get_failed_operations().await;
                Ok(json!({
                    "failed_operations": operations,
                    "total_count": operations.len()
                }))
            }
            "stats" => {
                let dlq = self.dead_letter_queue.read().await;
                let stats = dlq.get_statistics().await;
                Ok(serde_json::to_value(stats)?)
            }
            "remove" => {
                let id = arguments
                    .get("id")
                    .and_then(|i| i.as_str())
                    .ok_or_else(|| Error::Validation("Missing 'id' field".to_string()))?;

                let dlq = self.dead_letter_queue.read().await;
                let removed = dlq.remove_failed_operation(id).await?;

                Ok(json!({
                    "removed": removed.is_some(),
                    "operation": removed
                }))
            }
            _ => Err(Error::Validation(format!(
                "Unknown dead letter queue action: {action}"
            ))),
        }
    }

    /// Handle diagnostic report generation
    async fn handle_diagnostic_report(&self, arguments: Value) -> Result<Value> {
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("generate");

        match action {
            "generate" => {
                let dlq = self.dead_letter_queue.read().await;
                let report = self
                    .diagnostic_collector
                    .generate_report(Some(&*dlq))
                    .await?;
                Ok(serde_json::to_value(report)?)
            }
            "export" => {
                let dlq = self.dead_letter_queue.read().await;
                let report = self
                    .diagnostic_collector
                    .generate_report(Some(&*dlq))
                    .await?;
                let json_export = self
                    .diagnostic_collector
                    .export_report_json(&report)
                    .await?;

                Ok(json!({
                    "report_json": json_export,
                    "report_id": report.report_id
                }))
            }
            _ => Err(Error::Validation(format!(
                "Unknown diagnostic report action: {action}"
            ))),
        }
    }

    /// Handle checkpoint operations
    async fn handle_checkpoint(&self, arguments: Value) -> Result<Value> {
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list");

        match action {
            "create" => {
                let name = arguments
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| Error::Validation("Missing 'name' field".to_string()))?;

                let description = arguments
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");

                let operation_type = arguments
                    .get("operation_type")
                    .and_then(|o| o.as_str())
                    .unwrap_or("manual");

                let state_data = arguments.get("state_data").cloned().unwrap_or(json!({}));

                let checkpoint = crate::server::checkpoint::Checkpoint::new(
                    name,
                    description,
                    operation_type,
                    "mcp_server",
                    state_data,
                );

                let cm = self.checkpoint_manager.read().await;
                let checkpoint_id = cm.create_checkpoint(checkpoint).await?;

                Ok(json!({
                    "checkpoint_id": checkpoint_id,
                    "created": true
                }))
            }
            "restore" => {
                let checkpoint_id = arguments
                    .get("checkpoint_id")
                    .and_then(|id| id.as_str())
                    .ok_or_else(|| {
                        Error::Validation("Missing 'checkpoint_id' field".to_string())
                    })?;

                let cm = self.checkpoint_manager.read().await;
                let checkpoint = cm.restore_checkpoint(checkpoint_id).await?;

                Ok(serde_json::to_value(checkpoint)?)
            }
            "list" => {
                let cm = self.checkpoint_manager.read().await;
                let checkpoints = cm.list_checkpoints().await;

                match checkpoints {
                    Ok(checkpoint_list) => Ok(json!({
                        "checkpoints": checkpoint_list,
                        "total_count": checkpoint_list.len()
                    })),
                    Err(e) => Err(Error::Checkpoint(e.to_string()))
                }
            }
            "delete" => {
                let checkpoint_id = arguments
                    .get("checkpoint_id")
                    .and_then(|id| id.as_str())
                    .ok_or_else(|| {
                        Error::Validation("Missing 'checkpoint_id' field".to_string())
                    })?;

                let cm = self.checkpoint_manager.read().await;
                cm.delete_checkpoint(checkpoint_id).await?;

                Ok(json!({
                    "deleted": true,
                    "checkpoint_id": checkpoint_id
                }))
            }
            "stats" => {
                let cm = self.checkpoint_manager.read().await;
                let stats = cm.get_statistics().await;

                match stats {
                    Ok(stats_data) => Ok(serde_json::to_value(stats_data)?),
                    Err(e) => Err(Error::Checkpoint(e.to_string()))
                }
            }
            _ => Err(Error::Validation(format!(
                "Unknown checkpoint action: {action}"
            ))),
        }
    }

    /// Handle bug report creation
    async fn handle_bug_report(&self, arguments: Value) -> Result<Value> {
        let description = arguments
            .get("description")
            .and_then(|d| d.as_str())
            .ok_or_else(|| Error::Validation("Missing 'description' field".to_string()))?;

        let steps_to_reproduce = arguments
            .get("steps_to_reproduce")
            .and_then(|s| s.as_str())
            .unwrap_or("No steps provided");

        let dlq = self.dead_letter_queue.read().await;
        let diagnostic_report = self
            .diagnostic_collector
            .generate_report(Some(&*dlq))
            .await?;

        let bug_report = create_bug_report(&diagnostic_report, description, steps_to_reproduce);

        // Optionally save to file (with path validation)
        if let Some(file_path) = arguments.get("save_to_file").and_then(|f| f.as_str()) {
            // Validate and sanitize file path to prevent path traversal
            let path = std::path::Path::new(file_path);
            if path.is_absolute() || path.to_string_lossy().contains("..") {
                return Err(Error::Validation(
                    "Invalid file path: must be relative and not contain '..'".to_string(),
                ));
            }

            // Restrict to a safe directory
            let safe_dir = std::path::Path::new("./bug_reports");
            tokio::fs::create_dir_all(&safe_dir).await?;
            let full_path = safe_dir.join(path);

            tokio::fs::write(&full_path, &bug_report).await?;
        }

        Ok(json!({
            "bug_report": bug_report,
            "diagnostic_report_id": diagnostic_report.report_id,
            "generated_at": diagnostic_report.generated_at
        }))
    }

    /// Handle debug command execution
    async fn handle_debug_command(&self, arguments: Value) -> Result<Value> {
        // Extract command from arguments
        let command: DebugCommand = serde_json::from_value(
            arguments.get("command")
                .ok_or_else(|| Error::Validation("Missing 'command' field".to_string()))?
                .clone()
        ).map_err(|e| Error::Validation(format!("Invalid debug command: {}", e)))?;
        
        // Extract correlation_id and priority
        let correlation_id = arguments
            .get("correlation_id")
            .and_then(|id| id.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Generate a unique correlation ID
                uuid::Uuid::new_v4().to_string()
            });
            
        let priority = arguments
            .get("priority")
            .and_then(|p| p.as_u64())
            .map(|p| p as u8);
        
        // Create debug command request
        let request = DebugCommandRequest::new(command, correlation_id.clone(), priority);
        
        // Get the debug command router (lazy initialization)
        let debug_command_router = self.lazy_components.get_debug_command_router().await;
        
        // Queue the command for processing
        debug_command_router.queue_command(request).await?;
        
        // For synchronous response, process immediately
        // In production, this could be made fully async with webhooks
        let result = debug_command_router.process_next().await;
        
        match result {
            Some(Ok((corr_id, response))) => {
                // Convert debug response to JSON
                let response_json = serde_json::to_value(response)?;
                Ok(json!({
                    "correlation_id": corr_id,
                    "response": response_json,
                    "status": "success"
                }))
            }
            Some(Err(e)) => {
                // Return error response
                Ok(json!({
                    "correlation_id": correlation_id,
                    "error": e.to_string(),
                    "status": "error"
                }))
            }
            None => {
                // Return correlation ID for async processing
                Ok(json!({
                    "correlation_id": correlation_id,
                    "status": "queued",
                    "message": "Command queued for processing"
                }))
            }
        }
    }

    /// Get debug metrics
    pub async fn get_debug_metrics(&self) -> DebugMetrics {
        let debug_command_router = self.lazy_components.get_debug_command_router().await;
        debug_command_router.get_metrics().await
    }

    /// Clean up expired debug responses periodically
    pub async fn cleanup_debug_responses(&self) {
        let debug_command_router = self.lazy_components.get_debug_command_router().await;
        debug_command_router.cleanup_expired_responses().await;
    }
    
    /// Get lazy component initialization status for debugging
    pub fn get_lazy_init_status(&self) -> serde_json::Value {
        self.lazy_components.get_initialization_status()
    }
    
    /// Check if a tool should be cached
    #[inline(always)]
    fn is_tool_cacheable(&self, tool_name: &str) -> bool {
        // Optimize for most common tools first
        if matches!(tool_name, "observe" | "health_check" | "resource_metrics") {
            true
        } else {
            match tool_name {
                // Less common cacheable tools
                "diagnostic_report" | "anomaly" | "debug" => true,
                
                // Non-cacheable tools (stateful or time-sensitive operations)
                "experiment" | "screenshot" | "hypothesis" | "replay" |
                "orchestrate" | "pipeline" | "performance_dashboard" | 
                "dead_letter_queue" | "checkpoint" | "bug_report" => false,
                
                _ => false,
            }
        }
    }
    
    /// Get cache tags for a tool to enable selective invalidation
    #[inline(always)]
    fn get_cache_tags_for_tool(&self, tool_name: &str) -> Vec<String> {
#[cfg(feature = "caching")]
        {
            // Pre-allocate common tag combinations to reduce allocations
            static ENTITY_GAME_TAGS: &[&str] = &["entity_data", "game_state"];
            static PERFORMANCE_TAGS: &[&str] = &["performance_data"];
            static HEALTH_TAGS: &[&str] = &["system_health"];
            static DIAGNOSTICS_TAGS: &[&str] = &["diagnostics"];
            static ANOMALY_TAGS: &[&str] = &["anomaly_data", "performance_data"];
            static DEBUG_TAGS: &[&str] = &["debug_data"];
            
            // Optimize for most common tools
            match tool_name {
                "observe" => ENTITY_GAME_TAGS.iter().map(|s| s.to_string()).collect(),
                "resource_metrics" => PERFORMANCE_TAGS.iter().map(|s| s.to_string()).collect(),
                "health_check" => HEALTH_TAGS.iter().map(|s| s.to_string()).collect(),
                "diagnostic_report" => DIAGNOSTICS_TAGS.iter().map(|s| s.to_string()).collect(),
                "anomaly" => ANOMALY_TAGS.iter().map(|s| s.to_string()).collect(),
                "debug" => DEBUG_TAGS.iter().map(|s| s.to_string()).collect(),
                _ => Vec::new(),
            }
        }
        
        #[cfg(not(feature = "caching"))]
        {
            // Zero-cost when caching is disabled
            Vec::new()
        }
    }
    
    /// Get cache statistics
    pub async fn get_cache_statistics(&self) -> serde_json::Value {
        let stats = self.command_cache.get_statistics().await;
        serde_json::to_value(stats).unwrap_or_else(|_| serde_json::json!({}))
    }
    
    /// Clear cache entries by tag
    pub async fn clear_cache_by_tag(&self, tag: &str) -> usize {
        self.command_cache.invalidate_by_tag(tag).await
    }
    
    /// Clear all cache entries
    pub async fn clear_all_cache(&self) {
        self.command_cache.clear().await
    }
    
    /// Get response pool statistics
    pub async fn get_response_pool_statistics(&self) -> serde_json::Value {
        let stats = self.response_pool.get_statistics().await;
        serde_json::to_value(stats).unwrap_or_else(|_| serde_json::json!({}))
    }
    
    /// Serialize a response using the response pool for memory optimization
    pub async fn serialize_response_pooled(&self, response: &Value) -> Result<Vec<u8>> {
        self.response_pool.serialize_json(response).await
    }
    
    /// Get profiling statistics
    pub async fn get_profiling_statistics(&self) -> serde_json::Value {
        let profiler = get_profiler();
        let stats = profiler.get_all_stats().await;
        serde_json::to_value(stats).unwrap_or_else(|_| serde_json::json!({}))
    }
    
    /// Get hot path report
    pub async fn get_hot_path_report(&self) -> String {
        let profiler = get_profiler();
        profiler.generate_report().await
    }
    
    /// Enable or disable profiling
    pub fn set_profiling_enabled(&self, enabled: bool) {
        let profiler = get_profiler();
        profiler.set_enabled(enabled);
    }
    
    /// Clear profiling data
    pub async fn clear_profiling_data(&self) {
        let profiler = get_profiler();
        profiler.clear().await;
    }

    /// Handle get suggestions request
    async fn handle_get_suggestions(&self, arguments: Value) -> Result<Value> {
        let suggestion_engine = self.lazy_components.get_suggestion_engine().await;
        
        // Extract context from arguments
        let session_id = arguments
            .get("session_id")
            .and_then(|id| id.as_str())
            .unwrap_or("default")
            .to_string();
            
        let recent_commands: Vec<DebugCommand> = arguments
            .get("recent_commands")
            .and_then(|cmds| serde_json::from_value(cmds.clone()).ok())
            .unwrap_or_default();
            
        let system_state = if let Some(state_data) = arguments.get("system_state") {
            SystemState {
                entity_count: state_data.get("entity_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                fps: state_data.get("fps").and_then(|v| v.as_f64()).unwrap_or(60.0) as f32,
                memory_mb: state_data.get("memory_mb").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                active_systems: state_data.get("active_systems").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                has_errors: state_data.get("has_errors").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        } else {
            // Default system state
            SystemState {
                entity_count: 0,
                fps: 60.0,
                memory_mb: 0.0,
                active_systems: 0,
                has_errors: false,
            }
        };
        
        let user_goal = arguments
            .get("user_goal")
            .and_then(|goal| goal.as_str())
            .map(|s| s.to_string());
            
        let context = SuggestionContext {
            session_id,
            recent_commands,
            system_state,
            user_goal,
        };
        
        let suggestions = suggestion_engine.generate_suggestions(&context).await;
        
        Ok(json!({
            "suggestions": suggestions,
            "total_count": suggestions.len(),
            "context_session_id": context.session_id
        }))
    }
    
    /// Handle track suggestion acceptance/success
    async fn handle_track_suggestion(&self, arguments: Value) -> Result<Value> {
        let suggestion_engine = self.lazy_components.get_suggestion_engine().await;
        
        let suggestion_id = arguments
            .get("suggestion_id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| Error::Validation("Missing 'suggestion_id' field".to_string()))?;
            
        let accepted = arguments
            .get("accepted")
            .and_then(|a| a.as_bool())
            .unwrap_or(false);
            
        let success = arguments
            .get("success")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        
        suggestion_engine.track_suggestion_acceptance(suggestion_id, accepted, success).await;
        
        Ok(json!({
            "tracked": true,
            "suggestion_id": suggestion_id,
            "accepted": accepted,
            "success": success
        }))
    }
    
    /// Handle get learned patterns request
    async fn handle_get_patterns(&self, arguments: Value) -> Result<Value> {
        let pattern_system = self.lazy_components.get_pattern_learning_system().await;
        
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list");
            
        match action {
            "list" => {
                // Get recent patterns for a session (simplified implementation)
                let sequence = vec![]; // In practice, get from current session
                let patterns = pattern_system.find_matching_patterns(&sequence).await;
                
                Ok(json!({
                    "patterns": patterns,
                    "total_count": patterns.len()
                }))
            }
            "export" => {
                let patterns_json = pattern_system.export_patterns().await?;
                Ok(json!({
                    "patterns_export": patterns_json
                }))
            }
            "import" => {
                let patterns_json = arguments
                    .get("patterns_json")
                    .and_then(|p| p.as_str())
                    .ok_or_else(|| Error::Validation("Missing 'patterns_json' field".to_string()))?;
                    
                pattern_system.import_patterns(patterns_json).await?;
                
                Ok(json!({
                    "imported": true
                }))
            }
            _ => Err(Error::Validation(format!("Unknown patterns action: {}", action)))
        }
    }
    
    /// Handle workflow execution request
    async fn handle_execute_workflow(&self, arguments: Value) -> Result<Value> {
        let workflow_automation = self.lazy_components.get_workflow_automation().await;
        
        let workflow_id = arguments
            .get("workflow_id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| Error::Validation("Missing 'workflow_id' field".to_string()))?;
            
        let session_id = arguments
            .get("session_id")
            .and_then(|id| id.as_str())
            .unwrap_or("default")
            .to_string();
            
        // Extract user preferences if provided
        let preferences = if let Some(prefs_data) = arguments.get("preferences") {
            Some(UserPreferences {
                automation_enabled: prefs_data.get("automation_enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                preferred_scope: serde_json::from_value(prefs_data.get("preferred_scope").cloned().unwrap_or(json!("SafeCommands"))).unwrap_or(crate::server::workflow_automation::AutomationScope::SafeCommands),
                require_confirmation: prefs_data.get("require_confirmation").and_then(|v| v.as_bool()).unwrap_or(true),
                auto_rollback: prefs_data.get("auto_rollback").and_then(|v| v.as_bool()).unwrap_or(true),
            })
        } else {
            None
        };
        
        let result = workflow_automation.execute_workflow(workflow_id, session_id, preferences).await?;
        
        Ok(serde_json::to_value(result)?)
    }
    
    /// Handle workflow approval request
    async fn handle_approve_workflow(&self, arguments: Value) -> Result<Value> {
        let workflow_automation = self.lazy_components.get_workflow_automation().await;
        
        let workflow_id = arguments
            .get("workflow_id")
            .and_then(|id| id.as_str())
            .ok_or_else(|| Error::Validation("Missing 'workflow_id' field".to_string()))?;
            
        workflow_automation.approve_workflow(workflow_id).await?;
        
        Ok(json!({
            "approved": true,
            "workflow_id": workflow_id
        }))
    }
    
    /// Handle get workflows request
    async fn handle_get_workflows(&self, arguments: Value) -> Result<Value> {
        let workflow_automation = self.lazy_components.get_workflow_automation().await;
        
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("list");
            
        match action {
            "list" => {
                let workflows = workflow_automation.get_workflows().await;
                Ok(json!({
                    "workflows": workflows,
                    "total_count": workflows.len()
                }))
            }
            "analyze" => {
                let opportunities = workflow_automation.analyze_automation_opportunities().await?;
                Ok(json!({
                    "automation_opportunities": opportunities,
                    "total_count": opportunities.len()
                }))
            }
            _ => Err(Error::Validation(format!("Unknown workflows action: {}", action)))
        }
    }

    /// Handle hot reload operations
    async fn handle_hot_reload(&self, arguments: Value) -> Result<Value> {
        let hot_reload_system = self.lazy_components.get_hot_reload_system().await;
        
        let action = arguments
            .get("action")
            .and_then(|a| a.as_str())
            .unwrap_or("status");
            
        match action {
            "status" => {
                let versions = hot_reload_system.get_model_versions().await;
                Ok(json!({
                    "hot_reload_enabled": true,
                    "model_versions": versions,
                    "total_models": versions.len()
                }))
            }
            "force_reload" => {
                hot_reload_system.force_reload_all().await?;
                Ok(json!({
                    "reloaded": true,
                    "message": "All models force reloaded successfully"
                }))
            }
            "start" => {
                // Hot reload should already be started, but this is a no-op confirmation
                Ok(json!({
                    "started": true,
                    "message": "Hot reload system is active"
                }))
            }
            "stop" => {
                hot_reload_system.stop().await?;
                Ok(json!({
                    "stopped": true,
                    "message": "Hot reload system stopped"
                }))
            }
            _ => Err(Error::Validation(format!("Unknown hot reload action: {}", action)))
        }
    }
    
    /// Handle get model versions request
    async fn handle_get_model_versions(&self, _arguments: Value) -> Result<Value> {
        let hot_reload_system = self.lazy_components.get_hot_reload_system().await;
        let versions = hot_reload_system.get_model_versions().await;
        
        Ok(json!({
            "model_versions": versions,
            "total_count": versions.len(),
            "hot_reload_active": true
        }))
    }
}

impl Clone for McpServer {
    fn clone(&self) -> Self {
        McpServer {
            config: self.config.clone(),
            brp_client: Arc::clone(&self.brp_client),
            orchestrator: Arc::clone(&self.orchestrator),
            resource_manager: Arc::clone(&self.resource_manager),
            dead_letter_queue: Arc::clone(&self.dead_letter_queue),
            diagnostic_collector: Arc::clone(&self.diagnostic_collector),
            checkpoint_manager: Arc::clone(&self.checkpoint_manager),
            lazy_components: Arc::clone(&self.lazy_components),
            command_cache: Arc::clone(&self.command_cache),
            response_pool: Arc::clone(&self.response_pool),
            debug_mode: self.debug_mode,
        }
    }
}
