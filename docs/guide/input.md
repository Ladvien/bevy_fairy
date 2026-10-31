# Injecting input

The agent sends input through `brp` with the method `bevy_fairy/input`:

```jsonc
{"kind": "Keyboard", "key": "Space"}                      // a tap: pressed one frame, released the next
{"kind": "Keyboard", "text": "site_67"}                   // typed into a focused text field
{"kind": "Cursor", "x": 640, "y": 360}                    // logical window pixels (see below)
{"kind": "Mouse", "button": "Left", "action": "Press"}
{"kind": "Cursor", "clear": true}                         // hand the pointer back to the real mouse
```

Input is written into Bevy's own input messages before its input systems run, so `just_pressed`,
text fields and `ButtonInput` all see it like a real key.

A field the chosen `kind` does not read is refused by name, never ignored: there is no `modifiers`, a
`Mouse` call takes no `x`/`y` (aim with a `Cursor` call first), and a `Scroll` needs `x` and/or `y`.

A Back, Forward or Other mouse press reaches `ButtonInput<MouseButton>` and the message stream only:
`bevy_picking` skips those buttons, so no `bevy_ui` button and no pointer observer can see them, and
the reply's `note` says so.

## The pointer reaches picking

`Cursor` and `Mouse` also write the `WindowEvent` messages a real mouse produces, so `bevy_ui`
buttons, `Interaction` and `On<Pointer<Click | Over | Out | Drag..>>` observers fire with no code on
your side. A press at one position, a `Cursor` move and a release elsewhere is a drag. A click lands
at the last `Cursor` position; with none set the reply carries a `note` saying so.

## Coordinates

`x`/`y` are *logical window pixels*, top-left origin, the same space `Window::cursor_position`
reports, unless the host inserted a `DebugCaptureTarget` (feature `plugin-screenshot`). Then the
pointer is routed to that image instead of the window, because a camera that renders to an image is
not hit by a window pointer, and `x`/`y` are pixels of that image. One route per game: a host with
both a window UI and a capture target is driven through the image.

## A host that reads the pointer itself

A host that reads the pointer rather than going through picking has one line to write: read it
through `bevy_fairy::cursor_position(&window, &debug_cursor)`, not `Window::cursor_position`. The
plugin never moves the real pointer, because Bevy would move the physical mouse out from under
whoever is at the machine.

## Time

`bevy_fairy/time` pauses the clock or changes its speed, from above 0 to 1000. The request is
re-applied every frame and the reply says whether it `held` — the clock paused when asked to be,
running when not, at the asked speed — and whether the engine cut the last frame to the clock's
`max_delta` (`clamped_by_max_delta`). A host that owns `Time<Virtual>` adds `.host_owns_clock()`:
the plugin then never un-pauses the host's own pause, and reports it instead.
