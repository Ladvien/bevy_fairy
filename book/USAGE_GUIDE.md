# bevy-fairy usage guide

How to answer "what is this game doing, and why?" with the tools the server actually has. Setup
(the plugin in your game, the server in your MCP client) is in the [README](../README.md). Every
tool's arguments are in [`docs/mcp-tools.md`](../docs/mcp-tools.md), which is generated from the
server's own `tools/list`; when this guide and that file disagree, that file is right.

## How the pieces talk

```
agent <-- MCP over stdio --> bevy-fairy (server) <-- BRP, JSON-RPC over HTTP :15702 --> your game + DebuggerPlugin
```

The server holds no copy of the game. Every answer is read from the running game when you ask,
and the replies carry the game frame they were read on, so you can tell a fresh reading from a
stale one.

## The first two calls

```
Tool: status
{}
```

Is the game up, is it ready, what frame is it on, how many commands did it register, and where
would a capture come from (`offscreen`, `window` or `none`).

```
Tool: commands
{}
```

The commands the game registered, each with its parameters and a working example. Run one with
`run`. The plugin adds built-in commands of its own: `bevy_fairy/time` (pause the clock or change
its speed), `bevy_fairy/ring` (record a query every frame inside the game), and, with the
`plugin-ui-text` feature, `bevy_fairy/ui_text` (the UI text the player can read).

## Reading the world

`observe` takes a short phrase. The shapes it accepts are listed in its schema; anything else is
refused rather than guessed at.

```
Tool: observe
{"query": "find entities with component my_game::Health"}
```

Component names are full type paths. A name the game does not register is an error that lists the
registered paths ending in it, so `Health` gets you the path to use. `{"query": "list components"}`
lists every registered component type.

For anything `observe` has no phrase for (a resource, a filter with `without`, a component's field
types), use `brp` with Bevy's own methods:

```
Tool: brp
{"method": "world.get_resources", "params": {"resource": "my_game::Score"}}
```

`world.query`, `world.list_components`, `registry.schema` and `rpc.discover` (every method the game
serves) all work the same way.

## A value that changes over time, or stops changing

Most game bugs only show across frames. `watch` samples one BRP method repeatedly and returns the
series, each sample bracketed by the two game frames it was read between.

```
Tool: watch
{"method": "world.query",
 "params": {"data": {"components": ["my_game::Spinner"]}},
 "pointer": "/0/components/my_game::Spinner/angle",
 "samples": 20, "interval_ms": 100}
```

The reply says whether the value changed, how many distinct values it saw, the frame of the first
change, and `constant_since_frame`. It polls over HTTP, so a change that comes and goes between two
samples is not seen, and one call lasts at most 30 s.

When you need every frame, or a recording that runs while you do other things, use the in-game
ring buffer through `run`:

```
Tool: run
{"command": "bevy_fairy/ring",
 "params": {"action": "start", "query": {"data": {"components": ["my_game::Health"]}}, "frames": 120}}
```

`{"action": "dump"}` returns the samples, each with its frame number. A `trigger` stops the
recording on the first frame a value matches, which catches a transient you could not time by
hand.

## What the player sees

```
Tool: screenshot
{"inline": true}
```

The default source is `offscreen`: the image a camera renders to, which leaves out UI. `{"source":
"window"}` reads the game window, UI included; a covered or minimised window can give a stale
frame. The reply names the source actually read and the frames the request and the delivery
happened on.

`burst` captures a short sequence as one contact sheet: a reference frame from just before, then
up to seven frames at the rate you ask for, each cell labelled with its game frame and time. Use it
for a flash, a transition, or a sprite that should be moving.

To read the HUD as text rather than pixels, run `bevy_fairy/ui_text` (needs `plugin-ui-text`): it
returns each text node the player can read, with its rect. With `include_hidden`, it also returns
the hidden ones, each with the first check it failed (empty, invisible, zero size, transparent,
clipped, off screen).

## Driving the game

Input goes in through the `bevy_fairy/input` BRP method, as Bevy input messages inside the game
process: the OS keyboard and mouse are never touched, and the game window does not need focus.

```
Tool: brp
{"method": "bevy_fairy/input", "params": {"kind": "Keyboard", "key": "Space"}}
```

`kind` is `Keyboard` (`key`, or `text` to type a string), `Mouse` (`button`), `Scroll`, or `Cursor`
(`x`, `y` in logical window pixels, or in pixels of the capture image when the game has a
`DebugCaptureTarget`; `clear` hands the pointer back). `action` is `Tap` (the default), `Press` or
`Release`. Input is a request, not a result: confirm it arrived by reading the state it should
have changed, with `observe` or `watch`.

Game commands go through `run`. `scene save` writes every command run this session to a file, and
a game started with `BEVY_FAIRY_SCENE=<path>` runs them again, one per frame, once it is ready.
Scenes hold commands, not input, and Bevy is not deterministic by default, so a replayed scene
repeats the commands, not the exact frames.

## Changing the world to test an idea

`experiment` spawns, modifies or deletes entities and reads back what changed. `undo` reverses a
step when its result said `rollback_available`.

```
Tool: experiment
{"action": "execute",
 "actions": [{"type": "modify", "entity_id": 4294967298,
              "components": [{"type_id": "my_game::Crate", "value": {"weight": 99.0}}]}]}
```

`hypothesis` tests a claim against the running game; it needs a `description` and a
`success_condition`. `detect_anomaly` reads every entity once and flags outliers by the thresholds
you give it. A detection is a lead to check with `observe` or `watch`, not a verdict.

## Walking a person through it

`guide` is the one tool that talks to the person at the keyboard rather than the game: it draws one
step at a time over their window and advances when a condition the app registered holds. Tell them
in chat that the guide is up, poll it with `{"read": true}`, and when a step is waiting on a person,
ask them in words. The tool's description has the full protocol.

## What the tools cannot see

- **Events.** BRP has no method that reports the events a game has seen.
- **Between samples.** `watch` polls; a change shorter than `interval_ms` can fall between two
  samples. The ring buffer samples every frame.
- **One frame across calls.** A JSON-RPC batch is not one frame: its requests can be answered on
  different frames, which is why `watch` brackets each sample with two frame readings.
- **UI in the default capture.** `offscreen` leaves out UI; `window` includes it but can be stale.
- **Exact replay.** Scenes repeat commands, not input or frames.
