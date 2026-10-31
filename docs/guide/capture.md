# Capture what the game shows

The plugin captures what your game renders, without reaching outside the game.

## Two sources, and the plugin picks between them

- **offscreen** — an `Image` that one of your cameras renders to. This is the default when you
  insert a `DebugCaptureTarget`, and it is the source to use. It works while the game is behind
  other windows, on another Space, or minimised, and it never touches the screen.
- **window** — the game's own primary window surface. This is the default when there is no
  `DebugCaptureTarget`. It includes the UI drawn to the window, and it does not raise or focus the
  window, but the operating system only keeps a window's surface current while the window is on
  screen. A covered or minimised window can give a stale frame, and on macOS an unfocused window has
  been measured to give a single flat colour.

Neither source is a screen capture. There is no window manager, no focus change, and no screenshot
of your desktop: that is the whole point of the crate.

## Give the plugin something to capture

Enable `plugin-screenshot`, then give the plugin an image one of your cameras renders to:

```rust,no_run
use bevy_fairy::DebugCaptureTarget;

commands.insert_resource(DebugCaptureTarget { image: render_target_handle });
```

With a `DebugCaptureTarget`, captures come from that image and never depend on the window. Without
one, `screenshot` falls back to the game window. A full example with the camera setup is
[`examples/plugin_agent_scene.rs`](../../examples/plugin_agent_scene.rs).

A camera that renders to an offscreen image draws no UI, because the UI is drawn to the window. Use
the window source when the UI is what you need to see.

## Using it from an agent

The server exposes this as the `screenshot` tool:

```json
{"save_to": "shot.png", "source": "offscreen", "region": {"x": 0, "y": 0, "width": 640, "height": 360}, "zoom": 2}
```

`source` is `offscreen` or `window`; `region` crops in physical pixels and is clamped to the image, a
region entirely outside it being an error rather than an empty file; `zoom` magnifies after the crop
and is refused when the result would exceed 4096 x 4096 pixels, so crop first. Pass `inline: true` to
have the PNG come back in the reply as well as the file.

The plugin registers `bevy_fairy/screenshot` and `bevy_fairy/capture` itself, so a game never
registers them by hand: registering them again would add a second `RemotePlugin`, and the game would
panic on start.

A capture that goes to the window is not flagged as possibly stale, because the server cannot know
whether the window was on screen at the time. If that matters, insert a `DebugCaptureTarget` and use
the offscreen source.

## A sequence, as one sheet

`burst` captures a reference frame and then up to seven more at up to 10 fps, and returns them as one
contact sheet of at most two rows of four. Each cell is labelled with the game frame the capture was
delivered on (`F<frame>`) and the milliseconds since the reference was requested (`+<ms>`), so two
cells with the same frame number are the same game frame — the game is slower than the requested fps,
and the labels, not the request, say what actually happened.

## A ring inside the game

`bevy_fairy/ring` records the newest N frames (1 to 600) of a `world.query` result inside the game
itself, with an optional trigger that freezes the buffer on the frame it matches, and `dump` returns
the samples with their frame numbers. It exists for the case a round trip per sample cannot reach: a
change that happens in two frames. A trigger written as a number matches an `f32` field that reads
back as the same value, so `{"equals": 9}` matches a float holding `9.0`.
