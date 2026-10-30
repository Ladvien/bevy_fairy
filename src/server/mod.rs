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
    pub use crate::server::experiment_system::{Action, ActionExecutor, ActionResult};
    pub use crate::server::hypothesis_system::{Hypothesis, TestResult, TestRunner};
    pub use crate::server::timeline_branching::{BranchId, TimelineBranchManager};
    pub use crate::server::recording_system::{Recording, RecordingState};
    pub use crate::server::playback_system::PlaybackState;
}


// Core functionality
pub mod error;
pub mod config;
pub mod circuit_breaker;

// Communication
pub mod brp;
pub mod brp_client;
pub mod brp_command_handler;
pub mod brp_validation;
pub mod debug_command_processor;
pub mod entity_inspector;
pub mod mcp_server_v2;
pub mod mcp_tools;

// Performance profiling and visual debugging

// Issue detection

// Performance budget monitoring


// Query and observation
pub mod query_parser;
pub mod semantic_analyzer;

// Experimentation and testing
pub mod experiment_system;
pub mod hypothesis_system;

// State management
pub mod recording_system;
pub mod playback_system;
pub mod timeline_branching;
pub mod state_diff;
pub mod replay_actor;

// Analysis and monitoring
pub mod anomaly_detector;
pub mod resource_manager;

// Infrastructure
pub mod tool_orchestration;
pub mod tools;

// Panic prevention and reliability testing

// Machine learning and pattern recognition (Epic BEVDBG-013)

// Bevy reflection integration (Epic BEVDBG-012)
pub mod bevy_reflection;

// Query performance optimization (Epic BEVDBG-014)
pub mod query_optimization;
pub mod parallel_query_executor;

// Production features

// Epic 6: Production features - Observability stack

// Legacy name for the `brp` module, removed hand-rolled `brp_messages` module.
pub use brp as brp_messages;
