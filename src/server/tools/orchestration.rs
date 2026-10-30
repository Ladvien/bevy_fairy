use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::server::brp_client::BrpClient;
use crate::server::error::Result;
use crate::server::tool_orchestration::{ToolContext, ToolExecutor};

/// Tool executor for the observe tool
pub struct ObserveExecutor;

#[async_trait]
impl ToolExecutor for ObserveExecutor {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        _context: &mut ToolContext,
    ) -> Result<Value> {
        crate::server::tools::observe::handle(arguments, brp_client).await
    }
}

/// Tool executor for the experiment tool
pub struct ExperimentExecutor;

#[async_trait]
impl ToolExecutor for ExperimentExecutor {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        _context: &mut ToolContext,
    ) -> Result<Value> {
        crate::server::tools::experiment::handle(arguments, brp_client).await
    }
}

/// Tool executor for the hypothesis tool
pub struct HypothesisExecutor;

#[async_trait]
impl ToolExecutor for HypothesisExecutor {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        _context: &mut ToolContext,
    ) -> Result<Value> {
        crate::server::tools::hypothesis::handle(arguments, brp_client).await
    }
}

/// Tool executor for the replay tool
pub struct ReplayExecutor;

#[async_trait]
impl ToolExecutor for ReplayExecutor {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        _context: &mut ToolContext,
    ) -> Result<Value> {
        crate::server::tools::replay::handle(arguments, brp_client).await
    }
}

/// Tool executor for the anomaly tool
pub struct AnomalyExecutor;

#[async_trait]
impl ToolExecutor for AnomalyExecutor {
    async fn execute(
        &self,
        arguments: Value,
        brp_client: Arc<RwLock<BrpClient>>,
        _context: &mut ToolContext,
    ) -> Result<Value> {
        crate::server::tools::anomaly::handle(arguments, brp_client).await
    }
}

/// Create and configure a tool orchestrator with all available tools
pub fn create_orchestrator(
    brp_client: Arc<RwLock<BrpClient>>,
) -> crate::server::tool_orchestration::ToolOrchestrator {
    let mut orchestrator = crate::server::tool_orchestration::ToolOrchestrator::new(brp_client);

    // Register all tool executors
    orchestrator.register_tool("observe".to_string(), Arc::new(ObserveExecutor));
    orchestrator.register_tool("experiment".to_string(), Arc::new(ExperimentExecutor));
    orchestrator.register_tool("hypothesis".to_string(), Arc::new(HypothesisExecutor));
    orchestrator.register_tool("replay".to_string(), Arc::new(ReplayExecutor));
    orchestrator.register_tool("anomaly".to_string(), Arc::new(AnomalyExecutor));

    // Register common pipeline templates
    orchestrator.register_pipeline_template(
        crate::server::tool_orchestration::WorkflowDSL::observe_experiment_replay(),
    );

    orchestrator
}
