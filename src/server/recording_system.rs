use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};

/// A frame of recorded game state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frame {
    /// Frame number in the recording
    pub frame_number: usize,
    /// Timestamp relative to recording start
    pub timestamp: Duration,
    /// Entity states in this frame
    pub entities: HashMap<u64, EntityState>,
    /// Events that occurred in this frame
    pub events: Vec<RecordedEvent>,
    /// Why the event list is empty, when it is empty because events were not captured.
    ///
    /// BRP has no method that reports the events a game has seen, so the event list is always
    /// empty. Without this field a reader could not tell an empty list meaning "nothing happened"
    /// from one meaning "we cannot see events", and would draw the wrong conclusion from a
    /// recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events_not_captured: Option<String>,
    /// Frame checksum for validation
    pub checksum: Option<String>,
}

/// State of an entity at a point in time
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntityState {
    pub entity_id: u64,
    pub components: HashMap<String, serde_json::Value>,
    pub active: bool,
}

/// A recorded game event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedEvent {
    pub event_type: String,
    pub entity_id: Option<u64>,
    pub data: serde_json::Value,
    pub timestamp: Duration,
}

/// A marker in the timeline for important moments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Marker {
    pub name: String,
    pub frame_number: usize,
    pub timestamp: Duration,
    pub description: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Delta frame storing only changes from previous frame
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaFrame {
    pub frame_number: usize,
    pub timestamp: Duration,
    /// Entities that were added
    pub added_entities: HashMap<u64, EntityState>,
    /// Entities that were removed
    pub removed_entities: Vec<u64>,
    /// Components that changed
    pub changed_components: HashMap<u64, HashMap<String, serde_json::Value>>,
    /// Events in this frame
    pub events: Vec<RecordedEvent>,
}

/// Recording configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingConfig {
    /// Samples per second
    pub sample_rate: f32,
    /// Maximum buffer size in frames
    pub max_buffer_size: usize,
    /// Enable compression
    pub compression: bool,
    /// Enable checksums
    pub checksums: bool,
    /// Component types to record
    pub component_filter: Option<Vec<String>>,
    /// Event types to record
    pub event_filter: Option<Vec<String>>,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            sample_rate: 30.0,
            max_buffer_size: 10000,
            compression: true,
            checksums: true,
            component_filter: None,
            event_filter: None,
        }
    }
}

/// Circular buffer for recording game state
pub struct RecordingBuffer {
    config: RecordingConfig,
    frames: VecDeque<Frame>,
    delta_frames: VecDeque<DeltaFrame>,
    markers: Vec<Marker>,
    start_time: Option<Instant>,
    last_frame_time: Option<Instant>,
    frame_counter: usize,
    recording: bool,
    last_full_frame: Option<Frame>,
}

impl RecordingBuffer {
    /// Create a new recording buffer
    pub fn new(config: RecordingConfig) -> Self {
        Self {
            frames: VecDeque::with_capacity(config.max_buffer_size),
            delta_frames: VecDeque::with_capacity(config.max_buffer_size),
            markers: Vec::new(),
            start_time: None,
            last_frame_time: None,
            frame_counter: 0,
            recording: false,
            last_full_frame: None,
            config,
        }
    }

    /// Start recording
    pub fn start_recording(&mut self) {
        info!("Starting recording with {} fps", self.config.sample_rate);
        self.recording = true;
        self.start_time = Some(Instant::now());
        // `last_frame_time` stays `None`, so the first frame is recorded straight away. Setting it
        // here made the first frame skip itself as "too soon", and a recording that starts with no
        // frame is a recording that starts with nothing.
        self.last_frame_time = None;
        self.frame_counter = 0;
        self.frames.clear();
        self.delta_frames.clear();
        self.markers.clear();
        self.last_full_frame = None;
    }

    /// Stop recording
    pub fn stop_recording(&mut self) {
        info!("Stopping recording. Recorded {} frames", self.frame_counter);
        self.recording = false;
    }

    /// Check if recording is active
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Record a frame
    pub async fn record_frame(&mut self, brp_client: &mut BrpClient) -> Result<()> {
        if !self.recording {
            return Ok(());
        }

        let now = Instant::now();

        // Check sample rate
        if let Some(last_time) = self.last_frame_time {
            let elapsed = now.duration_since(last_time);
            let min_interval = Duration::from_secs_f32(1.0 / self.config.sample_rate);

            if elapsed < min_interval {
                return Ok(()); // Skip this frame
            }
        }

        // Get current game state
        let entities = self.fetch_entities(brp_client).await?;
        let events = self.fetch_events(brp_client).await?;

        let timestamp = self
            .start_time
            .map(|start| now.duration_since(start))
            .unwrap_or_default();

        // Create frame
        let mut frame = Frame {
            frame_number: self.frame_counter,
            timestamp,
            entities,
            events,
            events_not_captured: Some(
                "BRP reports no events, so this recording holds entity states only".to_string(),
            ),
            checksum: None,
        };

        // Calculate checksum if enabled
        if self.config.checksums {
            frame.checksum = Some(self.calculate_checksum(&frame));
        }

        // Store as delta frame if we have a previous frame
        if let Some(ref last_frame) = self.last_full_frame {
            let delta = self.create_delta_frame(last_frame, &frame);

            // Add to circular buffer
            if self.delta_frames.len() >= self.config.max_buffer_size {
                self.delta_frames.pop_front();
            }
            self.delta_frames.push_back(delta);
        } else {
            // First frame - store as full frame
            if self.frames.len() >= self.config.max_buffer_size {
                self.frames.pop_front();
            }
            self.frames.push_back(frame.clone());
        }

        // Store every Nth frame as full frame for seeking. The first frame is already stored
        // above, so it is not stored twice: a recording that starts with one frame should hold one
        // frame, not two copies of it.
        if self.frame_counter > 0 && self.frame_counter.is_multiple_of(30) {
            if self.frames.len() >= self.config.max_buffer_size / 30 {
                self.frames.pop_front();
            }
            self.frames.push_back(frame.clone());
        }

        self.last_full_frame = Some(frame);
        self.last_frame_time = Some(now);
        self.frame_counter += 1;

        Ok(())
    }

    /// Add a marker at the current position
    pub fn add_marker(&mut self, name: String, description: Option<String>) {
        if !self.recording {
            warn!("Cannot add marker when not recording");
            return;
        }

        let timestamp = self
            .start_time
            .map(|start| Instant::now().duration_since(start))
            .unwrap_or_default();

        let marker = Marker {
            name,
            frame_number: self.frame_counter,
            timestamp,
            description,
            metadata: HashMap::new(),
        };

        info!(
            "Added marker '{}' at frame {}",
            marker.name, marker.frame_number
        );
        self.markers.push(marker);
    }

    /// Save recording to file
    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        info!("Saving recording to {:?}", path);

        let recording = Recording {
            config: self.config.clone(),
            frames: self.frames.clone().into(),
            delta_frames: self.delta_frames.clone().into(),
            markers: self.markers.clone(),
            total_frames: self.frame_counter,
            duration: self
                .start_time
                .map(|start| Instant::now().duration_since(start))
                .unwrap_or_default(),
            version: crate::server::playback_system::RecordingVersion::current(),
        };

        let file = File::create(path)?;

        if self.config.compression {
            let encoder = GzEncoder::new(file, Compression::default());
            let writer = BufWriter::new(encoder);
            bincode::serialize_into(writer, &recording)
                .map_err(|e| Error::Serialization(format!("Failed to serialize recording: {e}")))?;
        } else {
            let writer = BufWriter::new(file);
            bincode::serialize_into(writer, &recording)
                .map_err(|e| Error::Serialization(format!("Failed to serialize recording: {e}")))?;
        }

        info!("Recording saved successfully");
        Ok(())
    }

    /// Load recording from file
    pub fn load_from_file(path: &Path) -> Result<Recording> {
        info!("Loading recording from {:?}", path);

        let file = File::open(path)?;

        // Try to load as compressed first
        let recording: Recording = match Self::try_load_compressed(&file) {
            Ok(rec) => rec,
            Err(_) => {
                // Try uncompressed
                let reader = BufReader::new(file);
                bincode::deserialize_from(reader).map_err(|e| {
                    Error::Serialization(format!("Failed to deserialize recording: {e}"))
                })?
            }
        };

        info!(
            "Recording loaded: {} frames, {} duration",
            recording.total_frames,
            recording.duration.as_secs()
        );

        Ok(recording)
    }

    fn try_load_compressed(file: &File) -> Result<Recording> {
        let decoder = GzDecoder::new(file);
        let reader = BufReader::new(decoder);
        bincode::deserialize_from(reader).map_err(|e| {
            Error::Serialization(format!("Failed to deserialize compressed recording: {e}"))
        })
    }

    /// Fetch current entity states from the game.
    ///
    /// The entity list comes from `world.query`, so what is recorded is what the game holds at
    /// that moment.
    async fn fetch_entities(
        &self,
        brp_client: &mut BrpClient,
    ) -> Result<HashMap<u64, EntityState>> {
        debug!("Fetching entity states");

        let request = crate::server::brp::BrpRequest {
            method: crate::server::brp::builtin_methods::BRP_QUERY_METHOD.to_string(),
            id: None,
            params: Some(crate::server::brp::query_params(None, true)),
        };
        let response = brp_client.send_request(&request).await?;
        let entities = match response.payload {
            crate::server::brp::BrpPayload::Result(value) => crate::server::brp::entity_data_from_query_result(&value)
                .ok_or_else(|| {
                    Error::Validation(
                        "the game answered world.query with a shape this server cannot read an \
                         entity list from"
                            .to_string(),
                    )
                })?,
            crate::server::brp::BrpPayload::Error(error) => {
                return Err(Error::Brp(error.message));
            }
        };

        Ok(entities
            .into_iter()
            .map(|entity| {
                (
                    entity.id,
                    EntityState {
                        entity_id: entity.id,
                        components: entity.components,
                        active: true,
                    },
                )
            })
            .collect())
    }

    /// Fetch recent events from the game.
    ///
    /// BRP has no method that reports the events a game has seen, so there is nothing to fetch.
    /// The recording keeps an empty event list, and says so in the frame it produces, rather than
    /// leaving the caller to believe the events were captured and there were none.
    async fn fetch_events(&self, _brp_client: &mut BrpClient) -> Result<Vec<RecordedEvent>> {
        debug!("Fetching events");

        Ok(Vec::new())
    }

    /// Create a delta frame from two full frames
    fn create_delta_frame(&self, prev: &Frame, curr: &Frame) -> DeltaFrame {
        let mut added_entities = HashMap::new();
        let mut removed_entities = Vec::new();
        let mut changed_components = HashMap::new();

        // Find added entities
        for (id, state) in &curr.entities {
            if !prev.entities.contains_key(id) {
                added_entities.insert(*id, state.clone());
            }
        }

        // Find removed entities
        for id in prev.entities.keys() {
            if !curr.entities.contains_key(id) {
                removed_entities.push(*id);
            }
        }

        // Find changed components
        for (id, curr_state) in &curr.entities {
            if let Some(prev_state) = prev.entities.get(id) {
                let mut changes = HashMap::new();

                for (comp_name, comp_value) in &curr_state.components {
                    let changed = prev_state
                        .components
                        .get(comp_name)
                        .map(|prev_val| prev_val != comp_value)
                        .unwrap_or(true);

                    if changed {
                        changes.insert(comp_name.clone(), comp_value.clone());
                    }
                }

                if !changes.is_empty() {
                    changed_components.insert(*id, changes);
                }
            }
        }

        DeltaFrame {
            frame_number: curr.frame_number,
            timestamp: curr.timestamp,
            added_entities,
            removed_entities,
            changed_components,
            events: curr.events.clone(),
        }
    }

    /// Calculate checksum for a frame
    fn calculate_checksum(&self, frame: &Frame) -> String {
        let mut hasher = Sha256::new();

        // Hash frame data
        let frame_bytes = bincode::serialize(frame).unwrap_or_default();
        hasher.update(&frame_bytes);

        format!("{:x}", hasher.finalize())
    }

    /// The frames recorded so far, oldest first.
    ///
    /// Without this, nothing outside the buffer can see what a recording holds: the `replay` tool
    /// reports a frame count but cannot say what was in the frames, and a caller has no way to
    /// check that the recording captured anything.
    pub fn get_frames(&self) -> Vec<Frame> {
        self.frames.iter().cloned().collect()
    }

    /// Get current recording statistics
    pub fn get_stats(&self) -> RecordingStats {
        RecordingStats {
            frame_count: self.frame_counter,
            delta_frame_count: self.delta_frames.len(),
            full_frame_count: self.frames.len(),
            marker_count: self.markers.len(),
            duration: self
                .start_time
                .map(|start| Instant::now().duration_since(start))
                .unwrap_or_default(),
            is_recording: self.recording,
            buffer_usage: (self.delta_frames.len() + self.frames.len()) as f32
                / self.config.max_buffer_size as f32,
        }
    }
}

/// Complete recording with all data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    pub config: RecordingConfig,
    pub frames: Vec<Frame>,
    pub delta_frames: Vec<DeltaFrame>,
    pub markers: Vec<Marker>,
    pub total_frames: usize,
    pub duration: Duration,
    pub version: crate::server::playback_system::RecordingVersion,
}

/// Recording statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingStats {
    pub frame_count: usize,
    pub delta_frame_count: usize,
    pub full_frame_count: usize,
    pub marker_count: usize,
    pub duration: Duration,
    pub is_recording: bool,
    pub buffer_usage: f32,
}

/// Timeline for navigating recordings
pub struct Timeline {
    pub(crate) recording: Option<Recording>,
    pub(crate) current_frame: usize,
    pub(crate) cached_frames: HashMap<usize, Frame>,
    pub(crate) max_cache_size: usize,
}

impl Timeline {
    /// Create a new timeline
    pub fn new() -> Self {
        Self {
            recording: None,
            current_frame: 0,
            cached_frames: HashMap::new(),
            max_cache_size: 100, // Limit cache to 100 frames
        }
    }

    /// Load a recording into the timeline
    pub fn load_recording(&mut self, recording: Recording) {
        info!("Loading recording with {} frames", recording.total_frames);
        self.recording = Some(recording);
        self.current_frame = 0;
        self.cached_frames.clear();
    }

    /// Get frame at specific index
    pub fn get_frame(&mut self, frame_number: usize) -> Option<Frame> {
        // Check cache first
        if let Some(frame) = self.cached_frames.get(&frame_number) {
            return Some(frame.clone());
        }

        let recording = self.recording.as_ref()?;

        // Find nearest full frame
        let full_frame = recording
            .frames
            .iter()
            .filter(|f| f.frame_number <= frame_number)
            .max_by_key(|f| f.frame_number)?;

        let mut reconstructed = full_frame.clone();

        // Apply delta frames
        for delta in &recording.delta_frames {
            if delta.frame_number <= frame_number && delta.frame_number > full_frame.frame_number {
                self.apply_delta(&mut reconstructed, delta);
            }
        }

        // Cache the reconstructed frame with size limit
        if self.cached_frames.len() >= self.max_cache_size {
            // Remove oldest cached frame (simple strategy)
            if let Some(&oldest) = self.cached_frames.keys().min() {
                self.cached_frames.remove(&oldest);
            }
        }
        self.cached_frames
            .insert(frame_number, reconstructed.clone());

        Some(reconstructed)
    }

    /// Apply a delta frame to a full frame
    fn apply_delta(&self, frame: &mut Frame, delta: &DeltaFrame) {
        // Remove entities
        for id in &delta.removed_entities {
            frame.entities.remove(id);
        }

        // Add entities
        for (id, state) in &delta.added_entities {
            frame.entities.insert(*id, state.clone());
        }

        // Apply component changes
        for (id, changes) in &delta.changed_components {
            if let Some(entity) = frame.entities.get_mut(id) {
                for (comp_name, comp_value) in changes {
                    entity
                        .components
                        .insert(comp_name.clone(), comp_value.clone());
                }
            }
        }

        // Update frame metadata
        frame.frame_number = delta.frame_number;
        frame.timestamp = delta.timestamp;
        frame.events = delta.events.clone();
    }

    /// Seek to a specific frame
    pub fn seek(&mut self, frame_number: usize) -> bool {
        if let Some(recording) = &self.recording {
            if frame_number < recording.total_frames {
                self.current_frame = frame_number;
                return true;
            }
        }
        false
    }

    /// Seek to a marker
    pub fn seek_to_marker(&mut self, marker_name: &str) -> bool {
        if let Some(recording) = &self.recording {
            if let Some(marker) = recording.markers.iter().find(|m| m.name == marker_name) {
                self.current_frame = marker.frame_number;
                return true;
            }
        }
        false
    }

    /// Get current frame
    pub fn current(&mut self) -> Option<Frame> {
        self.get_frame(self.current_frame)
    }

    /// Move to next frame
    pub fn next(&mut self) -> Option<Frame> {
        if let Some(recording) = &self.recording {
            if self.current_frame + 1 < recording.total_frames {
                self.current_frame += 1;
                return self.current();
            }
        }
        None
    }

    /// Move to previous frame
    pub fn previous(&mut self) -> Option<Frame> {
        if self.current_frame > 0 {
            self.current_frame -= 1;
            return self.current();
        }
        None
    }

    /// Get all markers
    pub fn markers(&self) -> Vec<Marker> {
        self.recording
            .as_ref()
            .map(|r| r.markers.clone())
            .unwrap_or_default()
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Global recording state
pub struct RecordingState {
    pub buffer: Arc<RwLock<RecordingBuffer>>,
    pub timeline: Arc<RwLock<Timeline>>,
}

impl RecordingState {
    pub fn new(config: RecordingConfig) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(RecordingBuffer::new(config))),
            timeline: Arc::new(RwLock::new(Timeline::new())),
        }
    }
}

// bincode is now a direct dependency, no fallback needed

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recording_buffer_creation() {
        let config = RecordingConfig::default();
        let buffer = RecordingBuffer::new(config);

        assert!(!buffer.is_recording());
        assert_eq!(buffer.frame_counter, 0);
    }

    #[test]
    fn test_recording_start_stop() {
        let config = RecordingConfig::default();
        let mut buffer = RecordingBuffer::new(config);

        buffer.start_recording();
        assert!(buffer.is_recording());

        buffer.stop_recording();
        assert!(!buffer.is_recording());
    }

    #[test]
    fn test_marker_addition() {
        let config = RecordingConfig::default();
        let mut buffer = RecordingBuffer::new(config);

        buffer.start_recording();
        buffer.add_marker(
            "test_marker".to_string(),
            Some("Test description".to_string()),
        );

        assert_eq!(buffer.markers.len(), 1);
        assert_eq!(buffer.markers[0].name, "test_marker");
    }

    #[test]
    fn test_delta_frame_creation() {
        let config = RecordingConfig::default();
        let buffer = RecordingBuffer::new(config);

        let mut prev = Frame {
            frame_number: 0,
            timestamp: Duration::from_secs(0),
            entities: HashMap::new(),
            events: Vec::new(),
            events_not_captured: None,
            checksum: None,
        };

        let mut entity1 = EntityState {
            entity_id: 1,
            components: HashMap::new(),
            active: true,
        };
        entity1
            .components
            .insert("Transform".to_string(), serde_json::json!({"x": 0, "y": 0}));
        prev.entities.insert(1, entity1);

        let mut curr = Frame {
            frame_number: 1,
            timestamp: Duration::from_secs(1),
            entities: HashMap::new(),
            events: Vec::new(),
            events_not_captured: None,
            checksum: None,
        };

        let mut entity1_new = EntityState {
            entity_id: 1,
            components: HashMap::new(),
            active: true,
        };
        entity1_new.components.insert(
            "Transform".to_string(),
            serde_json::json!({"x": 10, "y": 0}),
        );
        curr.entities.insert(1, entity1_new);

        let mut entity2 = EntityState {
            entity_id: 2,
            components: HashMap::new(),
            active: true,
        };
        entity2
            .components
            .insert("Transform".to_string(), serde_json::json!({"x": 5, "y": 5}));
        curr.entities.insert(2, entity2);

        let delta = buffer.create_delta_frame(&prev, &curr);

        assert_eq!(delta.added_entities.len(), 1);
        assert!(delta.added_entities.contains_key(&2));
        assert_eq!(delta.changed_components.len(), 1);
        assert!(delta.changed_components.contains_key(&1));
    }

    #[test]
    fn test_timeline_creation() {
        let timeline = Timeline::new();
        assert_eq!(timeline.current_frame, 0);
        assert!(timeline.recording.is_none());
    }

    #[test]
    fn test_timeline_navigation() {
        let mut timeline = Timeline::new();

        let recording = Recording {
            config: RecordingConfig::default(),
            frames: vec![Frame {
                frame_number: 0,
                timestamp: Duration::from_secs(0),
                entities: HashMap::new(),
                events: Vec::new(),
                events_not_captured: None,
                checksum: None,
            }],
            delta_frames: Vec::new(),
            markers: vec![Marker {
                name: "test".to_string(),
                frame_number: 0,
                timestamp: Duration::from_secs(0),
                description: None,
                metadata: HashMap::new(),
            }],
            total_frames: 1,
            duration: Duration::from_secs(1),
            version: crate::server::playback_system::RecordingVersion::current(),
        };

        timeline.load_recording(recording);

        assert!(timeline.seek(0));
        assert!(!timeline.seek(100));
        assert!(timeline.seek_to_marker("test"));
        assert!(!timeline.seek_to_marker("nonexistent"));
    }

    #[test]
    fn test_recording_stats() {
        let config = RecordingConfig::default();
        let buffer = RecordingBuffer::new(config);

        let stats = buffer.get_stats();
        assert_eq!(stats.frame_count, 0);
        assert_eq!(stats.delta_frame_count, 0);
        assert_eq!(stats.full_frame_count, 0);
        assert!(!stats.is_recording);
    }

    #[test]
    fn test_recording_config_default() {
        let config = RecordingConfig::default();
        assert_eq!(config.sample_rate, 30.0);
        assert_eq!(config.max_buffer_size, 10000);
        assert!(config.compression);
        assert!(config.checksums);
    }
}
