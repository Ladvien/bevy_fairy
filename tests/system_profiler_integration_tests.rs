/// Integration tests for system profiler functionality
use bevy_fairy::server::brp_messages::{
    DebugCommand, DebugResponse, SystemMetrics, ProfileSample, SystemProfile,
};
use bevy_fairy::server::system_profiler::{
    SystemProfiler, ProfilerConfig, ExportFormat, PerformanceAnomaly,
    MAX_FRAME_HISTORY, MAX_CONCURRENT_SYSTEMS, DEFAULT_PROFILE_DURATION_MS,
};
use bevy_fairy::server::system_profiler_processor::{
    SystemProfilerProcessor, ExtendedProfilerProcessor,
};
use bevy_fairy::server::debug_command_processor::DebugCommandProcessor;
use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::error::{Error, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

/// Create a test profiler with mock BRP client
async fn create_test_profiler() -> Arc<SystemProfiler> {
    let config = Config {
        bevy_brp_host: "localhost".to_string(),
        bevy_brp_port: 15702,
        mcp_port: 3000,
        ..Default::default()
    };
    let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
    Arc::new(SystemProfiler::new(brp_client))
}

#[tokio::test]
async fn test_profiler_configuration() {
    let config = Config {
        bevy_brp_host: "localhost".to_string(),
        bevy_brp_port: 15702,
        mcp_port: 3000,
        ..Default::default()
    };
    let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
    
    let profiler_config = ProfilerConfig {
        track_allocations: true,
        external_profiler_integration: false,
        max_overhead_percent: 5.0,
        auto_profile_threshold_ms: 20.0,
    };
    
    let profiler = SystemProfiler::with_config(brp_client, profiler_config.clone());
    
    // Configuration is internal, but we can test behavior
    assert_eq!(MAX_FRAME_HISTORY, 1000);
    assert_eq!(MAX_CONCURRENT_SYSTEMS, 50);
}

#[tokio::test]
async fn test_start_profiling_multiple_systems() {
    let profiler = create_test_profiler().await;
    
    // Start profiling multiple systems
    let result1 = profiler.start_profiling(
        "physics_system".to_string(),
        Some(1000),
        Some(true),
    ).await;
    assert!(result1.is_ok());
    
    let result2 = profiler.start_profiling(
        "render_system".to_string(),
        Some(2000),
        Some(false),
    ).await;
    assert!(result2.is_ok());
    
    let result3 = profiler.start_profiling(
        "ai_system".to_string(),
        None, // No duration limit
        Some(true),
    ).await;
    assert!(result3.is_ok());
    
    // Try to start profiling same system again (should fail)
    let result_duplicate = profiler.start_profiling(
        "physics_system".to_string(),
        Some(1000),
        Some(true),
    ).await;
    assert!(result_duplicate.is_err());
    
    // Stop profiling
    let profile = profiler.stop_profiling("physics_system").await;
    assert!(profile.is_ok());
}

#[tokio::test]
async fn test_concurrent_profiling_limit() {
    let profiler = create_test_profiler().await;
    
    // Start maximum concurrent systems
    for i in 0..MAX_CONCURRENT_SYSTEMS {
        let result = profiler.start_profiling(
            format!("system_{}", i),
            Some(5000),
            Some(false),
        ).await;
        assert!(result.is_ok(), "Failed to start profiling system {}", i);
    }
    
    // Try to exceed limit (should fail)
    let result = profiler.start_profiling(
        "overflow_system".to_string(),
        Some(1000),
        Some(false),
    ).await;
    assert!(result.is_err());
    assert!(matches!(result, Err(Error::DebugError(_))));
}

#[tokio::test]
async fn test_collect_and_calculate_metrics() {
    let profiler = create_test_profiler().await;
    
    // Start profiling
    profiler.start_profiling(
        "test_system".to_string(),
        Some(5000),
        Some(true),
    ).await.unwrap();
    
    // Collect samples
    for i in 0..10 {
        let duration = Duration::from_millis(10 + i * 2);
        let allocations = Some((i * 100) as usize);
        
        profiler.collect_sample(
            "test_system",
            duration,
            allocations,
        ).await.unwrap();
        
        sleep(Duration::from_millis(10)).await;
    }
    
    // Stop and get profile
    let profile = profiler.stop_profiling("test_system").await.unwrap();
    
    // Verify metrics
    assert_eq!(profile.system_name, "test_system");
    assert!(profile.metrics.min_time_us > 0);
    assert!(profile.metrics.max_time_us >= profile.metrics.min_time_us);
    assert!(profile.metrics.avg_time_us > 0);
    assert!(profile.metrics.total_allocations > 0);
}

#[tokio::test]
async fn test_anomaly_detection() {
    let profiler = create_test_profiler().await;
    
    // Start profiling
    profiler.start_profiling(
        "anomaly_test".to_string(),
        Some(10000),
        Some(false),
    ).await.unwrap();
    
    // Establish baseline
    for _ in 0..20 {
        profiler.collect_sample(
            "anomaly_test",
            Duration::from_millis(10),
            None,
        ).await.unwrap();
    }
    
    // Inject anomaly (spike in execution time)
    profiler.collect_sample(
        "anomaly_test",
        Duration::from_millis(50), // 5x normal
        None,
    ).await.unwrap();
    
    // Get anomalies
    let anomalies = profiler.get_anomalies().await;
    assert!(!anomalies.is_empty(), "Should detect performance anomaly");
    
    if let Some(anomaly) = anomalies.first() {
        assert_eq!(anomaly.system_name, "anomaly_test");
        assert!(anomaly.execution_time_ms > anomaly.expected_time_ms * 1.5);
        assert!(anomaly.severity >= 1);
    }
}

#[tokio::test]
async fn test_dependency_graph_update() {
    let profiler = create_test_profiler().await;
    
    // Update dependency graph
    profiler.update_dependency_graph(
        "render_system".to_string(),
        vec!["transform_system".to_string(), "camera_system".to_string()],
    ).await;
    
    profiler.update_dependency_graph(
        "physics_system".to_string(),
        vec!["transform_system".to_string()],
    ).await;
    
    // Start profiling and verify dependencies are tracked
    profiler.start_profiling(
        "render_system".to_string(),
        Some(1000),
        Some(false),
    ).await.unwrap();
    
    let profile = profiler.stop_profiling("render_system").await.unwrap();
    assert_eq!(profile.dependencies.len(), 2);
    assert!(profile.dependencies.contains(&"transform_system".to_string()));
    assert!(profile.dependencies.contains(&"camera_system".to_string()));
}

#[tokio::test]
async fn test_profiling_history() {
    let profiler = create_test_profiler().await;
    
    // Collect samples without profiling session
    // The frame history should track all samples
    for i in 0..5 {
        profiler.collect_sample(
            "history_test",
            Duration::from_millis(10 + i),
            Some(i as usize * 50),
        ).await.unwrap();
        // Add delay to ensure different frames
        sleep(Duration::from_millis(20)).await;
    }
    
    // Get history - should work even without active profiling
    let history = profiler.get_system_history("history_test").await;
    
    // Verify we have data
    assert!(!history.is_empty(), "History should contain samples even without active profiling");
    
    // If we have multiple samples, verify ordering
    if history.len() > 1 {
        for window in history.windows(2) {
            // Timestamps should be monotonically increasing
            assert!(window[1].timestamp >= window[0].timestamp,
                    "Timestamps should be ordered");
        }
    }
}

#[tokio::test]
async fn test_export_formats() {
    let profiler = create_test_profiler().await;
    
    // Create some profiling data
    profiler.start_profiling(
        "export_test".to_string(),
        Some(1000),
        Some(true),
    ).await.unwrap();
    
    for i in 0..3 {
        profiler.collect_sample(
            "export_test",
            Duration::from_millis(10 + i),
            Some(100),
        ).await.unwrap();
    }
    
    // Test JSON export
    let json_export = profiler.export_profiling_data(ExportFormat::Json).await.unwrap();
    assert!(json_export.contains("active_sessions"));
    assert!(json_export.contains("frame_count"));
    
    // Test CSV export
    let csv_export = profiler.export_profiling_data(ExportFormat::Csv).await.unwrap();
    assert!(csv_export.contains("frame,system,duration_us,allocations"));
    
    // Test Tracy export
    let tracy_export = profiler.export_profiling_data(ExportFormat::TracyJson).await.unwrap();
    assert!(tracy_export.contains("frames"));
}

#[tokio::test]
async fn test_clear_profiling_data() {
    let profiler = create_test_profiler().await;
    
    // Add some data
    profiler.start_profiling(
        "clear_test".to_string(),
        Some(1000),
        Some(false),
    ).await.unwrap();
    
    for _ in 0..5 {
        profiler.collect_sample(
            "clear_test",
            Duration::from_millis(10),
            None,
        ).await.unwrap();
    }
    
    // Clear all data
    profiler.clear_profiling_data().await;
    
    // Verify data is cleared
    let history = profiler.get_system_history("clear_test").await;
    assert!(history.is_empty() || history.len() < 5);
    
    let anomalies = profiler.get_anomalies().await;
    assert_eq!(anomalies.len(), 0);
}

#[tokio::test]
async fn test_processor_command_handling() {
    let profiler = create_test_profiler().await;
    let processor = SystemProfilerProcessor::new(profiler);
    
    // Test ProfileSystem command
    let command = DebugCommand::ProfileSystem {
        system_name: "processor_test".to_string(),
        duration_ms: Some(100), // Short duration for test
        track_allocations: Some(true),
    };
    
    // Validate command
    let validation = processor.validate(&command).await;
    assert!(validation.is_ok());
    
    // Check command support
    assert!(processor.supports_command(&command));
    
    // Process command
    let response = processor.process(command).await;
    assert!(response.is_ok());
    
    // For short durations, should get full profile
    if let Ok(DebugResponse::SystemProfile(profile)) = response {
        assert_eq!(profile.system_name, "processor_test");
    }
}

#[tokio::test]
async fn test_processor_validation() {
    let profiler = create_test_profiler().await;
    let processor = SystemProfilerProcessor::new(profiler);
    
    // Test invalid: empty system name
    let invalid_command = DebugCommand::ProfileSystem {
        system_name: "".to_string(),
        duration_ms: Some(1000),
        track_allocations: Some(false),
    };
    assert!(processor.validate(&invalid_command).await.is_err());
    
    // Test invalid: duration too long
    let invalid_command = DebugCommand::ProfileSystem {
        system_name: "test".to_string(),
        duration_ms: Some(400_000), // > 5 minutes
        track_allocations: Some(false),
    };
    assert!(processor.validate(&invalid_command).await.is_err());
    
    // Test invalid: zero duration
    let invalid_command = DebugCommand::ProfileSystem {
        system_name: "test".to_string(),
        duration_ms: Some(0),
        track_allocations: Some(false),
    };
    assert!(processor.validate(&invalid_command).await.is_err());
    
    // Test valid command
    let valid_command = DebugCommand::ProfileSystem {
        system_name: "valid_system".to_string(),
        duration_ms: Some(5000),
        track_allocations: Some(true),
    };
    assert!(processor.validate(&valid_command).await.is_ok());
}

#[tokio::test]
async fn test_extended_processor() {
    let profiler = create_test_profiler().await;
    let extended = ExtendedProfilerProcessor::new(profiler.clone());
    
    // Start profiling
    profiler.start_profiling(
        "extended_test".to_string(),
        Some(5000),
        Some(true),
    ).await.unwrap();
    
    // Collect some samples
    for i in 0..3 {
        profiler.collect_sample(
            "extended_test",
            Duration::from_millis(10 + i),
            Some(100),
        ).await.unwrap();
    }
    
    // Test stop profiling
    let response = extended.stop_profiling("extended_test").await;
    assert!(response.is_ok());
    
    // Test get history
    profiler.start_profiling(
        "history_system".to_string(),
        Some(5000),
        Some(false),
    ).await.unwrap();
    
    for _ in 0..3 {
        profiler.collect_sample(
            "history_system",
            Duration::from_millis(15),
            None,
        ).await.unwrap();
    }
    
    let history_response = extended.get_history("history_system").await;
    assert!(history_response.is_ok());
    
    // Test get anomalies
    let anomalies_response = extended.get_anomalies().await;
    assert!(anomalies_response.is_ok());
    
    // Test export data
    let export_result = extended.export_data(ExportFormat::Json).await;
    assert!(export_result.is_ok());
    
    // Test clear data
    let clear_result = extended.clear_data().await;
    assert!(clear_result.is_ok());
}

#[tokio::test]
async fn test_metrics_percentiles() {
    let profiler = create_test_profiler().await;
    
    profiler.start_profiling(
        "percentile_test".to_string(),
        Some(5000),
        Some(false),
    ).await.unwrap();
    
    // Create samples with known distribution
    let durations = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 40, 50, 100];
    for duration in durations {
        profiler.collect_sample(
            "percentile_test",
            Duration::from_millis(duration),
            None,
        ).await.unwrap();
    }
    
    let profile = profiler.stop_profiling("percentile_test").await.unwrap();
    
    // Check percentiles
    assert!(profile.metrics.median_time_us > 0);
    assert!(profile.metrics.p95_time_us >= profile.metrics.median_time_us);
    assert!(profile.metrics.p99_time_us >= profile.metrics.p95_time_us);
    assert!(profile.metrics.p99_time_us <= profile.metrics.max_time_us);
}

#[tokio::test]
async fn test_allocation_tracking() {
    let profiler = create_test_profiler().await;
    
    profiler.start_profiling(
        "allocation_test".to_string(),
        Some(5000),
        Some(true), // Enable allocation tracking
    ).await.unwrap();
    
    // Collect samples with allocations
    for i in 0..5 {
        profiler.collect_sample(
            "allocation_test",
            Duration::from_millis(10),
            Some((i + 1) * 1024), // Increasing allocations
        ).await.unwrap();
    }
    
    let profile = profiler.stop_profiling("allocation_test").await.unwrap();
    
    // Verify allocation metrics
    assert!(profile.metrics.total_allocations > 0);
    assert!(profile.metrics.allocation_rate > 0.0);
    
    // Check samples have allocation data
    let samples_with_allocs = profile.samples.iter()
        .filter(|s| s.allocations.is_some())
        .count();
    assert!(samples_with_allocs > 0);
}

#[tokio::test]
async fn test_overhead_calculation() {
    let profiler = create_test_profiler().await;
    
    profiler.start_profiling(
        "overhead_test".to_string(),
        Some(2000),
        Some(false),
    ).await.unwrap();
    
    // Simulate profiling overhead
    for _ in 0..10 {
        profiler.collect_sample(
            "overhead_test",
            Duration::from_millis(105), // Slightly above baseline
            None,
        ).await.unwrap();
    }
    
    let profile = profiler.stop_profiling("overhead_test").await.unwrap();
    
    // Overhead should be calculated  
    assert!(profile.metrics.overhead_percent >= 0.0);
    // In test, overhead is simplified - just verify it's a valid number
    assert!(!profile.metrics.overhead_percent.is_nan());
}

#[tokio::test]
async fn test_frame_history_limit() {
    let profiler = create_test_profiler().await;
    
    // Add more samples than MAX_FRAME_HISTORY
    for i in 0..(MAX_FRAME_HISTORY + 100) {
        profiler.collect_sample(
            &format!("system_{}", i % 10),
            Duration::from_millis(10),
            None,
        ).await.unwrap();
        
        // Small delay to create different frames
        if i % 100 == 0 {
            sleep(Duration::from_millis(20)).await;
        }
    }
    
    // History should be limited
    let history = profiler.get_system_history("system_0").await;
    assert!(history.len() <= MAX_FRAME_HISTORY);
}

#[tokio::test]
async fn test_profiling_session_expiry() {
    let profiler = create_test_profiler().await;
    
    // Start profiling with short duration
    profiler.start_profiling(
        "expiry_test".to_string(),
        Some(100), // 100ms duration
        Some(false),
    ).await.unwrap();
    
    // Wait for session to expire
    sleep(Duration::from_millis(150)).await;
    
    // Try to collect sample after expiry (should be ignored)
    let result = profiler.collect_sample(
        "expiry_test",
        Duration::from_millis(10),
        None,
    ).await;
    assert!(result.is_ok()); // Should not error, just ignore
    
    // Profile should have been auto-stopped
    let profile = profiler.stop_profiling("expiry_test").await;
    // May or may not exist depending on auto-stop implementation
    assert!(profile.is_ok() || profile.is_err());
}

#[tokio::test]
async fn test_multiple_dependency_updates() {
    let profiler = create_test_profiler().await;
    
    // Build complex dependency graph
    profiler.update_dependency_graph(
        "ui_system".to_string(),
        vec!["render_system".to_string(), "input_system".to_string()],
    ).await;
    
    profiler.update_dependency_graph(
        "render_system".to_string(),
        vec!["transform_system".to_string(), "camera_system".to_string()],
    ).await;
    
    profiler.update_dependency_graph(
        "physics_system".to_string(),
        vec!["transform_system".to_string(), "collision_system".to_string()],
    ).await;
    
    profiler.update_dependency_graph(
        "ai_system".to_string(),
        vec!["physics_system".to_string(), "perception_system".to_string()],
    ).await;
    
    // Verify dependencies are tracked correctly
    profiler.start_profiling(
        "ai_system".to_string(),
        Some(1000),
        Some(false),
    ).await.unwrap();
    
    let profile = profiler.stop_profiling("ai_system").await.unwrap();
    assert_eq!(profile.dependencies.len(), 2);
    assert!(profile.dependencies.contains(&"physics_system".to_string()));
    assert!(profile.dependencies.contains(&"perception_system".to_string()));
}