use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::playback_system::{DirectSync, PlaybackController};
use crate::server::recording_system::{RecordingBuffer, RecordingConfig, RecordingState};
use crate::server::timeline_branching::{
    BranchId, MergeStrategy, Modification, ModificationLayer, TimelineBranchManager,
};

// Global recording state using std::sync::OnceLock
static RECORDING_STATE: OnceLock<RecordingState> = OnceLock::new();
static PLAYBACK_CONTROLLER: OnceLock<Arc<RwLock<PlaybackController>>> = OnceLock::new();
static BRANCH_MANAGER: OnceLock<Arc<RwLock<TimelineBranchManager>>> = OnceLock::new();

fn get_recording_state() -> &'static RecordingState {
    RECORDING_STATE.get_or_init(|| RecordingState::new(RecordingConfig::default()))
}

fn get_playback_controller() -> &'static Arc<RwLock<PlaybackController>> {
    PLAYBACK_CONTROLLER.get_or_init(|| {
        Arc::new(RwLock::new(PlaybackController::new(Box::new(DirectSync))))
    })
}

fn get_branch_manager() -> &'static Arc<RwLock<TimelineBranchManager>> {
    BRANCH_MANAGER.get_or_init(|| Arc::new(RwLock::new(TimelineBranchManager::new())))
}

/// Handle replay tool requests
pub async fn handle(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    debug!("Replay tool called with arguments: {}", arguments);

    let action = arguments
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or("status");

    info!("Processing replay action: {}", action);

    match action {
        "record" => handle_record(arguments, brp_client).await,
        "stop" => handle_stop(arguments, brp_client).await,
        "status" => handle_status(arguments, brp_client).await,
        "marker" => handle_marker(arguments, brp_client).await,
        "save" => handle_save(arguments, brp_client).await,
        "load" => handle_load(arguments, brp_client).await,
        "stats" => handle_stats(arguments, brp_client).await,
        "play" => handle_play(arguments, brp_client).await,
        "pause" => handle_pause(arguments, brp_client).await,
        "seek" => handle_seek(arguments, brp_client).await,
        "step" => handle_step(arguments, brp_client).await,
        "set_speed" => handle_set_speed(arguments, brp_client).await,
        "playback_status" => handle_playback_status(arguments, brp_client).await,
        "create_branch" => handle_create_branch(arguments, brp_client).await,
        "list_branches" => handle_list_branches(arguments, brp_client).await,
        "switch_branch" => handle_switch_branch(arguments, brp_client).await,
        "add_modification" => handle_add_modification(arguments, brp_client).await,
        "merge_branch" => handle_merge_branch(arguments, brp_client).await,
        "compare_branches" => handle_compare_branches(arguments, brp_client).await,
        "delete_branch" => handle_delete_branch(arguments, brp_client).await,
        "branch_tree" => handle_branch_tree(arguments, brp_client).await,
        _ => Ok(json!({
            "error": "Unknown action",
            "message": format!("Unknown action: {}", action),
            "available_actions": [
                "record", "stop", "status", "marker", "save", "load", "stats",
                "play", "pause", "seek", "step", "set_speed", "playback_status",
                "create_branch", "list_branches", "switch_branch", "add_modification",
                "merge_branch", "compare_branches", "delete_branch", "branch_tree"
            ]
        })),
    }
}

/// Handle record action - start recording
async fn handle_record(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    // Check connection
    let is_connected = {
        let client = brp_client.read().await;
        client.is_connected()
    };

    if !is_connected {
        warn!("BRP client not connected");
        return Ok(json!({
            "error": "BRP client not connected",
            "message": "Cannot start recording - not connected to Bevy game",
            "brp_connected": false
        }));
    }

    // Parse configuration
    let config = match parse_recording_config(&arguments) {
        Ok(config) => config,
        Err(message) => {
            return Ok(json!({
                "error": "invalid sample_rate",
                "message": message,
                "example": {"action": "record", "sample_rate": 10.0},
            }));
        }
    };
    let sample_rate = config.sample_rate;
    let max_buffer_size = config.max_buffer_size;
    let compression = config.compression;
    let checksums = config.checksums;

    // Start recording
    let mut buffer = get_recording_state().buffer.write().await;

    if buffer.is_recording() {
        return Ok(json!({
            "error": "Already recording",
            "message": "Recording is already in progress",
            "recording": true
        }));
    }

    // Update configuration if provided
    *buffer = RecordingBuffer::new(config);
    buffer.start_recording();

    // Start background recording task
    let buffer_clone = get_recording_state().buffer.clone();
    let brp_clone = brp_client.clone();

    let poll_interval = poll_interval(sample_rate);
    tokio::spawn(async move {
        loop {
            {
                let mut buffer = buffer_clone.write().await;
                if !buffer.is_recording() {
                    break;
                }

                let mut client = brp_clone.write().await;
                if let Err(e) = buffer.record_frame(&mut client).await {
                    error!("Failed to record frame: {}", e);
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    });

    Ok(json!({
        "success": true,
        "message": "Recording started",
        "config": {
            "sample_rate": sample_rate,
            "max_buffer_size": max_buffer_size,
            "compression": compression,
            "checksums": checksums,
        },
        "captures": ["entity states"],
        "not_captured": {
            "events": "BRP reports no events, so the recording holds entity states only. An empty \
                       event list in a frame means \"not observable\", not \"nothing happened\"."
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle stop action - stop recording
async fn handle_stop(_arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let mut buffer = get_recording_state().buffer.write().await;

    if !buffer.is_recording() {
        return Ok(json!({
            "error": "Not recording",
            "message": "No recording in progress",
            "recording": false
        }));
    }

    buffer.stop_recording();
    let stats = buffer.get_stats();
    let frames = buffer.get_frames();
    // What the recording actually holds, so a caller can see it is not empty and can see what it
    // left out.
    let entities_recorded: usize = frames.iter().map(|frame| frame.entities.len()).sum();
    let frames_with_entities = frames.iter().filter(|frame| !frame.entities.is_empty()).count();

    Ok(json!({
        "success": true,
        "message": "Recording stopped",
        "stats": {
            "frame_count": stats.frame_count,
            "duration_seconds": stats.duration.as_secs(),
            "marker_count": stats.marker_count,
            "entities_recorded": entities_recorded,
            "frames_with_entities": frames_with_entities,
        },
        "captures": ["entity states"],
        "not_captured": {
            "events": "BRP reports no events, so the recording holds entity states only. An empty \
                       event list in a frame means \"not observable\", not \"nothing happened\"."
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle status action - get recording status
async fn handle_status(_arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let buffer = get_recording_state().buffer.read().await;
    let stats = buffer.get_stats();

    let is_connected = {
        let client = brp_client.read().await;
        client.is_connected()
    };

    Ok(json!({
        "recording": stats.is_recording,
        "brp_connected": is_connected,
        "stats": {
            "frame_count": stats.frame_count,
            "delta_frame_count": stats.delta_frame_count,
            "full_frame_count": stats.full_frame_count,
            "marker_count": stats.marker_count,
            "duration_seconds": stats.duration.as_secs(),
            "buffer_usage": stats.buffer_usage,
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle marker action - add a marker
async fn handle_marker(arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let mut buffer = get_recording_state().buffer.write().await;

    if !buffer.is_recording() {
        return Ok(json!({
            "error": "Not recording",
            "message": "Cannot add marker when not recording",
            "recording": false
        }));
    }

    let name = arguments
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("marker")
        .to_string();

    let description = arguments
        .get("description")
        .and_then(|d| d.as_str())
        .map(String::from);

    buffer.add_marker(name.clone(), description.clone());

    Ok(json!({
        "success": true,
        "message": "Marker added",
        "marker": {
            "name": name,
            "description": description,
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle save action - save recording to file
async fn handle_save(arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let buffer = get_recording_state().buffer.read().await;

    if buffer.is_recording() {
        return Ok(json!({
            "error": "Still recording",
            "message": "Stop recording before saving",
            "recording": true
        }));
    }

    let filename = arguments
        .get("filename")
        .and_then(|f| f.as_str())
        .unwrap_or("recording.bevy");

    let path = PathBuf::from(filename);

    match buffer.save_to_file(&path) {
        Ok(()) => Ok(json!({
            "success": true,
            "message": "Recording saved",
            "filename": filename,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to save recording: {}", e);
            Ok(json!({
                "error": "Save failed",
                "message": format!("Failed to save recording: {}", e),
            }))
        }
    }
}

/// Handle load action - load recording from file
async fn handle_load(arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let filename = arguments
        .get("filename")
        .and_then(|f| f.as_str())
        .ok_or_else(|| Error::Validation("Missing 'filename' parameter".to_string()))?;

    let path = PathBuf::from(filename);

    match RecordingBuffer::load_from_file(&path) {
        Ok(recording) => {
            let mut timeline = get_recording_state().timeline.write().await;

            let total_frames = recording.total_frames;
            let duration = recording.duration;
            let marker_count = recording.markers.len();

            // Load into timeline for navigation
            timeline.load_recording(recording.clone());

            // Also load into playback controller
            let controller = get_playback_controller().read().await;
            controller.load_recording(recording.clone()).await?;

            // Load into branch manager
            let mut branch_manager = get_branch_manager().write().await;
            branch_manager.set_base_recording(recording);

            Ok(json!({
                "success": true,
                "message": "Recording loaded for playback",
                "filename": filename,
                "stats": {
                    "total_frames": total_frames,
                    "duration_seconds": duration.as_secs(),
                    "marker_count": marker_count,
                },
                "timestamp": chrono::Utc::now().to_rfc3339()
            }))
        }
        Err(e) => {
            error!("Failed to load recording: {}", e);
            Ok(json!({
                "error": "Load failed",
                "message": format!("Failed to load recording: {}", e),
            }))
        }
    }
}

/// Handle stats action - get detailed statistics
async fn handle_stats(_arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let buffer = get_recording_state().buffer.read().await;
    let stats = buffer.get_stats();

    let timeline = get_recording_state().timeline.read().await;
    let markers = timeline.markers();

    Ok(json!({
        "recording_stats": {
            "is_recording": stats.is_recording,
            "frame_count": stats.frame_count,
            "delta_frame_count": stats.delta_frame_count,
            "full_frame_count": stats.full_frame_count,
            "marker_count": stats.marker_count,
            "duration_seconds": stats.duration.as_secs(),
            "buffer_usage": stats.buffer_usage,
        },
        "timeline_stats": {
            "has_recording": timeline.recording.is_some(),
            "current_frame": timeline.current_frame,
            "cached_frames": timeline.cached_frames.len(),
            "markers": markers.iter().map(|m| {
                json!({
                    "name": m.name,
                    "frame": m.frame_number,
                    "description": m.description,
                })
            }).collect::<Vec<_>>(),
        },
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Parse recording configuration from arguments
fn parse_recording_config(arguments: &Value) -> std::result::Result<RecordingConfig, String> {
    let mut config = RecordingConfig::default();

    if let Some(rate) = arguments.get("sample_rate").and_then(|r| r.as_f64()) {
        // 1 / rate becomes the minimum gap between frames; zero, negative or non-finite rates
        // used to panic the recorder task, which left `status` saying "recording" with no frames.
        if !rate.is_finite() || rate <= 0.0 {
            return Err(format!(
                "sample_rate is {rate}; it must be a positive number of samples per second"
            ));
        }
        config.sample_rate = rate as f32;
    }

    if let Some(size) = arguments.get("max_buffer_size").and_then(|s| s.as_u64()) {
        config.max_buffer_size = size as usize;
    }

    if let Some(compress) = arguments.get("compression").and_then(|c| c.as_bool()) {
        config.compression = compress;
    }

    if let Some(checksums) = arguments.get("checksums").and_then(|c| c.as_bool()) {
        config.checksums = checksums;
    }

    if let Some(comp_filter) = arguments.get("component_filter").and_then(|f| f.as_array()) {
        config.component_filter = Some(
            comp_filter
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        );
    }

    if let Some(event_filter) = arguments.get("event_filter").and_then(|f| f.as_array()) {
        config.event_filter = Some(
            event_filter
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        );
    }

    Ok(config)
}

/// How long the recorder task sleeps between polls: one sample period, never under 1 ms. The
/// task used to sleep a fixed 33 ms, so any requested rate above 30 was silently recorded at 30.
fn poll_interval(sample_rate: f32) -> std::time::Duration {
    std::time::Duration::from_secs_f32((1.0 / sample_rate).max(0.001))
}

/// Handle play action - start or resume playback
async fn handle_play(_arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let controller = get_playback_controller().read().await;

    match controller.play(brp_client).await {
        Ok(()) => Ok(json!({
            "success": true,
            "message": "Playback started",
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to start playback: {}", e);
            Ok(json!({
                "error": "Play failed",
                "message": format!("Failed to start playback: {}", e),
            }))
        }
    }
}

/// Handle pause action - pause playback
async fn handle_pause(_arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let controller = get_playback_controller().read().await;

    match controller.pause().await {
        Ok(()) => Ok(json!({
            "success": true,
            "message": "Playback paused",
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to pause playback: {}", e);
            Ok(json!({
                "error": "Pause failed",
                "message": format!("Failed to pause playback: {}", e),
            }))
        }
    }
}

/// Handle seek action - seek to frame or marker
async fn handle_seek(arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let controller = get_playback_controller().read().await;

    if let Some(frame_number) = arguments.get("frame").and_then(|f| f.as_u64()) {
        match controller.seek_to_frame(frame_number as usize).await {
            Ok(()) => Ok(json!({
                "success": true,
                "message": format!("Seeked to frame {}", frame_number),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            Err(e) => {
                error!("Failed to seek to frame: {}", e);
                Ok(json!({
                    "error": "Seek failed",
                    "message": format!("Failed to seek to frame: {}", e),
                }))
            }
        }
    } else if let Some(marker_name) = arguments.get("marker").and_then(|m| m.as_str()) {
        match controller.seek_to_marker(marker_name).await {
            Ok(()) => Ok(json!({
                "success": true,
                "message": format!("Seeked to marker '{}'", marker_name),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            Err(e) => {
                error!("Failed to seek to marker: {}", e);
                Ok(json!({
                    "error": "Seek failed",
                    "message": format!("Failed to seek to marker: {}", e),
                }))
            }
        }
    } else {
        Ok(json!({
            "error": "Invalid parameters",
            "message": "Provide either 'frame' or 'marker' parameter",
        }))
    }
}

/// Handle step action - step forward or backward
async fn handle_step(arguments: Value, brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let controller = get_playback_controller().read().await;
    let direction = arguments
        .get("direction")
        .and_then(|d| d.as_str())
        .unwrap_or("forward");

    let mut client = brp_client.write().await;

    let result = match direction {
        "forward" => controller.step_forward(&mut client).await,
        "backward" => controller.step_backward(&mut client).await,
        _ => {
            return Ok(json!({
                "error": "Invalid direction",
                "message": "Direction must be 'forward' or 'backward'",
            }));
        }
    };

    match result {
        Ok(()) => Ok(json!({
            "success": true,
            "message": format!("Stepped {}", direction),
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to step {}: {}", direction, e);
            Ok(json!({
                "error": "Step failed",
                "message": format!("Failed to step {}: {}", direction, e),
            }))
        }
    }
}

/// Handle set_speed action - set playback speed
async fn handle_set_speed(arguments: Value, _brp_client: Arc<RwLock<BrpClient>>) -> Result<Value> {
    let speed = arguments
        .get("speed")
        .and_then(|s| s.as_f64())
        .ok_or_else(|| Error::Validation("Missing 'speed' parameter".to_string()))?;

    let controller = get_playback_controller().read().await;

    match controller.set_speed(speed as f32).await {
        Ok(()) => Ok(json!({
            "success": true,
            "message": format!("Playback speed set to {}x", speed),
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to set speed: {}", e);
            Ok(json!({
                "error": "Set speed failed",
                "message": format!("Failed to set speed: {}", e),
            }))
        }
    }
}

/// Handle playback_status action - get playback status
async fn handle_playback_status(
    _arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let controller = get_playback_controller().read().await;
    let stats = controller.get_stats().await;

    Ok(json!({
        "state": format!("{:?}", stats.state),
        "current_frame": stats.current_frame,
        "total_frames": stats.total_frames,
        "playback_time_seconds": stats.playback_time.as_secs(),
        "speed": stats.speed,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle create_branch action - create a new timeline branch
async fn handle_create_branch(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let name = arguments
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| Error::Validation("Missing 'name' parameter".to_string()))?;

    let parent_id = arguments
        .get("parent_id")
        .and_then(|p| p.as_str())
        .map(BranchId::from_string)
        .transpose()?;

    let branch_point_frame = arguments
        .get("branch_point_frame")
        .and_then(|f| f.as_u64())
        .unwrap_or(0) as usize;

    let mut branch_manager = get_branch_manager().write().await;

    match branch_manager.create_branch(name.to_string(), parent_id, branch_point_frame) {
        Ok(branch_id) => Ok(json!({
            "success": true,
            "message": "Branch created successfully",
            "branch_id": branch_id.to_string(),
            "name": name,
            "parent_id": parent_id.map(|id| id.to_string()),
            "branch_point_frame": branch_point_frame,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to create branch: {}", e);
            Ok(json!({
                "error": "Branch creation failed",
                "message": format!("Failed to create branch: {}", e),
            }))
        }
    }
}

/// Handle list_branches action - list all timeline branches
async fn handle_list_branches(
    _arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_manager = get_branch_manager().read().await;
    let branches = branch_manager.list_branches();

    let branches_json: Vec<Value> = branches
        .iter()
        .map(|branch| {
            json!({
                "id": branch.id.to_string(),
                "name": branch.name,
                "description": branch.description,
                "parent_id": branch.parent_id.map(|id| id.to_string()),
                "branch_point_frame": branch.branch_point_frame,
                "created_at": branch.created_at.duration_since(UNIX_EPOCH)
                    .unwrap_or_default().as_secs(),
                "last_modified": branch.last_modified.duration_since(UNIX_EPOCH)
                    .unwrap_or_default().as_secs(),
                "tags": branch.tags,
            })
        })
        .collect();

    Ok(json!({
        "branches": branches_json,
        "total_count": branches.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

/// Handle switch_branch action - switch active branch for playback
async fn handle_switch_branch(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_id_str = arguments
        .get("branch_id")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_id' parameter".to_string()))?;

    let branch_id = BranchId::from_string(branch_id_str)?;

    let mut branch_manager = get_branch_manager().write().await;

    match branch_manager.set_active_branch(branch_id) {
        Ok(()) => {
            let branch = branch_manager.get_branch(branch_id).unwrap();
            Ok(json!({
                "success": true,
                "message": "Switched to branch",
                "branch_id": branch_id.to_string(),
                "branch_name": branch.metadata.name,
                "modification_count": branch.modification_count(),
                "timestamp": chrono::Utc::now().to_rfc3339()
            }))
        }
        Err(e) => {
            error!("Failed to switch branch: {}", e);
            Ok(json!({
                "error": "Switch failed",
                "message": format!("Failed to switch branch: {}", e),
            }))
        }
    }
}

/// Handle add_modification action - add modification to a branch
async fn handle_add_modification(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_id_str = arguments
        .get("branch_id")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_id' parameter".to_string()))?;

    let branch_id = BranchId::from_string(branch_id_str)?;

    let frame_number = arguments
        .get("frame_number")
        .and_then(|f| f.as_u64())
        .ok_or_else(|| Error::Validation("Missing 'frame_number' parameter".to_string()))?
        as usize;

    let description = arguments
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("User modification")
        .to_string();

    // Parse modification based on type
    let modification = if let Some(entity_mod) = arguments.get("entity_modification") {
        let entity_id = entity_mod
            .get("entity_id")
            .and_then(|e| e.as_u64())
            .ok_or_else(|| {
                Error::Validation("Missing entity_id in entity_modification".to_string())
            })?;

        let component_name = entity_mod
            .get("component_name")
            .and_then(|c| c.as_str())
            .ok_or_else(|| {
                Error::Validation("Missing component_name in entity_modification".to_string())
            })?;

        let new_value = entity_mod.get("new_value").ok_or_else(|| {
            Error::Validation("Missing new_value in entity_modification".to_string())
        })?;

        Modification::EntityModification {
            entity_id,
            component_name: component_name.to_string(),
            new_value: new_value.clone(),
        }
    } else if let Some(entity_removal) = arguments.get("entity_removal") {
        let entity_id = entity_removal
            .get("entity_id")
            .and_then(|e| e.as_u64())
            .ok_or_else(|| Error::Validation("Missing entity_id in entity_removal".to_string()))?;

        Modification::EntityRemoval { entity_id }
    } else {
        return Ok(json!({
            "error": "Invalid modification",
            "message": "No valid modification type provided",
        }));
    };

    let mut layer = ModificationLayer::new(frame_number, description);
    layer.add_modification(modification);

    let mut branch_manager = get_branch_manager().write().await;

    if let Some(branch) = branch_manager.get_branch_mut(branch_id) {
        match branch.add_modification_layer(layer) {
            Ok(()) => Ok(json!({
                "success": true,
                "message": "Modification added to branch",
                "branch_id": branch_id.to_string(),
                "frame_number": frame_number,
                "modification_count": branch.modification_count(),
                "timestamp": chrono::Utc::now().to_rfc3339()
            })),
            Err(e) => {
                error!("Failed to add modification: {}", e);
                Ok(json!({
                    "error": "Modification failed",
                    "message": format!("Failed to add modification: {}", e),
                }))
            }
        }
    } else {
        Ok(json!({
            "error": "Branch not found",
            "message": format!("Branch {} not found", branch_id_str),
        }))
    }
}

/// Handle merge_branch action - merge branch into parent
async fn handle_merge_branch(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_id_str = arguments
        .get("branch_id")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_id' parameter".to_string()))?;

    let branch_id = BranchId::from_string(branch_id_str)?;

    let strategy_str = arguments
        .get("strategy")
        .and_then(|s| s.as_str())
        .unwrap_or("take_source");

    let strategy = match strategy_str {
        "take_source" => MergeStrategy::TakeSource,
        "take_target" => MergeStrategy::TakeTarget,
        "manual" => MergeStrategy::Manual,
        _ => {
            return Ok(json!({
                "error": "Invalid strategy",
                "message": "Strategy must be 'take_source', 'take_target', or 'manual'",
            }))
        }
    };

    let mut branch_manager = get_branch_manager().write().await;

    match branch_manager.merge_branch(branch_id, strategy) {
        Ok(resolutions) => Ok(json!({
            "success": true,
            "message": "Branch merged successfully",
            "branch_id": branch_id.to_string(),
            "conflicts_resolved": resolutions.len(),
            "strategy": strategy_str,
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to merge branch: {}", e);
            Ok(json!({
                "error": "Merge failed",
                "message": format!("Failed to merge branch: {}", e),
            }))
        }
    }
}

/// Handle compare_branches action - compare two branches
async fn handle_compare_branches(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_a_str = arguments
        .get("branch_a")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_a' parameter".to_string()))?;

    let branch_b_str = arguments
        .get("branch_b")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_b' parameter".to_string()))?;

    let branch_a = BranchId::from_string(branch_a_str)?;
    let branch_b = BranchId::from_string(branch_b_str)?;

    let start_frame = arguments
        .get("start_frame")
        .and_then(|f| f.as_u64())
        .unwrap_or(0) as usize;

    let end_frame = arguments
        .get("end_frame")
        .and_then(|f| f.as_u64())
        .unwrap_or(10) as usize;

    let mut branch_manager = get_branch_manager().write().await;

    match branch_manager.compare_branches(branch_a, branch_b, start_frame..end_frame) {
        Ok(comparison) => {
            let differences_json: Vec<Value> = comparison
                .frame_differences
                .iter()
                .map(|frame_diff| {
                    json!({
                        "frame_number": frame_diff.frame_number,
                        "difference_count": frame_diff.differences.len(),
                        "differences": frame_diff.differences.iter().map(|diff| {
                            json!({
                                "entity_id": diff.entity_id,
                                "type": format!("{:?}", diff.difference_type),
                                "details": diff.details,
                            })
                        }).collect::<Vec<_>>()
                    })
                })
                .collect();

            Ok(json!({
                "success": true,
                "branch_a": branch_a.to_string(),
                "branch_b": branch_b.to_string(),
                "frame_range": {"start": start_frame, "end": end_frame},
                "differences": differences_json,
                "total_differences": comparison.frame_differences.len(),
                "timestamp": chrono::Utc::now().to_rfc3339()
            }))
        }
        Err(e) => {
            error!("Failed to compare branches: {}", e);
            Ok(json!({
                "error": "Comparison failed",
                "message": format!("Failed to compare branches: {}", e),
            }))
        }
    }
}

/// Handle delete_branch action - delete a timeline branch
async fn handle_delete_branch(
    arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_id_str = arguments
        .get("branch_id")
        .and_then(|b| b.as_str())
        .ok_or_else(|| Error::Validation("Missing 'branch_id' parameter".to_string()))?;

    let branch_id = BranchId::from_string(branch_id_str)?;

    let mut branch_manager = get_branch_manager().write().await;

    match branch_manager.delete_branch(branch_id) {
        Ok(()) => Ok(json!({
            "success": true,
            "message": "Branch deleted successfully",
            "branch_id": branch_id.to_string(),
            "timestamp": chrono::Utc::now().to_rfc3339()
        })),
        Err(e) => {
            error!("Failed to delete branch: {}", e);
            Ok(json!({
                "error": "Delete failed",
                "message": format!("Failed to delete branch: {}", e),
            }))
        }
    }
}

/// Handle branch_tree action - get branch tree visualization
async fn handle_branch_tree(
    _arguments: Value,
    _brp_client: Arc<RwLock<BrpClient>>,
) -> Result<Value> {
    let branch_manager = get_branch_manager().read().await;
    let tree = branch_manager.get_branch_tree();

    fn serialize_branch_node(node: &crate::server::timeline_branching::BranchNode) -> Value {
        json!({
            "id": node.metadata.id.to_string(),
            "name": node.metadata.name,
            "description": node.metadata.description,
            "branch_point_frame": node.metadata.branch_point_frame,
            "created_at": node.metadata.created_at.duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_secs(),
            "children": node.children.iter().map(serialize_branch_node).collect::<Vec<_>>()
        })
    }

    let tree_json: Vec<Value> = tree.roots.iter().map(serialize_branch_node).collect();

    Ok(json!({
        "success": true,
        "branch_tree": tree_json,
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

// Static globals now use std::sync::OnceLock instead of lazy_static

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_handle_status() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let result = handle(json!({"action": "status"}), brp_client)
            .await
            .unwrap();

        assert!(result.get("recording").is_some());
        assert!(result.get("brp_connected").is_some());
        assert!(result.get("stats").is_some());
    }

    #[tokio::test]
    async fn test_handle_record_without_connection() {
        let config = Config {
            bevy_brp_host: "localhost".to_string(),
            bevy_brp_port: 15702,
            mcp_port: 3000,
        ..Default::default()
        };
        let brp_client = Arc::new(RwLock::new(crate::server::brp_client::BrpClient::new(&config)));

        let result = handle(json!({"action": "record"}), brp_client)
            .await
            .unwrap();

        assert_eq!(result.get("error").unwrap(), "BRP client not connected");
    }

    #[test]
    fn test_parse_recording_config() {
        let args = json!({
            "sample_rate": 60.0,
            "max_buffer_size": 5000,
            "compression": false,
            "checksums": false,
            "component_filter": ["Transform", "Health"],
            "event_filter": ["Collision"]
        });

        let config = parse_recording_config(&args).unwrap();

        assert_eq!(config.sample_rate, 60.0);
        assert_eq!(config.max_buffer_size, 5000);
        assert!(!config.compression);
        assert!(!config.checksums);
        assert_eq!(config.component_filter.unwrap().len(), 2);
        assert_eq!(config.event_filter.unwrap().len(), 1);
    }

    #[test]
    fn test_parse_recording_config_defaults() {
        let args = json!({});
        let config = parse_recording_config(&args).unwrap();

        assert_eq!(config.sample_rate, 30.0);
        assert_eq!(config.max_buffer_size, 10000);
        assert!(config.compression);
        assert!(config.checksums);
    }

    #[test]
    fn record_refuses_a_sample_rate_that_cannot_be_a_period() {
        for bad in [json!(0.0), json!(-5.0)] {
            let err = parse_recording_config(&json!({ "sample_rate": bad })).unwrap_err();
            assert!(err.contains("sample_rate"), "{err}");
        }
    }

    #[test]
    fn recorder_polls_at_the_requested_rate_not_a_fixed_33_ms() {
        let sixty = poll_interval(60.0).as_secs_f32();
        assert!((sixty - 1.0 / 60.0).abs() < 1e-4, "{sixty}");
        let two = poll_interval(2.0).as_secs_f32();
        assert!((two - 0.5).abs() < 1e-4, "{two}");
        assert!(poll_interval(1.0e9) >= std::time::Duration::from_millis(1));
    }
}
