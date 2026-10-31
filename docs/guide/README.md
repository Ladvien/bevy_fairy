# Guides

The README is the tour; these pages are the detail. Each one names what the server or the plugin
does and what it refuses.

| Page | What it covers |
|---|---|
| [install.md](install.md) | Adding the plugin to a game, the feature flags, ports, and loading it only in debug builds. |
| [clients.md](clients.md) | Registering the `bevy-fairy` server with Claude Code, Claude Desktop or a project `.mcp.json`, and fixing a client that cannot find it. |
| [tools.md](tools.md) | The 14 MCP tools, the BRP methods the plugin registers, the environment variables, and every example. |
| [commands.md](commands.md) | Letting an agent run the game's own actions: typed params, worked examples, the startup check. |
| [input.md](input.md) | Injecting keyboard, text, mouse, scroll and cursor input, and how the pointer reaches Bevy picking. |
| [capture.md](capture.md) | Offscreen and window capture, crop and zoom, and why the offscreen target is the one to use. |
| [guide.md](guide.md) | The agent-to-person overlay: steps, checkpoints, placement, and what the transcript counts. |
| [scene.md](scene.md) | Recording the commands a session ran and replaying them at startup. |
| [claims.md](claims.md) | `experiment`, `hypothesis` and `detect_anomaly`: changing the game, testing a claim, scoring anomalies — and why a game restart matters. |
| [workflows.md](workflows.md) | Six worked sessions: a value that stops changing, a missing entity, a capture, a regression test. |
| [../research/README.md](../research/README.md) | Why the crate is built this way: the papers and the measurements behind the no-OS rule. |

The generated tool reference is [`../mcp-tools.md`](../mcp-tools.md); it is written from the
server's own `tools/list` by `tests/tool_list.rs`, so it cannot describe a tool that does not exist.
