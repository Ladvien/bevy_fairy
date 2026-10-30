# bevy-fairy - API Reference

Complete API reference for all tools, configurations, and integrations.

## Table of Contents

- [Tool APIs](#tool-apis)
- [Configuration](#configuration) 
- [Environment Variables](#environment-variables)
- [Response Formats](#response-formats)
- [Error Handling](#error-handling)
- [Performance Budgets](#performance-budgets)
- [Integration APIs](#integration-apis)

## Tool APIs

### Core Debugging Tools

#### `observe`
**Description**: Query game state using natural language or structured queries.

**Parameters**:
```typescript
interface ObserveRequest {
  query: string;              // Natural language query
  diff?: boolean;             // Show changes since last observation (default: false)
  reflection?: boolean;       // Enable deep component analysis (default: false)
  limit?: number;            // Max entities to return (default: 100)
  format?: 'json' | 'table' | 'summary'; // Output format (default: 'json')
}
```

**Response**:
```typescript
interface ObserveResponse {
  success: boolean;
  data: {
    entities: EntityData[];
    total_count: number;
    query_time_ms: number;
    diff_info?: DiffInfo;
  };
  timestamp: string;
}
```

#### `experiment`
**Description**: Run controlled experiments on game state and behavior.

**Parameters**:
```typescript
interface ExperimentRequest {
  experiment_type: string;    // Type of experiment
  params?: Record<string, any>; // Experiment-specific parameters
  duration?: number;          // Duration in seconds (default: 30)
  iterations?: number;        // Number of repetitions (default: 1)
  safety_limits?: SafetyLimits; // Resource usage limits
}
```

**Response**:
```typescript
interface ExperimentResponse {
  success: boolean;
  data: {
    experiment_id: string;
    results: ExperimentResults;
    performance_impact: PerformanceMetrics;
    artifacts: string[];      // Screenshots, logs, data files
  };
}
```

#### `hypothesis`
**Description**: Test hypotheses using statistical analysis.

**Parameters**:
```typescript
interface HypothesisRequest {
  hypothesis: string;         // Hypothesis in natural language
  confidence?: number;        // Confidence level (0.0-1.0, default: 0.95)
  test_duration?: number;     // Test duration in seconds (default: 60)
  sample_size?: number;       // Minimum samples needed (default: 100)
}
```

**Response**:
```typescript
interface HypothesisResponse {
  success: boolean;
  data: {
    hypothesis_supported: boolean | null; // null if inconclusive
    confidence: number;
    statistical_tests: StatisticalResults;
    evidence: Evidence;
    conclusion: string;
    recommendations: string[];
  };
}
```

#### `detect_anomaly`
**Description**: Detect anomalies in game behavior and performance.

**Parameters**:
```typescript
interface AnomalyRequest {
  detection_type: string;     // Type of anomaly detection
  sensitivity?: number;       // Sensitivity (0.0-1.0, default: 0.8)
  window_size?: number;      // Analysis window size (default: 100)
  baseline_period?: number;   // Baseline establishment time (default: 60)
}
```

**Response**:
```typescript
interface AnomalyResponse {
  success: boolean;
  data: {
    anomalies_detected: number;
    alerts: AnomalyAlert[];
    baseline_established: boolean;
    model_confidence: number;
    time_period: string;
  };
}
```

#### `replay`
**Description**: Record and replay game sessions.

**Parameters**:
```typescript
interface ReplayRequest {
  action: 'record' | 'replay' | 'stop' | 'analyze' | 'compare';
  checkpoint_id?: string;     // For replay/analyze actions
  speed_multiplier?: number;  // Playback speed (default: 1.0)
  start_frame?: number;       // Starting frame (default: 0)
  end_frame?: number;         // Ending frame
  params?: Record<string, any>; // Action-specific parameters
}
```

**Response**:
```typescript
interface ReplayResponse {
  success: boolean;
  data: {
    session_id?: string;
    status: ReplayStatus;
    current_frame?: number;
    analysis_results?: AnalysisResults;
    comparison_results?: ComparisonResults;
  };
}
```

## Configuration

### Server Configuration

**Location**: `~/.config/claude/claude_code_config.json`

```json
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "bevy-fairy",
      "args": ["--stdio"],
      "type": "stdio",
      "env": {
        "BEVY_BRP_HOST": "localhost",
        "BEVY_BRP_PORT": "15702",
        "RUST_LOG": "info",
        "BEVY_FAIRY_TIMEOUT": "30",
        "BEVY_FAIRY_MAX_RETRIES": "3"
      }
    }
  }
}
```

### Performance Budget Configuration

```toml
# config/performance_budget.toml
[budgets]
frame_time_ms = 16.67          # 60 FPS target
memory_mb = 512                # Memory limit
cpu_percent = 80               # CPU usage limit
gpu_percent = 85               # GPU usage limit

[monitoring]
violation_threshold = 3        # Consecutive violations before alert
history_size = 1000           # Number of violations to keep
compliance_samples = 5000     # Performance samples to maintain

[alerts]
email_notifications = true
log_violations = true
screenshot_on_violation = false
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `BEVY_BRP_HOST` | `localhost` | Bevy Remote Protocol host |
| `BEVY_BRP_PORT` | `15702` | Bevy Remote Protocol port |
| `RUST_LOG` | `info` | Logging level |
| `BEVY_FAIRY_TIMEOUT` | `30` | Connection timeout (seconds) |
| `BEVY_FAIRY_MAX_RETRIES` | `3` | Maximum retry attempts |
| `BEVY_FAIRY_CACHE_SIZE` | `1000` | Entity cache size |
| `BEVY_FAIRY_HISTORY_SIZE` | `10000` | Performance history size |

## Response Formats

### Success Response Format

```typescript
interface SuccessResponse<T> {
  success: true;
  data: T;
  timestamp: string;
  request_id?: string;
  performance_info?: {
    execution_time_ms: number;
    memory_used_mb: number;
    cache_hits: number;
  };
}
```

### Error Response Format

```typescript
interface ErrorResponse {
  success: false;
  error: {
    code: string;
    message: string;
    context?: Record<string, any>;
    suggestions?: string[];
    documentation_link?: string;
  };
  timestamp: string;
  request_id?: string;
}
```

## Error Handling

### Error Codes

#### Connection Errors
- `CONNECTION_ERROR`: Failed to connect to game
- `BRP_UNAVAILABLE`: Bevy Remote Protocol not available
- `TIMEOUT_ERROR`: Operation timed out
- `NETWORK_ERROR`: Network communication failed

#### Validation Errors
- `VALIDATION_ERROR`: Invalid parameters
- `QUERY_ERROR`: Invalid query syntax
- `PARAMETER_MISSING`: Required parameter missing
- `TYPE_ERROR`: Parameter type mismatch

#### Resource Errors
- `RESOURCE_EXHAUSTED`: Insufficient system resources
- `MEMORY_LIMIT`: Memory usage exceeded
- `CPU_LIMIT`: CPU usage exceeded
- `DISK_FULL`: Insufficient disk space

#### Authentication Errors
- `AUTH_REQUIRED`: Authentication required
- `INVALID_TOKEN`: Invalid or expired token
- `PERMISSION_DENIED`: Insufficient permissions
- `RATE_LIMITED`: Request rate limit exceeded

### Error Context Examples

```json
{
  "error": {
    "code": "CONNECTION_ERROR",
    "message": "Failed to connect to Bevy game",
    "context": {
      "host": "localhost",
      "port": 15702,
      "timeout_ms": 30000,
      "retry_count": 3
    },
    "suggestions": [
      "Ensure Bevy game is running",
      "Verify RemotePlugin is enabled",
      "Check firewall settings for port 15702"
    ],
    "documentation_link": "https://docs.bevyengine.org/bevy_remote/"
  }
}
```

## Performance Budgets

### Budget Configuration API

```typescript
interface PerformanceBudget {
  frame_time_ms: number;       // Maximum frame time
  memory_mb: number;           // Maximum memory usage  
  cpu_percent: number;         // Maximum CPU usage
  gpu_percent: number;         // Maximum GPU usage
  entity_count: number;        // Maximum entities
  draw_calls: number;          // Maximum draw calls per frame
  network_bandwidth_kbps: number; // Network bandwidth limit
}

interface BudgetViolation {
  metric: string;              // Which metric violated
  actual_value: number;        // Actual measured value
  budget_value: number;        // Budget threshold
  violation_percent: number;   // How much over budget
  timestamp: string;           // When violation occurred
  duration_ms: number;         // How long violation lasted
  severity: 'low' | 'medium' | 'high' | 'critical';
}
```

### Budget Management Commands

```typescript
// Set performance budget
interface SetBudgetRequest {
  budgets: Partial<PerformanceBudget>;
  enforcement_mode: 'monitor' | 'warn' | 'throttle' | 'abort';
  violation_threshold: number;  // Consecutive violations before action
}

// Get budget violations
interface GetViolationsRequest {
  time_range?: {
    start: string;
    end: string;
  };
  severity_filter?: string[];
  metric_filter?: string[];
  limit?: number;
}
```

## Integration APIs

### Claude Code Integration

The MCP server integrates seamlessly with Claude Code through the Model Context Protocol:

```typescript
// Tool discovery
interface ListToolsResponse {
  tools: {
    name: string;
    description: string;
    inputSchema: JSONSchema;
  }[];
}

// Tool execution
interface CallToolRequest {
  name: string;
  arguments: Record<string, any>;
}

interface CallToolResponse {
  content: ToolResponseContent[];
  isError?: boolean;
}
```

### REST API (Optional)

When running in HTTP mode:

```http
POST /api/v1/tools/{tool_name}
Content-Type: application/json
Authorization: Bearer {token}

{
  "parameters": {
    "query": "entities with Transform"
  }
}
```

### WebSocket API (Optional)

For real-time monitoring:

```typescript
// Connect to WebSocket
const ws = new WebSocket('ws://localhost:3001/ws');

// Subscribe to events
ws.send(JSON.stringify({
  type: 'subscribe',
  topics: ['performance_violations', 'anomaly_alerts']
}));

// Receive events
ws.onmessage = (event) => {
  const data = JSON.parse(event.data);
  if (data.type === 'performance_violation') {
    handleViolation(data.payload);
  }
};
```

### Prometheus Metrics

Expose metrics for monitoring systems:

```
# HELP bevy_fairy_frame_time_seconds Current frame time
# TYPE bevy_fairy_frame_time_seconds gauge
bevy_fairy_frame_time_seconds 0.016

# HELP bevy_fairy_entity_count Total entity count
# TYPE bevy_fairy_entity_count gauge  
bevy_fairy_entity_count 1547

# HELP bevy_fairy_tool_calls_total Total tool calls
# TYPE bevy_fairy_tool_calls_total counter
bevy_fairy_tool_calls_total{tool="observe",status="success"} 234
```

## Data Types

### Core Data Types

```typescript
interface EntityData {
  id: number;
  components: ComponentData[];
  archetype: string;
}

interface ComponentData {
  type_name: string;
  data: any;                  // Component-specific data
  reflection_info?: ReflectionInfo;
}

interface PerformanceMetrics {
  frame_time_ms: number;
  memory_mb: number;
  cpu_percent: number;
  gpu_time_ms: number;
  entity_count: number;
  draw_calls: number;
  timestamp: string;
}

interface SafetyLimits {
  max_duration_seconds?: number;
  max_memory_mb?: number;
  max_cpu_percent?: number;
  min_fps?: number;
  auto_stop_on_violation?: boolean;
}
```

### Analysis Result Types

```typescript
interface StatisticalResults {
  p_value: number;
  effect_size: number;
  confidence_interval: [number, number];
  test_type: string;
  sample_size: number;
}

interface AnomalyAlert {
  timestamp: string;
  type: string;
  severity: 'low' | 'medium' | 'high' | 'critical';
  description: string;
  values: Record<string, number>;
  possible_causes: string[];
  recommended_actions: string[];
}
```

## Rate Limiting

### Default Limits

- **observe**: 60 requests/minute
- **experiment**: 10 requests/minute  
- **hypothesis**: 5 requests/minute
- **detect_anomaly**: 20 requests/minute
- **replay**: 5 requests/minute

### Custom Rate Limiting

```json
{
  "rate_limits": {
    "observe": {"requests_per_minute": 100, "burst": 10}
  },
  "rate_limit_mode": "sliding_window",
  "rate_limit_headers": true
}
```

## Version Compatibility

### MCP Protocol Versions
- **Supported**: 2024-11-05 (current)
- **Minimum**: 2024-06-25
- **Recommended**: Latest stable

### Bevy Versions
- **Supported**: 0.14+ 
- **Recommended**: 0.14.2+
- **Remote Protocol**: Required

### Claude Code Versions
- **Minimum**: 1.0.0
- **Recommended**: Latest stable
- **Features**: MCP support required

---

*This API reference covers all public interfaces. For implementation details, see the source code documentation.*