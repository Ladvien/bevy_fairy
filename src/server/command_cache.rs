use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::server::error::{Error, Result};

/// Configuration for command result caching
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Maximum number of cached entries
    pub max_entries: usize,
    /// Default TTL for cache entries
    pub default_ttl: Duration,
    /// Cleanup interval for expired entries
    pub cleanup_interval: Duration,
    /// Maximum size of cached response data in bytes
    pub max_response_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1000,
            default_ttl: Duration::from_secs(300), // 5 minutes
            cleanup_interval: Duration::from_secs(60), // 1 minute
            max_response_size: 1024 * 1024, // 1MB per response
        }
    }
}

/// Cache key components for deterministic caching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheKey {
    pub tool_name: String,
    pub arguments_hash: String,
    pub feature_flags_hash: String,
}

impl CacheKey {
    /// Create a new cache key from tool name and arguments
    pub fn new(tool_name: &str, arguments: &Value) -> Result<Self> {
        let arguments_json = serde_json::to_string(arguments)
            .map_err(|e| Error::Validation(format!("Failed to serialize arguments: {}", e)))?;
        
        let mut hasher = Sha256::new();
        hasher.update(arguments_json.as_bytes());
        let arguments_hash = format!("{:x}", hasher.finalize());
        
        // Include feature flags in cache key to prevent cache poisoning
        let feature_flags = get_active_feature_flags();
        let mut feature_hasher = Sha256::new();
        feature_hasher.update(feature_flags.as_bytes());
        let feature_flags_hash = format!("{:x}", feature_hasher.finalize());
        
        Ok(Self {
            tool_name: tool_name.to_string(),
            arguments_hash,
            feature_flags_hash,
        })
    }
    
    /// Convert to a string key for HashMap storage
    pub fn to_string_key(&self) -> String {
        format!("{}:{}:{}", self.tool_name, self.arguments_hash, self.feature_flags_hash)
    }
}

/// Cached command result with metadata
#[derive(Debug, Clone)]
pub struct CachedResult {
    /// The cached response value
    pub response: Value,
    /// When this entry was cached
    pub cached_at: Instant,
    /// TTL for this specific entry
    pub ttl: Duration,
    /// Size of the cached response in bytes
    pub size_bytes: usize,
    /// Number of times this cache entry has been hit
    pub hit_count: u64,
    /// Tags for selective cache invalidation
    pub tags: Vec<String>,
}

impl CachedResult {
    /// Check if this cache entry has expired
    pub fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
    
    /// Get the age of this cache entry
    pub fn age(&self) -> Duration {
        self.cached_at.elapsed()
    }
    
    /// Increment hit count
    pub fn record_hit(&mut self) {
        self.hit_count += 1;
    }
}

/// Statistics about cache performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStatistics {
    pub total_entries: usize,
    pub total_hits: u64,
    pub total_misses: u64,
    pub total_size_bytes: usize,
    pub hit_rate: f64,
    pub average_entry_size: f64,
    pub oldest_entry_age_seconds: f64,
    pub newest_entry_age_seconds: f64,
    pub cleanup_runs: u64,
    pub evicted_entries: u64,
}

/// Command result cache with LRU eviction and TTL support
pub struct CommandCache {
    /// The cache storage
    cache: Arc<RwLock<HashMap<String, CachedResult>>>,
    /// Cache configuration
    config: CacheConfig,
    /// Statistics tracking
    stats: Arc<RwLock<CacheStatistics>>,
    /// Access order for LRU eviction
    access_order: Arc<RwLock<Vec<String>>>,
}

impl CommandCache {
    /// Create a new command cache
    pub fn new(config: CacheConfig) -> Self {
        let cache = Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            config,
            stats: Arc::new(RwLock::new(CacheStatistics {
                total_entries: 0,
                total_hits: 0,
                total_misses: 0,
                total_size_bytes: 0,
                hit_rate: 0.0,
                average_entry_size: 0.0,
                oldest_entry_age_seconds: 0.0,
                newest_entry_age_seconds: 0.0,
                cleanup_runs: 0,
                evicted_entries: 0,
            })),
            access_order: Arc::new(RwLock::new(Vec::new())),
        };
        
        // Start background cleanup task
        cache.start_cleanup_task();
        
        cache
    }
    
    /// Get a cached result if available and not expired
    pub async fn get(&self, key: &CacheKey) -> Option<Value> {
        let string_key = key.to_string_key();
        let mut cache = self.cache.write().await;
        let mut stats = self.stats.write().await;
        
        if let Some(cached_result) = cache.get_mut(&string_key) {
            if !cached_result.is_expired() {
                cached_result.record_hit();
                stats.total_hits += 1;
                
                // Update access order for LRU
                self.update_access_order(&string_key).await;
                
                debug!("Cache hit for {}", key.tool_name);
                return Some(cached_result.response.clone());
            } else {
                // Remove expired entry
                let removed = cache.remove(&string_key);
                if let Some(removed) = removed {
                    stats.total_size_bytes = stats.total_size_bytes.saturating_sub(removed.size_bytes);
                    stats.total_entries = stats.total_entries.saturating_sub(1);
                }
                debug!("Cache entry expired for {}", key.tool_name);
            }
        }
        
        stats.total_misses += 1;
        self.update_hit_rate(&mut stats).await;
        debug!("Cache miss for {}", key.tool_name);
        None
    }
    
    /// Store a result in the cache
    pub async fn put(&self, key: &CacheKey, response: Value, tags: Vec<String>) -> Result<()> {
        let string_key = key.to_string_key();
        
        // Calculate response size
        let response_json = serde_json::to_string(&response)
            .map_err(|e| Error::Validation(format!("Failed to serialize response: {}", e)))?;
        let size_bytes = response_json.len();
        
        // Check if response is too large
        if size_bytes > self.config.max_response_size {
            warn!("Response too large to cache: {} bytes > {} bytes limit", 
                  size_bytes, self.config.max_response_size);
            return Ok(());
        }
        
        let cached_result = CachedResult {
            response,
            cached_at: Instant::now(),
            ttl: self.config.default_ttl,
            size_bytes,
            hit_count: 0,
            tags,
        };
        
        let mut cache = self.cache.write().await;
        let mut stats = self.stats.write().await;
        
        // Check if we need to evict entries
        if cache.len() >= self.config.max_entries {
            self.evict_lru_entries(&mut cache, &mut stats).await;
        }
        
        // Add new entry
        if let Some(old_entry) = cache.insert(string_key.clone(), cached_result) {
            // Replace existing entry
            stats.total_size_bytes = stats.total_size_bytes.saturating_sub(old_entry.size_bytes);
        } else {
            // New entry
            stats.total_entries += 1;
        }
        
        stats.total_size_bytes += size_bytes;
        self.update_access_order(&string_key).await;
        
        debug!("Cached result for {} ({} bytes)", key.tool_name, size_bytes);
        Ok(())
    }
    
    /// Store a result with custom TTL
    pub async fn put_with_ttl(&self, key: &CacheKey, response: Value, ttl: Duration, tags: Vec<String>) -> Result<()> {
        let string_key = key.to_string_key();
        
        // Calculate response size
        let response_json = serde_json::to_string(&response)
            .map_err(|e| Error::Validation(format!("Failed to serialize response: {}", e)))?;
        let size_bytes = response_json.len();
        
        // Check if response is too large
        if size_bytes > self.config.max_response_size {
            warn!("Response too large to cache: {} bytes > {} bytes limit", 
                  size_bytes, self.config.max_response_size);
            return Ok(());
        }
        
        let cached_result = CachedResult {
            response,
            cached_at: Instant::now(),
            ttl,
            size_bytes,
            hit_count: 0,
            tags,
        };
        
        let mut cache = self.cache.write().await;
        let mut stats = self.stats.write().await;
        
        // Check if we need to evict entries
        if cache.len() >= self.config.max_entries {
            self.evict_lru_entries(&mut cache, &mut stats).await;
        }
        
        // Add new entry
        if let Some(old_entry) = cache.insert(string_key.clone(), cached_result) {
            // Replace existing entry
            stats.total_size_bytes = stats.total_size_bytes.saturating_sub(old_entry.size_bytes);
        } else {
            // New entry
            stats.total_entries += 1;
        }
        
        stats.total_size_bytes += size_bytes;
        self.update_access_order(&string_key).await;
        
        debug!("Cached result for {} with custom TTL {:?} ({} bytes)", key.tool_name, ttl, size_bytes);
        Ok(())
    }
    
    /// Invalidate cache entries by tag
    pub async fn invalidate_by_tag(&self, tag: &str) -> usize {
        let mut cache = self.cache.write().await;
        let mut stats = self.stats.write().await;
        let mut access_order = self.access_order.write().await;
        
        let mut to_remove = Vec::new();
        for (key, cached_result) in cache.iter() {
            if cached_result.tags.contains(&tag.to_string()) {
                to_remove.push(key.clone());
            }
        }
        
        let count = to_remove.len();
        for key in to_remove {
            if let Some(removed) = cache.remove(&key) {
                stats.total_size_bytes = stats.total_size_bytes.saturating_sub(removed.size_bytes);
                stats.total_entries = stats.total_entries.saturating_sub(1);
                stats.evicted_entries += 1;
            }
            access_order.retain(|k| k != &key);
        }
        
        info!("Invalidated {} cache entries with tag '{}'", count, tag);
        count
    }
    
    /// Clear all cache entries
    pub async fn clear(&self) {
        let mut cache = self.cache.write().await;
        let mut stats = self.stats.write().await;
        let mut access_order = self.access_order.write().await;
        
        let count = cache.len();
        cache.clear();
        access_order.clear();
        
        stats.total_entries = 0;
        stats.total_size_bytes = 0;
        stats.evicted_entries += count as u64;
        
        info!("Cleared {} cache entries", count);
    }
    
    /// Get cache statistics
    pub async fn get_statistics(&self) -> CacheStatistics {
        let cache = self.cache.read().await;
        let mut stats = self.stats.write().await;
        
        // Update real-time statistics
        stats.total_entries = cache.len();
        self.update_hit_rate(&mut stats).await;
        stats.average_entry_size = if stats.total_entries > 0 {
            stats.total_size_bytes as f64 / stats.total_entries as f64
        } else {
            0.0
        };
        
        // Calculate entry age statistics
        let now = Instant::now();
        let mut oldest_age = Duration::ZERO;
        let mut newest_age = Duration::MAX;
        
        for cached_result in cache.values() {
            let age = now.duration_since(cached_result.cached_at);
            if age > oldest_age {
                oldest_age = age;
            }
            if age < newest_age {
                newest_age = age;
            }
        }
        
        stats.oldest_entry_age_seconds = oldest_age.as_secs_f64();
        stats.newest_entry_age_seconds = if newest_age == Duration::MAX {
            0.0
        } else {
            newest_age.as_secs_f64()
        };
        
        stats.clone()
    }
    
    /// Update access order for LRU eviction
    async fn update_access_order(&self, key: &str) {
        let mut access_order = self.access_order.write().await;
        
        // Remove if exists
        access_order.retain(|k| k != key);
        
        // Add to front (most recently used)
        access_order.insert(0, key.to_string());
    }
    
    /// Evict least recently used entries
    async fn evict_lru_entries(&self, cache: &mut HashMap<String, CachedResult>, stats: &mut CacheStatistics) {
        let access_order = self.access_order.read().await;
        let entries_to_evict = (cache.len() + 1).saturating_sub(self.config.max_entries);
        
        // Evict from the end of access_order (least recently used)
        let mut evicted = 0;
        for key in access_order.iter().rev().take(entries_to_evict) {
            if let Some(removed) = cache.remove(key) {
                stats.total_size_bytes = stats.total_size_bytes.saturating_sub(removed.size_bytes);
                stats.total_entries = stats.total_entries.saturating_sub(1);
                stats.evicted_entries += 1;
                evicted += 1;
            }
        }
        
        debug!("Evicted {} LRU cache entries", evicted);
    }
    
    /// Update hit rate statistics
    async fn update_hit_rate(&self, stats: &mut CacheStatistics) {
        let total_requests = stats.total_hits + stats.total_misses;
        stats.hit_rate = if total_requests > 0 {
            stats.total_hits as f64 / total_requests as f64
        } else {
            0.0
        };
    }
    
    /// Start background cleanup task
    fn start_cleanup_task(&self) {
        let cache = self.cache.clone();
        let stats = self.stats.clone();
        let access_order = self.access_order.clone();
        let cleanup_interval = self.config.cleanup_interval;
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(cleanup_interval);
            
            loop {
                interval.tick().await;
                
                let mut cache_guard = cache.write().await;
                let mut stats_guard = stats.write().await;
                let mut access_order_guard = access_order.write().await;
                
                let mut expired_keys = Vec::new();
                let now = Instant::now();
                
                for (key, cached_result) in cache_guard.iter() {
                    if cached_result.is_expired() {
                        expired_keys.push(key.clone());
                    }
                }
                
                let expired_count = expired_keys.len();
                for key in &expired_keys {
                    if let Some(removed) = cache_guard.remove(key) {
                        stats_guard.total_size_bytes = stats_guard.total_size_bytes.saturating_sub(removed.size_bytes);
                        stats_guard.total_entries = stats_guard.total_entries.saturating_sub(1);
                    }
                    access_order_guard.retain(|k| k != key);
                }
                
                stats_guard.cleanup_runs += 1;
                
                if expired_count > 0 {
                    debug!("Cache cleanup removed {} expired entries", expired_count);
                }
            }
        });
    }
}

/// Get active feature flags as a string for cache key generation
///
/// The list is built from a fixed array of `(flag, is_enabled)` pairs rather than from
/// `cfg`-gated pushes. A push behind a `cfg` needs a `mut` binding, and the binding is
/// unused when the feature is off -- which is a clippy warning, and the crate is not allowed
/// to add warnings. This form has no `mut` at all.
fn get_active_feature_flags() -> String {
    const FLAGS: &[(&str, bool)] = &[
        ("basic-debugging", cfg!(feature = "basic-debugging")),
        ("entity-inspection", cfg!(feature = "entity-inspection")),
        ("performance-profiling", cfg!(feature = "performance-profiling")),
        ("visual-debugging", cfg!(feature = "visual-debugging")),
        ("session-management", cfg!(feature = "session-management")),
        ("issue-detection", cfg!(feature = "issue-detection")),
        ("memory-profiling", cfg!(feature = "memory-profiling")),
        ("time-travel", cfg!(feature = "time-travel")),
        ("orchestration", cfg!(feature = "orchestration")),
        ("optimizations", cfg!(feature = "optimizations")),
        ("caching", cfg!(feature = "caching")),
        ("pooling", cfg!(feature = "pooling")),
        ("lazy-init", cfg!(feature = "lazy-init")),
    ];

    FLAGS
        .iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    
    #[tokio::test]
    async fn test_cache_basic_operations() {
        let config = CacheConfig {
            max_entries: 2,
            default_ttl: Duration::from_secs(1),
            cleanup_interval: Duration::from_secs(10),
            max_response_size: 1024,
        };
        let cache = CommandCache::new(config);
        
        let key1 = CacheKey::new("test_tool", &json!({"param": "value1"})).unwrap();
        let key2 = CacheKey::new("test_tool", &json!({"param": "value2"})).unwrap();
        let response1 = json!({"result": "data1"});
        let response2 = json!({"result": "data2"});
        
        // Test cache miss
        assert!(cache.get(&key1).await.is_none());
        
        // Test cache put and hit
        cache.put(&key1, response1.clone(), vec!["tag1".to_string()]).await.unwrap();
        assert_eq!(cache.get(&key1).await.unwrap(), response1);
        
        // Test cache capacity limit
        cache.put(&key2, response2.clone(), vec!["tag2".to_string()]).await.unwrap();
        assert_eq!(cache.get(&key2).await.unwrap(), response2);
    }
    
    #[tokio::test]
    async fn test_cache_expiration() {
        let config = CacheConfig {
            max_entries: 10,
            default_ttl: Duration::from_millis(50),
            cleanup_interval: Duration::from_secs(10),
            max_response_size: 1024,
        };
        let cache = CommandCache::new(config);
        
        let key = CacheKey::new("test_tool", &json!({"param": "value"})).unwrap();
        let response = json!({"result": "data"});
        
        cache.put(&key, response.clone(), vec![]).await.unwrap();
        assert_eq!(cache.get(&key).await.unwrap(), response);
        
        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(cache.get(&key).await.is_none());
    }
    
    #[tokio::test]
    async fn test_tag_invalidation() {
        let config = CacheConfig::default();
        let cache = CommandCache::new(config);
        
        let key1 = CacheKey::new("tool1", &json!({"param": "value1"})).unwrap();
        let key2 = CacheKey::new("tool2", &json!({"param": "value2"})).unwrap();
        let response1 = json!({"result": "data1"});
        let response2 = json!({"result": "data2"});
        
        cache.put(&key1, response1.clone(), vec!["entity_data".to_string()]).await.unwrap();
        cache.put(&key2, response2.clone(), vec!["performance_data".to_string()]).await.unwrap();
        
        assert_eq!(cache.get(&key1).await.unwrap(), response1);
        assert_eq!(cache.get(&key2).await.unwrap(), response2);
        
        // Invalidate by tag
        let invalidated = cache.invalidate_by_tag("entity_data").await;
        assert_eq!(invalidated, 1);
        
        assert!(cache.get(&key1).await.is_none());
        assert_eq!(cache.get(&key2).await.unwrap(), response2);
    }
}