use async_trait::async_trait;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::interval;
use tracing::{debug, error, info, warn};

use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use crate::server::recording_system::{Frame, Recording, Timeline};

/// Playback state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
    Seeking,
    Buffering,
    Error,
}

/// Playback speed multiplier
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlaybackSpeed(f32);

impl PlaybackSpeed {
    pub fn new(speed: f32) -> Result<Self> {
        if speed <= 0.0 || speed > 100.0 {
            return Err(Error::Validation(format!(
                "Invalid playback speed: {speed}"
            )));
        }
        Ok(Self(speed))
    }

    pub fn normal() -> Self {
        Self(1.0)
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self::normal()
    }
}

/// Playback sync strategy trait
#[async_trait]
pub trait PlaybackSync: Send + Sync {
    /// Sync a frame to the game
    async fn sync_frame(&self, frame: &Frame, brp_client: &mut BrpClient) -> Result<()>;

    /// Prepare for playback
    async fn prepare(&self, brp_client: &mut BrpClient) -> Result<()>;

    /// Cleanup after playback
    async fn cleanup(&self, brp_client: &mut BrpClient) -> Result<()>;

    /// Check if drift correction is needed
    fn needs_drift_correction(&self, expected_time: Duration, actual_time: Duration) -> bool {
        let drift = expected_time.abs_diff(actual_time);

        // Drift correction needed if more than 100ms off
        drift > Duration::from_millis(100)
    }
}

/// Default sync strategy - sends state directly to game
pub struct DirectSync;

#[async_trait]
impl PlaybackSync for DirectSync {
    async fn sync_frame(&self, frame: &Frame, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Syncing frame {} to game", frame.frame_number);
        // TODO: Implement actual sync to BRP
        // This would send entity states and events to the game
        Ok(())
    }

    async fn prepare(&self, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Preparing for direct sync playback");
        // TODO: Save current game state for restoration
        Ok(())
    }

    async fn cleanup(&self, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Cleaning up after direct sync playback");
        // TODO: Optionally restore original game state
        Ok(())
    }
}

/// Interpolated sync strategy - interpolates between frames
pub struct InterpolatedSync {
    interpolation_factor: f32,
}

impl InterpolatedSync {
    pub fn new(factor: f32) -> Self {
        Self {
            interpolation_factor: factor.clamp(0.0, 1.0),
        }
    }
}

#[async_trait]
impl PlaybackSync for InterpolatedSync {
    async fn sync_frame(&self, frame: &Frame, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Syncing frame {} with interpolation", frame.frame_number);
        // TODO: Implement interpolated sync
        // This would interpolate between frames for smooth playback
        Ok(())
    }

    async fn prepare(&self, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Preparing for interpolated sync playback");
        Ok(())
    }

    async fn cleanup(&self, _brp_client: &mut BrpClient) -> Result<()> {
        debug!("Cleaning up after interpolated sync playback");
        Ok(())
    }
}

/// Playback controller with state machine
pub struct PlaybackController {
    state: Arc<RwLock<PlaybackState>>,
    timeline: Arc<RwLock<Timeline>>,
    speed: Arc<RwLock<PlaybackSpeed>>,
    sync_strategy: Arc<Box<dyn PlaybackSync>>,
    frame_sender: Arc<Mutex<Option<mpsc::Sender<Frame>>>>,
    control_sender: Arc<Mutex<Option<mpsc::Sender<PlaybackControl>>>>,
    start_time: Arc<RwLock<Option<Instant>>>,
    playback_time: Arc<RwLock<Duration>>,
    version: RecordingVersion,
    playback_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl PlaybackController {
    /// Create a new playback controller
    pub fn new(sync_strategy: Box<dyn PlaybackSync>) -> Self {
        Self {
            state: Arc::new(RwLock::new(PlaybackState::Stopped)),
            timeline: Arc::new(RwLock::new(Timeline::new())),
            speed: Arc::new(RwLock::new(PlaybackSpeed::normal())),
            sync_strategy: Arc::new(sync_strategy),
            frame_sender: Arc::new(Mutex::new(None)),
            control_sender: Arc::new(Mutex::new(None)),
            start_time: Arc::new(RwLock::new(None)),
            playback_time: Arc::new(RwLock::new(Duration::ZERO)),
            version: RecordingVersion::current(),
            playback_task: Arc::new(Mutex::new(None)),
        }
    }

    /// Load a recording for playback
    pub async fn load_recording(&self, recording: Recording) -> Result<()> {
        // Check version compatibility
        if !self.version.is_compatible(&recording.version) {
            warn!(
                "Recording version mismatch: expected {:?}, got {:?}",
                self.version, recording.version
            );
        }

        let mut timeline = self.timeline.write().await;
        timeline.load_recording(recording);

        *self.state.write().await = PlaybackState::Stopped;
        *self.playback_time.write().await = Duration::ZERO;

        info!("Recording loaded for playback");
        Ok(())
    }

    /// Start or resume playback
    pub async fn play(&self, brp_client: Arc<RwLock<BrpClient>>) -> Result<()> {
        let current_state = *self.state.read().await;

        match current_state {
            PlaybackState::Playing => {
                debug!("Already playing");
                return Ok(());
            }
            PlaybackState::Error => {
                return Err(Error::Validation(
                    "Cannot play from error state".to_string(),
                ));
            }
            _ => {}
        }

        *self.state.write().await = PlaybackState::Playing;
        *self.start_time.write().await = Some(Instant::now());

        // Start playback stream
        self.start_playback_stream(brp_client).await?;

        info!("Playback started");
        Ok(())
    }

    /// Pause playback
    pub async fn pause(&self) -> Result<()> {
        let current_state = *self.state.read().await;

        if current_state != PlaybackState::Playing {
            return Err(Error::Validation("Not currently playing".to_string()));
        }

        *self.state.write().await = PlaybackState::Paused;

        // Store current playback time
        if let Some(start) = *self.start_time.read().await {
            let elapsed = Instant::now().duration_since(start);
            *self.playback_time.write().await += elapsed;
        }

        *self.start_time.write().await = None;

        info!("Playback paused");
        Ok(())
    }

    /// Stop playback
    pub async fn stop(&self) -> Result<()> {
        *self.state.write().await = PlaybackState::Stopped;
        *self.start_time.write().await = None;
        *self.playback_time.write().await = Duration::ZERO;

        // Stop the playback stream
        if let Some(sender) = self.control_sender.lock().await.as_ref() {
            let _ = sender.send(PlaybackControl::Stop).await;
        }

        // Cancel the playback task if running
        let mut task_guard = self.playback_task.lock().await;
        if let Some(task) = task_guard.take() {
            task.abort();
            // Wait for task to finish (with timeout)
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
        }

        // Clear channel senders
        *self.frame_sender.lock().await = None;
        *self.control_sender.lock().await = None;

        // Reset timeline to beginning
        self.timeline.write().await.seek(0);

        info!("Playback stopped");
        Ok(())
    }

    /// Seek to a specific frame
    pub async fn seek_to_frame(&self, frame_number: usize) -> Result<()> {
        *self.state.write().await = PlaybackState::Seeking;

        let mut timeline = self.timeline.write().await;
        if !timeline.seek(frame_number) {
            *self.state.write().await = PlaybackState::Error;
            return Err(Error::Validation(format!(
                "Invalid frame number: {frame_number}"
            )));
        }

        // Update playback time based on frame
        if let Some(frame) = timeline.get_frame(frame_number) {
            *self.playback_time.write().await = frame.timestamp;
        }

        *self.state.write().await = PlaybackState::Paused;

        info!("Seeked to frame {}", frame_number);
        Ok(())
    }

    /// Seek to a marker
    pub async fn seek_to_marker(&self, marker_name: &str) -> Result<()> {
        *self.state.write().await = PlaybackState::Seeking;

        let mut timeline = self.timeline.write().await;
        if !timeline.seek_to_marker(marker_name) {
            *self.state.write().await = PlaybackState::Error;
            return Err(Error::Validation(format!(
                "Marker not found: {marker_name}"
            )));
        }

        *self.state.write().await = PlaybackState::Paused;

        info!("Seeked to marker '{}'", marker_name);
        Ok(())
    }

    /// Step to next frame
    pub async fn step_forward(&self, brp_client: &mut BrpClient) -> Result<()> {
        let current_state = *self.state.read().await;

        if current_state == PlaybackState::Playing {
            return Err(Error::Validation("Cannot step while playing".to_string()));
        }

        let mut timeline = self.timeline.write().await;
        if let Some(frame) = timeline.next() {
            self.sync_strategy.sync_frame(&frame, brp_client).await?;
            *self.playback_time.write().await = frame.timestamp;
            debug!("Stepped to frame {}", frame.frame_number);
        } else {
            warn!("No next frame available");
        }

        Ok(())
    }

    /// Step to previous frame
    pub async fn step_backward(&self, brp_client: &mut BrpClient) -> Result<()> {
        let current_state = *self.state.read().await;

        if current_state == PlaybackState::Playing {
            return Err(Error::Validation("Cannot step while playing".to_string()));
        }

        let mut timeline = self.timeline.write().await;
        if let Some(frame) = timeline.previous() {
            self.sync_strategy.sync_frame(&frame, brp_client).await?;
            *self.playback_time.write().await = frame.timestamp;
            debug!("Stepped to frame {}", frame.frame_number);
        } else {
            warn!("No previous frame available");
        }

        Ok(())
    }

    /// Set playback speed
    pub async fn set_speed(&self, speed: f32) -> Result<()> {
        let new_speed = PlaybackSpeed::new(speed)?;
        *self.speed.write().await = new_speed;

        // Send speed update to playback stream
        if let Some(sender) = self.control_sender.lock().await.as_ref() {
            let _ = sender.send(PlaybackControl::SetSpeed(new_speed)).await;
        }

        info!("Playback speed set to {}x", speed);
        Ok(())
    }

    /// Get current playback state
    pub async fn get_state(&self) -> PlaybackState {
        *self.state.read().await
    }

    /// Get playback statistics
    pub async fn get_stats(&self) -> PlaybackStats {
        let timeline = self.timeline.read().await;
        let current_frame = timeline.current_frame;
        let total_frames = timeline
            .recording
            .as_ref()
            .map(|r| r.total_frames)
            .unwrap_or(0);

        let playback_time = *self.playback_time.read().await;
        let speed = self.speed.read().await.value();
        let state = *self.state.read().await;

        PlaybackStats {
            state,
            current_frame,
            total_frames,
            playback_time,
            speed,
        }
    }

    /// Start the playback stream
    async fn start_playback_stream(&self, brp_client: Arc<RwLock<BrpClient>>) -> Result<()> {
        let (frame_tx, _frame_rx) = mpsc::channel::<Frame>(100);
        let (control_tx, mut control_rx) = mpsc::channel::<PlaybackControl>(10);

        *self.frame_sender.lock().await = Some(frame_tx.clone());
        *self.control_sender.lock().await = Some(control_tx.clone());
        // Note: frame_rx would be used by external consumers via frame_stream()

        let timeline = self.timeline.clone();
        let state = self.state.clone();
        let speed = self.speed.clone();
        let sync_strategy = self.sync_strategy.clone();
        let playback_time = self.playback_time.clone();

        // Spawn playback task and store handle
        let task = tokio::spawn(async move {
            let mut frame_interval = interval(Duration::from_millis(33)); // ~30fps
            let mut last_frame_time = Instant::now();

            loop {
                tokio::select! {
                    _ = frame_interval.tick() => {
                        let current_state = *state.read().await;
                        if current_state != PlaybackState::Playing {
                            continue;
                        }

                        let speed_value = speed.read().await.value();
                        let elapsed = last_frame_time.elapsed();
                        let adjusted_elapsed = Duration::from_secs_f32(
                            elapsed.as_secs_f32() * speed_value
                        );

                        let mut timeline = timeline.write().await;

                        // Check if we need to advance frames based on time
                        let current_playback_time = *playback_time.read().await + adjusted_elapsed;

                        while let Some(frame) = timeline.current() {
                            if frame.timestamp <= current_playback_time {
                                // Send frame for syncing
                                if let Err(e) = frame_tx.send(frame.clone()).await {
                                    error!("Failed to send frame: {}", e);
                                    break;
                                }

                                // Sync to game
                                let mut client = brp_client.write().await;
                                if let Err(e) = sync_strategy.sync_frame(&frame, &mut client).await {
                                    error!("Failed to sync frame: {}", e);
                                }

                                // Move to next frame
                                if timeline.next().is_none() {
                                    // End of recording
                                    *state.write().await = PlaybackState::Stopped;
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        last_frame_time = Instant::now();
                    }

                    Some(control) = control_rx.recv() => {
                        match control {
                            PlaybackControl::Stop => {
                                info!("Stopping playback stream");
                                break;
                            }
                            PlaybackControl::SetSpeed(new_speed) => {
                                *speed.write().await = new_speed;
                            }
                        }
                    }

                    else => {
                        break;
                    }
                }
            }

            // Clean up on exit
            info!("Playback task ended");
        });

        // Store task handle for cleanup
        *self.playback_task.lock().await = Some(task);

        Ok(())
    }

    /// Create a frame stream for async iteration
    pub fn frame_stream(&self) -> impl Stream<Item = Frame> {
        let (tx, rx) = mpsc::channel(100);
        let timeline = self.timeline.clone();

        tokio::spawn(async move {
            let mut timeline = timeline.write().await;
            while let Some(frame) = timeline.next() {
                if tx.send(frame).await.is_err() {
                    break;
                }
            }
        });

        tokio_stream::wrappers::ReceiverStream::new(rx)
    }
}

/// Playback control messages
#[derive(Debug, Clone)]
enum PlaybackControl {
    Stop,
    SetSpeed(PlaybackSpeed),
}

/// Playback statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackStats {
    pub state: PlaybackState,
    pub current_frame: usize,
    pub total_frames: usize,
    pub playback_time: Duration,
    pub speed: f32,
}

/// Recording format version for compatibility checking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordingVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl RecordingVersion {
    pub fn current() -> Self {
        Self {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }

    pub fn is_compatible(&self, other: &RecordingVersion) -> bool {
        // Major version must match
        if self.major != other.major {
            return false;
        }

        // Minor version must be >= for backwards compatibility
        if self.minor < other.minor {
            return false;
        }

        true
    }
}

impl Default for RecordingVersion {
    fn default() -> Self {
        Self::current()
    }
}

/// Frame interpolation for smooth playback
pub struct FrameInterpolator {
    from_frame: Option<Frame>,
    to_frame: Option<Frame>,
}

impl FrameInterpolator {
    pub fn new() -> Self {
        Self {
            from_frame: None,
            to_frame: None,
        }
    }

    /// Set frames for interpolation
    pub fn set_frames(&mut self, from: Frame, to: Frame) {
        self.from_frame = Some(from);
        self.to_frame = Some(to);
    }

    /// Interpolate between frames
    pub fn interpolate(&self, t: f32) -> Option<Frame> {
        let from = self.from_frame.as_ref()?;
        let to = self.to_frame.as_ref()?;

        // Simple linear interpolation of timestamp
        let from_secs = from.timestamp.as_secs_f32();
        let to_secs = to.timestamp.as_secs_f32();
        let interp_secs = from_secs + (to_secs - from_secs) * t;

        let mut interpolated = from.clone();
        interpolated.timestamp = Duration::from_secs_f32(interp_secs);

        // TODO: Interpolate entity positions and other numeric values

        Some(interpolated)
    }
}

impl Default for FrameInterpolator {
    fn default() -> Self {
        Self::new()
    }
}

/// Drift detection and correction
pub struct DriftDetector {
    max_drift: Duration,
    samples: Vec<(Duration, Duration)>, // (expected, actual)
}

impl DriftDetector {
    pub fn new(max_drift: Duration) -> Self {
        Self {
            max_drift,
            samples: Vec::with_capacity(10),
        }
    }

    /// Add a timing sample
    pub fn add_sample(&mut self, expected: Duration, actual: Duration) {
        self.samples.push((expected, actual));

        // Keep only recent samples
        if self.samples.len() > 10 {
            self.samples.remove(0);
        }
    }

    /// Check if drift correction is needed
    pub fn needs_correction(&self) -> bool {
        if self.samples.len() < 3 {
            return false;
        }

        // Calculate average drift
        let total_drift: Duration = self
            .samples
            .iter()
            .map(|(expected, actual)| {
                if expected > actual {
                    *expected - *actual
                } else {
                    *actual - *expected
                }
            })
            .sum();

        let avg_drift = total_drift / self.samples.len() as u32;

        avg_drift > self.max_drift
    }

    /// Get correction amount
    pub fn get_correction(&self) -> Option<Duration> {
        if !self.needs_correction() {
            return None;
        }

        // Calculate the drift direction and amount
        let total_signed_drift: i64 = self
            .samples
            .iter()
            .map(|(expected, actual)| {
                if expected > actual {
                    (expected.as_millis() - actual.as_millis()) as i64
                } else {
                    -((actual.as_millis() - expected.as_millis()) as i64)
                }
            })
            .sum();

        let avg_drift_ms = total_signed_drift / self.samples.len() as i64;

        Some(Duration::from_millis(avg_drift_ms.unsigned_abs()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playback_speed() {
        assert!(PlaybackSpeed::new(1.0).is_ok());
        assert!(PlaybackSpeed::new(0.5).is_ok());
        assert!(PlaybackSpeed::new(2.0).is_ok());
        assert!(PlaybackSpeed::new(0.0).is_err());
        assert!(PlaybackSpeed::new(-1.0).is_err());
        assert!(PlaybackSpeed::new(101.0).is_err());
    }

    #[test]
    fn test_recording_version_compatibility() {
        let v1 = RecordingVersion {
            major: 1,
            minor: 0,
            patch: 0,
        };
        let v2 = RecordingVersion {
            major: 1,
            minor: 0,
            patch: 1,
        };
        let v3 = RecordingVersion {
            major: 1,
            minor: 1,
            patch: 0,
        };
        let v4 = RecordingVersion {
            major: 2,
            minor: 0,
            patch: 0,
        };

        assert!(v1.is_compatible(&v1));
        assert!(v1.is_compatible(&v2)); // Patch version doesn't matter
        assert!(!v1.is_compatible(&v3)); // Minor version too high
        assert!(!v1.is_compatible(&v4)); // Major version mismatch
    }

    #[tokio::test]
    async fn test_playback_controller_creation() {
        let controller = PlaybackController::new(Box::new(DirectSync));
        assert_eq!(controller.get_state().await, PlaybackState::Stopped);
    }

    #[test]
    fn test_frame_interpolator() {
        let mut interpolator = FrameInterpolator::new();

        let frame1 = Frame {
            frame_number: 0,
            timestamp: Duration::from_secs(0),
            entities: Default::default(),
            events: Vec::new(),
            events_not_captured: None,
            checksum: None,
        };

        let frame2 = Frame {
            frame_number: 1,
            timestamp: Duration::from_secs(1),
            entities: Default::default(),
            events: Vec::new(),
            events_not_captured: None,
            checksum: None,
        };

        interpolator.set_frames(frame1, frame2);

        let interp = interpolator.interpolate(0.5).unwrap();
        assert_eq!(interp.timestamp, Duration::from_millis(500));
    }

    #[test]
    fn test_drift_detector() {
        let mut detector = DriftDetector::new(Duration::from_millis(100));

        // Add samples with small drift (should not trigger correction)
        detector.add_sample(Duration::from_millis(100), Duration::from_millis(105));
        detector.add_sample(Duration::from_millis(200), Duration::from_millis(208));
        detector.add_sample(Duration::from_millis(300), Duration::from_millis(310));

        assert!(!detector.needs_correction());
        assert!(detector.get_correction().is_none());

        // Add samples with large drift (should trigger correction)
        detector.add_sample(Duration::from_millis(400), Duration::from_millis(550));
        detector.add_sample(Duration::from_millis(500), Duration::from_millis(700));
        detector.add_sample(Duration::from_millis(600), Duration::from_millis(850));

        assert!(detector.needs_correction());
        assert!(detector.get_correction().is_some());
    }
}
