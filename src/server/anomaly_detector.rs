/// Anomaly detection system for automatic game state monitoring
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::server::brp_messages::EntityData;
use crate::server::error::{Error, Result};

/// Types of anomalies the detectors report. Each is emitted by one detector; see
/// [`AnomalyDetector::emits`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AnomalyType {
    /// Entity velocity is far outside that entity's own recent speeds
    PhysicsViolation,
    /// Entity components have contradictory values
    StateInconsistency,
    /// Entity count growing abnormally
    EntityCountSpike,
}

impl AnomalyType {
    /// Get human-readable description
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::PhysicsViolation => "Entity velocity far outside its own recent speeds",
            Self::StateInconsistency => "Entity has contradictory component values",
            Self::EntityCountSpike => "Abnormal increase in entity count",
        }
    }
}

/// What one detector could read on its last `detect` call. A detector that finds nothing it
/// reads reports nothing, and an empty anomaly list from it says nothing about the game.
#[derive(Debug, Clone, Serialize)]
pub struct DetectorCoverage {
    pub detector: &'static str,
    /// What the detector reads, in the game's terms.
    pub reads: &'static str,
    /// Readings it took: entities carrying what it reads (the performance detector reads one
    /// number, the entity count).
    pub examined: usize,
    /// Readings it had enough history to judge.
    pub evaluated: usize,
    /// `evaluated`, or `not evaluated: <why>`.
    pub status: String,
}

/// BRP keys a component by its full type path (`my_game::physics::Velocity`), so a detector that
/// reads a component by name matches the whole last path segment. Two components with the same
/// last segment resolve to the one whose path sorts first, so a read never depends on hash order.
fn component_named<'a>(entity: &'a EntityData, name: &str) -> Option<&'a serde_json::Value> {
    entity
        .components
        .iter()
        .filter(|(path, _)| path.rsplit("::").next() == Some(name))
        .min_by_key(|(path, _)| path.as_str())
        .map(|(_, value)| value)
}

/// A vector as BRP sends it: `[x, y, z]` for a `Vec3` (`[x, y]` for a `Vec2`), or `{x, y, z}`.
fn vector_of(value: &serde_json::Value) -> Option<[f32; 3]> {
    let axis = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_f64).map(|n| n as f32);
    match value {
        serde_json::Value::Array(items) if items.len() == 2 || items.len() == 3 => {
            let z = if items.len() == 3 { axis(items.get(2))? } else { 0.0 };
            Some([axis(items.first())?, axis(items.get(1))?, z])
        }
        serde_json::Value::Object(fields) => {
            Some([axis(fields.get("x"))?, axis(fields.get("y"))?, axis(fields.get("z")).unwrap_or(0.0)])
        }
        _ => None,
    }
}

/// How far past its threshold a reading is: 0 at the threshold, 0.5 at twice it, towards 1. It is
/// only asked about a reading already over the threshold, where `value / threshold` is above 1
/// and a `.min(1.0)` on it reads 1 for every detection.
fn severity_past(value: f32, threshold: f32) -> f32 {
    (1.0 - threshold / value).clamp(0.0, 1.0)
}

/// Detected anomaly with context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anomaly {
    pub anomaly_type: AnomalyType,
    pub entity_id: Option<u64>,
    pub component: Option<String>,
    /// How far past its threshold the reading is, from 0 (at the threshold) towards 1: 0.5 is
    /// twice the threshold, 0.9 ten times. A rule violation with no magnitude (a state
    /// inconsistency) carries a fixed 0.9 rating, which is not a measurement.
    pub severity: f32,
    pub description: String,
    pub detected_at: chrono::DateTime<chrono::Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Configuration for anomaly detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyConfig {
    /// Window size for statistical analysis (number of samples)
    pub window_size: usize,
    /// Z-score threshold for outlier detection
    pub z_score_threshold: f32,
    /// Minimum samples required before detection
    pub min_samples: usize,
    /// Entity count growth threshold (entities per sample)
    pub entity_growth_threshold: f32,
    /// Known acceptable anomalies to whitelist
    pub whitelist: Vec<AnomalyPattern>,
}

impl Default for AnomalyConfig {
    fn default() -> Self {
        Self {
            window_size: 100,
            z_score_threshold: 3.0,
            min_samples: 10,
            entity_growth_threshold: 10.0,
            whitelist: Vec::new(),
        }
    }
}

/// Pattern for whitelisting known acceptable anomalies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalyPattern {
    pub anomaly_type: AnomalyType,
    pub entity_pattern: Option<String>, // Regex for entity ID or component
    pub threshold_override: Option<f32>,
}

/// Historical data point for statistical analysis
#[derive(Debug, Clone)]
struct DataPoint {
    value: f32,
}

/// Ring buffer for efficient sliding window operations
pub struct RingBuffer<T> {
    data: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with specified capacity
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        // Not `VecDeque::with_capacity(capacity)`: a capacity is a limit the caller chose, and
        // `configure` takes any positive window size, so a window of `usize::MAX` has to be a
        // window and not an allocation.
        Self {
            data: VecDeque::new(),
            capacity: capacity.max(1), // Ensure minimum capacity of 1
        }
    }

    /// Change the capacity. Shrinking keeps the newest values; growing keeps all of them.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.max(1);
        while self.data.len() > self.capacity {
            self.data.pop_front();
        }
    }

    /// Add a value to the buffer, removing oldest if at capacity
    pub fn push(&mut self, value: T) {
        if self.data.len() >= self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(value);
    }

    /// Get all values in the buffer
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.data.iter()
    }

    /// Get the number of items in the buffer
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if the buffer is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Clear all values from the buffer
    pub fn clear(&mut self) {
        self.data.clear();
    }
}

/// Statistical analysis utilities
pub struct Statistics;

impl Statistics {
    /// Calculate mean of values
    #[must_use]
    pub fn mean(values: &[f32]) -> f32 {
        if values.is_empty() {
            return 0.0;
        }
        values.iter().sum::<f32>() / values.len() as f32
    }

    /// Calculate standard deviation
    #[must_use]
    pub fn std_dev(values: &[f32]) -> f32 {
        if values.len() < 2 {
            return 0.0;
        }
        let mean = Self::mean(values);
        let variance =
            values.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (values.len() - 1) as f32;
        if variance.is_finite() && variance >= 0.0 {
            variance.sqrt()
        } else {
            0.0
        }
    }

    /// Calculate z-score for a value
    #[must_use]
    pub fn z_score(value: f32, mean: f32, std_dev: f32) -> f32 {
        if std_dev == 0.0 || !std_dev.is_finite() || !value.is_finite() || !mean.is_finite() {
            return 0.0;
        }
        (value - mean) / std_dev
    }

    /// Calculate quartiles for IQR analysis
    #[must_use]
    pub fn quartiles(values: &[f32]) -> (f32, f32, f32) {
        if values.is_empty() {
            return (0.0, 0.0, 0.0);
        }

        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = sorted.len();
        let q1_idx = len / 4;
        let q2_idx = len / 2;
        let q3_idx = (3 * len) / 4;

        (sorted[q1_idx], sorted[q2_idx], sorted[q3_idx.min(len - 1)])
    }

    /// Detect outliers using IQR method
    #[must_use]
    pub fn is_outlier_iqr(value: f32, values: &[f32], multiplier: f32) -> bool {
        let (q1, _, q3) = Self::quartiles(values);
        let iqr = q3 - q1;
        let lower_bound = q1 - multiplier * iqr;
        let upper_bound = q3 + multiplier * iqr;
        value < lower_bound || value > upper_bound
    }
}

/// Trait for anomaly detection implementations
pub trait AnomalyDetector: Send + Sync {
    /// Process game state data and detect anomalies
    fn detect(&mut self, entities: &[EntityData]) -> Result<Vec<Anomaly>>;

    /// Get detector name for logging
    fn name(&self) -> &str;

    /// Update detector configuration
    fn configure(&mut self, config: &AnomalyConfig);

    /// The anomaly types this detector can report; `status` lists them, so they cannot drift.
    fn emits(&self) -> &'static [AnomalyType];

    /// What the detector could read on its last `detect` call (before the first, nothing).
    fn coverage(&self) -> DetectorCoverage;
}

/// Physics violation detector
pub struct PhysicsDetector {
    velocity_history: HashMap<u64, RingBuffer<DataPoint>>,
    config: AnomalyConfig,
    /// What the last `detect` call could read, for `coverage`.
    last_examined: usize,
    last_evaluated: usize,
    longest_history: usize,
}

impl PhysicsDetector {
    /// Create a new physics detector
    #[must_use]
    pub fn new(config: AnomalyConfig) -> Self {
        Self {
            velocity_history: HashMap::new(),
            config,
            last_examined: 0,
            last_evaluated: 0,
            longest_history: 0,
        }
    }

    fn extract_velocity_magnitude(&self, entity: &EntityData) -> Option<f32> {
        let [x, y, z] = vector_of(component_named(entity, "Velocity")?.get("linear")?)?;
        Some((x * x + y * y + z * z).sqrt())
    }
}

impl AnomalyDetector for PhysicsDetector {
    fn detect(&mut self, entities: &[EntityData]) -> Result<Vec<Anomaly>> {
        let mut anomalies = Vec::new();
        self.last_examined = 0;
        self.last_evaluated = 0;
        self.longest_history = 0;

        for entity in entities {
            if let Some(velocity_mag) = self.extract_velocity_magnitude(entity) {
                let history = self
                    .velocity_history
                    .entry(entity.id)
                    .or_insert_with(|| RingBuffer::new(self.config.window_size));

                history.push(DataPoint {
                    value: velocity_mag,
                });
                self.last_examined += 1;
                self.longest_history = self.longest_history.max(history.len());

                // Only analyze if we have enough samples
                if history.len() >= self.config.min_samples {
                    self.last_evaluated += 1;
                    let values: Vec<f32> = history.values().map(|dp| dp.value).collect();
                    let mean = Statistics::mean(&values);
                    let std_dev = Statistics::std_dev(&values);
                    let z_score = Statistics::z_score(velocity_mag, mean, std_dev);

                    if z_score.abs() > self.config.z_score_threshold {
                        let severity = severity_past(z_score.abs(), self.config.z_score_threshold);

                        let metadata = [
                            ("velocity_magnitude", serde_json::json!(velocity_mag)),
                            ("mean_velocity", serde_json::json!(mean)),
                            ("z_score", serde_json::json!(z_score)),
                        ]
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect();

                        anomalies.push(Anomaly {
                            anomaly_type: AnomalyType::PhysicsViolation,
                            entity_id: Some(entity.id),
                            component: Some("Velocity".to_string()),
                            severity,
                            description: format!(
                                "Entity {} velocity magnitude {:.2} is {:.2} standard deviations from mean {:.2}",
                                entity.id, velocity_mag, z_score, mean
                            ),
                            detected_at: chrono::Utc::now(),
                            metadata,
                        });
                    }
                }
            }
        }

        Ok(anomalies)
    }

    fn emits(&self) -> &'static [AnomalyType] {
        &[AnomalyType::PhysicsViolation]
    }

    fn coverage(&self) -> DetectorCoverage {
        let min = self.config.min_samples;
        let status = if self.last_examined == 0 {
            "not evaluated: no entity has a `Velocity` component with a `linear` vector".to_string()
        } else if self.last_evaluated == 0 {
            format!(
                "not evaluated: the longest velocity history holds {} of the {min} samples needed; call detect again",
                self.longest_history
            )
        } else if self.last_evaluated < self.last_examined {
            format!(
                "evaluated {} of {} entities; the rest are short of the {min} samples needed",
                self.last_evaluated, self.last_examined
            )
        } else {
            "evaluated".to_string()
        };
        DetectorCoverage {
            detector: "PhysicsDetector",
            reads: "the `linear` vector (`[x, y, z]` or `{x, y, z}`) of a component named `Velocity`, compared with that entity's own recent speeds",
            examined: self.last_examined,
            evaluated: self.last_evaluated,
            status,
        }
    }

    fn name(&self) -> &str {
        "PhysicsDetector"
    }

    fn configure(&mut self, config: &AnomalyConfig) {
        // Each entity's history was sized when the entity was first seen.
        for history in self.velocity_history.values_mut() {
            history.set_capacity(config.window_size);
        }
        self.config = config.clone();
    }
}

/// Performance metrics detector
pub struct PerformanceDetector {
    entity_counts: RingBuffer<DataPoint>,
    config: AnomalyConfig,
}

impl PerformanceDetector {
    /// Create a new performance detector
    #[must_use]
    pub fn new(config: AnomalyConfig) -> Self {
        Self {
            entity_counts: RingBuffer::new(config.window_size),
            config,
        }
    }

    /// Samples held before growth can be judged: at least 2, since growth is a change between them.
    fn samples_needed(&self) -> usize {
        self.config.min_samples.max(2)
    }
}

impl AnomalyDetector for PerformanceDetector {
    fn detect(&mut self, entities: &[EntityData]) -> Result<Vec<Anomaly>> {
        let mut anomalies = Vec::new();

        // Track entity count growth
        let current_count = entities.len();
        self.entity_counts.push(DataPoint {
            value: current_count as f32,
        });

        // Growth is the change across the buffer divided by the samples that change spans, which
        // is one fewer than the samples held. `window_size` is only how many samples the buffer
        // may hold, so dividing by it read a 20-per-sample leak as 0.2 under a window of 100.
        if self.entity_counts.len() >= self.samples_needed() {
            let values: Vec<f32> = self.entity_counts.values().map(|dp| dp.value).collect();
            let samples_spanned = values.len();

            // Check for rapid entity growth
            if let (Some(first), Some(last)) = (values.first(), values.last()) {
                let growth_rate = (last - first) / (samples_spanned - 1) as f32;

                if growth_rate > self.config.entity_growth_threshold {
                    let severity = severity_past(growth_rate, self.config.entity_growth_threshold);

                    let metadata = [
                        ("growth_rate", serde_json::json!(growth_rate)),
                        ("samples_spanned", serde_json::json!(samples_spanned)),
                        ("entity_count", serde_json::json!(current_count)),
                    ]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect();

                    anomalies.push(Anomaly {
                        anomaly_type: AnomalyType::EntityCountSpike,
                        entity_id: None,
                        component: None,
                        severity,
                        description: format!(
                            "Entity count growing at {:.2} entities per sample over the last {} samples (threshold: {:.2})",
                            growth_rate, samples_spanned, self.config.entity_growth_threshold
                        ),
                        detected_at: chrono::Utc::now(),
                        metadata,
                    });
                }
            }
        }

        Ok(anomalies)
    }

    fn emits(&self) -> &'static [AnomalyType] {
        &[AnomalyType::EntityCountSpike]
    }

    fn coverage(&self) -> DetectorCoverage {
        let (held, needed) = (self.entity_counts.len(), self.samples_needed());
        let evaluated = usize::from(held >= needed);
        DetectorCoverage {
            detector: "PerformanceDetector",
            reads: "the number of entities the query returned, compared across `detect` calls",
            examined: usize::from(held > 0),
            evaluated,
            status: if evaluated == 1 {
                "evaluated".to_string()
            } else {
                format!("not evaluated: holds {held} of the {needed} samples it needs; call detect again")
            },
        }
    }

    fn name(&self) -> &str {
        "PerformanceDetector"
    }

    fn configure(&mut self, config: &AnomalyConfig) {
        self.entity_counts.set_capacity(config.window_size);
        self.config = config.clone();
    }
}

/// State consistency detector
pub struct ConsistencyDetector {
    config: AnomalyConfig,
    /// Entities that carried what the detector reads on its last `detect` call.
    last_examined: usize,
}

impl ConsistencyDetector {
    /// Create a new consistency detector
    #[must_use]
    pub fn new(config: AnomalyConfig) -> Self {
        Self { config, last_examined: 0 }
    }

    /// Health and alive as the entity carries them, when it carries both. An entity with no
    /// `Alive` component is not marked as anything, so the absence is not read as alive.
    fn health_and_alive(entity: &EntityData) -> Option<(f32, bool)> {
        let health = component_named(entity, "Health")?.get("current")?.as_f64()? as f32;
        let alive = component_named(entity, "Alive")?.as_bool()?;
        Some((health, alive))
    }

    fn inconsistency(entity_id: u64, health: f32, alive: bool) -> Anomaly {
        let metadata = [
            ("health", serde_json::json!(health)),
            ("alive", serde_json::json!(alive)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        Anomaly {
            anomaly_type: AnomalyType::StateInconsistency,
            entity_id: Some(entity_id),
            component: Some("Health/Alive".to_string()),
            severity: 0.9, // A fixed rating for a broken rule, not a measurement.
            description: format!("Entity {entity_id} has health {health:.1} but is marked as alive"),
            detected_at: chrono::Utc::now(),
            metadata,
        }
    }
}

impl AnomalyDetector for ConsistencyDetector {
    fn detect(&mut self, entities: &[EntityData]) -> Result<Vec<Anomaly>> {
        let mut anomalies = Vec::new();
        self.last_examined = 0;

        for entity in entities {
            let Some((health, alive)) = Self::health_and_alive(entity) else {
                continue;
            };
            self.last_examined += 1;
            if health <= 0.0 && alive {
                anomalies.push(Self::inconsistency(entity.id, health, alive));
            }
        }

        Ok(anomalies)
    }

    fn emits(&self) -> &'static [AnomalyType] {
        &[AnomalyType::StateInconsistency]
    }

    fn coverage(&self) -> DetectorCoverage {
        DetectorCoverage {
            detector: "ConsistencyDetector",
            reads: "`current` of a component named `Health` together with a boolean component named `Alive`; flags health at or below zero on an entity marked alive",
            examined: self.last_examined,
            evaluated: self.last_examined,
            status: if self.last_examined == 0 {
                "not evaluated: no entity has both a `Health` component with a numeric `current` and a boolean `Alive` component".to_string()
            } else {
                "evaluated".to_string()
            },
        }
    }

    fn name(&self) -> &str {
        "ConsistencyDetector"
    }

    fn configure(&mut self, config: &AnomalyConfig) {
        self.config = config.clone();
    }
}

/// Composite anomaly detection system
pub struct AnomalyDetectionSystem {
    detectors: Vec<Box<dyn AnomalyDetector>>,
    config: AnomalyConfig,
    monitoring_channel: Option<mpsc::Receiver<Vec<EntityData>>>,
    anomaly_sender: Option<mpsc::Sender<Vec<Anomaly>>>,
}

impl AnomalyDetectionSystem {
    /// Create a new anomaly detection system
    #[must_use]
    pub fn new(config: AnomalyConfig) -> Self {
        let detectors: Vec<Box<dyn AnomalyDetector>> = vec![
            Box::new(PhysicsDetector::new(config.clone())),
            Box::new(PerformanceDetector::new(config.clone())),
            Box::new(ConsistencyDetector::new(config.clone())),
        ];

        Self {
            detectors,
            config,
            monitoring_channel: None,
            anomaly_sender: None,
        }
    }

    /// Set up monitoring channels for async operation
    pub fn setup_channels(
        &mut self,
    ) -> (mpsc::Sender<Vec<EntityData>>, mpsc::Receiver<Vec<Anomaly>>) {
        let (entity_sender, entity_receiver) = mpsc::channel::<Vec<EntityData>>(100);
        let (anomaly_sender, anomaly_receiver) = mpsc::channel::<Vec<Anomaly>>(100);

        self.monitoring_channel = Some(entity_receiver);
        self.anomaly_sender = Some(anomaly_sender);

        (entity_sender, anomaly_receiver)
    }

    /// What each detector could read on the last `detect_anomalies` call.
    #[must_use]
    pub fn coverage(&self) -> Vec<DetectorCoverage> {
        self.detectors.iter().map(|d| d.coverage()).collect()
    }

    /// The anomaly types the detectors can report, in detector order.
    #[must_use]
    pub fn supported_anomaly_types(&self) -> Vec<AnomalyType> {
        self.detectors.iter().flat_map(|d| d.emits().iter().cloned()).collect()
    }

    /// Process entities through all detectors
    pub fn detect_anomalies(&mut self, entities: &[EntityData]) -> Result<Vec<Anomaly>> {
        let mut all_anomalies = Vec::new();

        for detector in &mut self.detectors {
            match detector.detect(entities) {
                Ok(mut anomalies) => {
                    debug!("{} detected {} anomalies", detector.name(), anomalies.len());
                    all_anomalies.append(&mut anomalies);
                }
                Err(e) => {
                    warn!("Detector {} failed: {}", detector.name(), e);
                }
            }
        }

        // Filter out whitelisted anomalies
        all_anomalies = self.filter_whitelisted(all_anomalies);

        // Sort by severity (highest first)
        all_anomalies.sort_by(|a, b| {
            b.severity
                .partial_cmp(&a.severity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        info!("Detected {} anomalies total", all_anomalies.len());
        Ok(all_anomalies)
    }

    /// Start monitoring loop for async operation
    pub async fn start_monitoring(mut self) -> Result<()> {
        let mut receiver = self
            .monitoring_channel
            .take()
            .ok_or_else(|| Error::Brp("Monitoring channel not set up".to_string()))?;

        let anomaly_sender = self
            .anomaly_sender
            .take()
            .ok_or_else(|| Error::Brp("Anomaly sender not set up".to_string()))?;

        info!("Starting anomaly detection monitoring loop");

        while let Some(entities) = receiver.recv().await {
            match self.detect_anomalies(&entities) {
                Ok(anomalies) => {
                    if !anomalies.is_empty() {
                        if let Err(e) = anomaly_sender.send(anomalies).await {
                            warn!("Failed to send anomalies: {}", e);
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("Anomaly detection failed: {}", e);
                }
            }
        }

        info!("Anomaly detection monitoring loop ended");
        Ok(())
    }

    fn filter_whitelisted(&self, anomalies: Vec<Anomaly>) -> Vec<Anomaly> {
        anomalies
            .into_iter()
            .filter(|anomaly| !self.is_whitelisted(anomaly))
            .collect()
    }

    fn is_whitelisted(&self, anomaly: &Anomaly) -> bool {
        // For now, simple type-based whitelisting
        // In a real implementation, this would use regex patterns
        self.config
            .whitelist
            .iter()
            .any(|pattern| pattern.anomaly_type == anomaly.anomaly_type)
    }

    /// The configuration currently in force
    #[must_use]
    pub fn config(&self) -> &AnomalyConfig {
        &self.config
    }

    /// Update configuration for all detectors
    pub fn update_config(&mut self, config: AnomalyConfig) {
        for detector in &mut self.detectors {
            detector.configure(&config);
        }
        self.config = config;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_ring_buffer() {
        let mut buffer = RingBuffer::new(3);

        assert!(buffer.is_empty());

        buffer.push(1);
        buffer.push(2);
        buffer.push(3);

        assert_eq!(buffer.len(), 3);

        buffer.push(4); // Should evict 1

        let values: Vec<_> = buffer.values().cloned().collect();
        assert_eq!(values, vec![2, 3, 4]);
    }

    #[test]
    fn test_statistics() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];

        assert_eq!(Statistics::mean(&values), 3.0);

        let std_dev = Statistics::std_dev(&values);
        assert!((std_dev - 1.58).abs() < 0.01);

        let z_score = Statistics::z_score(6.0, 3.0, std_dev);
        assert!((z_score - 1.9).abs() < 0.1);
    }

    #[test]
    fn test_quartiles() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let (q1, q2, q3) = Statistics::quartiles(&values);

        // Quartile calculation gives different results based on implementation
        // Let's check that the quartiles are reasonable
        assert!(q1 <= q2);
        assert!(q2 <= q3);
        assert!(q1 >= 1.0 && q1 <= 3.0);
        assert!(q2 >= 3.0 && q2 <= 5.0);
        assert!(q3 >= 5.0 && q3 <= 8.0);
    }

    #[test]
    fn test_physics_detector() {
        let config = AnomalyConfig::default();
        let mut detector = PhysicsDetector::new(config);

        // Create test entity with velocity
        let entity = EntityData {
            id: 1,
            components: [(
                "Velocity".to_string(),
                json!({
                    "linear": {"x": 100.0, "y": 0.0, "z": 0.0}
                }),
            )]
            .into_iter()
            .collect(),
        };

        // First detection should not trigger (not enough samples)
        let anomalies = detector.detect(&[entity.clone()]).unwrap();
        assert!(anomalies.is_empty());

        // Add more samples to build history
        for _ in 0..15 {
            let _ = detector.detect(&[entity.clone()]);
        }

        // Now add an extreme value
        let extreme_entity = EntityData {
            id: 1,
            components: [(
                "Velocity".to_string(),
                json!({
                    "linear": {"x": 1000.0, "y": 0.0, "z": 0.0}
                }),
            )]
            .into_iter()
            .collect(),
        };

        let anomalies = detector.detect(&[extreme_entity]).unwrap();
        assert!(!anomalies.is_empty());
        assert_eq!(anomalies[0].anomaly_type, AnomalyType::PhysicsViolation);
    }

    #[test]
    fn test_consistency_detector() {
        let config = AnomalyConfig::default();
        let mut detector = ConsistencyDetector::new(config);

        // Create entity with inconsistent health/alive state
        let entity = EntityData {
            id: 1,
            components: [
                ("Health".to_string(), json!({"current": -10.0})),
                ("Alive".to_string(), json!(true)),
            ]
            .into_iter()
            .collect(),
        };

        let anomalies = detector.detect(&[entity]).unwrap();
        assert!(!anomalies.is_empty());
        assert_eq!(anomalies[0].anomaly_type, AnomalyType::StateInconsistency);
        assert!(anomalies[0].severity > 0.8);
    }

    #[test]
    fn test_performance_detector() {
        let config = AnomalyConfig {
            entity_growth_threshold: 0.5, // Very low threshold for testing
            min_samples: 5,               // Lower minimum samples for testing
            window_size: 10,              // Smaller window for testing
            ..Default::default()
        };
        let mut detector = PerformanceDetector::new(config);

        // Gradually increase entity count to trigger growth anomaly
        // Start with smaller counts to establish baseline
        for i in 1..=10 {
            let entities: Vec<EntityData> = (0..i)
                .map(|id| EntityData {
                    id: id as u64,
                    components: HashMap::new(),
                })
                .collect();

            let _anomalies = detector.detect(&entities).unwrap();
        }

        // Now jump to much larger count to trigger anomaly
        let entities: Vec<EntityData> = (0..50)
            .map(|id| EntityData {
                id: id as u64,
                components: HashMap::new(),
            })
            .collect();

        let anomalies = detector.detect(&entities).unwrap();
        assert!(
            !anomalies.is_empty(),
            "Expected entity count spike anomaly but none found"
        );
        assert_eq!(anomalies[0].anomaly_type, AnomalyType::EntityCountSpike);
    }

    #[test]
    fn test_anomaly_detection_system() {
        let config = AnomalyConfig::default();
        let mut system = AnomalyDetectionSystem::new(config);

        // Create test entities
        let entities = vec![
            EntityData {
                id: 1,
                components: [
                    ("Health".to_string(), json!({"current": -5.0})),
                    ("Alive".to_string(), json!(true)),
                ]
                .into_iter()
                .collect(),
            },
            EntityData {
                id: 2,
                components: [(
                    "Velocity".to_string(),
                    json!({
                        "linear": {"x": 10.0, "y": 0.0, "z": 0.0}
                    }),
                )]
                .into_iter()
                .collect(),
            },
        ];

        let anomalies = system.detect_anomalies(&entities).unwrap();

        // Should detect at least the health/alive inconsistency
        assert!(!anomalies.is_empty());
        assert!(anomalies
            .iter()
            .any(|a| a.anomaly_type == AnomalyType::StateInconsistency));
    }

    fn entities(count: u64) -> Vec<EntityData> {
        (0..count).map(|id| EntityData { id, components: HashMap::new() }).collect()
    }

    /// Growth is entities per sample over the samples the change spans: 20 more entities on each
    /// call is 20 per sample whatever `window_size` is (issue #13).
    #[test]
    fn entity_growth_is_per_sample_spanned_not_per_window_size() {
        let config = AnomalyConfig {
            min_samples: 2,
            window_size: 100,
            entity_growth_threshold: 10.0,
            ..Default::default()
        };
        let mut detector = PerformanceDetector::new(config);
        detector.detect(&entities(0)).unwrap();
        let found = detector.detect(&entities(20)).unwrap();
        assert_eq!(found.len(), 1, "20 entities in one sample is over a threshold of 10 per sample");
        assert_eq!(found[0].metadata["growth_rate"], json!(20.0));
        assert_eq!(found[0].metadata["samples_spanned"], json!(2));
    }

    /// `configure` changes the window the detectors measure over, not only the number `status`
    /// echoes: a window of 2 samples holds 2 samples.
    #[test]
    fn configure_resizes_the_window_that_growth_is_measured_over() {
        let config = AnomalyConfig {
            min_samples: 2,
            window_size: 100,
            entity_growth_threshold: 10.0,
            ..Default::default()
        };
        let mut detector = PerformanceDetector::new(config.clone());
        detector.configure(&AnomalyConfig { window_size: 2, ..config.clone() });
        for n in [0, 20, 40, 60] {
            detector.detect(&entities(n)).unwrap();
        }
        let last = detector.detect(&entities(80)).unwrap();
        assert_eq!(last[0].metadata["growth_rate"], json!(20.0));
        assert_eq!(last[0].metadata["samples_spanned"], json!(2), "a window of 2 spans 2 samples");

        // Widening keeps what was seen and then holds more.
        detector.configure(&AnomalyConfig { window_size: 1000, ..config });
        for n in [100, 120, 140, 160] {
            detector.detect(&entities(n)).unwrap();
        }
        let last = detector.detect(&entities(180)).unwrap();
        assert_eq!(last[0].metadata["samples_spanned"], json!(7), "{last:?}");
        assert_eq!(last[0].metadata["growth_rate"], json!(20.0));
    }

    /// The physics history of an entity is sized when the entity is first seen; `configure` has to
    /// resize it too.
    #[test]
    fn configure_resizes_the_per_entity_velocity_histories() {
        let moving = |speed: f64| EntityData {
            id: 1,
            components: [("Velocity".to_string(), json!({"linear": {"x": speed, "y": 0.0, "z": 0.0}}))]
                .into_iter()
                .collect(),
        };
        let mut detector = PhysicsDetector::new(AnomalyConfig { window_size: 100, ..Default::default() });
        for speed in [1.0, 2.0, 3.0, 4.0] {
            detector.detect(&[moving(speed)]).unwrap();
        }
        assert_eq!(detector.velocity_history[&1].len(), 4);
        detector.configure(&AnomalyConfig { window_size: 2, ..Default::default() });
        assert_eq!(detector.velocity_history[&1].len(), 2, "the history kept more than the window");
        detector.detect(&[moving(5.0)]).unwrap();
        assert_eq!(detector.velocity_history[&1].len(), 2);
    }

    /// `configure` takes any positive number for `window_size`. Sizing a buffer to it must not
    /// allocate it: a window of `usize::MAX` aborted the server.
    #[test]
    fn a_window_larger_than_memory_is_a_window_not_an_allocation() {
        let mut system = AnomalyDetectionSystem::new(AnomalyConfig::default());
        system.update_config(AnomalyConfig { window_size: usize::MAX, ..Default::default() });
        system.detect_anomalies(&entities(3)).unwrap();
        assert_eq!(system.config().window_size, usize::MAX);
    }

    /// B-087: severity was `(value / threshold).min(1.0)`, computed only once `value > threshold`,
    /// so the ratio was already above 1 and every detection read exactly 1.0. `min_severity`
    /// could not tell a reading just past its threshold from one a hundred times past it.
    /// Severity is how far past the threshold a reading is: 0 at it, 0.5 at twice it, towards 1.
    #[test]
    fn entity_growth_severity_rises_with_how_far_past_the_threshold_it_is() {
        let severity_of = |growth: u64| {
            let mut detector = PerformanceDetector::new(AnomalyConfig {
                min_samples: 2,
                window_size: 2,
                entity_growth_threshold: 10.0,
                ..Default::default()
            });
            detector.detect(&entities(0)).unwrap();
            detector.detect(&entities(growth)).unwrap()[0].severity
        };
        let (just_over, double, far_over) = (severity_of(11), severity_of(20), severity_of(1000));
        assert!(just_over < double && double < far_over, "{just_over} {double} {far_over}");
        assert!((double - 0.5).abs() < 1e-6, "twice the threshold is 0.5, got {double}");
        assert!(far_over < 1.0, "severity stays below 1, got {far_over}");
    }

    /// The same data judged against a stricter z-score threshold is further past it.
    #[test]
    fn physics_severity_rises_with_how_far_past_the_z_score_threshold_it_is() {
        let moving = |speed: f64| EntityData {
            id: 1,
            components: [("Velocity".to_string(), json!({"linear": {"x": speed, "y": 0.0, "z": 0.0}}))]
                .into_iter()
                .collect(),
        };
        let severity_under = |z_score_threshold: f32| {
            let mut detector = PhysicsDetector::new(AnomalyConfig {
                min_samples: 5,
                window_size: 20,
                z_score_threshold,
                ..Default::default()
            });
            for _ in 0..9 {
                detector.detect(&[moving(10.0)]).unwrap();
            }
            let found = detector.detect(&[moving(1000.0)]).unwrap();
            assert_eq!(found.len(), 1, "a spike after a flat history is over a threshold of {z_score_threshold}");
            found[0].severity
        };
        let (strict, lax) = (severity_under(1.0), severity_under(2.0));
        assert!(lax < strict, "a lax threshold ({lax}) is not as far past as a strict one ({strict})");
        assert!(strict < 1.0 && lax > 0.0, "{strict} {lax}");
    }

    /// B-090: `world.query` keys a component by its full type path (`my_game::physics::Velocity`)
    /// and sends a `Vec3` as `[x, y, z]`. The detectors looked for the literal key `Velocity` and
    /// an `{x, y, z}` object, which no game sends, so they could not fire against a real one.
    #[test]
    fn the_physics_detector_reads_a_velocity_as_brp_sends_it() {
        let moving = |speed: f64| EntityData {
            id: 1,
            components: [("my_game::physics::Velocity".to_string(), json!({"linear": [speed, 0.0, 0.0]}))]
                .into_iter()
                .collect(),
        };
        let mut detector = PhysicsDetector::new(AnomalyConfig { min_samples: 5, window_size: 20, ..Default::default() });
        for _ in 0..11 {
            assert!(detector.detect(&[moving(10.0)]).unwrap().is_empty());
        }
        let found = detector.detect(&[moving(1000.0)]).unwrap();
        assert_eq!(found.len(), 1, "a spike in a BRP-shaped Velocity must be seen: {found:?}");
        assert_eq!(found[0].anomaly_type, AnomalyType::PhysicsViolation);
        assert_eq!(found[0].metadata["velocity_magnitude"], json!(1000.0));
    }

    fn with_health(id: u64, health_key: &str, current: f64, alive: Option<bool>) -> EntityData {
        let mut components = HashMap::new();
        components.insert(health_key.to_string(), json!({"current": current, "max": 10.0}));
        if let Some(alive) = alive {
            components.insert("my_game::Alive".to_string(), json!(alive));
        }
        EntityData { id, components }
    }

    #[test]
    fn the_consistency_detector_reads_health_and_alive_as_brp_sends_them() {
        let mut detector = ConsistencyDetector::new(AnomalyConfig::default());
        let found = detector.detect(&[with_health(1, "my_game::Health", -3.0, Some(true))]).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].anomaly_type, AnomalyType::StateInconsistency);
        assert_eq!(found[0].entity_id, Some(1));
    }

    /// The description said "is marked as alive" for an entity that has no `Alive` component at
    /// all: the absence was read as `true`. Nothing is contradicted when nothing is marked.
    #[test]
    fn zero_health_with_no_alive_component_is_not_an_inconsistency() {
        let mut detector = ConsistencyDetector::new(AnomalyConfig::default());
        assert!(detector.detect(&[with_health(1, "my_game::Health", -10.0, None)]).unwrap().is_empty());
        assert!(detector.detect(&[with_health(2, "my_game::Health", -10.0, Some(false))]).unwrap().is_empty());
    }

    /// A component is matched on its whole last path segment: `MaxHealth` is not `Health`.
    #[test]
    fn a_component_whose_name_merely_ends_in_health_is_not_health() {
        let mut detector = ConsistencyDetector::new(AnomalyConfig::default());
        assert!(detector.detect(&[with_health(1, "my_game::MaxHealth", -10.0, Some(true))]).unwrap().is_empty());
    }
}
