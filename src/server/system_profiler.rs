/// System Performance Profiler for comprehensive ECS profiling
use crate::server::brp_messages::{
    DebugCommand, DebugResponse, SystemMetrics, ProfileSample, SystemProfile,
};
use crate::server::brp_client::BrpClient;
use crate::server::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Maximum number of frames to store in history
pub const MAX_FRAME_HISTORY: usize = 1000;

/// Maximum number of systems to profile concurrently
pub const MAX_CONCURRENT_SYSTEMS: usize = 50;

/// Default profiling duration in milliseconds
pub const DEFAULT_PROFILE_DURATION_MS: u64 = 5000;

/// Performance threshold for anomaly detection (150% of average)
pub const ANOMALY_THRESHOLD_MULTIPLIER: f32 = 1.5;

/// System profiler with comprehensive performance tracking
pub struct SystemProfiler {
    /// BRP client for communication with Bevy
    brp_client: Arc<RwLock<BrpClient>>,
    /// Active profiling sessions
    active_sessions: Arc<RwLock<HashMap<String, ProfileSession>>>,
    /// Historical frame data
    frame_history: Arc<RwLock<FrameHistory>>,
    /// System dependency graph
    dependency_graph: Arc<RwLock<SystemDependencyGraph>>,
    /// Performance anomaly detector
    anomaly_detector: Arc<RwLock<AnomalyDetector>>,
    /// Profiling configuration
    config: ProfilerConfig,
}

/// Configuration for the system profiler
#[derive(Debug, Clone)]
pub struct ProfilerConfig {
    /// Whether to track memory allocations
    pub track_allocations: bool,
    /// Whether to integrate with external profilers (Tracy/puffin)
    pub external_profiler_integration: bool,
    /// Maximum overhead percentage allowed
    pub max_overhead_percent: f32,
    /// Automatic profiling trigger threshold (frame time in ms)
    pub auto_profile_threshold_ms: f32,
}

impl Default for ProfilerConfig {
    fn default() -> Self {
        Self {
            track_allocations: false,
            external_profiler_integration: true,
            max_overhead_percent: 3.0,
            auto_profile_threshold_ms: 33.0, // Trigger on frames > 33ms (30 FPS)
        }
    }
}

/// Active profiling session for a system
#[derive(Debug, Clone)]
struct ProfileSession {
    /// System name being profiled
    system_name: String,
    /// Start time of profiling
    started_at: Instant,
    /// Duration to profile for
    duration: Duration,
    /// Collected samples
    samples: Vec<ProfileSample>,
    /// Whether to track allocations
    track_allocations: bool,
}

/// Frame history storage with ring buffer
#[derive(Debug)]
struct FrameHistory {
    /// Ring buffer of frame data
    frames: VecDeque<FrameData>,
    /// Maximum frames to store
    max_frames: usize,
    /// Current frame number
    current_frame: u64,
}

/// Data for a single frame
#[derive(Debug, Clone)]
struct FrameData {
    /// Frame number
    frame_number: u64,
    /// Frame start time
    start_time: Instant,
    /// Wall-clock start, microseconds since the Unix epoch; the same clock
    /// `collect_sample` stamps live samples with.
    started_at_us: u64,
    /// Total frame duration
    duration: Duration,
    /// System execution times
    system_times: HashMap<String, Duration>,
    /// Memory allocations per system
    system_allocations: HashMap<String, usize>,
}

/// System dependency graph for impact analysis
#[derive(Debug, Default)]
struct SystemDependencyGraph {
    /// Dependencies: system -> systems it depends on
    dependencies: HashMap<String, Vec<String>>,
    /// Reverse dependencies: system -> systems that depend on it
    dependents: HashMap<String, Vec<String>>,
    /// Execution order
    execution_order: Vec<String>,
}

/// Anomaly detector for performance spikes
#[derive(Debug)]
struct AnomalyDetector {
    /// Moving average of frame times
    moving_average: MovingAverage,
    /// Detected anomalies
    anomalies: Vec<PerformanceAnomaly>,
    /// Threshold multiplier for detection
    threshold_multiplier: f32,
}

/// Performance anomaly detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceAnomaly {
    /// Frame where anomaly occurred
    pub frame_number: u64,
    /// System that caused the anomaly
    pub system_name: String,
    /// Execution time that triggered detection
    pub execution_time_ms: f32,
    /// Expected time based on average
    pub expected_time_ms: f32,
    /// Severity level (1-5)
    pub severity: u8,
    /// Timestamp of detection (microseconds since start)
    pub detected_at_us: u64,
}

/// Moving average calculator for frame times
#[derive(Debug)]
struct MovingAverage {
    /// Window of values
    window: VecDeque<f32>,
    /// Maximum window size
    window_size: usize,
    /// Current sum for efficiency
    sum: f32,
}

impl SystemProfiler {
    /// Create new system profiler
    pub fn new(brp_client: Arc<RwLock<BrpClient>>) -> Self {
        Self::with_config(brp_client, ProfilerConfig::default())
    }

    /// Create profiler with custom configuration
    pub fn with_config(brp_client: Arc<RwLock<BrpClient>>, config: ProfilerConfig) -> Self {
        Self {
            brp_client,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            frame_history: Arc::new(RwLock::new(FrameHistory::new(MAX_FRAME_HISTORY))),
            dependency_graph: Arc::new(RwLock::new(SystemDependencyGraph::default())),
            anomaly_detector: Arc::new(RwLock::new(AnomalyDetector::new(
                ANOMALY_THRESHOLD_MULTIPLIER,
            ))),
            config,
        }
    }

    /// Start profiling a specific system
    pub async fn start_profiling(
        &self,
        system_name: String,
        duration_ms: Option<u64>,
        track_allocations: Option<bool>,
    ) -> Result<()> {
        let mut sessions = self.active_sessions.write().await;
        
        // Check if already profiling
        if sessions.contains_key(&system_name) {
            return Err(Error::DebugError(format!(
                "System '{}' is already being profiled",
                system_name
            )));
        }

        // Check concurrent limit
        if sessions.len() >= MAX_CONCURRENT_SYSTEMS {
            return Err(Error::DebugError(format!(
                "Maximum concurrent profiling sessions reached ({})",
                MAX_CONCURRENT_SYSTEMS
            )));
        }

        let session = ProfileSession {
            system_name: system_name.clone(),
            started_at: Instant::now(),
            duration: Duration::from_millis(duration_ms.unwrap_or(DEFAULT_PROFILE_DURATION_MS)),
            samples: Vec::new(),
            track_allocations: track_allocations.unwrap_or(self.config.track_allocations),
        };

        sessions.insert(system_name.clone(), session);
        info!("Started profiling system: {}", system_name);

        Ok(())
    }

    /// Stop profiling a specific system
    pub async fn stop_profiling(&self, system_name: &str) -> Result<SystemProfile> {
        let mut sessions = self.active_sessions.write().await;
        
        let session = sessions.remove(system_name).ok_or_else(|| {
            Error::DebugError(format!("System '{}' is not being profiled", system_name))
        })?;

        // Calculate metrics from collected samples
        let metrics = self.calculate_metrics(&session.samples).await?;

        Ok(SystemProfile {
            system_name: session.system_name,
            metrics,
            samples: session.samples,
            dependencies: self.get_system_dependencies(system_name).await,
        })
    }

    /// Process a profiling command
    pub async fn process_command(&self, command: DebugCommand) -> Result<DebugResponse> {
        match command {
            DebugCommand::ProfileSystem {
                system_name,
                duration_ms,
                track_allocations,
            } => {
                self.start_profiling(system_name.clone(), duration_ms, track_allocations)
                    .await?;

                // If duration specified, schedule auto-stop
                if let Some(duration) = duration_ms {
                    let profiler = self.clone();
                    let system = system_name.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(duration)).await;
                        let _ = profiler.stop_profiling(&system).await;
                    });
                }

                Ok(DebugResponse::ProfilingStarted {
                    system_name,
                    duration_ms,
                })
            }
            _ => Err(Error::DebugError("Unsupported profiling command".to_string())),
        }
    }

    /// Collect profiling sample for a system
    pub async fn collect_sample(
        &self,
        system_name: &str,
        execution_time: Duration,
        allocations: Option<usize>,
    ) -> Result<()> {
        let mut sessions = self.active_sessions.write().await;
        
        if let Some(session) = sessions.get_mut(system_name) {
            // Check if session expired
            if session.started_at.elapsed() > session.duration {
                return Ok(()); // Session expired, ignore sample
            }

            let sample = ProfileSample {
                timestamp: chrono::Utc::now().timestamp_micros() as u64,
                duration_us: execution_time.as_micros() as u64,
                allocations,
            };

            session.samples.push(sample);

            // Check for anomalies
            self.check_for_anomaly(system_name, execution_time).await;
        }

        // Update frame history
        self.update_frame_history(system_name, execution_time, allocations)
            .await;

        Ok(())
    }

    /// Calculate metrics from samples
    async fn calculate_metrics(&self, samples: &[ProfileSample]) -> Result<SystemMetrics> {
        if samples.is_empty() {
            return Ok(SystemMetrics {
                total_time_us: 0,
                min_time_us: 0,
                max_time_us: 0,
                avg_time_us: 0,
                median_time_us: 0,
                p95_time_us: 0,
                p99_time_us: 0,
                total_allocations: 0,
                allocation_rate: 0.0,
                overhead_percent: 0.0,
            });
        }

        let mut durations: Vec<u64> = samples.iter().map(|s| s.duration_us).collect();
        durations.sort_unstable();

        let total_time_us: u64 = durations.iter().sum();
        let min_time_us = *durations.first().unwrap();
        let max_time_us = *durations.last().unwrap();
        let avg_time_us = total_time_us / durations.len() as u64;

        let median_time_us = if durations.len() % 2 == 0 {
            (durations[durations.len() / 2 - 1] + durations[durations.len() / 2]) / 2
        } else {
            durations[durations.len() / 2]
        };

        let p95_index = (durations.len() as f32 * 0.95) as usize;
        let p99_index = (durations.len() as f32 * 0.99) as usize;
        let p95_time_us = durations[p95_index.min(durations.len() - 1)];
        let p99_time_us = durations[p99_index.min(durations.len() - 1)];

        let total_allocations: usize = samples
            .iter()
            .filter_map(|s| s.allocations)
            .sum();

        let allocation_rate = if !samples.is_empty() {
            total_allocations as f32 / samples.len() as f32
        } else {
            0.0
        };

        // Calculate overhead (simplified - would need baseline in real implementation)
        let overhead_percent = self.calculate_overhead(avg_time_us).await;

        Ok(SystemMetrics {
            total_time_us,
            min_time_us,
            max_time_us,
            avg_time_us,
            median_time_us,
            p95_time_us,
            p99_time_us,
            total_allocations,
            allocation_rate,
            overhead_percent,
        })
    }

    /// Calculate profiling overhead
    async fn calculate_overhead(&self, avg_time_us: u64) -> f32 {
        // Simplified overhead calculation
        // In real implementation, would compare with baseline
        let baseline_us = 100; // Assumed baseline execution time
        if baseline_us > 0 {
            ((avg_time_us as f32 - baseline_us as f32) / baseline_us as f32) * 100.0
        } else {
            0.0
        }
    }

    /// Update frame history with system execution data
    async fn update_frame_history(
        &self,
        system_name: &str,
        execution_time: Duration,
        allocations: Option<usize>,
    ) {
        let mut history = self.frame_history.write().await;
        
        // Get or create current frame
        let frame = history.get_or_create_current_frame();
        
        frame.system_times.insert(system_name.to_string(), execution_time);
        if let Some(allocs) = allocations {
            frame.system_allocations.insert(system_name.to_string(), allocs);
        }
    }

    /// Check for performance anomalies
    async fn check_for_anomaly(&self, system_name: &str, execution_time: Duration) {
        let mut detector = self.anomaly_detector.write().await;
        detector.check_anomaly(system_name, execution_time);
    }

    /// Get system dependencies
    async fn get_system_dependencies(&self, system_name: &str) -> Vec<String> {
        let graph = self.dependency_graph.read().await;
        graph.dependencies.get(system_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Update system dependency graph
    pub async fn update_dependency_graph(
        &self,
        system_name: String,
        dependencies: Vec<String>,
    ) {
        let mut graph = self.dependency_graph.write().await;
        
        // Update forward dependencies
        graph.dependencies.insert(system_name.clone(), dependencies.clone());
        
        // Update reverse dependencies
        for dep in &dependencies {
            graph.dependents
                .entry(dep.clone())
                .or_insert_with(Vec::new)
                .push(system_name.clone());
        }
    }

    /// Get profiling history for a system
    pub async fn get_system_history(&self, system_name: &str) -> Vec<ProfileSample> {
        let history = self.frame_history.read().await;
        let mut samples = Vec::new();

        for frame in &history.frames {
            if let Some(&duration) = frame.system_times.get(system_name) {
                samples.push(ProfileSample {
                    timestamp: frame.started_at_us,
                    duration_us: duration.as_micros() as u64,
                    allocations: frame.system_allocations.get(system_name).copied(),
                });
            }
        }

        samples
    }

    /// Get detected anomalies
    pub async fn get_anomalies(&self) -> Vec<PerformanceAnomaly> {
        let detector = self.anomaly_detector.read().await;
        detector.anomalies.clone()
    }

    /// Clear profiling data
    pub async fn clear_profiling_data(&self) {
        let mut sessions = self.active_sessions.write().await;
        sessions.clear();

        let mut history = self.frame_history.write().await;
        history.clear();

        let mut detector = self.anomaly_detector.write().await;
        detector.anomalies.clear();
    }

    /// Export profiling data in various formats
    pub async fn export_profiling_data(&self, format: ExportFormat) -> Result<String> {
        let sessions = self.active_sessions.read().await;
        let history = self.frame_history.read().await;
        
        match format {
            ExportFormat::Json => {
                let data = serde_json::json!({
                    "active_sessions": sessions.keys().collect::<Vec<_>>(),
                    "frame_count": history.frames.len(),
                    "current_frame": history.current_frame,
                });
                Ok(serde_json::to_string_pretty(&data)?)
            }
            ExportFormat::TracyJson => {
                // Format for Tracy profiler integration
                self.export_tracy_format(&history).await
            }
            ExportFormat::Csv => {
                // CSV format for analysis
                self.export_csv_format(&history).await
            }
        }
    }

    /// Export data in Tracy-compatible format
    async fn export_tracy_format(&self, history: &FrameHistory) -> Result<String> {
        // Simplified Tracy export
        let tracy_data = serde_json::json!({
            "frames": history.frames.iter().map(|f| {
                serde_json::json!({
                    "number": f.frame_number,
                    "duration_ms": f.duration.as_millis(),
                    "systems": f.system_times.iter().map(|(name, duration)| {
                        serde_json::json!({
                            "name": name,
                            "duration_us": duration.as_micros()
                        })
                    }).collect::<Vec<_>>()
                })
            }).collect::<Vec<_>>()
        });
        
        Ok(serde_json::to_string(&tracy_data)?)
    }

    /// Export data in CSV format
    async fn export_csv_format(&self, history: &FrameHistory) -> Result<String> {
        let mut csv = String::from("frame,system,duration_us,allocations\n");
        
        for frame in &history.frames {
            for (system, duration) in &frame.system_times {
                let allocations = frame.system_allocations.get(system)
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "N/A".to_string());
                
                csv.push_str(&format!(
                    "{},{},{},{}\n",
                    frame.frame_number,
                    system,
                    duration.as_micros(),
                    allocations
                ));
            }
        }
        
        Ok(csv)
    }
}

impl Clone for SystemProfiler {
    fn clone(&self) -> Self {
        Self {
            brp_client: self.brp_client.clone(),
            active_sessions: self.active_sessions.clone(),
            frame_history: self.frame_history.clone(),
            dependency_graph: self.dependency_graph.clone(),
            anomaly_detector: self.anomaly_detector.clone(),
            config: self.config.clone(),
        }
    }
}

impl FrameHistory {
    fn new(max_frames: usize) -> Self {
        Self {
            frames: VecDeque::with_capacity(max_frames),
            max_frames,
            current_frame: 0,
        }
    }

    fn get_or_create_current_frame(&mut self) -> &mut FrameData {
        if self.frames.is_empty() || self.should_create_new_frame() {
            self.create_new_frame();
        }
        self.frames.back_mut().unwrap()
    }

    fn should_create_new_frame(&self) -> bool {
        // Create new frame if last frame is older than 16ms (60 FPS)
        if let Some(last_frame) = self.frames.back() {
            last_frame.start_time.elapsed() > Duration::from_millis(16)
        } else {
            true
        }
    }

    fn create_new_frame(&mut self) {
        self.current_frame += 1;
        
        let frame = FrameData {
            frame_number: self.current_frame,
            start_time: Instant::now(),
            started_at_us: chrono::Utc::now().timestamp_micros() as u64,
            duration: Duration::default(),
            system_times: HashMap::new(),
            system_allocations: HashMap::new(),
        };

        self.frames.push_back(frame);

        // Maintain size limit
        if self.frames.len() > self.max_frames {
            self.frames.pop_front();
        }
    }

    fn clear(&mut self) {
        self.frames.clear();
        self.current_frame = 0;
    }
}

impl AnomalyDetector {
    fn new(threshold_multiplier: f32) -> Self {
        Self {
            moving_average: MovingAverage::new(100), // 100-frame window
            anomalies: Vec::new(),
            threshold_multiplier,
        }
    }

    fn check_anomaly(&mut self, system_name: &str, execution_time: Duration) {
        let time_ms = execution_time.as_millis() as f32;
        let average = self.moving_average.get_average();
        
        if average > 0.0 && time_ms > average * self.threshold_multiplier {
            let anomaly = PerformanceAnomaly {
                frame_number: 0, // Would get from frame history
                system_name: system_name.to_string(),
                execution_time_ms: time_ms,
                expected_time_ms: average,
                severity: self.calculate_severity(time_ms, average),
                detected_at_us: Instant::now().elapsed().as_micros() as u64,
            };
            
            self.anomalies.push(anomaly);
            
            // Keep only last 100 anomalies
            if self.anomalies.len() > 100 {
                self.anomalies.remove(0);
            }
            
            warn!(
                "Performance anomaly detected in system '{}': {}ms (expected: {}ms)",
                system_name, time_ms, average
            );
        }
        
        self.moving_average.add_value(time_ms);
    }

    fn calculate_severity(&self, actual: f32, expected: f32) -> u8 {
        let ratio = actual / expected;
        match ratio {
            r if r < 1.5 => 1,
            r if r < 2.0 => 2,
            r if r < 3.0 => 3,
            r if r < 5.0 => 4,
            _ => 5,
        }
    }
}

impl MovingAverage {
    fn new(window_size: usize) -> Self {
        Self {
            window: VecDeque::with_capacity(window_size),
            window_size,
            sum: 0.0,
        }
    }

    fn add_value(&mut self, value: f32) {
        self.window.push_back(value);
        self.sum += value;

        if self.window.len() > self.window_size {
            if let Some(old_value) = self.window.pop_front() {
                self.sum -= old_value;
            }
        }
    }

    fn get_average(&self) -> f32 {
        if self.window.is_empty() {
            0.0
        } else {
            self.sum / self.window.len() as f32
        }
    }
}

/// Export format for profiling data
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    /// JSON format
    Json,
    /// Tracy profiler format
    TracyJson,
    /// CSV format for spreadsheet analysis
    Csv,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Config;

    #[tokio::test]
    async fn test_profiler_creation() {
        let mut config = Config::default();
        config.mcp_port = 3000;
        let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
        let profiler = SystemProfiler::new(brp_client);
        
        assert!(profiler.config.max_overhead_percent > 0.0);
    }

    #[tokio::test]
    async fn test_start_stop_profiling() {
        let mut config = Config::default();
        config.mcp_port = 3000;
        let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
        let profiler = SystemProfiler::new(brp_client);
        
        // Start profiling
        let result = profiler.start_profiling(
            "test_system".to_string(),
            Some(1000),
            Some(false),
        ).await;
        assert!(result.is_ok());
        
        // Try to start again (should fail)
        let result = profiler.start_profiling(
            "test_system".to_string(),
            Some(1000),
            Some(false),
        ).await;
        assert!(result.is_err());
        
        // Stop profiling
        let result = profiler.stop_profiling("test_system").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_metrics_calculation() {
        let mut config = Config::default();
        config.mcp_port = 3000;
        let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
        let profiler = SystemProfiler::new(brp_client);
        
        let samples = vec![
            ProfileSample {
                timestamp: 1000,
                duration_us: 100,
                allocations: Some(10),
            },
            ProfileSample {
                timestamp: 2000,
                duration_us: 150,
                allocations: Some(15),
            },
            ProfileSample {
                timestamp: 3000,
                duration_us: 200,
                allocations: Some(20),
            },
        ];
        
        let metrics = profiler.calculate_metrics(&samples).await.unwrap();
        
        assert_eq!(metrics.min_time_us, 100);
        assert_eq!(metrics.max_time_us, 200);
        assert_eq!(metrics.avg_time_us, 150);
        assert_eq!(metrics.total_allocations, 45);
    }

    #[test]
    fn test_moving_average() {
        let mut avg = MovingAverage::new(3);
        
        avg.add_value(10.0);
        assert_eq!(avg.get_average(), 10.0);
        
        avg.add_value(20.0);
        assert_eq!(avg.get_average(), 15.0);
        
        avg.add_value(30.0);
        assert_eq!(avg.get_average(), 20.0);
        
        avg.add_value(40.0); // Should remove 10.0
        assert_eq!(avg.get_average(), 30.0);
    }

    #[test]
    fn test_anomaly_detection() {
        let mut detector = AnomalyDetector::new(1.5);
        
        // Build baseline
        for _ in 0..10 {
            detector.check_anomaly("test_system", Duration::from_millis(10));
        }
        
        // Normal execution
        detector.check_anomaly("test_system", Duration::from_millis(12));
        assert_eq!(detector.anomalies.len(), 0);
        
        // Anomaly
        detector.check_anomaly("test_system", Duration::from_millis(30));
        assert_eq!(detector.anomalies.len(), 1);
        assert_eq!(detector.anomalies[0].system_name, "test_system");
    }
}