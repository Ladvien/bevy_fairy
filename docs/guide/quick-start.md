# Five minutes, start to first answer

The shortest path, with a link at each step to the page that explains it.

1. **Install the server.**

   ```sh
   cargo install bevy_fairy --no-default-features --features server
   bevy-fairy --version
   ```

2. **Add the plugin to your game.** `bevy_fairy = { version = "0.12", default-features = false,
   features = ["plugin"] }`, then `.add_plugins(DebuggerPlugin::default())` and a transport
   ([install.md](install.md)).

3. **Register the server with your agent**, from your project directory:

   ```sh
   claude mcp add bevy-fairy -- bevy-fairy --stdio
   ```

   ([clients.md](clients.md) has Claude Desktop and `.mcp.json`.)

4. **Run the game and ask the agent for `status`.** It should report `ready: true`, a frame number
   and how many commands the game registered. Then ask it about your game.

## The first questions worth asking

| Ask | The tool it should reach for |
|---|---|
| "Which entities are in the game, and what components do they have?" | `observe` — `list all entities` |
| "What resources does the game have?" | `brp` — `world.get_resources` |
| "What is the score doing over the next five seconds?" | `watch` on a `world.query` for the component, with a `pointer` into the field |
| "What does the screen look like right now?" | `screenshot`, with `inline: true` |
| "Press space and tell me what happened." | `brp` with `bevy_fairy/input` |

## If nothing answers

- **The game is not up.** `curl -s localhost:15702 -d '{"jsonrpc":"2.0","id":1,"method":"bevy_fairy/status"}'`
  answers, or the port is wrong (`lsof -i :15702`).
- **The agent has no tools.** Its client cannot find the binary; give `command` as the full path and
  restart the client ([clients.md](clients.md)).
- **The game panicked on start.** A second `RemotePlugin`, or no `InputPlugin`
  ([install.md](install.md)).
- **A tool refuses with a reason.** Read the reason: every refusal names what it wanted and what to
  do instead. The server never answers a question it could not measure.
