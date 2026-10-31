# Worked sessions

Six sessions, each with what the agent asks, which tool answers it, and what a real reply looks
like. The fixture game (`examples/fixture_game.rs`) is the one these were checked against, and every
value below is one of its constants.

## 1. A value that should change and stops

**Ask:** *"Something that should spin forever stops. Which of its values stops changing, and on what
frame?"*

The agent calls `status` for the frame, `brp` with `world.query` for the component, then `watch` on
the same method:

```json
{"method": "world.query",
 "params": {"data": {"components": ["fixture_game::Spinner"]}},
 "pointer": "/0/components/fixture_game::Spinner/frames",
 "samples": 30, "interval_ms": 100}
```

`watch` answers with `changed: true`, the number of distinct values, the frame of the first change,
and `constant_since_frame` — the frame from which the value never changed again. In the fixture game
that is frame 120.

**Why not three `brp` calls?** Because a round trip is a frame or more, and the interesting frames are
between them. `watch` stamps every sample with the frames it was read between, so two samples that
report the same frame are the same game frame.

## 2. An entity that is not where it should be

**Ask:** *"How many crates are there, and where is the one nearest the origin?"*

`observe` with `find entities with component fixture_game::Crate` returns each entity with its
component values, and the reply's `frame`/`frame_until` says which frames the read lies between. Ask
again with `diff: true` and the reply compares the two reads, naming the entities that appeared, left
or changed.

**Why not `brp` directly?** `observe` is a phrase-query front end for the same data, and it fills in
the frame stamps. `brp` is still there for a method `observe` does not shape.

## 3. A picture of the current frame, with state beside it

**Ask:** *"What is on screen, and what state does it correspond to?"*

`screenshot` with `inline: true` returns the PNG and the game frames the request and the delivery
happened on; `status` gives the frame now. Two frames apart is fresh; fifty is not, and the numbers
say which it is. `burst` does the same for a change: a reference cell plus up to seven more, labelled
with the frame and the milliseconds since the reference.

## 4. Driving the game the way a player would

**Ask:** *"Press space, wait a second, then tell me whether the player moved."*

`brp` with `bevy_fairy/input` and `{"kind": "Keyboard", "key": "Space"}`, then `watch` on the
player's `Transform.translation`. The key is `just_pressed` for exactly one frame, and the input is
written into Bevy's own messages, so the game's systems see it as they see a real key.

A `bevy_ui` button or a pointer observer needs `{"kind": "Cursor", "x": .., "y": ..}` first and then
`{"kind": "Mouse", "button": "Left", "action": "Press"}` / `"Release"`: the click lands at the last
cursor position, and a press at one place, a move, and a release elsewhere is a drag
([input.md](input.md)).

## 5. A regression test that is a command, not a unit test

**Ask:** *"Spawn three cubes at y = 0.5 and tell me how many there are."*

`run` with `demo/spawn_cube` three times, then `observe` for the count. Then `scene save` writes the
recorded commands to a file, and `BEVY_FAIRY_SCENE=<file>` replays them at startup: the same three
cubes, in the same order, without the agent ([scene.md](scene.md)).

For a claim rather than a script, `hypothesis` runs it over many iterations and puts the game back
after each ([claims.md](claims.md)):

```json
{"action": "quick_test",
 "description": "there are six crates",
 "actions": [{"type": "spawn", "components": {"fixture_game::Crate": {}}}],
 "success_condition": {"type": "entity_exists", "component_types": ["fixture_game::Crate"], "count": 7}}
```

## 6. Walking a person through a reproduction

**Ask:** *"I cannot reproduce your bug — walk me through it."*

`guide` posts the steps; the game draws one at a time over the person's own window, and a step with a
checkpoint moves itself on when the game's condition passes. `read` reports where they are and
whether the current step is waiting on them, which is the agent's cue to ask in chat
([guide.md](guide.md)).

## What the tools cannot do

- **Frame time, CPU and memory.** BRP does not report them, and the tools say so rather than
  invent a number.
- **Anything outside the game.** No window manager, no OS screenshots, no synthetic OS keystrokes.
  That is the crate's one non-negotiable rule.
- **A frame-exact replay.** Bevy is not deterministic by default, and a scene is commands, not input.
