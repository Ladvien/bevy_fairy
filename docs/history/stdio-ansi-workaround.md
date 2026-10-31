# Fixing MCP Server stdio Mode for Claude Desktop

> **History, not instructions.** This was a 2025 workaround for an `rmcp` version that logged to
> stdout. The server now writes its logs to stderr with ANSI off in every mode
> (`src/main.rs`), so no wrapper script is needed: a client spawns `bevy-fairy --stdio` directly.
> Kept for the record, because the symptom is worth recognising. The current setup is
> [`../guide/clients.md`](../guide/clients.md).

## Problem

When running bevy-fairy with Claude Desktop, you may encounter "Unexpected token" JSON parsing errors in the logs:

```
SyntaxError: Unexpected token '\x1B', "\x1B[2m2025-0"... is not valid JSON
```

This occurs because:
1. Log output is being written to stdout instead of stderr
2. ANSI color codes contaminate the JSON-RPC protocol stream
3. The rmcp library (v0.2.1) has a bug where it logs errors to stdout

## Root Cause

MCP servers communicate with Claude Desktop via JSON-RPC over stdio:
- **stdout** is reserved for JSON-RPC protocol messages
- **stderr** should be used for all logging output

When logs are written to stdout, they contaminate the protocol stream and break communication.

## Solution

### 1. Fix Tracing Configuration (Already Applied)

The main.rs file has been updated to properly configure tracing for stdio mode:

```rust
// Determine if we're in stdio mode (for MCP protocol)
let is_stdio_mode = args.iter().any(|arg| arg == "--stdio") || 
                    (!args.iter().any(|arg| arg == "--tcp" || arg == "--server") && !std::io::stdout().is_terminal());

// Initialize tracing to stderr when in stdio mode (stdout is reserved for MCP protocol)
if is_stdio_mode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)  // Disable ANSI color codes in stdio mode
        .init();
} else {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}
```

### 2. Python Wrapper Script (Workaround)

Until the rmcp library is fixed, use the Python wrapper script at `/Users/ladvien/.local/bin/bevy-fairy-stdio`:

```python
#!/usr/bin/env python3
"""
Wrapper for bevy-fairy that filters out log lines from stdout.
This fixes the stdio mode for Claude Desktop integration.
"""
import sys
import subprocess
import json

def is_json_line(line):
    """Check if a line is valid JSON-RPC."""
    try:
        data = json.loads(line)
        return 'jsonrpc' in data or 'method' in data or 'id' in data
    except:
        return False

# Start subprocess and filter output...
```

### 3. Claude Desktop Configuration

Update your Claude Desktop config at `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "/Users/ladvien/.local/bin/bevy-fairy-stdio",
      "args": [],
      "env": {
        "RUST_LOG": "off",
        "BEVY_BRP_HOST": "localhost",
        "BEVY_BRP_PORT": "15702"
      }
    }
  }
}
```

## Installation

1. Ensure the wrapper script is executable:
   ```bash
   chmod +x /Users/ladvien/.local/bin/bevy-fairy-stdio
   ```

2. Restart Claude Desktop to load the new configuration

3. The MCP server should now connect without errors

## Testing

Test the stdio mode manually:
```bash
echo '{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}},"id":1}' | \
  /Users/ladvien/.local/bin/bevy-fairy-stdio
```

You should see only JSON output, no log messages.

## References

- [MCP Protocol Specification](https://modelcontextprotocol.io/docs)
- [Common MCP Server Issues](https://github.com/modelcontextprotocol/servers/issues/516)
- [Tracing to stderr in Rust](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/trait.MakeWriter.html)

## Future Work

- Submit PR to rmcp library to fix logging to stderr
- Consider using the official rust-sdk instead of rmcp
- Implement proper MCP log notifications instead of stderr logging