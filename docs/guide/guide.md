# Guide a person through a test

Enable `plugin-guide`. The agent posts a script with `guide`; your game shows one step at a time, in
a card over the window:

```json
{"steps": [{
  "label": "drop a floor and a wall",
  "goal": "a tile needs two pieces before the solver can match",
  "do": ["walk the library with up and down", "press Enter to bring a row in"],
  "checkpoint": "tile has two members",
  "recovery": "if nothing lands, the filter is hiding every row: press Backspace"
}]}
```

Say **why** before what: people ignore a suggestion whose reason they cannot see, which is what
`goal` is for. `do` is two to four short imperatives. `recovery` is the field that makes this better
than a numbered list in a chat window.

## Checkpoints

A checkpoint is a condition your game registers by name, so the plugin never needs to know your
game's vocabulary:

```rust,no_run
use bevy_fairy::Checkpoints;

let has_two = app.register_system(|_: In<serde_json::Value>, tile: Res<Tile>| tile.members >= 2);
app.world_mut().resource_mut::<Checkpoints>().register("tile has two members", has_two);
```

Every checkpoint takes `In<serde_json::Value>`, whether or not it reads it, so a step can always
state what it means.

The plugin runs a step's checkpoint once a frame and moves the step on by itself when it returns
`true`, whether or not anything is watching; `bevy_fairy/guide+watch` reports each pass as it
happens, and a watcher that attaches late is told the passes it missed, in order.

A script that names a checkpoint the game has not registered is refused when it is posted, naming the
step, the checkpoint and what the app does register, and nothing is posted. A step with no checkpoint
never advances on its own: it waits for the person, and the agent's job is to ask them in chat and
then `skip` it. For each step the agent gets back how many times it came up, how many of those its
checkpoint passed, and how long it took — a count of passes over attempts, because a boolean "it
worked" is the thing people get wrong.

## One verb per call

`steps`, `read`, `skip` and `clear` are the four verbs, and a call naming two is refused. `read` is
the safe one: it changes nothing and says whether the current step is waiting on a person. `clear`
takes the card down and keeps the transcript, because the transcript is the result.

## Placement

The card has no camera of its own — a second `Camera2d` in somebody else's app is a bug waiting to
happen — so it spawns into whatever UI tree the host already renders, and an app with no UI camera
shows nothing. Its position and width are the host's: `DebuggerPlugin` `init_resource`s
`GuidePlacement`, so a host that `insert_resource`s its own value wins whichever order the two
plugins are added in. Grimoire sets it to sit below the title line, so the card does not cover the
panels it is talking about.

The card is `Pickable::IGNORE`, so it never eats a click meant for the game underneath.
