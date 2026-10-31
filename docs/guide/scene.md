# Save and replay a session

Every game command that changes the game is recorded; a command registered read-only
(`bevy_fairy/ring` and `bevy_fairy/ui_text` are) is not, because replaying it would change nothing.

```text
scene { "action": "save", "path": "run.scene" }     # write the recorded commands
BEVY_FAIRY_SCENE=run.scene cargo run                # replay them when the game starts
```

The game replays the file one step per frame, once it reports ready. `scene show` returns what has
been recorded so far, and `scene clear` forgets it.

Scenes record commands, not raw input: keys, text and clicks sent with `bevy_fairy/input` are not in
the file. Replay is command-level, not frame-exact, because Bevy is not deterministic by default.

A scene file the game cannot read or that has the wrong format is reported by `status` as
`{path, rejected: <why>}`, with no counts because nothing ran. A scene nobody asked for is `null`,
which is a different answer: the first says the game was told to replay something and refused, the
second says nothing was asked.
