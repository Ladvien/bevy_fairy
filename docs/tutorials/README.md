# Debugging Tutorials - bevy-fairy

Step-by-step tutorials for common debugging scenarios using bevy-fairy.

## Table of Contents

1. [Getting Started](#tutorial-1-getting-started)
2. [Performance Debugging](#tutorial-2-performance-debugging) 
3. [Entity Investigation](#tutorial-3-entity-investigation)
4. [Visual Debugging](#tutorial-4-visual-debugging)
5. [Automated Testing](#tutorial-5-automated-testing)
6. [Advanced Workflows](#tutorial-6-advanced-workflows)

---

## Tutorial 1: Getting Started

### Goal
Set up bevy-fairy and perform your first debugging session.

### Prerequisites
- Bevy game project with the debugger plugin enabled
- Claude Code installed and configured
- Basic understanding of Bevy ECS

### Step 1: Add the debugger plugin to your game

```rust
// main.rs
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(DebuggerPlugin::default())  // Enable BRP
        .add_plugins(RemoteHttpPlugin::default())
        .add_systems(Startup, setup)
        .add_systems(Update, (movement_system, animation_system))
        .run();
}

fn setup(mut commands: Commands) {
    // Your game setup code
    commands.spawn((
        Transform::from_translation(Vec3::new(0.0, 0.0, 0.0)),
        Player { health: 100.0, speed: 5.0 },
    ));
}

#[derive(Component)]
struct Player {
    health: f32,
    speed: f32,
}
```

### Step 2: Start Your Game
```bash
cargo run
```

### Step 3: Start MCP Server
```bash
# In another terminal
bevy-fairy --stdio
```

### Step 4: First Debugging Commands

In Claude Code, try these commands:

**Observe game state:**
```
Can you observe what entities are currently in my Bevy game?
```

**Check system health:**
```  
Please check the health and performance status of my game.
```

**Take a screenshot:**
```
Take a screenshot of my game for documentation.
```

### Expected Results
- You should see a list of entities in your game
- Health check should show system is running
- Screenshot should be saved to your project directory

---

## Tutorial 2: Performance Debugging

### Goal
Identify and resolve performance bottlenecks in a Bevy game.

### Scenario
Your game is experiencing frame rate drops during gameplay.

### Step 1: Check Current Performance

```
What's the current performance status of my game? Check frame times and memory usage.
```

### Step 2: Set Up Performance Budget

```
Set up a performance budget to monitor my game. I want 60 FPS (16.67ms frame time) and memory usage under 500MB.
```

### Step 3: Profile System Performance

```
Start profiling my game systems to identify which ones are taking the most time.
```

### Step 4: Count Entities Over Time

```
Observe the entity count now, wait ten seconds, then observe it again and tell me whether it changed.
```

### Step 5: Analyze Results

```
Generate a performance compliance report for the last 5 minutes and give me recommendations.
```

### Sample Analysis Workflow

```rust
// If you discover a slow system, you might optimize it:

// Before (slow)
fn inefficient_system(mut query: Query<(&mut Transform, &Velocity)>) {
    for (mut transform, velocity) in query.iter_mut() {
        // Expensive calculation every frame
        let expensive_result = complex_calculation(&transform);
        transform.translation += velocity.linear * expensive_result;
    }
}

// After (optimized)  
fn efficient_system(
    mut query: Query<(&mut Transform, &Velocity)>,
    mut cache: Local<HashMap<Entity, f32>>,
) {
    for (entity, (mut transform, velocity)) in query.iter_mut().enumerate() {
        // Cache expensive calculations
        let cached_result = cache.entry(entity).or_insert_with(|| {
            complex_calculation(&transform)
        });
        
        transform.translation += velocity.linear * *cached_result;
    }
}
```

---

## Tutorial 3: Entity Investigation

### Goal
Debug issues with specific entities in your game.

### Scenario
Your player character is not behaving correctly - it's either not moving or moving too fast.

### Step 1: Find Player Entity

```
Find the player entity in my game. Look for entities with Player component or entities that might represent the player character.
```

### Step 2: Inspect Player Components

```
Show me all the components attached to the player entity and their current values.
```

### Step 3: Monitor Player Over Time

```
Start an experiment to monitor the player entity's Transform and velocity over the next 30 seconds while I move around.
```

### Step 4: Test Movement Hypothesis

```
I think the player movement speed is affected by framerate. Can you test this hypothesis by monitoring player movement at different frame rates?
```

### Step 5: Visual Confirmation

```
Take a screenshot showing the player's current position and any debug overlays that might help visualize the issue.
```

### Common Entity Issues and Solutions

**Issue: Entity not found**
```
Search more broadly for entities. Show me all entities with Transform components, maybe the player component name is different.
```

**Issue: Component values look wrong**
```
Compare the current player values with what they should be. Here's what I expect: position (0,0,0), health 100, speed 5.0.
```

**Issue: Entity disappearing**
```
Set up monitoring to track when entities get despawned. I want to know if my player entity is being removed accidentally.
```

---

## Tutorial 4: Visual Debugging

### Goal
Use visual debugging techniques to understand your game's behavior.

### Scenario
You're working on a physics-based game and need to visualize collision boxes, movement paths, and debug information.

### Step 1: Enable Visual Debug Overlays

```
Enable visual debug overlays for my game. I want to see collision boundaries and entity transforms.
```

### Step 2: Screenshot Comparison

```
Take a screenshot, then wait 5 seconds and take another one. I want to compare the before and after to see what changed.
```

### Step 3: Debug Information Overlay

```
Add a performance metrics overlay to my game that shows frame time, entity count, and memory usage in real-time.
```

### Step 4: Record Visual Session

```
Start recording a debug session while I reproduce the bug. Include screenshots every 2 seconds for the next minute.
```

### Step 5: Analyze Visual Data

```
Look at the recorded session and identify any visual anomalies or patterns that might explain the issue.
```

### Custom Visual Debug Setup

```rust
// Add this to your game for enhanced visual debugging
use bevy::prelude::*;

#[derive(Component)]
struct DebugMarker;

fn visual_debug_system(
    mut gizmos: Gizmos,
    query: Query<&Transform, With<DebugMarker>>,
) {
    for transform in query.iter() {
        // Draw debug information
        gizmos.sphere(transform.translation, Quat::IDENTITY, 0.5, Color::RED);
        gizmos.line(
            transform.translation,
            transform.translation + transform.forward() * 2.0,
            Color::BLUE,
        );
    }
}
```

---

## Tutorial 5: Automated Testing

### Goal
Set up automated debugging tests to catch issues before they become problems.

### Scenario
You want to ensure your game maintains good performance and doesn't have memory leaks during extended play sessions.

### Step 1: Set Up Performance Monitoring

```
Configure continuous performance monitoring with these budgets:
- Frame time: 16.67ms (60 FPS)
- Memory: 500MB maximum
- Entity count: under 10,000
- Draw calls: under 1,000
```

### Step 2: Create Automated Test Scenarios

```
Set up an automated test that:
1. Runs the game for 10 minutes
2. Spawns and despawns entities periodically 
3. Moves the player around the scene
4. Takes performance measurements every second
5. Reports any budget violations
```

### Step 3: Hypothesis-Based Testing

```
Test this hypothesis: "Memory usage increases linearly with entity count and doesn't decrease when entities are despawned, indicating a memory leak."
```

### Step 4: Anomaly Detection

```
Enable anomaly detection for frame time and memory usage. Alert me if there are any unusual patterns or sudden changes.
```

### Step 5: Generate Test Report

```
Generate a comprehensive test report covering the automated session. Include performance trends, violations, and recommendations.
```

### Custom Test Framework Integration

```rust
// Example integration with your test framework
#[cfg(test)]
mod integration_tests {
    use super::*;
    use bevy_fairy::testing::*;
    
    #[tokio::test]
    async fn test_memory_leak_detection() {
        let harness = TestHarness::new().await.unwrap();
        
        // Run game simulation
        harness.simulate_gameplay(Duration::from_secs(300)).await;
        
        // Check for memory leaks
        let report = harness.generate_memory_report().await;
        assert!(report.memory_growth_rate < 0.01, "Memory leak detected");
    }
    
    #[tokio::test] 
    async fn test_performance_regression() {
        let harness = TestHarness::new().await.unwrap();
        
        // Baseline performance
        let baseline = harness.measure_baseline_performance().await;
        
        // Test with new code
        let current = harness.measure_current_performance().await;
        
        // Ensure no regression
        assert!(current.frame_time <= baseline.frame_time * 1.1);
    }
}
```

---

## Tutorial 6: Advanced Workflows

### Goal
Combine multiple debugging techniques for complex problem solving.

### Scenario
You're debugging a complex multiplayer synchronization issue that involves entities, networking, and timing.

### Step 1: Set Up Comprehensive Monitoring

```
Set up a comprehensive debugging workflow:
1. Monitor all player entities every frame
2. Track network messages and timing
3. Record screenshots every 5 seconds
4. Profile system performance continuously
5. Detect anomalies in entity positions and network lag
```

### Step 2: Orchestrated Investigation

```
Run an orchestrated debugging session that:
1. Observes all networked entities
2. Experiments with different network latency scenarios
3. Takes screenshots during state transitions
4. Analyzes timing patterns for synchronization issues
```

### Step 3: Multi-Phase Analysis

```
Perform this multi-phase analysis:

Phase 1: Establish baseline behavior with single player
Phase 2: Add second player and monitor for desync
Phase 3: Introduce network lag and packet loss
Phase 4: Analyze results and identify root cause
```

### Step 4: Hypothesis Testing Chain

```
Test these related hypotheses in sequence:
1. "Entity positions desync when network latency > 100ms"  
2. "Desync is caused by client-side prediction errors"
3. "Server reconciliation happens too infrequently"
4. "The issue only occurs with specific entity types"
```

### Step 5: Automated Regression Suite

```
Create an automated test suite that:
1. Tests various network conditions
2. Validates synchronization accuracy
3. Measures performance impact
4. Generates detailed reports with visual evidence
5. Runs continuously in CI/CD pipeline
```

### Complex Workflow Example

```rust
// Advanced debugging workflow implementation
use bevy_fairy::orchestration::*;

async fn complex_debugging_workflow() -> Result<DebugReport> {
    let orchestrator = WorkflowOrchestrator::new();
    
    // Define complex workflow
    let workflow = WorkflowBuilder::new()
        .phase("baseline", |phase| {
            phase.observe("single player entities")
                 .screenshot("baseline_state.png")
                 .profile_systems(Duration::from_secs(60))
        })
        .phase("multiplayer", |phase| {
            phase.experiment("add_second_player")
                 .observe("entity synchronization")  
                 .monitor_network("packet_timing")
        })
        .phase("analysis", |phase| {
            phase.hypothesis_test("desync_correlation_with_latency")
                 .generate_report("synchronization_analysis")
        })
        .build();
    
    // Execute workflow
    orchestrator.execute(workflow).await
}
```

---

## Best Practices

### 1. Start Simple
Always begin with basic observations before moving to complex analysis.

### 2. Document Everything
Use screenshots and descriptions to document your debugging process.

### 3. Use Hypothesis-Driven Debugging
Form specific hypotheses and test them systematically.

### 4. Monitor Performance Impact
Be aware of the performance cost of debugging tools themselves.

### 5. Automate Repetitive Tasks
Create automated tests for issues you debug frequently.

### 6. Combine Techniques
Use multiple debugging approaches together for comprehensive analysis.

---

## Troubleshooting Tutorials

If a tutorial step fails, check the argument names against the [Tool Reference](../mcp-tools.md) or open an issue.

### Quick Fixes

**Connection Issues:**
```
Check the health status of my debugging connection and try to reconnect if needed.
```

**Performance Problems:**
```
Clear all debugging history and reset performance monitoring to start fresh.
```

**Command Failures:**
```
Show me the diagnostic report to understand what went wrong with the last command.
```

---

*These tutorials provide hands-on experience with the most common debugging scenarios. Practice them with your own games to become proficient with the bevy-fairy.*