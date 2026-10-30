# bevy-fairy - Quick Start Guide

*Get from zero to debugging your Bevy game in under 5 minutes*

## ⚡ 5-Minute Setup

### Prerequisites (30 seconds)
- ✅ Rust 1.70+ installed
- ✅ Claude Code installed
- ✅ A Bevy game project

### Step 1: Install the Debugger (1 minute)

```bash
# Install from crates.io
cargo install bevy_fairy --no-default-features --features server

# Verify installation
bevy-fairy --help
```

### Step 2: Add the debugger plugin (1 minute)

Add this to your Bevy game's `main.rs`:

```rust
use bevy::prelude::*;
use bevy::remote::http::RemoteHttpPlugin;
use bevy_fairy::DebuggerPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(DebuggerPlugin::default()) // ← Add this line
        .add_plugins(RemoteHttpPlugin::default())
        .run();
}
```

### Step 3: Configure Claude Code (1 minute)

Add to your Claude Code configuration:

**Location**: `~/.config/claude/claude_code_config.json` (macOS/Linux)

```json
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "bevy-fairy",
      "args": ["--stdio"]
    }
  }
}
```

### Step 4: Start Debugging (30 seconds)

1. **Run your game**: `cargo run`
2. **Open Claude Code** in your project directory
3. **Try your first command**:

```
Show me all entities in my Bevy game
```

### Step 5: Verify It Works (1 minute)

You should see output like:
```
Found 3 entities in your game:
• Entity 0: Transform, Camera
• Entity 1: Transform, Sprite, Player
• Entity 2: Transform, Light
```

## 🎯 First Debugging Commands

Try these to explore your game:

### Basic Observation
```
What components does the player have?
```

### Performance Check
```
Check the current frame rate and memory usage
```

### Take a Screenshot
```
Take a screenshot of my game for documentation
```

### Test Something
```
What happens if I spawn 10 new entities?
```

## 🚨 Quick Troubleshooting

**Nothing happens when I run commands?**
- Check your game is running with `lsof -i :15702`
- Restart Claude Code after config changes

**"BRP connection failed"?**
- Ensure `DebuggerPlugin::default()` is in your App
- Check port 15702 isn't blocked by firewall

**Tools not available?**
- Update to latest version: `cargo install bevy_fairy --no-default-features --features server --force`
- Verify installation: `bevy-fairy --help`

## 🎮 What's Next?

Now that you're connected, explore these advanced features:

- **[Performance Debugging](tutorials/README.md#tutorial-2-performance-debugging)**: Find bottlenecks
- **[Entity Investigation](tutorials/README.md#tutorial-3-entity-investigation)**: Debug specific entities
- **[Visual Debugging](tutorials/README.md#tutorial-4-visual-debugging)**: Screenshots and overlays
- **[Automated Testing](tutorials/README.md#tutorial-5-automated-testing)**: Set up monitoring

## 📚 Full Documentation

- **[Installation Guide](installation/)**
- **[Tool Reference](mcp-tools.md)**: every tool and argument, generated from the server

---

🎉 **Congratulations!** You're now debugging your Bevy game with AI assistance. Happy debugging!