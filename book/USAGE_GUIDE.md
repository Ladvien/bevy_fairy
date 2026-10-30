# bevy-fairy Usage Guide

A comprehensive guide to getting the most out of bevy-fairy for debugging your Bevy games.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Core Concepts](#core-concepts)
3. [Common Debugging Workflows](#common-debugging-workflows)
4. [Advanced Features](#advanced-features)
5. [Performance Optimization](#performance-optimization)
6. [Real-World Examples](#real-world-examples)
7. [Integration Patterns](#integration-patterns)
8. [Tips and Tricks](#tips-and-tricks)

## Architecture Overview

The bevy-fairy acts as a bridge between Claude Code (the AI assistant) and your Bevy game:

```
Claude Code <--> MCP Protocol <--> MCP Server <--> BRP Protocol <--> Bevy Game
                                         |
                                    Debug Tools
                                    (Observe, Experiment, etc.)
```

### Key Components

- **MCP Server**: Handles communication with Claude Code and tool orchestration
- **BRP Client**: Manages WebSocket connection to your Bevy game
- **Tool Modules**: Individual debugging capabilities (observe, experiment, etc.)
- **State Management**: Tracks game state, recordings, checkpoints
- **Error Recovery**: Automatic reconnection, checkpointing, dead letter queue

## Core Concepts

### 1. Entity-Component-System (ECS) Awareness

The debugger understands Bevy's ECS architecture:
- **Entities**: Unique identifiers for game objects
- **Components**: Data attached to entities
- **Resources**: Global game state
- **Systems**: Logic that operates on components

### 2. State Diffing

Sophisticated comparison of game states:
- Component-level differences
- Fuzzy matching for floating-point values
- Changes grouped by component

### 3. Time-Travel Debugging

Record and replay capabilities:
- Frame-by-frame state capture
- Deterministic replay
- Timeline branching for "what-if" scenarios

## Common Debugging Workflows

### Workflow 1: Performance Bottleneck Investigation

**Goal**: Find what's causing frame drops

```yaml
Step 1: Baseline Performance
  Tool: observe
  Query: "Show current performance metrics"
  
Step 2: Enable Monitoring
  Tool: anomaly
  Config: Monitor frame times with high sensitivity
  
Step 3: Stress Test
  Tool: stress
  Type: Gradual entity spawning
  Monitor: FPS, memory, system time
  
Step 4: Identify Bottleneck
  Tool: observe
  Query: "What changed when FPS dropped?"
  
Step 5: Generate Report
  Tool: diagnostic_report
  Include: Performance metrics, system state at degradation point
```

**Example Session**:
```
Human: The game starts lagging when there are many enemies on screen

Claude: I'll investigate the performance issue with multiple enemies. Let me run a systematic test.

[Creates checkpoint]
[Observes baseline with few enemies]
[Gradually spawns more enemies while monitoring]
[Identifies that pathfinding system time spikes at 50+ enemies]
[Suggests optimization: Use hierarchical pathfinding or LOD system]
```

### Workflow 2: Collision Bug Investigation

**Goal**: Fix entities passing through walls

```yaml
Step 1: Observe Current State
  Tool: observe
  Query: "Show all entities with Collider components"
  
Step 2: Set Up Monitoring
  Tool: anomaly
  Watch: Position discontinuities larger than velocity
  
Step 3: Reproduce Issue
  Tool: experiment
  Action: Move entities rapidly toward walls
  
Step 4: Capture Bug
  Tool: replay
  Record: When anomaly detected
  
Step 5: Analyze
  Tool: observe
  Compare: Frame before and after collision failure
```

### Workflow 3: Game Balance Testing

**Goal**: Ensure combat system is balanced

```yaml
Step 1: Define Hypothesis
  Tool: hypothesis
  Test: "Player can defeat boss within 3-5 minutes"
  
Step 2: Run Controlled Tests
  Tool: experiment
  Setup: Standard player stats vs boss
  Repeat: 10 times with different strategies
  
Step 3: Analyze Results
  Tool: observe
  Query: "Average time to defeat, damage dealt/received"
  
Step 4: Adjust and Retest
  Tool: experiment
  Modify: Tweak damage values
  Rerun: Previous tests
```

### Workflow 4: Memory Leak Detection

**Goal**: Find and fix memory leaks

```yaml
Step 1: Baseline Memory
  Tool: observe
  Capture: Initial memory usage
  
Step 2: Stress Creation/Destruction
  Tool: stress
  Action: Rapidly spawn and despawn entities
  Duration: 5 minutes
  
Step 3: Monitor Growth
  Tool: anomaly
  Watch: Memory usage trend
  
Step 4: Identify Leak
  Tool: observe
  Query: "What components/resources grew without bound?"
  
Step 5: Pinpoint Source
  Tool: experiment
  Test: Isolate suspected systems
```

## Advanced Features

### Pipeline Orchestration

Create complex debugging workflows:

```rust
// Example pipeline definition
{
  "pipeline": [
    {
      "tool": "checkpoint",
      "action": "create",
      "id": "safe_state"
    },
    {
      "tool": "stress",
      "test_type": "physics",
      "intensity": 0.8,
      "duration": 30,
      "save_results_as": "physics_stress"
    },
    {
      "tool": "observe",
      "query": "entities with abnormal positions",
      "if_found": "continue",
      "if_not_found": "skip_to_end"
    },
    {
      "tool": "diagnostic_report",
      "include_context": ["physics_stress"]
    }
  ]
}
```

### Checkpoint/Restore System

Save and restore debugging sessions:

```yaml
Before risky operation:
  - checkpoint.create("before_experiment")
  
If something goes wrong:
  - checkpoint.restore("before_experiment")
  
List available checkpoints:
  - checkpoint.list()
```

### Dead Letter Queue

Track failed operations for post-mortem analysis:

```yaml
Check failed operations:
  - dead_letter_queue.list()
  
Retry failed operation:
  - dead_letter_queue.retry(operation_id)
  
Clear old failures:
  - dead_letter_queue.clear(older_than="1h")
```

## Performance Optimization

### Resource Management

The debugger includes built-in resource management:

```yaml
Automatic Optimizations:
  - Request batching (reduces network overhead)
  - State compression (efficient storage)
  - Incremental updates (only send changes)
  - Automatic cleanup (remove old data)

Manual Controls:
  - Set entity limits in queries
  - Configure sampling rates
  - Enable/disable specific monitoring
  - Adjust checkpoint frequency
```

### Best Practices for Performance

1. **Limit Entity Queries**
   ```
   ❌ "Show me all entities"
   ✅ "Show me the first 100 entities with Health components"
   ```

2. **Use Targeted Monitoring**
   ```
   ❌ Monitor everything continuously
   ✅ Monitor specific systems when needed
   ```

3. **Batch Operations**
   ```
   ❌ Multiple individual tool calls
   ✅ Use orchestration for batch operations
   ```

4. **Clean Up Resources**
   ```
   - Stop recordings when done
   - Clear old checkpoints
   - Disable unused monitors
   ```

## Real-World Examples

### Example 1: Debugging AI Pathfinding

**Problem**: Enemies sometimes get stuck on corners

```
1. Observe stuck enemy:
   "Show me enemies with PathfindingAgent that haven't moved in 2 seconds"

2. Examine state:
   "What is the pathfinding target and current path for entity 1234?"

3. Visualize problem:
   "Show the navigation mesh around the stuck enemy"

4. Test fix:
   "Nudge the enemy by 1 unit and see if pathfinding recovers"

5. Implement solution:
   "Add recovery behavior: if stuck for >2s, recalculate path with wider clearance"
```

### Example 2: Multiplayer Desync Investigation

**Problem**: Players see different game states

```
1. Record both viewpoints:
   "Start recording on both client and server"

2. Trigger desync:
   "Simulate packet loss at 10% for 30 seconds"

3. Compare states:
   "Show entities that differ between recordings at frame 1000"

4. Identify cause:
   "Which components weren't properly replicated?"

5. Verify fix:
   "Replay scenario with fixed replication logic"
```

### Example 3: Physics Instability

**Problem**: Objects jitter at high speeds

```
1. Identify threshold:
   "Gradually increase object velocity until jitter appears"

2. Analyze behavior:
   "Record position/velocity for jittering object over 100 frames"

3. Test solutions:
   "Compare: fixed timestep vs variable, different integration methods"

4. Validate fix:
   "Stress test with 100 high-speed objects"
```

### Example 4: Save System Corruption

**Problem**: Save files occasionally corrupt

```
1. Monitor save process:
   "Track all entity changes during save operation"

2. Stress test:
   "Rapidly save/load while modifying game state"

3. Detect corruption:
   "Compare: saved state vs loaded state"

4. Identify cause:
   "Which components fail to serialize correctly?"

5. Fix and verify:
   "Test save/load 1000 times with fix applied"
```

## Integration Patterns

### CI/CD Integration

Automate debugging in your pipeline:

```yaml
# .github/workflows/debug-test.yml
name: Automated Debug Tests

on: [push, pull_request]

jobs:
  debug-tests:
    steps:
      - name: Start Bevy Game
        run: cargo run --features=bevy/remote &
        
      - name: Start MCP Server
        run: ./bevy_fairy &
        
      - name: Run Debug Suite
        run: |
          claude-code run-script debug-suite.claude
        
      - name: Check Results
        run: |
          claude-code evaluate "Any performance regressions?"
```

### Automated Testing Scripts

Create reusable debugging scripts:

```claude
# performance-regression.claude
checkpoint.create("baseline")
observe("current FPS and memory")
stress.spawn_entities(count=1000, duration=60)
observe("FPS after stress")
if (FPS_drop > 20%):
    diagnostic_report.generate()
    alert("Performance regression detected!")
```

### Integration with Game Analytics

Export debugging data for analysis:

```yaml
Workflow:
  1. Record gameplay session
  2. Detect anomalies
  3. Export to analytics:
     - Timestamps of issues
     - State snapshots
     - Performance metrics
  4. Correlate with player reports
```

## Tips and Tricks

### 1. Debugging Shortcuts

```yaml
Quick Commands:
  - "health" → Full system health check
  - "panic" → Create checkpoint + full diagnostic dump
  - "baseline" → Capture current state as reference
  - "compare" → Diff against last baseline
```

### 2. Entity Filters

```yaml
Useful Patterns:
  - "entities where Component.field > value"
  - "entities with [Component1, Component2] but not Component3"
  - "entities matching /regex/"
  - "top 10 entities by Component.field"
```

### 3. Time-based Queries

```yaml
Temporal Filters:
  - "entities created in last 10 seconds"
  - "components modified since checkpoint"
  - "state at timestamp 1234567890"
  - "changes between frame 100 and 200"
```

### 4. Correlation Analysis

```yaml
Finding Relationships:
  - "when FPS drops, what components increase?"
  - "entities that appear before crashes"
  - "correlation between player_count and memory"
```

### 5. Automatic Watchers

```yaml
Set and Forget:
  - "alert when memory exceeds 1GB"
  - "checkpoint every 1000 frames"
  - "record when boss fight starts"
  - "profile when FPS < 30"
```

### 6. Debug Macros

Create custom debugging commands:

```yaml
Macro: find_rendering_issues
  1. observe("entities with Mesh but no Transform")
  2. observe("entities with Transform.scale = 0")
  3. observe("entities outside camera frustum")
  4. report("Potential rendering issues found")
```

### 7. State Validation

```yaml
Invariant Checking:
  - "ensure no entities have Health > MaxHealth"
  - "verify all Children have valid Parent"
  - "check no Transform has NaN values"
```

### 8. Performance Profiling

```yaml
System Analysis:
  - "measure time spent in each system"
  - "count component access patterns"
  - "identify system dependencies"
  - "find parallel execution opportunities"
```

## Common Pitfalls and Solutions

### Pitfall 1: Over-monitoring
**Problem**: Too much monitoring slows down the game
**Solution**: Use targeted, temporary monitors

### Pitfall 2: Large State Captures
**Problem**: Recording everything uses too much memory
**Solution**: Filter entities and components, use sampling

### Pitfall 3: Timing-Dependent Bugs
**Problem**: Bug disappears when debugging
**Solution**: Use passive observation, minimal intervention

### Pitfall 4: State Pollution
**Problem**: Debug modifications affect subsequent tests
**Solution**: Always checkpoint before modifications

### Pitfall 5: Replay Divergence
**Problem**: Replays don't match original
**Solution**: Ensure deterministic systems, control random seeds

## Conclusion

The bevy-fairy provides a powerful toolkit for debugging Bevy games. By combining intelligent observation, controlled experimentation, and systematic testing, you can quickly identify and resolve issues in your game.

Remember:
- Start with observation to understand the problem
- Use checkpoints liberally to enable safe experimentation
- Chain tools together for complex debugging workflows
- Generate reports for documentation and future reference

Happy debugging!