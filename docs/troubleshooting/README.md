# Troubleshooting Guide - bevy-fairy

This guide helps you diagnose and resolve common issues when using bevy-fairy.

## Table of Contents

1. [Connection Issues](#connection-issues)
2. [Performance Problems](#performance-problems)  
3. [Command Failures](#command-failures)
4. [Screenshot Issues](#screenshot-issues)
5. [Integration Test Failures](#integration-test-failures)
6. [Configuration Problems](#configuration-problems)
7. [Platform-Specific Issues](#platform-specific-issues)
8. [Bevy Game Development Issues](#bevy-game-development-issues)
9. [Advanced Diagnostics](#advanced-diagnostics)

---

## Connection Issues

### Issue 1: "BRP client not connected" Error

**Symptoms**: MCP tools fail with "BRP client not connected to Bevy game"

**Causes**:
- Bevy game is not running
- The debugger plugin is not in the Bevy app
- Incorrect host/port configuration
- Firewall blocking connections

**Solutions**:
1. **Verify Bevy game is running** with the debugger plugin:
   ```rust
   use bevy::prelude::*;
   use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_plugins(DebuggerPlugin::default()) // This line is crucial
           .add_plugins(RemoteHttpPlugin::default())
           .run();
   }
   ```

2. **Check configuration**:
   ```bash
   # Check if MCP server is using correct BRP port
   export BEVY_BRP_HOST=localhost
   export BEVY_BRP_PORT=15702
   ```

3. **Test BRP connection directly**:
   ```bash
   curl -X POST http://localhost:15702/bevy/list_entities
   ```

4. **Check firewall settings** on macOS/Linux:
   ```bash
   # macOS
   sudo pfctl -sr | grep 15702
   
   # Linux
   sudo iptables -L | grep 15702
   ```

---

### Issue 2: WebSocket Connection Failures

**Symptoms**: Connection drops, reconnection attempts fail

**Causes**:
- Network instability
- Resource exhaustion
- Port conflicts

**Solutions**:
1. **Check for port conflicts**:
   ```bash
   lsof -i :15702  # BRP port
   lsof -i :3000   # MCP port (default)
   ```

2. **Enable connection logging**:
   ```bash
   RUST_LOG=debug bevy-fairy --stdio
   ```

3. **Increase reconnection timeout**:
   ```toml
   # config.toml
   [connection]
   timeout_seconds = 30
   retry_attempts = 5
   ```

---

## Performance Problems

### Issue 3: High Latency (>100ms) Tool Calls

**Symptoms**: MCP commands take longer than expected, performance violations reported

**Causes**:
- Complex queries
- Large datasets
- Resource contention
- System overload

**Solutions**:
1. **Check performance budget violations**:
   ```javascript
   await mcpClient.callTool("debug", {
     command: {"GetBudgetStatistics": {}}
   });
   ```

2. **Optimize queries**:
   ```javascript
   // Instead of this (slow)
   await mcpClient.callTool("observe", {
     query: "all entities with any component"
   });
   
   // Use this (fast)  
   await mcpClient.callTool("observe", {
     query: "entities with Transform limit 100"
   });
   ```

3. **Monitor resource usage**:
   ```javascript
   const metrics = await mcpClient.callTool("resource_metrics", {});
   console.log("CPU:", metrics.cpu_percent);
   console.log("Memory:", metrics.memory_bytes);
   ```

---

### Issue 4: Memory Usage Growing Over Time

**Symptoms**: MCP server memory usage increases during long sessions

**Causes**:
- Unbounded history retention
- Memory leaks in processors
- Large screenshot buffers

**Solutions**:
1. **Clear violation history periodically**:
   ```javascript
   await mcpClient.callTool("debug", {
     command: {"ClearBudgetHistory": {}}
   });
   ```

2. **Configure history limits**:
   ```toml
   # config.toml
   [performance_budget]
   max_violation_history = 500
   max_compliance_samples = 5000
   ```

3. **Monitor memory with diagnostic report**:
   ```javascript
   const report = await mcpClient.callTool("diagnostic_report", {
     action: "generate"
   });
   ```

---

## Command Failures

### Issue 5: "Invalid Query" Errors

**Symptoms**: `observe` tool fails with query parsing errors

**Causes**:
- Malformed natural language queries
- Unrecognized component names
- Query too complex

**Solutions**:
1. **Use simple, clear queries**:
   ```javascript
   // Good queries
   "entities with Transform"
   "player entities"
   "entities moving fast"
   "entities near position 0,0,0"
   
   // Problematic queries  
   "all the things that are doing stuff"
   "entities with Transform and Velocity and Sprite and..."
   ```

2. **Check available components**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "list component types"
   });
   ```

3. **Use debug mode for detailed errors**:
   ```bash
   RUST_LOG=debug,bevy_fairy=trace bevy-fairy
   ```

---

### Issue 6: Screenshot Tool Failures

**Symptoms**: Screenshots fail to save or are blank/corrupted

**Causes**:
- Insufficient warmup time
- Invalid file paths
- Permission issues
- Graphics driver problems

**Solutions**:
1. **Increase warmup duration**:
   ```javascript
   await mcpClient.callTool("screenshot", {
     path: "debug/test.png",
     warmup_duration: 3000,  // 3 seconds
     wait_for_render: true
   });
   ```

2. **Check file permissions**:
   ```bash
   # Ensure directory exists and is writable
   mkdir -p debug/
   chmod 755 debug/
   ```

3. **Test with absolute path**:
   ```javascript
   await mcpClient.callTool("screenshot", {
     path: "/tmp/debug_screenshot.png"
   });
   ```

4. **Verify graphics setup** (headless environments):
   ```bash
   # Install virtual display for CI
   sudo apt-get install xvfb
   xvfb-run -a bevy-fairy
   ```

---

## Integration Test Failures

### Issue 7: "Mock BRP client connection timeout"

**Symptoms**: Integration tests fail with connection timeouts

**Causes**:
- Test environment configuration
- Resource limits in CI
- Race conditions

**Solutions**:
1. **Increase test timeouts**:
   ```rust
   #[tokio::test]
   async fn test_with_longer_timeout() {
       let config = TestConfig {
           timeout_ms: 30000, // 30 seconds
           ..Default::default()
       };
       // ... test code
   }
   ```

2. **Use mock clients for CI**:
   ```rust
   let config = TestConfig {
       enable_mock_brp: true,  // Use mocks in CI
       ..Default::default()
   };
   ```

3. **Add retry logic**:
   ```rust
   let mut attempts = 0;
   while attempts < 3 {
       match harness.execute_tool_call("observe", args).await {
           Ok(result) => return Ok(result),
           Err(_) if attempts < 2 => {
               attempts += 1;
               tokio::time::sleep(Duration::from_millis(1000)).await;
           }
           Err(e) => return Err(e),
       }
   }
   ```

---

### Issue 8: Performance Regression Test Failures

**Symptoms**: CI fails on performance tests that pass locally

**Causes**:
- CI resource constraints
- Different CPU/memory configurations
- Timing variations

**Solutions**:
1. **Adjust performance thresholds for CI**:
   ```rust
   let threshold = if std::env::var("CI").is_ok() {
       Duration::from_millis(200)  // More lenient in CI
   } else {
       Duration::from_millis(100)  // Strict locally
   };
   ```

2. **Use relative performance metrics**:
   ```rust
   // Instead of absolute time limits
   let baseline = measure_baseline_performance().await;
   let actual = measure_actual_performance().await;
   assert!(actual <= baseline * 1.5);  // 50% tolerance
   ```

---

## Configuration Problems

### Issue 9: Environment Variables Not Loading

**Symptoms**: Default values used instead of environment configuration

**Causes**:
- Variable naming mismatches
- Shell environment issues
- Service configuration problems

**Solutions**:
1. **Verify environment variables**:
   ```bash
   env | grep BEVY_
   echo $BEVY_BRP_HOST
   echo $BEVY_BRP_PORT
   echo $MCP_PORT
   ```

2. **Use configuration file**:
   ```toml
   # config/config.toml
   bevy_brp_host = "localhost"
   bevy_brp_port = 15702
   mcp_port = 3000
   
   [debug]
   enable_performance_tracking = true
   ```

3. **Check service configuration** (macOS):
   ```bash
   launchctl list | grep bevy-fairy
   cat ~/Library/LaunchAgents/com.bevy-fairy.plist
   ```

---

## Platform-Specific Issues

### Issue 10: macOS Service Integration Problems

**Symptoms**: MCP server doesn't start automatically, Claude Code can't connect

**Solutions**:
1. **Check service status**:
   ```bash
   launchctl list com.bevy-fairy
   ```

2. **Reload service**:
   ```bash
   launchctl unload ~/Library/LaunchAgents/com.bevy-fairy.plist
   launchctl load ~/Library/LaunchAgents/com.bevy-fairy.plist
   ```

3. **Check service logs**:
   ```bash
   tail -f /tmp/bevy-fairy.log
   ```

---

### Issue 11: Linux Permission Issues

**Symptoms**: Cannot write screenshots, configuration files not accessible

**Solutions**:
1. **Check file permissions**:
   ```bash
   ls -la ~/.config/bevy-fairy/
   ```

2. **Fix permissions**:
   ```bash
   chmod 755 ~/.config/bevy-fairy/
   chmod 644 ~/.config/bevy-fairy/config.toml
   ```

---

### Issue 12: Windows Path Issues

**Symptoms**: File paths with spaces or special characters cause failures

**Solutions**:
1. **Use forward slashes**:
   ```javascript
   await mcpClient.callTool("screenshot", {
     path: "C:/Debug/Screenshots/test.png"  // Not C:\Debug\Screenshots\test.png
   });
   ```

2. **Escape paths properly**:
   ```javascript
   const path = "C:\\Users\\Name\\Documents\\screenshot.png";
   ```

---

## Bevy Game Development Issues

### Issue 16: "Component Not Found" During Observations

**Symptoms**: Queries for specific components return empty results despite components existing

**Causes**:
- Component not registered with Bevy reflection system
- Custom component types not implementing required traits
- Component names changed but queries still use old names

**Solutions**:
1. **Ensure components implement required traits**:
   ```rust
   use bevy::prelude::*;
   
   #[derive(Component, Reflect)] // Reflect is essential for MCP tools
   #[reflect(Component)] // Register with component registry
   struct Player {
       health: f32,
       score: i32,
   }
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .register_type::<Player>() // Must register custom types
           .run();
   }
   ```

2. **Check component registration**:
   ```javascript
   // List all registered component types
   await mcpClient.callTool("observe", {
     query: "list all registered component types"
   });
   ```

3. **Use generic queries to verify entities exist**:
   ```javascript
   // Start broad, then narrow down
   await mcpClient.callTool("observe", { query: "all entities" });
   await mcpClient.callTool("observe", { query: "entities with Transform" });
   ```

---

### Issue 17: Bevy ECS Query Performance Issues

**Symptoms**: Simple queries take longer than expected, frame drops during observation

**Causes**:
- Querying too many entities without filters
- Complex component combinations
- World fragmentation from frequent spawning/despawning

**Solutions**:
1. **Use specific filters in queries**:
   ```javascript
   // Instead of this (slow with 10k+ entities)
   await mcpClient.callTool("observe", { 
     query: "all entities with components" 
   });
   
   // Use this (much faster)
   await mcpClient.callTool("observe", { 
     query: "entities with Player and Transform limit 50" 
   });
   ```

2. **Query by archetype when possible**:
   ```javascript
   // More efficient for Bevy's ECS
   await mcpClient.callTool("observe", { 
     query: "entities in PlayerArchetype" 
   });
   ```

3. **Monitor entity fragmentation**:
   ```javascript
   const metrics = await mcpClient.callTool("observe", {
     query: "archetype statistics and entity distribution"
   });
   ```

---

### Issue 18: Physics Debug Visualization Problems

**Symptoms**: Physics colliders not visible in screenshots, collision detection issues

**Causes**:
- Physics debug rendering not enabled
- Collision shapes don't match visual sprites
- Physics world out of sync with transform world

**Solutions**:
1. **Enable physics debug rendering**:
   ```rust
   use bevy::prelude::*;
   use bevy_rapier2d::prelude::*;
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_plugins(RapierPhysicsPlugin::<NoUserData>::pixels_per_meter(100.0))
           .add_plugins(RapierDebugRenderPlugin::default()) // Enable debug rendering
           .run();
   }
   ```

2. **Check physics-visual alignment**:
   ```javascript
   // Compare physics colliders with visual transforms
   await mcpClient.callTool("observe", {
     query: "entities with both Collider and Transform components",
     reflection: true  // Deep inspection to compare values
   });
   ```

3. **Monitor physics world state**:
   ```javascript
   await mcpClient.callTool("experiment", {
     experiment_type: "physics_debug",
     params: {
       visualize_colliders: true,
       check_transform_sync: true,
       duration_seconds: 10
     }
   });
   ```

---

### Issue 19: Animation State Debugging Issues

**Symptoms**: Animations not playing correctly, state machines stuck

**Causes**:
- Animation graph not properly configured
- State transitions not triggering
- Animation clips missing or corrupted

**Solutions**:
1. **Inspect animation state**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "entities with AnimationPlayer and animation state",
     reflection: true
   });
   ```

2. **Monitor animation transitions**:
   ```javascript
   await mcpClient.callTool("experiment", {
     experiment_type: "animation_debug",
     params: {
       track_state_changes: true,
       duration_seconds: 30
     }
   });
   ```

3. **Verify animation clips are loaded**:
   ```rust
   // In your game code, ensure assets are properly loaded
   fn debug_animation_assets(
       animation_players: Query<&AnimationPlayer>,
       assets: Res<Assets<AnimationClip>>,
   ) {
       for player in animation_players.iter() {
           // Check if animation clips are loaded
           for (handle, _) in player.animations() {
               if assets.get(handle).is_none() {
                   println!("Animation clip not loaded: {:?}", handle);
               }
           }
       }
   }
   ```

---

### Issue 20: Bevy Remote Plugin (BRP) Version Compatibility

**Symptoms**: MCP tools work locally but fail in different Bevy versions

**Causes**:
- BRP protocol changes between Bevy versions
- Component serialization format changes
- New/removed built-in components

**Solutions**:
1. **Check Bevy version compatibility**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "bevy version and protocol info"
   });
   ```

2. **Use version-specific configurations**:
   ```rust
   // Bevy 0.12+
   use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_plugins(DebuggerPlugin::default())
           .add_plugins(RemoteHttpPlugin::default())
           .run();
   }
   
   // Older versions might need different setup
   ```

3. **Test with minimal reproduction**:
   ```rust
   // Create minimal test case
   use bevy::prelude::*;
   use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;
   
   #[derive(Component, Reflect)]
   #[reflect(Component)]
   struct TestComponent(f32);
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_plugins(DebuggerPlugin::default())
           .add_plugins(RemoteHttpPlugin::default())
           .register_type::<TestComponent>()
           .add_systems(Startup, setup)
           .run();
   }
   
   fn setup(mut commands: Commands) {
       commands.spawn(TestComponent(42.0));
   }
   ```

---

### Issue 21: Asset Loading and Debug Monitoring

**Symptoms**: Assets not loading, missing textures/sounds in debug screenshots

**Causes**:
- Asset loading not complete when debugging starts
- Asset paths incorrect in different environments
- Asset hot reloading interfering with debugging

**Solutions**:
1. **Monitor asset loading state**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "asset loading progress and failed assets"
   });
   ```

2. **Add warmup period for asset loading**:
   ```javascript
   // Wait for assets to load before debugging
   await mcpClient.callTool("experiment", {
     experiment_type: "asset_loading_wait",
     params: {
       wait_for_completion: true,
       timeout_seconds: 30
     }
   });
   ```

3. **Check asset paths and loading**:
   ```rust
   // In your game, add asset loading diagnostics
   fn monitor_asset_loading(
       server: Res<AssetServer>,
       images: Res<Assets<Image>>,
   ) {
       // Check loading status of critical assets
       let texture_handle = server.load("player.png");
       match server.get_load_state(&texture_handle) {
           Some(bevy::asset::LoadState::Loaded) => {
               println!("Player texture loaded successfully");
           }
           Some(bevy::asset::LoadState::Failed) => {
               println!("Player texture failed to load");
           }
           _ => {
               println!("Player texture still loading...");
           }
       }
   }
   ```

---

### Issue 22: Bevy System Ordering and Debug Timing

**Symptoms**: Debug observations show inconsistent state, race conditions in debugging

**Causes**:
- Debug tools running at wrong time in frame cycle
- System dependencies not properly defined
- Debugging affecting game system timing

**Solutions**:
1. **Check system execution order**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "system execution order and dependencies"
   });
   ```

2. **Use proper system sets for debugging**:
   ```rust
   use bevy::prelude::*;
   
   // Define debug system set that runs after game logic
   #[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
   struct DebugSystemSet;
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_systems(Update, (
               game_logic_system,
               debug_monitoring_system.in_set(DebugSystemSet)
           ))
           .configure_sets(Update, DebugSystemSet.after(game_logic_system))
           .run();
   }
   ```

3. **Monitor frame timing consistency**:
   ```javascript
   await mcpClient.callTool("experiment", {
     experiment_type: "frame_timing_analysis",
     params: {
       measure_system_order_impact: true,
       duration_seconds: 60
     }
   });
   ```

---

### Issue 23: Bevy UI Debug Layout Problems

**Symptoms**: UI elements not visible in screenshots, layout issues not apparent in debug

**Causes**:
- UI rendering happens after screenshot capture
- Z-index/layer problems with UI elements
- UI systems disabled or not updating

**Solutions**:
1. **Enable UI debug visualization**:
   ```rust
   use bevy::prelude::*;
   
   fn main() {
       App::new()
           .add_plugins(DefaultPlugins)
           .add_systems(Update, debug_ui_layout)
           .run();
   }
   
   fn debug_ui_layout(
       mut gizmos: Gizmos,
       ui_query: Query<&Style, With<Node>>,
   ) {
       // Draw UI bounds for debugging
       for style in ui_query.iter() {
           // Visualize UI layout boxes
       }
   }
   ```

2. **Check UI element visibility**:
   ```javascript
   await mcpClient.callTool("observe", {
     query: "UI nodes with visibility and layout data",
     reflection: true
   });
   ```

3. **Take screenshots with UI timing consideration**:
   ```javascript
   await mcpClient.callTool("screenshot", {
     path: "ui_debug.png",
     warmup_duration: 2000, // Extra time for UI rendering
     wait_for_ui_updates: true
   });
   ```

---

## Advanced Diagnostics

### Issue 13: Intermittent Connection Drops

**Symptoms**: Connection works sometimes but fails unpredictably

**Diagnostic Steps**:
1. **Enable comprehensive logging**:
   ```bash
   RUST_LOG=trace,tokio=debug bevy-fairy --stdio 2>&1 | tee debug.log
   ```

2. **Monitor network activity**:
   ```bash
   # Linux
   sudo tcpdump -i lo port 15702
   
   # macOS  
   sudo tcpdump -i lo0 port 15702
   ```

3. **Check system resources**:
   ```bash
   # Monitor during issues
   top -p $(pgrep bevy-fairy)
   iostat 1
   ```

---

### Issue 14: Memory Corruption or Panics

**Symptoms**: Segfaults, memory corruption errors, unexpected panics

**Diagnostic Steps**:
1. **Run with debug symbols**:
   ```bash
   RUST_BACKTRACE=full cargo run --bin bevy-fairy
   ```

2. **Use memory debugging tools**:
   ```bash
   # Linux
   valgrind --tool=memcheck ./target/debug/bevy-fairy
   
   # macOS
   leaks -atExit -- ./target/debug/bevy-fairy
   ```

3. **Enable additional runtime checks**:
   ```bash
   RUSTFLAGS="-Z sanitizer=address" cargo run --bin bevy-fairy
   ```

---

### Issue 15: Performance Profiling

**When to Profile**:
- Commands consistently exceed latency budgets
- Memory usage grows unexpectedly
- CPU usage is higher than expected

**Profiling Steps**:
1. **CPU profiling with perf** (Linux):
   ```bash
   perf record -g --call-graph=dwarf ./target/release/bevy-fairy
   perf report
   ```

2. **Memory profiling with heaptrack**:
   ```bash
   heaptrack ./target/release/bevy-fairy
   heaptrack_gui heaptrack.bevy-fairy.*.zst
   ```

3. **Built-in performance monitoring**:
   ```javascript
   const report = await mcpClient.callTool("resource_metrics", {});
   const dashboard = await mcpClient.callTool("performance_dashboard", {});
   ```

---

## Emergency Recovery Procedures

### Complete Reset
If nothing else works, perform a complete reset:

```bash
# Stop all services
launchctl unload ~/Library/LaunchAgents/com.bevy-fairy.plist

# Clear all state
rm -rf ~/.local/share/bevy-fairy/
rm -rf debug_sessions/

# Reinstall  
cargo install --force bevy-fairy

# Reconfigure
bevy-fairy --help
```

### Data Recovery
If you need to recover debugging session data:

```bash
# Session data locations
ls ~/.local/share/bevy-fairy/sessions/
ls debug_sessions/checkpoints/

# The session files are plain JSON; copy them to back them up
cp -r ~/.local/share/bevy-fairy/sessions/ sessions_backup/
```

---

## Getting Additional Help

1. **Enable debug logging** and capture the full output
2. **Create a minimal reproduction case**
3. **Include system information**:
   ```bash
   uname -a
   cargo --version
   rustc --version
   ```
4. **File an issue** at: https://github.com/anthropics/bevy-fairy/issues

### Useful Commands for Bug Reports

```bash
# System info
echo "OS: $(uname -a)"
echo "Rust: $(rustc --version)"
echo "Cargo: $(cargo --version)"

# Configuration
echo "Config: $(cat ~/.config/bevy-fairy/config.toml)"

# Service status (macOS)
echo "Service: $(launchctl list com.bevy-fairy)"

# Process info
echo "Process: $(ps aux | grep bevy-fairy)"

# Network connections
echo "Network: $(lsof -i :15702 -i :3000)"
```

---

*This troubleshooting guide covers the most common issues. For additional help, consult the API documentation and usage guide.*