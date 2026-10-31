# Registering the server with an agent

`bevy-fairy` is an [MCP](https://modelcontextprotocol.io) server that speaks stdio: its client
spawns it, writes JSON-RPC on its stdin and reads the answers from its stdout. Logs go to stderr, so
nothing contaminates the protocol stream.

```sh
cargo install bevy_fairy --no-default-features --features server
bevy-fairy --version
# bevy-fairy 0.12.0
```

`--stdio` is the mode; it is also what the binary does when its stdin is not a terminal, so a client
that passes no argument at all gets a working server.

## Claude Code

```sh
claude mcp add bevy-fairy -- bevy-fairy --stdio
```

## Claude Desktop, or any client with a JSON config

```json
{
  "mcpServers": {
    "bevy-fairy": {
      "command": "bevy-fairy",
      "args": ["--stdio"],
      "env": { "BEVY_BRP_HOST": "localhost", "BEVY_BRP_PORT": "15702" }
    }
  }
}
```

A project-scoped `.mcp.json` at the repository root takes the same shape. Claude Code reads it after
`claude mcp add`, and a project entry wins over the same-named user entry.

## Pointing it at a game

| Variable | Default | Meaning |
|---|---|---|
| `BEVY_BRP_HOST` | `localhost` | Where the game's BRP endpoint is |
| `BEVY_BRP_PORT` | `15702` | Its port |
| `BEVY_FAIRY_CAPTURE_DIR` | `./debug_captures` | Where `screenshot` writes PNGs when the tool is given no `save_to` |
| `RUST_LOG` | `info` | Log level; logs go to stderr |

A game whose HTTP transport runs on another port needs `BEVY_BRP_PORT` set to match
([install.md](install.md)).

## Check it before blaming the agent

```sh
bevy-fairy --version
curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"bevy_fairy/status"}'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}' | bevy-fairy --stdio
```

The last one should answer one JSON line and nothing else. If it prints log lines instead, the
binary is not the one this crate builds: check `which bevy-fairy`.

## If it doesn't work

| Symptom | Cause and fix |
|---|---|
| `status` says no game is answering | The game is not running, or `BEVY_BRP_HOST`/`BEVY_BRP_PORT` do not point at it. The default is `localhost:15702`. |
| The game panics on start | You added a second `RemotePlugin`. `DebuggerPlugin` adds its own; add only a transport such as `RemoteHttpPlugin`. Or `InputPlugin` is missing: `DefaultPlugins` has it, `MinimalPlugins` does not. |
| The agent sees no `bevy-fairy` tools | The client cannot find the binary. Run `bevy-fairy --version`; if that works, give `command` as the full path (`~/.cargo/bin/bevy-fairy`) and restart the client. |
| `screenshot` says `bevy_fairy/screenshot` is not registered | Build the game with the `plugin-screenshot` feature. |
| A `+watch` method is refused by `brp` | Watching methods stream; `brp` returns one answer per call. Sample it with `watch` instead. |
| A request timed out | A game in a long frame has the request queued and may still run it. The reply says so: check `status` before sending it again, or it runs twice. |
