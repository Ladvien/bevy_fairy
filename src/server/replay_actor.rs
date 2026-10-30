//! Actor-based replay system to replace `Arc<RwLock<T>>` heavy replay.rs
//!
//! This module implements the actor model for replay functionality, eliminating
//! the need for shared mutable state through locks.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::playback_system::{DirectSync, PlaybackController};
use crate::server::recording_system::{RecordingConfig, RecordingState};
use crate::server::timeline_branching::{
    BranchId, MergeStrategy, Modification, TimelineBranchManager,
};

/// Messages that can be sent to the replay actor
#[derive(Debug)]
pub enum ReplayMessage {
    StartRecording {
        config: RecordingConfig,
        respond_to: oneshot::Sender<Result<ReplayStatus>>,
    },
    StopRecording {
        respond_to: oneshot::Sender<Result<ReplayStatus>>,
    },
    GetStatus {
        respond_to: oneshot::Sender<ReplayStatus>,
    },
    AddMarker {
        name: String,
        respond_to: oneshot::Sender<Result<()>>,
    },
    SaveRecording {
        path: PathBuf,
        respond_to: oneshot::Sender<Result<()>>,
    },
    LoadRecording {
        path: PathBuf,
        respond_to: oneshot::Sender<Result<()>>,
    },
    GetStats {
        respond_to: oneshot::Sender<ReplayStats>,
    },
    // Playback controls
    Play {
        respond_to: oneshot::Sender<Result<()>>,
    },
    Pause {
        respond_to: oneshot::Sender<Result<()>>,
    },
    Seek {
        position: f64,
        respond_to: oneshot::Sender<Result<()>>,
    },
    Step {
        frames: u32,
        respond_to: oneshot::Sender<Result<()>>,
    },
    SetSpeed {
        speed: f64,
        respond_to: oneshot::Sender<Result<()>>,
    },
    GetPlaybackStatus {
        respond_to: oneshot::Sender<PlaybackStatus>,
    },
    // Branch management
    CreateBranch {
        name: String,
        description: Option<String>,
        respond_to: oneshot::Sender<Result<BranchId>>,
    },
    ListBranches {
        respond_to: oneshot::Sender<Vec<BranchInfo>>,
    },
    SwitchBranch {
        branch_id: BranchId,
        respond_to: oneshot::Sender<Result<()>>,
    },
    AddModification {
        modification: Modification,
        respond_to: oneshot::Sender<Result<()>>,
    },
    MergeBranch {
        source: BranchId,
        target: BranchId,
        strategy: MergeStrategy,
        respond_to: oneshot::Sender<Result<()>>,
    },
    CompareBranches {
        branch1: BranchId,
        branch2: BranchId,
        respond_to: oneshot::Sender<BranchComparison>,
    },
    DeleteBranch {
        branch_id: BranchId,
        respond_to: oneshot::Sender<Result<()>>,
    },
    GetBranchTree {
        respond_to: oneshot::Sender<BranchTree>,
    },
}

/// Current replay system status
#[derive(Debug, Clone)]
pub struct ReplayStatus {
    pub recording_active: bool,
    pub playback_active: bool,
    pub current_frame: u64,
    pub total_frames: u64,
    pub recording_duration: Duration,
    pub sample_rate: f64,
}

/// Playback-specific status
#[derive(Debug, Clone)]
pub struct PlaybackStatus {
    pub state: PlaybackState,
    pub current_position: f64,
    pub playback_speed: f64,
    pub total_duration: Duration,
    pub loop_enabled: bool,
}

#[derive(Debug, Clone)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Seeking,
}

/// Statistics about recording/playback
#[derive(Debug, Clone)]
pub struct ReplayStats {
    pub total_recordings: usize,
    pub total_frames_recorded: u64,
    pub total_branches: usize,
    pub disk_usage_bytes: u64,
    pub memory_usage_bytes: u64,
    pub performance_metrics: PerformanceMetrics,
}

#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub avg_frame_processing_time: Duration,
    pub max_frame_processing_time: Duration,
    pub dropped_frames: u64,
    pub compression_ratio: f64,
}

/// Information about a timeline branch
#[derive(Debug, Clone)]
pub struct BranchInfo {
    pub id: BranchId,
    pub name: String,
    pub description: Option<String>,
    pub created_at: Instant,
    pub frame_count: u64,
    pub modifications: usize,
}

/// Result of comparing two branches
#[derive(Debug, Clone)]
pub struct BranchComparison {
    pub differences: Vec<BranchDifference>,
    pub similarity_score: f64,
    pub divergence_frame: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct BranchDifference {
    pub frame: u64,
    pub difference_type: DifferenceType,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum DifferenceType {
    ComponentChange,
    EntityChange,
    SystemChange,
    ResourceChange,
}

/// Tree representation of timeline branches
#[derive(Debug, Clone)]
pub struct BranchTree {
    pub nodes: Vec<BranchNode>,
    pub edges: Vec<BranchEdge>,
}

#[derive(Debug, Clone)]
pub struct BranchNode {
    pub branch_id: BranchId,
    pub name: String,
    pub frame_count: u64,
}

#[derive(Debug, Clone)]
pub struct BranchEdge {
    pub parent: BranchId,
    pub child: BranchId,
    pub merge_point: Option<u64>,
}

/// Internal state of the replay actor
struct ReplayActorState {
    recording_state: RecordingState,
    playback_controller: PlaybackController,
    branch_manager: TimelineBranchManager,
    brp_client: Arc<BrpClient>, // Note: no longer wrapped in RwLock
    current_recording_config: Option<RecordingConfig>,
    playback_status: PlaybackStatus,
    stats: ReplayStats,
}

/// The replay actor that manages all replay functionality
pub struct ReplayActor {
    receiver: mpsc::UnboundedReceiver<ReplayMessage>,
    state: ReplayActorState,
}

impl ReplayActor {
    pub fn new(
        receiver: mpsc::UnboundedReceiver<ReplayMessage>,
        brp_client: Arc<BrpClient>,
    ) -> Self {
        let state = ReplayActorState {
            recording_state: RecordingState::new(RecordingConfig::default()),
            playback_controller: PlaybackController::new(Box::new(DirectSync)),
            branch_manager: TimelineBranchManager::new(),
            brp_client,
            current_recording_config: None,
            playback_status: PlaybackStatus {
                state: PlaybackState::Stopped,
                current_position: 0.0,
                playback_speed: 1.0,
                total_duration: Duration::ZERO,
                loop_enabled: false,
            },
            stats: ReplayStats {
                total_recordings: 0,
                total_frames_recorded: 0,
                total_branches: 0,
                disk_usage_bytes: 0,
                memory_usage_bytes: 0,
                performance_metrics: PerformanceMetrics {
                    avg_frame_processing_time: Duration::ZERO,
                    max_frame_processing_time: Duration::ZERO,
                    dropped_frames: 0,
                    compression_ratio: 1.0,
                },
            },
        };

        Self { receiver, state }
    }

    /// Run the actor's message processing loop
    pub async fn run(mut self) {
        info!("Replay actor started");

        while let Some(message) = self.receiver.recv().await {
            debug!("Processing replay message: {:?}", message);
            self.handle_message(message).await;
        }

        info!("Replay actor stopped");
    }

    async fn handle_message(&mut self, message: ReplayMessage) {
        match message {
            ReplayMessage::StartRecording { config, respond_to } => {
                let result = self.start_recording(config).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::StopRecording { respond_to } => {
                let result = self.stop_recording().await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::GetStatus { respond_to } => {
                let status = self.get_status();
                let _ = respond_to.send(status);
            }
            ReplayMessage::AddMarker { name, respond_to } => {
                let result = self.add_marker(name).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::SaveRecording { path, respond_to } => {
                let result = self.save_recording(path).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::LoadRecording { path, respond_to } => {
                let result = self.load_recording(path).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::GetStats { respond_to } => {
                let stats = self.get_stats();
                let _ = respond_to.send(stats);
            }
            ReplayMessage::Play { respond_to } => {
                let result = self.play().await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::Pause { respond_to } => {
                let result = self.pause().await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::Seek { position, respond_to } => {
                let result = self.seek(position).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::Step { frames, respond_to } => {
                let result = self.step(frames).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::SetSpeed { speed, respond_to } => {
                let result = self.set_speed(speed).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::GetPlaybackStatus { respond_to } => {
                let status = self.get_playback_status();
                let _ = respond_to.send(status);
            }
            ReplayMessage::CreateBranch { name, description, respond_to } => {
                let result = self.create_branch(name, description).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::ListBranches { respond_to } => {
                let branches = self.list_branches();
                let _ = respond_to.send(branches);
            }
            ReplayMessage::SwitchBranch { branch_id, respond_to } => {
                let result = self.switch_branch(branch_id).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::AddModification { modification, respond_to } => {
                let result = self.add_modification(modification).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::MergeBranch { source, target, strategy, respond_to } => {
                let result = self.merge_branch(source, target, strategy).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::CompareBranches { branch1, branch2, respond_to } => {
                let comparison = self.compare_branches(branch1, branch2);
                let _ = respond_to.send(comparison);
            }
            ReplayMessage::DeleteBranch { branch_id, respond_to } => {
                let result = self.delete_branch(branch_id).await;
                let _ = respond_to.send(result);
            }
            ReplayMessage::GetBranchTree { respond_to } => {
                let tree = self.get_branch_tree();
                let _ = respond_to.send(tree);
            }
        }
    }

    // Implementation methods (simplified for brevity)
    async fn start_recording(&mut self, config: RecordingConfig) -> Result<ReplayStatus> {
        if !self.state.brp_client.is_connected() {
            return Err(Error::Connection("BRP client not connected".to_string()));
        }

        self.state.current_recording_config = Some(config.clone());
        self.state.recording_state = RecordingState::new(config.clone());
        
        info!("Recording started with config: {:?}", config);
        Ok(self.get_status())
    }

    async fn stop_recording(&mut self) -> Result<ReplayStatus> {
        self.state.current_recording_config = None;
        info!("Recording stopped");
        Ok(self.get_status())
    }

    fn get_status(&self) -> ReplayStatus {
        ReplayStatus {
            recording_active: self.state.current_recording_config.is_some(),
            playback_active: matches!(self.state.playback_status.state, PlaybackState::Playing),
            current_frame: 0,
            total_frames: 0,
            recording_duration: Duration::ZERO,
            sample_rate: self.state.current_recording_config
                .as_ref()
                .map(|c| c.sample_rate as f64)
                .unwrap_or(60.0),
        }
    }

    async fn add_marker(&mut self, _name: String) -> Result<()> {
        // Implementation would add marker to recording
        Ok(())
    }

    async fn save_recording(&mut self, _path: PathBuf) -> Result<()> {
        // Implementation would save recording to disk
        Ok(())
    }

    async fn load_recording(&mut self, _path: PathBuf) -> Result<()> {
        // Implementation would load recording from disk
        Ok(())
    }

    fn get_stats(&self) -> ReplayStats {
        self.state.stats.clone()
    }

    async fn play(&mut self) -> Result<()> {
        self.state.playback_status.state = PlaybackState::Playing;
        Ok(())
    }

    async fn pause(&mut self) -> Result<()> {
        self.state.playback_status.state = PlaybackState::Paused;
        Ok(())
    }

    async fn seek(&mut self, position: f64) -> Result<()> {
        self.state.playback_status.current_position = position;
        Ok(())
    }

    async fn step(&mut self, _frames: u32) -> Result<()> {
        // Implementation would step through frames
        Ok(())
    }

    async fn set_speed(&mut self, speed: f64) -> Result<()> {
        self.state.playback_status.playback_speed = speed;
        Ok(())
    }

    fn get_playback_status(&self) -> PlaybackStatus {
        self.state.playback_status.clone()
    }

    async fn create_branch(&mut self, _name: String, _description: Option<String>) -> Result<BranchId> {
        // Implementation would create new branch
        Ok(BranchId::new())
    }

    fn list_branches(&self) -> Vec<BranchInfo> {
        // Implementation would return list of branches
        Vec::new()
    }

    async fn switch_branch(&mut self, _branch_id: BranchId) -> Result<()> {
        // Implementation would switch to different branch
        Ok(())
    }

    async fn add_modification(&mut self, _modification: Modification) -> Result<()> {
        // Implementation would add modification to current branch
        Ok(())
    }

    async fn merge_branch(&mut self, _source: BranchId, _target: BranchId, _strategy: MergeStrategy) -> Result<()> {
        // Implementation would merge branches
        Ok(())
    }

    fn compare_branches(&self, _branch1: BranchId, _branch2: BranchId) -> BranchComparison {
        // Implementation would compare branches
        BranchComparison {
            differences: Vec::new(),
            similarity_score: 1.0,
            divergence_frame: None,
        }
    }

    async fn delete_branch(&mut self, _branch_id: BranchId) -> Result<()> {
        // Implementation would delete branch
        Ok(())
    }

    fn get_branch_tree(&self) -> BranchTree {
        // Implementation would return branch tree structure
        BranchTree {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

/// Handle to communicate with the replay actor
#[derive(Clone)]
pub struct ReplayActorHandle {
    sender: mpsc::UnboundedSender<ReplayMessage>,
}

impl ReplayActorHandle {
    pub fn new(sender: mpsc::UnboundedSender<ReplayMessage>) -> Self {
        Self { sender }
    }

    pub async fn start_recording(&self, config: RecordingConfig) -> Result<ReplayStatus> {
        let (respond_to, response) = oneshot::channel();
        let message = ReplayMessage::StartRecording { config, respond_to };
        
        self.sender.send(message).map_err(|_| Error::Internal("Replay actor disconnected".to_string()))?;
        response.await.map_err(|_| Error::Internal("Response channel closed".to_string()))?
    }

    pub async fn stop_recording(&self) -> Result<ReplayStatus> {
        let (respond_to, response) = oneshot::channel();
        let message = ReplayMessage::StopRecording { respond_to };
        
        self.sender.send(message).map_err(|_| Error::Internal("Replay actor disconnected".to_string()))?;
        response.await.map_err(|_| Error::Internal("Response channel closed".to_string()))?
    }

    pub async fn get_status(&self) -> Result<ReplayStatus> {
        let (respond_to, response) = oneshot::channel();
        let message = ReplayMessage::GetStatus { respond_to };
        
        self.sender.send(message).map_err(|_| Error::Internal("Replay actor disconnected".to_string()))?;
        response.await.map_err(|_| Error::Internal("Response channel closed".to_string()))
    }

    // Add similar methods for all other operations...
}

/// Static handle to the global replay actor
static REPLAY_ACTOR_HANDLE: std::sync::OnceLock<ReplayActorHandle> = std::sync::OnceLock::new();

/// Initialize the replay actor system
pub async fn initialize_replay_actor(brp_client: Arc<BrpClient>) -> Result<()> {
    let (sender, receiver) = mpsc::unbounded_channel();
    let handle = ReplayActorHandle::new(sender);
    
    // Store the handle globally
    REPLAY_ACTOR_HANDLE.set(handle).map_err(|_| Error::Internal("Replay actor already initialized".to_string()))?;
    
    // Spawn the actor
    let actor = ReplayActor::new(receiver, brp_client);
    tokio::spawn(actor.run());
    
    info!("Replay actor initialized successfully");
    Ok(())
}

/// Get the global replay actor handle
pub fn get_replay_actor_handle() -> Result<&'static ReplayActorHandle> {
    REPLAY_ACTOR_HANDLE.get().ok_or_else(|| Error::Internal("Replay actor not initialized".to_string()))
}