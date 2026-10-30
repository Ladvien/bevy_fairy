use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{debug, info};
use uuid::Uuid;

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};

/// Unique identifier for tool executions and results
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionId(Uuid);

impl ExecutionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl Default for ExecutionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared context between tool invocations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContext {
    /// Unique execution ID for this context
    pub execution_id: ExecutionId,
    /// Results from previous tool executions
    pub results: HashMap<String, ToolResult>,
    /// Shared variables accessible to all tools
    pub variables: HashMap<String, Value>,
    /// Metadata about the current execution
    pub metadata: ContextMetadata,
    /// Configuration for tool behavior
    pub config: ToolContextConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMetadata {
    pub created_at: SystemTime,
    pub last_modified: SystemTime,
    pub execution_count: usize,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContextConfig {
    /// Enable automatic recording for replay
    pub auto_record: bool,
    /// Enable automatic experiment triggering
    pub auto_experiment: bool,
    /// Cache results for reuse
    pub cache_results: bool,
    /// Maximum execution time for tools
    pub max_execution_time: Duration,
    /// Enable debug logging
    pub debug_mode: bool,
}

impl Default for ToolContextConfig {
    fn default() -> Self {
        Self {
            auto_record: true,
            auto_experiment: false,
            cache_results: true,
            max_execution_time: Duration::from_secs(300), // 5 minutes
            debug_mode: false,
        }
    }
}

impl ToolContext {
    pub fn new() -> Self {
        let now = SystemTime::now();
        Self {
            execution_id: ExecutionId::new(),
            results: HashMap::new(),
            variables: HashMap::new(),
            metadata: ContextMetadata {
                created_at: now,
                last_modified: now,
                execution_count: 0,
                tags: Vec::new(),
            },
            config: ToolContextConfig::default(),
        }
    }

    /// Add a result from a tool execution
    pub fn add_result(&mut self, tool_name: String, result: ToolResult) {
        self.results.insert(tool_name, result);
        self.metadata.last_modified = SystemTime::now();
        self.metadata.execution_count += 1;
    }

    /// Get a result by tool name
    pub fn get_result(&self, tool_name: &str) -> Option<&ToolResult> {
        self.results.get(tool_name)
    }

    /// Set a shared variable
    pub fn set_variable(&mut self, name: String, value: Value) {
        self.variables.insert(name, value);
        self.metadata.last_modified = SystemTime::now();
    }

    /// Get a shared variable
    pub fn get_variable(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    /// Clear all results and variables
    pub fn clear(&mut self) {
        self.results.clear();
        self.variables.clear();
        self.metadata.last_modified = SystemTime::now();
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub execution_id: ExecutionId,
    pub success: bool,
    pub output: Value,
    pub error: Option<String>,
    pub execution_time: Duration,
    pub timestamp: SystemTime,
    pub cache_key: Option<String>,
}

/// A single step in a tool pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStep {
    pub name: String,
    pub tool: String,
    pub arguments: Value,
    pub condition: Option<StepCondition>,
    pub retry_config: Option<RetryConfig>,
    pub timeout: Option<Duration>,
}

/// Condition for executing a pipeline step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepCondition {
    pub condition_type: ConditionType,
    pub reference: String,
    pub expected_value: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionType {
    /// Execute if previous step succeeded
    OnSuccess,
    /// Execute if previous step failed
    OnFailure,
    /// Execute if variable equals value
    VariableEquals,
    /// Execute if result contains key
    ResultContains,
    /// Always execute
    Always,
}

/// Retry configuration for pipeline steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_attempts: usize,
    pub backoff_type: BackoffType,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackoffType {
    Linear,
    Exponential,
    Fixed,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff_type: BackoffType::Exponential,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// A pipeline of tool executions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPipeline {
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<PipelineStep>,
    pub parallel_execution: bool,
    pub fail_fast: bool,
    pub created_at: SystemTime,
}

impl ToolPipeline {
    pub fn new(name: String) -> Self {
        Self {
            name,
            description: None,
            steps: Vec::new(),
            parallel_execution: false,
            fail_fast: true,
            created_at: SystemTime::now(),
        }
    }

    pub fn add_step(&mut self, step: PipelineStep) {
        self.steps.push(step);
    }

    pub fn with_parallel_execution(mut self, parallel: bool) -> Self {
        self.parallel_execution = parallel;
        self
    }

    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }
}

/// Result of pipeline execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    pub pipeline_name: String,
    pub execution_id: ExecutionId,
    pub success: bool,
    pub step_results: Vec<StepResult>,
    pub total_execution_time: Duration,
    pub context: ToolContext,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_name: String,
    pub success: bool,
    pub result: Option<ToolResult>,
    pub error: Option<String>,
    pub execution_time: Duration,
    pub retry_count: usize,
}

/// Message for actor-based tool coordination
#[derive(Debug)]
pub enum ToolMessage {
    Execute {
        tool: String,
        arguments: Value,
        context: ToolContext,
        response: oneshot::Sender<Result<ToolResult>>,
    },
    ExecutePipeline {
        pipeline: ToolPipeline,
        context: ToolContext,
        response: oneshot::Sender<Result<PipelineResult>>,
    },
    GetResult {
        cache_key: String,
        response: oneshot::Sender<Option<ToolResult>>,
    },
    Shutdown,
}

/// Actor for coordinating tool executions
pub struct ToolOrchestrator {
    /// Available tools
    tools: HashMap<String, Arc<dyn ToolExecutor>>,
    /// Result cache
    result_cache: HashMap<String, ToolResult>,
    /// Dependency graph for execution ordering
    dependency_graph: DependencyGraph,
    /// BRP client for tool execution
    brp_client: Arc<RwLock<BrpClient>>,
    /// Pipeline templates
    pipeline_templates: HashMap<String, ToolPipeline>,
}

impl ToolOrchestrator {
    pub fn new(brp_client: Arc<RwLock<BrpClient>>) -> Self {
        Self {
            tools: HashMap::new(),
            result_cache: HashMap::new(),
            dependency_graph: DependencyGraph::new(),
            brp_client,
            pipeline_templates: HashMap::new(),
        }
    }

    /// Register a tool executor
    pub fn register_tool(&mut self, name: String, executor: Arc<dyn ToolExecutor>) {
        self.tools.insert(name, executor);
    }

    /// Register a pipeline template
    pub fn register_pipeline_template(&mut self, pipeline: ToolPipeline) {
        self.pipeline_templates
            .insert(pipeline.name.clone(), pipeline);
    }

    /// Execute a single tool
    pub async fn execute_tool(
        &mut self,
        tool: String,
        arguments: Value,
        context: &mut ToolContext,
    ) -> Result<ToolResult> {
        let start_time = Instant::now();

        // Check cache first
        if context.config.cache_results {
            if let Some(cache_key) = self.generate_cache_key(&tool, &arguments) {
                if let Some(cached_result) = self.result_cache.get(&cache_key) {
                    info!("Using cached result for {} with key {}", tool, cache_key);
                    return Ok(cached_result.clone());
                }
            }
        }

        // Get tool executor
        let executor = self
            .tools
            .get(&tool)
            .ok_or_else(|| Error::Validation(format!("Tool '{tool}' not found")))?;

        // Execute with timeout
        let result = if let Some(timeout) = context
            .config
            .max_execution_time
            .checked_sub(Duration::ZERO)
        {
            tokio::time::timeout(
                timeout,
                executor.execute(arguments.clone(), self.brp_client.clone(), context),
            )
            .await
            .map_err(|_| Error::Validation(format!("Tool '{tool}' execution timed out")))?
        } else {
            executor
                .execute(arguments.clone(), self.brp_client.clone(), context)
                .await
        };

        let execution_time = start_time.elapsed();

        let tool_result = match result {
            Ok(output) => ToolResult {
                tool_name: tool.clone(),
                execution_id: context.execution_id,
                success: true,
                output,
                error: None,
                execution_time,
                timestamp: SystemTime::now(),
                cache_key: self.generate_cache_key(&tool, &arguments),
            },
            Err(e) => ToolResult {
                tool_name: tool.clone(),
                execution_id: context.execution_id,
                success: false,
                output: Value::Null,
                error: Some(e.to_string()),
                execution_time,
                timestamp: SystemTime::now(),
                cache_key: None,
            },
        };

        // Cache successful results
        if tool_result.success && context.config.cache_results {
            if let Some(ref cache_key) = tool_result.cache_key {
                self.result_cache
                    .insert(cache_key.clone(), tool_result.clone());
            }
        }

        // Add result to context
        context.add_result(tool.clone(), tool_result.clone());

        // Trigger automatic actions based on configuration
        self.handle_automatic_actions(&tool_result, context).await?;

        Ok(tool_result)
    }

    /// Execute a pipeline
    pub async fn execute_pipeline(
        &mut self,
        pipeline: ToolPipeline,
        mut context: ToolContext,
    ) -> Result<PipelineResult> {
        let start_time = Instant::now();
        let execution_id = ExecutionId::new();
        

        // Enforce execution bounds
        if pipeline.steps.len() > 100 {
            return Err(Error::Validation(
                "Pipeline too complex: maximum 100 steps allowed".to_string(),
            ));
        }

        let max_pipeline_time = Duration::from_secs(1800); // 30 minutes max
        let pipeline_result = tokio::time::timeout(max_pipeline_time, async {
            if pipeline.parallel_execution {
                // Execute steps in parallel
                self.execute_parallel_steps(&pipeline.steps, &mut context)
                    .await
            } else {
                // Execute steps sequentially
                let mut results = Vec::new();
                for step in &pipeline.steps {
                    let step_result = self.execute_step(step, &mut context).await;
                    let success = step_result.success;
                    results.push(step_result);

                    if !success && pipeline.fail_fast {
                        break;
                    }
                }
                Ok(results)
            }
        })
        .await;

        let step_results = match pipeline_result {
            Ok(Ok(results)) => results,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                return Err(Error::Validation(
                    "Pipeline execution timed out".to_string(),
                ))
            }
        };

        let total_execution_time = start_time.elapsed();
        let pipeline_success = step_results.iter().all(|r| r.success);

        Ok(PipelineResult {
            pipeline_name: pipeline.name,
            execution_id,
            success: pipeline_success,
            step_results,
            total_execution_time,
            context,
        })
    }

    /// Execute a single pipeline step
    pub(crate) async fn execute_step(&mut self, step: &PipelineStep, context: &mut ToolContext) -> StepResult {
        let start_time = Instant::now();

        // Check step condition
        if !self.should_execute_step(step, context) {
            return StepResult {
                step_name: step.name.clone(),
                success: true,
                result: None,
                error: Some("Step condition not met".to_string()),
                execution_time: start_time.elapsed(),
                retry_count: 0,
            };
        }

        let mut retry_count = 0;
        let max_attempts = step
            .retry_config
            .as_ref()
            .map(|r| r.max_attempts)
            .unwrap_or(1);

        loop {
            let result = self
                .execute_tool(step.tool.clone(), step.arguments.clone(), context)
                .await;

            match result {
                Ok(tool_result) => {
                    return StepResult {
                        step_name: step.name.clone(),
                        success: true,
                        result: Some(tool_result),
                        error: None,
                        execution_time: start_time.elapsed(),
                        retry_count,
                    };
                }
                Err(e) => {
                    retry_count += 1;

                    if retry_count >= max_attempts {
                        return StepResult {
                            step_name: step.name.clone(),
                            success: false,
                            result: None,
                            error: Some(e.to_string()),
                            execution_time: start_time.elapsed(),
                            retry_count,
                        };
                    }

                    // Wait before retry
                    if let Some(ref retry_config) = step.retry_config {
                        let delay = self.calculate_retry_delay(retry_config, retry_count);
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }
    }

    /// Execute steps in parallel
    async fn execute_parallel_steps(
        &mut self,
        steps: &[PipelineStep],
        context: &mut ToolContext,
    ) -> Result<Vec<StepResult>> {
        let mut handles = Vec::new();
        let shared_context = Arc::new(RwLock::new(context.clone()));

        for step in steps {
            if self.should_execute_step(step, context) {
                let step_clone = step.clone();
                let context_ref = shared_context.clone();
                let tools = self.tools.clone();
                let brp_client = self.brp_client.clone();

                let handle = tokio::spawn(async move {
                    // Use shared context for parallel execution
                    let mut local_context = {
                        let ctx = context_ref.read().await;
                        ctx.clone()
                    };

                    let result = Self::execute_step_standalone(
                        step_clone,
                        &mut local_context,
                        tools,
                        brp_client,
                    )
                    .await;

                    // Update shared context with results
                    if let Some(ref tool_result) = result.result {
                        let mut ctx = context_ref.write().await;
                        ctx.add_result(tool_result.tool_name.clone(), tool_result.clone());
                    }

                    result
                });

                handles.push(handle);
            }
        }

        let mut results = Vec::new();
        for handle in handles {
            results.push(
                handle
                    .await
                    .map_err(|e| Error::Validation(format!("Task join error: {e}")))?,
            );
        }

        // Update the original context with final state
        *context = shared_context.read().await.clone();

        Ok(results)
    }

    /// Standalone step execution for parallel processing
    async fn execute_step_standalone(
        step: PipelineStep,
        context: &mut ToolContext,
        tools: HashMap<String, Arc<dyn ToolExecutor>>,
        brp_client: Arc<RwLock<BrpClient>>,
    ) -> StepResult {
        let start_time = Instant::now();

        let executor = match tools.get(&step.tool) {
            Some(executor) => executor,
            None => {
                return StepResult {
                    step_name: step.name,
                    success: false,
                    result: None,
                    error: Some(format!("Tool '{}' not found", step.tool)),
                    execution_time: start_time.elapsed(),
                    retry_count: 0,
                };
            }
        };

        match executor.execute(step.arguments, brp_client, context).await {
            Ok(output) => {
                let tool_result = ToolResult {
                    tool_name: step.tool,
                    execution_id: context.execution_id,
                    success: true,
                    output,
                    error: None,
                    execution_time: start_time.elapsed(),
                    timestamp: SystemTime::now(),
                    cache_key: None,
                };

                StepResult {
                    step_name: step.name,
                    success: true,
                    result: Some(tool_result),
                    error: None,
                    execution_time: start_time.elapsed(),
                    retry_count: 0,
                }
            }
            Err(e) => StepResult {
                step_name: step.name,
                success: false,
                result: None,
                error: Some(e.to_string()),
                execution_time: start_time.elapsed(),
                retry_count: 0,
            },
        }
    }

    /// Check if a step should be executed based on its condition
    fn should_execute_step(&self, step: &PipelineStep, context: &ToolContext) -> bool {
        match &step.condition {
            None => true,
            Some(condition) => match condition.condition_type {
                ConditionType::Always => true,
                ConditionType::OnSuccess => {
                    if let Some(result) = context.get_result(&condition.reference) {
                        result.success
                    } else {
                        false
                    }
                }
                ConditionType::OnFailure => {
                    if let Some(result) = context.get_result(&condition.reference) {
                        !result.success
                    } else {
                        true
                    }
                }
                ConditionType::VariableEquals => {
                    if let Some(var_value) = context.get_variable(&condition.reference) {
                        condition.expected_value.as_ref() == Some(var_value)
                    } else {
                        false
                    }
                }
                ConditionType::ResultContains => {
                    if let Some(result) = context.get_result(&condition.reference) {
                        // Check if result output contains expected value
                        condition
                            .expected_value
                            .as_ref()
                            .map(|expected| {
                                result.output.to_string().contains(&expected.to_string())
                            })
                            .unwrap_or(false)
                    } else {
                        false
                    }
                }
            },
        }
    }

    /// Calculate retry delay based on backoff strategy
    fn calculate_retry_delay(&self, retry_config: &RetryConfig, attempt: usize) -> Duration {
        match retry_config.backoff_type {
            BackoffType::Fixed => retry_config.initial_delay,
            BackoffType::Linear => {
                let delay = retry_config.initial_delay * attempt as u32;
                std::cmp::min(delay, retry_config.max_delay)
            }
            BackoffType::Exponential => {
                let delay = retry_config.initial_delay * (2_u32.pow(attempt as u32 - 1));
                std::cmp::min(delay, retry_config.max_delay)
            }
        }
    }

    /// Generate cache key for tool execution
    fn generate_cache_key(&self, tool: &str, arguments: &Value) -> Option<String> {
        use sha2::{Digest, Sha256};

        // Create a more robust cache key using SHA256
        let mut hasher = Sha256::new();
        hasher.update(tool.as_bytes());
        hasher.update(b":");

        // Normalize JSON to ensure consistent hashing
        if let Ok(normalized) = serde_json::to_string(arguments) {
            hasher.update(normalized.as_bytes());
        } else {
            // Fallback to string representation
            hasher.update(arguments.to_string().as_bytes());
        }

        let result = hasher.finalize();
        Some(format!(
            "{}_{:x}",
            tool,
            u64::from_be_bytes([
                result[0], result[1], result[2], result[3], result[4], result[5], result[6],
                result[7]
            ])
        ))
    }

    /// Handle automatic actions based on tool results
    async fn handle_automatic_actions(
        &mut self,
        result: &ToolResult,
        context: &mut ToolContext,
    ) -> Result<()> {
        // Auto-record for replay if enabled
        if context.config.auto_record && result.success {
            // This would trigger recording - placeholder for now
            debug!("Auto-recording enabled for tool: {}", result.tool_name);
        }

        // Auto-experiment if enabled and observe tool found anomalies
        if context.config.auto_experiment && result.tool_name == "observe" && result.success {
            if let Some(anomalies) = result.output.get("anomalies") {
                if anomalies.as_array().map(|a| !a.is_empty()).unwrap_or(false) {
                    debug!("Anomalies detected, triggering automatic experiment");
                    // This would trigger an experiment - placeholder for now
                }
            }
        }

        Ok(())
    }

    /// Start the orchestrator actor
    pub async fn start_actor(mut self) -> mpsc::Sender<ToolMessage> {
        let (sender, mut receiver) = mpsc::channel(100);

        tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                match message {
                    ToolMessage::Execute {
                        tool,
                        arguments,
                        mut context,
                        response,
                    } => {
                        let result = self.execute_tool(tool, arguments, &mut context).await;
                        let _ = response.send(result);
                    }
                    ToolMessage::ExecutePipeline {
                        pipeline,
                        context,
                        response,
                    } => {
                        let result = self.execute_pipeline(pipeline, context).await;
                        let _ = response.send(result);
                    }
                    ToolMessage::GetResult {
                        cache_key,
                        response,
                    } => {
                        let result = self.result_cache.get(&cache_key).cloned();
                        let _ = response.send(result);
                    }
                    ToolMessage::Shutdown => {
                        info!("Tool orchestrator shutting down");
                        break;
                    }
                }
            }
        });

        sender
    }
}

/// Trait for tool executors
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        context: &mut ToolContext,
    ) -> Result<Value>;
}

/// Dependency graph for tool execution ordering
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    dependencies: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
        }
    }

    pub fn add_dependency(&mut self, tool: String, depends_on: String) {
        self.dependencies.entry(tool).or_default().push(depends_on);
    }

    pub fn get_execution_order(&self, tools: &[String]) -> Result<Vec<String>> {
        let mut visited = std::collections::HashSet::new();
        let mut visiting = std::collections::HashSet::new();
        let mut result = Vec::new();

        for tool in tools {
            if !visited.contains(tool) {
                self.topological_sort(tool, &mut visited, &mut visiting, &mut result)?;
            }
        }

        Ok(result)
    }

    fn topological_sort(
        &self,
        tool: &str,
        visited: &mut std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
        result: &mut Vec<String>,
    ) -> Result<()> {
        if visiting.contains(tool) {
            return Err(Error::Validation(format!(
                "Circular dependency detected involving: {tool}"
            )));
        }

        if visited.contains(tool) {
            return Ok(());
        }

        visiting.insert(tool.to_string());

        if let Some(deps) = self.dependencies.get(tool) {
            for dep in deps {
                self.topological_sort(dep, visited, visiting, result)?;
            }
        }

        visiting.remove(tool);
        visited.insert(tool.to_string());
        result.push(tool.to_string());

        Ok(())
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple workflow DSL for common patterns
pub struct WorkflowDSL;

impl WorkflowDSL {
    /// Create a simple observe -> experiment -> replay pipeline
    pub fn observe_experiment_replay() -> ToolPipeline {
        let mut pipeline = ToolPipeline::new("observe_experiment_replay".to_string());
        pipeline.description =
            Some("Observe game state, run experiments, then replay results".to_string());

        pipeline.add_step(PipelineStep {
            name: "observe".to_string(),
            tool: "observe".to_string(),
            arguments: json!({"query": "entities with Health < 50"}),
            condition: None,
            retry_config: Some(RetryConfig::default()),
            timeout: Some(Duration::from_secs(30)),
        });

        pipeline.add_step(PipelineStep {
            name: "experiment".to_string(),
            tool: "experiment".to_string(),
            arguments: json!({"type": "heal_entities", "target": "@observe.entities"}),
            condition: Some(StepCondition {
                condition_type: ConditionType::OnSuccess,
                reference: "observe".to_string(),
                expected_value: None,
            }),
            retry_config: Some(RetryConfig::default()),
            timeout: Some(Duration::from_secs(60)),
        });

        pipeline.add_step(PipelineStep {
            name: "replay".to_string(),
            tool: "replay".to_string(),
            arguments: json!({"action": "record"}),
            condition: Some(StepCondition {
                condition_type: ConditionType::OnSuccess,
                reference: "experiment".to_string(),
                expected_value: None,
            }),
            retry_config: None,
            timeout: Some(Duration::from_secs(120)),
        });

        pipeline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_id_creation() {
        let id1 = ExecutionId::new();
        let id2 = ExecutionId::new();
        assert_ne!(id1, id2);

        let id_str = id1.to_string();
        assert!(!id_str.is_empty());
    }

    #[test]
    fn test_tool_context_operations() {
        let mut context = ToolContext::new();

        // Test variable operations
        context.set_variable("test_var".to_string(), json!("test_value"));
        assert_eq!(context.get_variable("test_var"), Some(&json!("test_value")));

        // Test result operations
        let result = ToolResult {
            tool_name: "test_tool".to_string(),
            execution_id: ExecutionId::new(),
            success: true,
            output: json!({"result": "success"}),
            error: None,
            execution_time: Duration::from_millis(100),
            timestamp: SystemTime::now(),
            cache_key: Some("test_key".to_string()),
        };

        context.add_result("test_tool".to_string(), result);
        assert!(context.get_result("test_tool").is_some());
        assert_eq!(context.metadata.execution_count, 1);
    }

    #[test]
    fn test_pipeline_creation() {
        let mut pipeline = ToolPipeline::new("test_pipeline".to_string());

        pipeline.add_step(PipelineStep {
            name: "step1".to_string(),
            tool: "observe".to_string(),
            arguments: json!({}),
            condition: None,
            retry_config: None,
            timeout: None,
        });

        assert_eq!(pipeline.steps.len(), 1);
        assert_eq!(pipeline.steps[0].name, "step1");
    }

    #[test]
    fn test_dependency_graph() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("experiment".to_string(), "observe".to_string());
        graph.add_dependency("replay".to_string(), "experiment".to_string());

        let order = graph
            .get_execution_order(&[
                "replay".to_string(),
                "experiment".to_string(),
                "observe".to_string(),
            ])
            .unwrap();

        // Should be in dependency order: observe, experiment, replay
        assert_eq!(order, vec!["observe", "experiment", "replay"]);
    }

    #[test]
    fn test_circular_dependency_detection() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a".to_string(), "b".to_string());
        graph.add_dependency("b".to_string(), "a".to_string());

        let result = graph.get_execution_order(&["a".to_string(), "b".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_retry_delay_calculation() {
        let orchestrator = ToolOrchestrator::new(Arc::new(RwLock::new(
            crate::server::brp_client::BrpClient::new(&crate::server::config::Config::default()),
        )));

        let retry_config = RetryConfig {
            max_attempts: 3,
            backoff_type: BackoffType::Exponential,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
        };

        assert_eq!(
            orchestrator.calculate_retry_delay(&retry_config, 1),
            Duration::from_millis(100)
        );
        assert_eq!(
            orchestrator.calculate_retry_delay(&retry_config, 2),
            Duration::from_millis(200)
        );
        assert_eq!(
            orchestrator.calculate_retry_delay(&retry_config, 3),
            Duration::from_millis(400)
        );
    }

    #[test]
    fn test_workflow_dsl() {
        let pipeline = WorkflowDSL::observe_experiment_replay();

        assert_eq!(pipeline.name, "observe_experiment_replay");
        assert_eq!(pipeline.steps.len(), 3);
        assert_eq!(pipeline.steps[0].tool, "observe");
        assert_eq!(pipeline.steps[1].tool, "experiment");
        assert_eq!(pipeline.steps[2].tool, "replay");

        // Check conditions
        assert!(pipeline.steps[0].condition.is_none());
        assert!(pipeline.steps[1].condition.is_some());
        assert!(pipeline.steps[2].condition.is_some());
    }
}
