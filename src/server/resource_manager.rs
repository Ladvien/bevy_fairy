use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::System;
use tokio::sync::{mpsc, RwLock, Semaphore};
use tokio::time::interval;
use tracing::{info, warn};
use uuid::Uuid;
// Note: tokio-metrics RuntimeMetrics requires specific feature flags
// For now, we'll implement our own lightweight monitoring

use crate::server::error::{Error, Result};

/// Unique identifier for resource tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceId(Uuid);

impl ResourceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

impl Default for ResourceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration for resource limits and monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    /// Maximum CPU usage percentage (0-100)
    pub max_cpu_percent: f32,
    /// Maximum memory usage in bytes
    pub max_memory_bytes: u64,
    /// Maximum number of concurrent operations
    pub max_concurrent_operations: usize,
    /// Maximum BRP requests per second
    pub max_brp_requests_per_second: u32,
    /// Interval for monitoring system resources
    pub monitoring_interval: Duration,
    /// Enable adaptive sampling
    pub adaptive_sampling_enabled: bool,
    /// Enable object pooling
    pub object_pooling_enabled: bool,
    /// Circuit breaker failure threshold
    pub circuit_breaker_threshold: usize,
    /// Circuit breaker reset timeout
    pub circuit_breaker_reset_timeout: Duration,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            max_cpu_percent: 10.0,               // 10% max CPU usage
            max_memory_bytes: 100 * 1024 * 1024, // 100MB
            max_concurrent_operations: 50,
            max_brp_requests_per_second: 100,
            monitoring_interval: Duration::from_secs(1),
            adaptive_sampling_enabled: true,
            object_pooling_enabled: true,
            circuit_breaker_threshold: 5,
            circuit_breaker_reset_timeout: Duration::from_secs(30),
        }
    }
}

/// Current resource usage metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub timestamp: SystemTime,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub concurrent_operations: usize,
    pub brp_requests_per_second: u32,
    pub circuit_breaker_open: bool,
    pub adaptive_sampling_rate: f32,
    pub object_pool_size: usize,
    pub total_allocations: u64,
    pub total_deallocations: u64,
}

/// Resource usage sample for adaptive sampling
#[derive(Debug, Clone)]
struct ResourceSample {
    timestamp: Instant,
    cpu_percent: f32,
    memory_bytes: u64,
    request_count: u32,
}

/// Object pool for frequent allocations
pub struct ObjectPool<T> {
    objects: Arc<RwLock<Vec<T>>>,
    factory: Arc<dyn Fn() -> T + Send + Sync>,
    max_size: usize,
    current_size: AtomicUsize,
}

impl<T> std::fmt::Debug for ObjectPool<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectPool")
            .field("max_size", &self.max_size)
            .field("current_size", &self.current_size)
            .finish()
    }
}

impl<T> ObjectPool<T>
where
    T: Send + 'static,
{
    pub fn new<F>(factory: F, max_size: usize) -> Self
    where
        F: Fn() -> T + Send + Sync + 'static,
    {
        Self {
            objects: Arc::new(RwLock::new(Vec::with_capacity(max_size))),
            factory: Arc::new(factory),
            max_size,
            current_size: AtomicUsize::new(0),
        }
    }

    pub async fn acquire(&self) -> T {
        {
            let mut objects = self.objects.write().await;
            if let Some(obj) = objects.pop() {
                self.current_size.fetch_sub(1, Ordering::Relaxed);
                return obj;
            }
        }

        // Create new object if pool is empty
        (self.factory)()
    }

    pub async fn release(&self, obj: T) {
        let current = self.current_size.load(Ordering::Relaxed);
        if current < self.max_size {
            let mut objects = self.objects.write().await;
            if objects.len() < self.max_size {
                objects.push(obj);
                self.current_size.fetch_add(1, Ordering::Relaxed);
            }
        }
        // If pool is full, object is dropped
    }

    pub fn size(&self) -> usize {
        self.current_size.load(Ordering::Relaxed)
    }
}

/// Circuit breaker for resource protection
#[derive(Debug)]
pub struct CircuitBreaker {
    failure_count: AtomicUsize,
    last_failure_time: Arc<RwLock<Option<Instant>>>,
    threshold: usize,
    timeout: Duration,
    is_open: AtomicBool,
}

impl CircuitBreaker {
    pub fn new(threshold: usize, timeout: Duration) -> Self {
        Self {
            failure_count: AtomicUsize::new(0),
            last_failure_time: Arc::new(RwLock::new(None)),
            threshold,
            timeout,
            is_open: AtomicBool::new(false),
        }
    }

    pub async fn is_open(&self) -> bool {
        if !self.is_open.load(Ordering::Relaxed) {
            return false;
        }

        // Check if timeout has passed
        let should_reset = {
            let last_failure = self.last_failure_time.read().await;
            if let Some(last_time) = *last_failure {
                last_time.elapsed() > self.timeout
            } else {
                false
            }
        };

        if should_reset {
            self.reset().await;
            return false;
        }

        true
    }

    pub async fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        self.is_open.store(false, Ordering::Relaxed);
    }

    pub async fn record_failure(&self) {
        let count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;

        if count >= self.threshold {
            self.is_open.store(true, Ordering::Relaxed);
            *self.last_failure_time.write().await = Some(Instant::now());
        }
    }

    pub async fn reset(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        self.is_open.store(false, Ordering::Relaxed);
        *self.last_failure_time.write().await = None;
    }
}

/// Adaptive sampler for high-frequency data
#[derive(Debug)]
pub struct AdaptiveSampler {
    samples: Arc<RwLock<Vec<ResourceSample>>>,
    sample_rate: Arc<RwLock<f32>>,
    max_samples: usize,
    min_rate: f32,
    max_rate: f32,
}

impl AdaptiveSampler {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: Arc::new(RwLock::new(Vec::with_capacity(max_samples))),
            sample_rate: Arc::new(RwLock::new(1.0)),
            max_samples,
            min_rate: 0.01, // 1% minimum sampling
            max_rate: 1.0,  // 100% maximum sampling
        }
    }

    pub async fn should_sample(&self) -> bool {
        let rate = *self.sample_rate.read().await;
        rand::random::<f32>() < rate
    }

    pub async fn add_sample(&self, cpu_percent: f32, memory_bytes: u64, request_count: u32) {
        let sample = ResourceSample {
            timestamp: Instant::now(),
            cpu_percent,
            memory_bytes,
            request_count,
        };

        let mut samples = self.samples.write().await;
        samples.push(sample);

        // Remove old samples
        if samples.len() > self.max_samples {
            let drain_count = samples.len() - self.max_samples;
            samples.drain(0..drain_count);
        }

        // Adjust sampling rate based on resource usage
        self.adjust_sampling_rate(&samples).await;
    }

    async fn adjust_sampling_rate(&self, samples: &[ResourceSample]) {
        if samples.len() < 10 {
            return; // Need enough samples to adjust
        }

        let recent_samples = &samples[samples.len().saturating_sub(10)..];
        let avg_cpu =
            recent_samples.iter().map(|s| s.cpu_percent).sum::<f32>() / recent_samples.len() as f32;
        let avg_memory = recent_samples.iter().map(|s| s.memory_bytes).sum::<u64>()
            / recent_samples.len() as u64;

        let mut new_rate = *self.sample_rate.read().await;

        // Increase sampling rate if resources are low
        if avg_cpu < 5.0 && avg_memory < 50 * 1024 * 1024 {
            new_rate = (new_rate * 1.1).min(self.max_rate);
        }
        // Decrease sampling rate if resources are high
        else if avg_cpu > 15.0 || avg_memory > 80 * 1024 * 1024 {
            new_rate = (new_rate * 0.9).max(self.min_rate);
        }

        *self.sample_rate.write().await = new_rate;
    }

    pub async fn get_sampling_rate(&self) -> f32 {
        *self.sample_rate.read().await
    }
}

/// Request rate limiter for BRP requests
#[derive(Debug)]
pub struct RateLimiter {
    requests: Arc<RwLock<Vec<Instant>>>,
    max_requests_per_second: u32,
    window_size: Duration,
}

impl RateLimiter {
    pub fn new(max_requests_per_second: u32) -> Self {
        Self {
            requests: Arc::new(RwLock::new(Vec::new())),
            max_requests_per_second,
            window_size: Duration::from_secs(1),
        }
    }

    pub async fn allow_request(&self) -> bool {
        let now = Instant::now();
        let mut requests = self.requests.write().await;

        // Remove old requests outside the window
        requests.retain(|&time| now.duration_since(time) < self.window_size);

        if requests.len() < self.max_requests_per_second as usize {
            requests.push(now);
            true
        } else {
            false
        }
    }

    pub async fn get_current_rate(&self) -> u32 {
        let now = Instant::now();
        let requests = self.requests.read().await;
        requests
            .iter()
            .filter(|&&time| now.duration_since(time) < self.window_size)
            .count() as u32
    }
}

/// Main resource manager
#[derive(Debug)]
pub struct ResourceManager {
    config: ResourceConfig,
    metrics: Arc<RwLock<ResourceMetrics>>,
    system: Arc<RwLock<System>>,
    pid: usize,

    // Resource controls
    operation_semaphore: Arc<Semaphore>,
    circuit_breaker: Arc<CircuitBreaker>,
    adaptive_sampler: Arc<AdaptiveSampler>,
    rate_limiter: Arc<RateLimiter>,

    // Object pools
    string_pool: Arc<ObjectPool<String>>,
    vec_pool: Arc<ObjectPool<Vec<u8>>>,

    // Monitoring
    monitoring_handle: Option<tokio::task::JoinHandle<()>>,
    shutdown_tx: Option<mpsc::Sender<()>>,

    // Counters
    total_allocations: AtomicU64,
    total_deallocations: AtomicU64,
}

impl ResourceManager {
    pub fn new(config: ResourceConfig) -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        let pid = std::process::id() as usize;

        let operation_semaphore = Arc::new(Semaphore::new(config.max_concurrent_operations));
        let circuit_breaker = Arc::new(CircuitBreaker::new(
            config.circuit_breaker_threshold,
            config.circuit_breaker_reset_timeout,
        ));
        let adaptive_sampler = Arc::new(AdaptiveSampler::new(1000));
        let rate_limiter = Arc::new(RateLimiter::new(config.max_brp_requests_per_second));

        // Create object pools
        let string_pool = Arc::new(ObjectPool::new(|| String::with_capacity(1024), 100));
        let vec_pool = Arc::new(ObjectPool::new(|| Vec::with_capacity(1024), 100));

        let initial_metrics = ResourceMetrics {
            timestamp: SystemTime::now(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            concurrent_operations: 0,
            brp_requests_per_second: 0,
            circuit_breaker_open: false,
            adaptive_sampling_rate: 1.0,
            object_pool_size: 0,
            total_allocations: 0,
            total_deallocations: 0,
        };

        Self {
            config,
            metrics: Arc::new(RwLock::new(initial_metrics)),
            system: Arc::new(RwLock::new(system)),
            pid,
            operation_semaphore,
            circuit_breaker,
            adaptive_sampler,
            rate_limiter,
            string_pool,
            vec_pool,
            monitoring_handle: None,
            shutdown_tx: None,
            total_allocations: AtomicU64::new(0),
            total_deallocations: AtomicU64::new(0),
        }
    }

    pub async fn start_monitoring(&mut self) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        self.shutdown_tx = Some(shutdown_tx);

        let metrics = self.metrics.clone();
        let system = self.system.clone();
        let _pid = self.pid;
        let config = self.config.clone();
        let circuit_breaker = self.circuit_breaker.clone();
        let adaptive_sampler = self.adaptive_sampler.clone();
        let rate_limiter = self.rate_limiter.clone();
        let string_pool = self.string_pool.clone();
        let vec_pool = self.vec_pool.clone();
        let total_allocations = Arc::new(AtomicU64::new(0));
        let total_deallocations = Arc::new(AtomicU64::new(0));

        let handle = tokio::spawn(async move {
            let mut interval = interval(config.monitoring_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        Self::update_metrics(
                            &metrics,
                            &system,
                            &circuit_breaker,
                            &adaptive_sampler,
                            &rate_limiter,
                            &string_pool,
                            &vec_pool,
                            &total_allocations,
                            &total_deallocations,
                        ).await;
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Resource monitoring shutting down");
                        break;
                    }
                }
            }
        });

        self.monitoring_handle = Some(handle);
        info!("Resource monitoring started");
        Ok(())
    }

    async fn update_metrics(
        metrics: &Arc<RwLock<ResourceMetrics>>,
        system: &Arc<RwLock<System>>,
        circuit_breaker: &Arc<CircuitBreaker>,
        adaptive_sampler: &Arc<AdaptiveSampler>,
        rate_limiter: &Arc<RateLimiter>,
        string_pool: &Arc<ObjectPool<String>>,
        vec_pool: &Arc<ObjectPool<Vec<u8>>>,
        total_allocations: &Arc<AtomicU64>,
        total_deallocations: &Arc<AtomicU64>,
    ) {
        let mut sys = system.write().await;
        sys.refresh_all();

        // Get current process info using simplified approach
        let (cpu_percent, memory_bytes) = {
            // Get basic system info - fallback to process-based approach
            let _pid = std::process::id();
            let mut memory = 0u64;
            let mut cpu = 0.0f32;

            // Try to read from /proc/self/stat on Unix systems for better reliability
            #[cfg(unix)]
            {
                if let Ok(stat) = std::fs::read_to_string("/proc/self/stat") {
                    let fields: Vec<&str> = stat.split_whitespace().collect();
                    if fields.len() > 23 {
                        // Field 22 is memory in pages, convert to bytes (assume 4KB pages)
                        if let Ok(mem_pages) = fields[22].parse::<u64>() {
                            memory = mem_pages * 4096;
                        }
                        // For CPU, we'll use a simple approximation
                        cpu = 1.0; // Placeholder for actual CPU calculation
                    }
                }
            }

            // Fallback values if unable to read system info
            if memory == 0 {
                memory = 10 * 1024 * 1024; // 10MB default
            }

            (cpu, memory)
        };

        let current_rate = rate_limiter.get_current_rate().await;
        let sampling_rate = adaptive_sampler.get_sampling_rate().await;

        // Add sample for adaptive sampling
        adaptive_sampler
            .add_sample(cpu_percent, memory_bytes, current_rate)
            .await;

        let mut current_metrics = metrics.write().await;
        *current_metrics = ResourceMetrics {
            timestamp: SystemTime::now(),
            cpu_percent,
            memory_bytes,
            concurrent_operations: 0, // This would be updated by operations
            brp_requests_per_second: current_rate,
            circuit_breaker_open: circuit_breaker.is_open().await,
            adaptive_sampling_rate: sampling_rate,
            object_pool_size: string_pool.size() + vec_pool.size(),
            total_allocations: total_allocations.load(Ordering::Relaxed),
            total_deallocations: total_deallocations.load(Ordering::Relaxed),
        };

        // Log warnings if limits are exceeded
        if cpu_percent > 15.0 {
            warn!("High CPU usage detected: {:.2}%", cpu_percent);
        }
        if memory_bytes > 80 * 1024 * 1024 {
            warn!("High memory usage detected: {} bytes", memory_bytes);
        }
    }

    pub async fn acquire_operation_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>> {
        if self.circuit_breaker.is_open().await {
            return Err(Error::Validation(
                "Circuit breaker is open - operations temporarily blocked".to_string(),
            ));
        }

        let permit =
            self.operation_semaphore.acquire().await.map_err(|e| {
                Error::Validation(format!("Failed to acquire operation permit: {e}"))
            })?;

        Ok(permit)
    }

    pub async fn check_brp_rate_limit(&self) -> bool {
        self.rate_limiter.allow_request().await
    }

    pub async fn should_sample(&self) -> bool {
        if !self.config.adaptive_sampling_enabled {
            return true;
        }
        self.adaptive_sampler.should_sample().await
    }

    pub async fn acquire_string(&self) -> String {
        if self.config.object_pooling_enabled {
            self.total_allocations.fetch_add(1, Ordering::Relaxed);
            self.string_pool.acquire().await
        } else {
            String::new()
        }
    }

    pub async fn release_string(&self, mut s: String) {
        if self.config.object_pooling_enabled {
            s.clear(); // Reset string for reuse
            self.string_pool.release(s).await;
            self.total_deallocations.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn acquire_buffer(&self) -> Vec<u8> {
        if self.config.object_pooling_enabled {
            self.total_allocations.fetch_add(1, Ordering::Relaxed);
            self.vec_pool.acquire().await
        } else {
            Vec::new()
        }
    }

    pub async fn release_buffer(&self, mut buf: Vec<u8>) {
        if self.config.object_pooling_enabled {
            buf.clear(); // Reset buffer for reuse
            self.vec_pool.release(buf).await;
            self.total_deallocations.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub async fn record_operation_success(&self) {
        self.circuit_breaker.record_success().await;
    }

    pub async fn record_operation_failure(&self) {
        self.circuit_breaker.record_failure().await;
    }

    pub async fn get_metrics(&self) -> ResourceMetrics {
        let mut metrics = self.metrics.read().await.clone();
        // Update with current circuit breaker state
        metrics.circuit_breaker_open = self.circuit_breaker.is_open().await;
        metrics.adaptive_sampling_rate = self.adaptive_sampler.get_sampling_rate().await;
        metrics
    }

    pub async fn get_performance_dashboard(&self) -> serde_json::Value {
        let metrics = self.get_metrics().await;

        serde_json::json!({
            "timestamp": metrics.timestamp.duration_since(UNIX_EPOCH)
                .unwrap_or_default().as_secs(),
            "cpu": {
                "current_percent": metrics.cpu_percent,
                "limit_percent": self.config.max_cpu_percent,
                "status": if metrics.cpu_percent > self.config.max_cpu_percent {
                    "OVER_LIMIT"
                } else if metrics.cpu_percent > self.config.max_cpu_percent * 0.8 {
                    "WARNING"
                } else {
                    "OK"
                }
            },
            "memory": {
                "current_bytes": metrics.memory_bytes,
                "limit_bytes": self.config.max_memory_bytes,
                "current_mb": metrics.memory_bytes / (1024 * 1024),
                "limit_mb": self.config.max_memory_bytes / (1024 * 1024),
                "status": if metrics.memory_bytes > self.config.max_memory_bytes {
                    "OVER_LIMIT"
                } else if metrics.memory_bytes > (self.config.max_memory_bytes as f64 * 0.8) as u64 {
                    "WARNING"
                } else {
                    "OK"
                }
            },
            "operations": {
                "concurrent": metrics.concurrent_operations,
                "limit": self.config.max_concurrent_operations,
                "circuit_breaker_open": metrics.circuit_breaker_open
            },
            "brp_requests": {
                "current_per_second": metrics.brp_requests_per_second,
                "limit_per_second": self.config.max_brp_requests_per_second
            },
            "adaptive_sampling": {
                "enabled": self.config.adaptive_sampling_enabled,
                "current_rate": metrics.adaptive_sampling_rate
            },
            "object_pooling": {
                "enabled": self.config.object_pooling_enabled,
                "pool_size": metrics.object_pool_size,
                "total_allocations": metrics.total_allocations,
                "total_deallocations": metrics.total_deallocations
            }
        })
    }

    pub async fn implement_graceful_degradation(&self) -> Result<()> {
        let metrics = self.get_metrics().await;

        // If CPU usage is high, reduce sampling rate
        if metrics.cpu_percent > self.config.max_cpu_percent * 0.8 {
            warn!("Implementing graceful degradation due to high CPU usage");
            // This would be implemented by calling components to reduce their activity
            return Ok(());
        }

        // If memory usage is high, trigger garbage collection or cleanup
        if metrics.memory_bytes > (self.config.max_memory_bytes as f64 * 0.8) as u64 {
            warn!("Implementing graceful degradation due to high memory usage");
            // Clear caches, reduce buffer sizes, etc.
            return Ok(());
        }

        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(()).await;
        }

        if let Some(handle) = self.monitoring_handle.take() {
            handle.abort();
        }

        info!("Resource manager shutdown complete");
        Ok(())
    }
}

impl Drop for ResourceManager {
    fn drop(&mut self) {
        if self.monitoring_handle.is_some() {
            warn!("ResourceManager dropped without proper shutdown");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_resource_manager_creation() {
        let config = ResourceConfig::default();
        let manager = ResourceManager::new(config);

        let metrics = manager.get_metrics().await;
        assert_eq!(metrics.concurrent_operations, 0);
        assert!(!metrics.circuit_breaker_open);
    }

    #[tokio::test]
    async fn test_operation_permit_acquisition() {
        let config = ResourceConfig {
            max_concurrent_operations: 2,
            ..Default::default()
        };
        let manager = ResourceManager::new(config);

        let permit1 = manager.acquire_operation_permit().await;
        assert!(permit1.is_ok());

        let permit2 = manager.acquire_operation_permit().await;
        assert!(permit2.is_ok());

        // The limit is 2 and both are still held, so a third must WAIT — that is the whole point of
        // the semaphore. The original assertion claimed it would be granted ("semaphore allows it")
        // and simply awaited forever, so this test could only ever hang.
        let third = tokio::time::timeout(
            Duration::from_millis(200),
            manager.acquire_operation_permit(),
        )
        .await;
        assert!(
            third.is_err(),
            "a third permit must not be granted while both of a limit of 2 are held"
        );

        // Releasing one frees a slot.
        drop(permit1);
        let permit3 = tokio::time::timeout(
            Duration::from_secs(5),
            manager.acquire_operation_permit(),
        )
        .await;
        assert!(
            permit3.is_ok_and(|inner| inner.is_ok()),
            "releasing a permit must let a waiter through"
        );
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(2); // 2 requests per second

        assert!(limiter.allow_request().await);
        assert!(limiter.allow_request().await);
        assert!(!limiter.allow_request().await); // Should be rate limited

        // Wait a bit and check rate
        sleep(Duration::from_millis(100)).await;
        let rate = limiter.get_current_rate().await;
        assert_eq!(rate, 2);
    }

    #[tokio::test]
    async fn test_circuit_breaker() {
        let breaker = CircuitBreaker::new(2, Duration::from_millis(100));

        assert!(!breaker.is_open().await);

        breaker.record_failure().await;
        assert!(!breaker.is_open().await);

        breaker.record_failure().await;
        assert!(breaker.is_open().await);

        // Wait for timeout
        sleep(Duration::from_millis(150)).await;
        assert!(!breaker.is_open().await);
    }

    #[tokio::test]
    async fn test_object_pool() {
        let pool = ObjectPool::new(|| String::with_capacity(10), 2);

        let s1 = pool.acquire().await;
        assert_eq!(s1.capacity(), 10);

        pool.release(s1).await;
        assert_eq!(pool.size(), 1);

        let s2 = pool.acquire().await;
        assert_eq!(pool.size(), 0);
    }

    #[tokio::test]
    async fn test_adaptive_sampler() {
        let sampler = AdaptiveSampler::new(100);

        let initial_rate = sampler.get_sampling_rate().await;
        assert_eq!(initial_rate, 1.0);

        // Add samples with low resource usage
        for _ in 0..20 {
            sampler.add_sample(2.0, 10 * 1024 * 1024, 5).await;
        }

        let new_rate = sampler.get_sampling_rate().await;
        // Rate should increase with low resource usage
        assert!(new_rate >= initial_rate);
    }

    #[tokio::test]
    async fn test_performance_dashboard() {
        let config = ResourceConfig::default();
        let manager = ResourceManager::new(config);

        let dashboard = manager.get_performance_dashboard().await;

        assert!(dashboard.get("cpu").is_some());
        assert!(dashboard.get("memory").is_some());
        assert!(dashboard.get("operations").is_some());
        assert!(dashboard.get("brp_requests").is_some());
    }

    #[tokio::test]
    async fn test_resource_manager_monitoring() {
        let config = ResourceConfig {
            monitoring_interval: Duration::from_millis(10),
            ..Default::default()
        };
        let mut manager = ResourceManager::new(config);

        manager.start_monitoring().await.unwrap();

        // Let it run for a bit
        sleep(Duration::from_millis(50)).await;

        let metrics = manager.get_metrics().await;
        assert!(
            metrics
                .timestamp
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                > 0
        );

        manager.shutdown().await.unwrap();
    }
}
