//! The MCP server half of `bevy_fairy`.
//!
//! This is the half an agent talks to: it speaks the Model Context Protocol over stdio, and
//! talks to a running Bevy game over the Bevy Remote Protocol. It is a separate process from
//! the game, and it does not link the plugin -- a game can be inspected by Bevy's own BRP
//! methods without this crate's plugin, and this server can drive any game that exposes BRP.
//!
//! Everything here is behind the `server` feature. A game that only wants the plugin compiles
//! none of it.

// Re-export commonly used types
pub mod prelude {
    //! Common imports for typical usage
    pub use crate::server::brp_client::BrpClient;
    pub use crate::server::brp_messages::{BrpRequest, BrpResponse};
    pub use crate::server::error::{Error, Result};
    pub use crate::server::query_parser::{QueryParser, RegexQueryParser};
    pub use crate::server::query_builder::{
        QueryBuilder, QueryCostEstimator, QueryOptimizer, QueryValidator,
    };
    pub use crate::server::query_builder_processor::QueryBuilderProcessor;
    pub use crate::server::mcp_server::McpServer;
    pub use crate::server::experiment_system::{Action, ActionExecutor, ActionResult};
    pub use crate::server::hypothesis_system::{Hypothesis, TestResult, TestRunner};
    pub use crate::server::timeline_branching::{BranchId, TimelineBranchManager};
    pub use crate::server::recording_system::{Recording, RecordingState};
    pub use crate::server::playback_system::PlaybackState;
    pub use crate::server::session_manager::{DebugSession, SessionManager, SessionManagerConfig};
    pub use crate::server::session_processor::SessionProcessor;
}


// Core functionality
pub mod error;
pub mod config;
pub mod circuit_breaker;
pub mod connection_pool;
pub mod heartbeat;

// Communication
pub mod brp;
pub mod brp_client;
pub mod brp_client_v2;
pub mod brp_command_handler;
pub mod brp_integration;
pub mod brp_validation;
pub mod debug_brp_handler;
pub mod debug_command_processor;
pub mod entity_inspector;
pub mod mcp_server;
pub mod mcp_server_v2;
pub mod mcp_tools;
pub mod query_builder_processor;

// Performance profiling and visual debugging
pub mod system_profiler;
pub mod system_profiler_processor;
pub mod memory_profiler;
pub mod memory_profiler_processor;
pub mod visual_debug_overlay;
pub mod visual_debug_overlay_processor;

// Issue detection
pub mod issue_detector;
pub mod issue_detector_processor;

// Performance budget monitoring
pub mod performance_budget;
pub mod performance_budget_processor;

#[cfg(feature = "visual_overlays")]
pub mod visual_overlays;

// Query and observation
pub mod query_parser;
pub mod query_builder;
pub mod semantic_analyzer;

// Experimentation and testing
pub mod experiment_system;
pub mod hypothesis_system;
pub mod stress_test_system;

// State management
pub mod recording_system;
pub mod playback_system;
pub mod timeline_branching;
pub mod checkpoint;
pub mod state_diff;
pub mod session_manager;
pub mod session_processor;
pub mod replay_actor;

// Analysis and monitoring
pub mod anomaly_detector;
pub mod diagnostics;
pub mod resource_manager;

// Infrastructure
pub mod tool_orchestration;
pub mod dead_letter_queue;
pub mod lazy_init;
pub mod command_cache;
pub mod response_pool;
pub mod profiling;
pub mod compile_opts;
pub mod memory_optimization_tracker;
pub mod memory_pools;
pub mod deadlock_detector;
pub mod lock_contention_benchmark;
pub mod benchmark_runner;
pub mod brp_client_refactored;
pub mod tools;

// Panic prevention and reliability testing
#[cfg(test)]
pub mod panic_prevention_tests;

// Machine learning and pattern recognition (Epic BEVDBG-013)
pub mod pattern_learning;
pub mod suggestion_engine;
pub mod workflow_automation;
pub mod hot_reload;

// Bevy reflection integration (Epic BEVDBG-012)
pub mod bevy_reflection;

// Query performance optimization (Epic BEVDBG-014)
pub mod query_optimization;
pub mod parallel_query_executor;
pub mod query_optimization_guide;

// Production features
pub mod bevy_observability_integration;

// Epic 6: Production features - Observability stack

// Legacy name for the `brp` module, removed hand-rolled `brp_messages` module.
pub use brp as brp_messages;
