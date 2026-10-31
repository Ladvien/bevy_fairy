# Media

Every image, GIF and video the README and the guide use, with the command that regenerates it, the
date it was captured and the crate version it shows. All of it is recorded from
[`examples/demo_game.rs`](../../examples/demo_game.rs), which is [`fixture_game`](../../examples/fixture_game.rs)'s
world rendered offscreen and headless: the capture needs no display, no window and no GPU.

`target/debug/examples/demo_game` and `target/debug/bevy-fairy` must exist first:

```sh
cargo build --example demo_game --features plugin-screenshot
cargo build --bin bevy-fairy
```

`scripts/record_demo.py` also needs ImageMagick (`magick`) and `ffmpeg` on `PATH`.

| File | What it shows | Regenerate with | Captured | Version shown |
|---|---|---|---|---|
| `hero.gif` | About seven seconds of the loop: `status`, a `brp` read of the spinner with the frame it captured, then three `run` calls spawning cubes. 960 px wide, 15 fps. | `python3 scripts/record_demo.py hero` | 2026-10-31 | the binary the footer prints; the run that wrote this file was `0.12.0` |
| `hero.png` | The still: the `brp` + `screenshot` reply beside the frame it saved. The README's one non-animated image. | `python3 scripts/record_demo.py hero` | 2026-10-31 | as above |
| `mcp-tour.gif` | The same game at 10 fps, so the planted bug lands mid-tour: `tools/list`, `status`, `commands`, a read of the spinner every half second until it stops at frame 120, then three `run` calls. 32 s, 1280 px. | `python3 scripts/record_demo.py tour` | 2026-10-31 | as above |
| `agent-session.gif` | A real agent session (`claude-sonnet-5-5` via omp) with no tools but this server's, answering where the spinner stops. The script runs the `spinner_stops` eval up to three times and draws the first session that passed; it never draws one that failed. 25 s, 1100 px. | `python3 scripts/record_demo.py agent` | 2026-10-31 | as above |

Two notes on honesty, because a recording is evidence and evidence that lies is worse than none:

- Nothing in either GIF is written by the script. Every reply is the server's text, and every
  picture is the frame its own `screenshot` tool saved; a tool error aborts the recording rather
  than being drawn as if it were the product.
- The footer of every frame names the `bevy-fairy` version that produced it, so a frame cannot be
  mistaken for the behaviour of the version you are reading about.

The GIFs are not in Git LFS: crates.io rewrites a relative image path to
`https://github.com/bevy-coven/bevy_fairy/raw/HEAD/<path>`, which an LFS pointer does not serve.

The recorder's own ffmpeg filter is the one the org's
[documentation guide](https://github.com/bevy-coven/.github/blob/main/DOCUMENTATION.md) gives, with
one palette built from every pixel of every frame (`stats_mode=full`): with `stats_mode=diff` the
crates, which never move, got no colours of their own and came out grey.

Frames come from the game's own capture path, never from the screen: there is no window manager, no
focus change and no desktop screenshot anywhere in this directory.
