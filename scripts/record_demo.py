#!/usr/bin/env python3
"""Records the README's GIFs and its still from examples/demo_game.rs. Stdlib only, plus ImageMagick
and ffmpeg.

    python3 scripts/record_demo.py hero     # docs/media/hero.gif and docs/media/hero.png
    python3 scripts/record_demo.py tour     # docs/media/mcp-tour.gif (32 s: the full tour)
    python3 scripts/record_demo.py agent    # docs/media/agent-session.gif (25 s: an agent session)
    python3 scripts/record_demo.py all      # tour and agent

`hero` is the README's hero: six seconds, one idea, `status` then a read-and-see then three `run`
calls, plus the still PNG beside it. `tour` is the same loop at more length. `agent` runs the
`spinner_stops` eval through evals/run.py until one session passes, at most three times, and draws
its transcript. All three drive the real bevy-fairy server over MCP stdio.

Nothing in any of them is written by this script: every reply is the server's text, pretty-printed
when it is JSON, and every picture is the game's own frame. A tool error aborts the recording,
because a demo that showed one would be showing a failure as if it were the product.
"""

import argparse
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import textwrap
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
# `CARGO_TARGET_DIR` is honoured, because a machine that shares one target across checkouts (the
# docs gate does) has no target/ inside the repository.
TARGET = Path(os.environ.get("CARGO_TARGET_DIR") or (REPO / "target"))
GAME_BIN = TARGET / "debug" / "examples" / "demo_game"
SERVER_BIN = TARGET / "debug" / "bevy-fairy"
WORK = TARGET / "demo"
MEDIA = REPO / "docs" / "media"
BUILD_COMMANDS = (
    "cargo build -j 4 --example demo_game --features plugin-screenshot",
    "cargo build -j 4 --bin bevy-fairy",
)

FONT = "/usr/share/fonts/TTF/DejaVuSansMono.ttf"
BACKGROUND = "#1e1e2e"
TITLE = "#89b4fa"
TEXT = "#cdd6f4"
MUTED = "#a6adc8"
PANEL = "#313244"
LEFT_PANEL = "#181825"

# The tour runs the game at 10 fps, so the spinner's freeze at frame 120 lands mid-tour.
TOUR_FPS = 10
SPINNER_STOPS_AFTER_FRAMES = 120  # examples/fixture_game.rs
# The spinner loop starts once the game has rendered for a while (the first frames are black while
# the renderer warms up), reads every half second, and ends once the freeze is well past.
LOOP_START_FRAME = 50
LOOP_PERIOD_S = 0.5
LOOP_UNTIL_FRAME = 140
LOOP_MAX_READS = 30
CUBES = ([-3.0, 0.5, 3.0], [0.0, 0.5, 3.0], [3.0, 0.5, 3.0])

AGENT_TASK = "spinner_stops"
AGENT_ATTEMPTS = 3
AGENT_LINES = 34
AGENT_WIDTH = 110
AGENT_LONG_SESSION = 14
AGENT_KEEP_CALLS = 6

MAX_GIF_BYTES = 10 * 1024 * 1024
# One palette from every pixel of every frame. `stats_mode=diff` counts only pixels that change,
# so the crates, which never move, got no colours of their own and came out grey.
PALETTE = "fps=10,split[a][b];[a]palettegen=stats_mode=full[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"


def load_evals():
    """evals/run.py, for its process helpers and transcript parsing. It is a script, not a package."""
    spec = importlib.util.spec_from_file_location("evals_run", REPO / "evals" / "run.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


EVALS = load_evals()


# --------------------------------------------------------------------------- MCP over stdio


class Mcp:
    """The bevy-fairy server over stdio: the exchange tests/tool_list.rs uses."""

    def __init__(self, port, work):
        env = dict(os.environ, BEVY_BRP_HOST="127.0.0.1", BEVY_BRP_PORT=str(port))
        with open(work / "server.log", "wb") as log:
            # The server's cwd is the work dir, so a relative `save_to` lands there and the call
            # shown in the GIF is the call that was made.
            self.proc = subprocess.Popen([str(SERVER_BIN), "--stdio"], env=env, cwd=work,
                                         stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                         stderr=log, start_new_session=True)
        self.next_id = 1
        try:
            info = self.request("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                               "clientInfo": {"name": "record_demo", "version": "1"}})
        except BaseException:
            EVALS.kill_group(self.proc)
            raise
        self.version = info["serverInfo"]["version"]
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        self.proc.stdin.write((json.dumps(message) + "\n").encode())
        self.proc.stdin.flush()

    def request(self, method, params=None):
        request_id = self.next_id
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError(f"the server closed stdout during {method}; see server.log")
            try:
                reply = json.loads(line)
            except json.JSONDecodeError:
                continue
            if reply.get("id") == request_id:
                break
        if "error" in reply:
            raise RuntimeError(f"{method} failed: {reply['error']}")
        return reply["result"]

    def call(self, tool, arguments):
        """One tool call's text. A tool error raises: a demo must not show one."""
        result = self.request("tools/call", {"name": tool, "arguments": arguments})
        text = "\n".join(block.get("text", "") for block in result.get("content") or []
                         if block.get("type") == "text")
        if result.get("isError"):
            raise RuntimeError(f"{tool} {json.dumps(arguments)} returned an error: {text}")
        return text


# --------------------------------------------------------------------------- drawing


def fit(lines, width, height):
    """`lines` wrapped at `width` columns, continuation lines indented under their own line, and
    cut to `height` lines with a final `…` when they do not fit."""
    out = []
    for line in lines:
        lead = line[: len(line) - len(line.lstrip(" "))]
        out.extend(textwrap.wrap(line, width, subsequent_indent=lead + "  ", break_on_hyphens=False)
                   or [""])
    if len(out) > height:
        out = out[: height - 1] + ["  …"]
    return out


def write_text(path, lines):
    """ImageMagick reads `-annotate @file` verbatim; an inline argument would expand `%` escapes."""
    path.write_text("\n".join(lines), encoding="utf-8")
    return f"@{path}"


def magick(args):
    subprocess.run(["magick", *args], check=True)


def render_frame(path, title, left_lines, right_png, footer):
    """One tour frame: a tool call and its reply on the left, the captured game frame on the right."""
    stem = path.with_suffix("")
    args = ["-size", "1280x440", f"xc:{BACKGROUND}", "-font", FONT, "-gravity", "NorthWest",
            "-pointsize", "18", "-fill", TITLE,
            "-annotate", "+14+10", write_text(stem.with_suffix(".title.txt"), [title]),
            "-fill", LEFT_PANEL, "-draw", "rectangle 10,50 609,429",
            "-pointsize", "14", "-fill", TEXT,
            "-annotate", "+20+58", write_text(stem.with_suffix(".left.txt"), fit(left_lines, 66, 22))]
    if right_png is not None:
        args += ["(", str(right_png), "-resize", "640x360!", ")", "-geometry", "+630+50", "-composite"]
    else:
        args += ["-fill", PANEL, "-draw", "rectangle 630,50 1269,409",
                 "-gravity", "Center", "-pointsize", "16", "-fill", MUTED,
                 "-annotate", "+310+10", "no capture yet", "-gravity", "NorthWest"]
    args += ["-pointsize", "12", "-fill", MUTED,
             "-annotate", "+630+418", write_text(stem.with_suffix(".footer.txt"), [footer]), str(path)]
    magick(args)


def render_terminal(path, title, lines):
    """One agent-session frame: a terminal showing the last lines of the session."""
    stem = path.with_suffix("")
    magick(["-size", "1100x620", f"xc:{BACKGROUND}", "-font", FONT, "-gravity", "NorthWest",
            "-pointsize", "18", "-fill", TITLE,
            "-annotate", "+14+10", write_text(stem.with_suffix(".title.txt"), [title]),
            "-pointsize", "15", "-interline-spacing", "-2", "-fill", TEXT,
            "-annotate", "+14+50", write_text(stem.with_suffix(".body.txt"), lines[-AGENT_LINES:]),
            str(path)])


def assemble_gif(frames, out, work, width=None, fps=10):
    """`frames` is [(png, seconds)]. Writes `out`, at `width` px if given, else 960 px when the full
    width would go over MAX_GIF_BYTES."""
    listing = work / "frames.txt"
    entries = []
    for png, seconds in frames:
        entries += [f"file '{png}'", f"duration {seconds}"]
    # The concat demuxer ignores the last entry's duration unless the file is listed again.
    entries.append(f"file '{frames[-1][0]}'")
    listing.write_text("\n".join(entries) + "\n")
    staged = work / out.name
    palette = PALETTE if fps == 10 else PALETTE.replace("fps=10,", f"fps={fps},")
    prefixes = [f"scale={width}:-1:flags=lanczos,"] if width else ["", "scale=960:-1:flags=lanczos,"]
    for prefix in prefixes:
        subprocess.run(["ffmpeg", "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", str(listing),
                        "-vf", prefix + palette, "-loop", "0", str(staged)], check=True)
        size = staged.stat().st_size
        if size <= MAX_GIF_BYTES:
            MEDIA.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(staged, out)
            print(f"wrote {out.relative_to(REPO)} ({size / 1e6:.1f} MB, {len(frames)} frames)")
            return
    raise RuntimeError(f"{staged} is {size} bytes at {width or 960} px wide; the limit is "
                       f"{MAX_GIF_BYTES}")


# --------------------------------------------------------------------------- tour


def pretty(reply):
    try:
        return json.dumps(json.loads(reply), indent=2, ensure_ascii=False)
    except json.JSONDecodeError:
        return reply


def call_lines(tool, arguments, reply):
    return [f"> {tool} {json.dumps(arguments, ensure_ascii=False)}"] + \
        ["  " + line for line in pretty(reply).splitlines()]


def spinner_reading(reply):
    for entity in json.loads(reply):
        spinner = (entity.get("components") or {}).get("fixture_game::Spinner")
        if spinner is not None:
            return {"angle": spinner["angle"], "frames": spinner["frames"]}
    raise RuntimeError(f"world.query returned no fixture_game::Spinner: {reply}")


def game_frame(mcp):
    return json.loads(mcp.call("status", {}))["frame"]


def tour():
    work = fresh_workdir("tour")
    steps = []  # (title, left lines, capture or None, seconds); drawn after the session
    readings = []
    game = mcp = None
    try:
        port = EVALS.free_port()
        game = EVALS.start_fixture(GAME_BIN, port, work / "game.log", fps=TOUR_FPS)
        mcp = Mcp(port, work)
        footer = f"real replies from bevy-fairy {mcp.version} · game: examples/demo_game.rs"

        names = [tool["name"] for tool in mcp.request("tools/list")["tools"]]
        steps.append(("tools/list: what an agent sees when it connects",
                      ["> tools/list", f"  {len(names)} tools: {', '.join(names)}"], None, 2.5))
        steps.append(("status: is the game answering, and ready?",
                      call_lines("status", {}, mcp.call("status", {})), None, 2.5))
        steps.append(("commands: what the game registered for an agent to run",
                      call_lines("commands", {}, mcp.call("commands", {})), None, 3))

        # Not drawn: wait out the renderer's first, black frames.
        while game_frame(mcp) < LOOP_START_FRAME:
            time.sleep(0.2)

        title = f"watch angle: the spinner stops at frame {SPINNER_STOPS_AFTER_FRAMES} (planted bug)"
        query = {"method": "world.query", "params": {"data": {"components": ["fixture_game::Spinner"]}}}
        started = time.monotonic()
        for n in range(1, LOOP_MAX_READS + 1):
            reply = mcp.call("brp", query)
            shot = {"save_to": f"cap_{n}.png"}
            saved = mcp.call("screenshot", shot)
            readings.append({"read": n, **spinner_reading(reply)})
            steps.append((title, call_lines("brp", query, reply) + [""] + call_lines("screenshot", shot, saved),
                          work / shot["save_to"], 0.6))
            if readings[-1]["frames"] >= LOOP_UNTIL_FRAME:
                break
            time.sleep(max(0.0, started + n * LOOP_PERIOD_S - time.monotonic()))
        (work / "spinner.jsonl").write_text("".join(json.dumps(r) + "\n" for r in readings))
        if readings[-1]["frames"] < LOOP_UNTIL_FRAME:
            raise RuntimeError(f"the spinner was on frame {readings[-1]['frames']} after "
                               f"{LOOP_MAX_READS} reads; the GIF would not show it stop")

        for k, at in enumerate(CUBES, start=1):
            command = {"command": "demo/spawn_cube", "params": {"at": at}}
            reply = mcp.call("run", command)
            shot = {"save_to": f"cube_{k}.png"}
            saved = mcp.call("screenshot", shot)
            steps.append(("run: drive the game through the commands it registered",
                          call_lines("run", command, reply) + [""] + call_lines("screenshot", shot, saved),
                          work / shot["save_to"], 1.5))

        steps.append(("bevy-fairy: an agent reads and drives a running Bevy game",
                      ["every reply in this GIF is bevy-fairy's real output", "",
                       "regenerate: python3 scripts/record_demo.py tour"], steps[-1][2], 4))
        mcp.call("quit", {})
    finally:
        EVALS.kill_group(mcp.proc if mcp else None)
        EVALS.kill_group(game)

    frames = []
    for i, (title, lines, capture, seconds) in enumerate(steps):
        png = work / f"frame_{i:03}.png"
        render_frame(png, title, lines, capture, footer)
        frames.append((png, seconds))
    assemble_gif(frames, MEDIA / "mcp-tour.gif", work)


# --------------------------------------------------------------------------- hero


HERO_STEPS = 5  # status, brp + screenshot, three runs: about six seconds; the budget is 3 to 8.


def hero():
    """A six-second GIF of the loop that matters: read the game, see the frame, drive the game.

    It is the README's hero, so it has to fit the house budget (3 to 8 seconds, 960 px wide,
    15 fps, 4 MB) and show one idea. The full tour stays in `mcp-tour.gif` for the book.
    """
    work = fresh_workdir("hero")
    steps = []  # (title, left lines, capture, seconds)
    game = mcp = None
    still = None
    try:
        port = EVALS.free_port()
        game = EVALS.start_fixture(GAME_BIN, port, work / "game.log", fps=TOUR_FPS)
        mcp = Mcp(port, work)
        footer = f"real replies from bevy-fairy {mcp.version} · game: examples/demo_game.rs"

        steps.append(("status: is the game answering, and ready?",
                      call_lines("status", {}, mcp.call("status", {})), None, 0.9))

        while game_frame(mcp) < LOOP_START_FRAME:
            time.sleep(0.2)

        query = {"method": "world.query", "params": {"data": {"components": ["fixture_game::Spinner"]}}}
        shot = {"save_to": "hero_1.png"}
        reply = mcp.call("brp", query)
        saved = mcp.call("screenshot", shot)
        steps.append(("brp + screenshot: the state, and the frame it came from",
                      call_lines("brp", query, reply) + [""] + call_lines("screenshot", shot, saved),
                      work / shot["save_to"], 1.7))
        still = work / shot["save_to"]

        for k, at in enumerate(CUBES, start=1):
            command = {"command": "demo/spawn_cube", "params": {"at": at}}
            reply = mcp.call("run", command)
            shot = {"save_to": f"hero_cube_{k}.png"}
            saved = mcp.call("screenshot", shot)
            steps.append(("run: an agent drives the game through its own command",
                          call_lines("run", command, reply) + [""] + call_lines("screenshot", shot, saved),
                          work / shot["save_to"], 1.1))
        mcp.call("quit", {})
    finally:
        EVALS.kill_group(mcp.proc if mcp else None)
        EVALS.kill_group(game)

    # One still, at the capture's own size, paired with the call that produced it: an image in the
    # README that is not a GIF, because stills correlate with a project's popularity and animations
    # do not (Venigalla & Chimalakonda 2022).
    render_still(MEDIA / "hero.png", "brp + screenshot: the state, and the frame it came from",
                 steps[1][1], still, footer)

    frames = []
    for i, (title, lines, capture, seconds) in enumerate(steps):
        png = work / f"frame_{i:03}.png"
        render_frame(png, title, lines, capture, footer)
        frames.append((png, seconds))
    if len(frames) != HERO_STEPS:
        raise RuntimeError(f"the hero is {len(frames)} frames; it was budgeted for {HERO_STEPS}")
    assemble_gif(frames, MEDIA / "hero.gif", work, width=960, fps=15)


def render_still(path, title, left_lines, right_png, footer):
    """The README's still PNG: the same panels as a tour frame, and no GIF machinery."""
    render_frame(path, title, left_lines, right_png, footer)


# --------------------------------------------------------------------------- agent


def clip(text, limit):
    return text if len(text) <= limit else text[: limit - 1] + "…"


def session_lines(block, results):
    """A tool call and the start of its result, the way a terminal would show them."""
    name = block.get("name", "").removeprefix(EVALS.TOOL_PREFIX)
    arguments = {k: v for k, v in (block.get("arguments") or {}).items() if k != "i"}
    lines = [f"> {name} {clip(json.dumps(arguments, separators=(',', ':'), ensure_ascii=False), 90)}"]
    result = results.get(block.get("id"))
    if result is None:
        lines.append("  x (the session ended before this call returned)")
    else:
        text = " ".join(" ".join(b.get("text", "") for b in result.get("content") or []
                                 if b.get("type") == "text").split())
        lines.append(("  x " if result.get("isError") else "  < ") + clip(text, 100))
    return fit(lines, AGENT_WIDTH, len(lines) * 3)


def passing_session(work):
    """The first of up to AGENT_ATTEMPTS eval runs whose record passed, as (record, transcript)."""
    records = work / "records.jsonl"
    for attempt in range(1, AGENT_ATTEMPTS + 1):
        transcripts = work / f"attempt{attempt}"
        before = len(records.read_text().splitlines()) if records.exists() else 0
        cmd = [sys.executable, str(REPO / "evals" / "run.py"), "--suite", "dev", "--task", AGENT_TASK,
               "--fixture-bin", str(GAME_BIN), "--server-bin", str(SERVER_BIN),
               "--out", str(records), "--transcripts", str(transcripts), "--sha", "demo"]
        print(f"attempt {attempt}/{AGENT_ATTEMPTS}", flush=True)
        subprocess.run(cmd, cwd=REPO)
        lines = records.read_text().splitlines() if records.exists() else []
        if len(lines) == before:
            print(f"attempt {attempt} wrote no record", flush=True)
            continue
        record = json.loads(lines[-1])
        if record.get("success") and not record.get("isolation_broken"):
            print(f"attempt {attempt} passed: {record['tool_calls']} tool calls, {record['seconds']} s")
            return record, transcripts / f"dev-{AGENT_TASK}.jsonl"
        print(f"attempt {attempt} did not pass: facts={record.get('facts')} "
              f"stopped={record.get('stopped')} infra_error={record.get('infra_error')}", flush=True)
    return None, None


def agent():
    work = fresh_workdir("agent")
    record, transcript = passing_session(work)
    if record is None:
        sys.exit(f"no passing session in {AGENT_ATTEMPTS} attempts")

    events = EVALS.parse_events(transcript)
    ended = [e["message"] for e in events if e.get("type") == "message_end" and e.get("message")]
    results = {m.get("toolCallId"): m for m in ended if m.get("role") == "toolResult"}
    calls = [block for m in ended if m.get("role") == "assistant"
             for block in m.get("content") or [] if block.get("type") == "toolCall"]
    prompt = next(t["prompt"] for t in EVALS.load_tasks("dev") if t["id"] == AGENT_TASK)
    title = f"real agent session · {record['model']} via omp · its only tools: bevy-fairy"

    screens = []  # (lines, seconds)
    shown = fit([f"$ {prompt}"], AGENT_WIDTH, 100) + [""]
    screens.append((list(shown), 3))
    if len(calls) > AGENT_LONG_SESSION:
        hidden = len(calls) - 2 * AGENT_KEEP_CALLS
        plan = calls[:AGENT_KEEP_CALLS] + [hidden] + calls[-AGENT_KEEP_CALLS:]
    else:
        plan = calls
    for item in plan:
        shown += [f"  … {item} more calls …"] if isinstance(item, int) else session_lines(item, results)
        screens.append((list(shown), 0.8))

    answer = ["Answer:"] + fit(record["answer"].splitlines(), AGENT_WIDTH, 1000)
    if len(answer) > AGENT_LINES:
        answer = answer[: AGENT_LINES - 1] + ["…"]
    screens.append((shown + [""] + answer, 6))

    frames = []
    for i, (lines, seconds) in enumerate(screens):
        png = work / f"frame_{i:03}.png"
        render_terminal(png, title, lines)
        frames.append((png, seconds))
    assemble_gif(frames, MEDIA / "agent-session.gif", work)


# --------------------------------------------------------------------------- main


def fresh_workdir(mode):
    work = WORK / mode
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    return work


def check_prerequisites():
    missing = [str(p.relative_to(REPO)) for p in (GAME_BIN, SERVER_BIN) if not p.is_file()]
    if missing:
        print(f"missing {', '.join(missing)}; build them first:", file=sys.stderr)
        for command in BUILD_COMMANDS:
            print(f"  {command}", file=sys.stderr)
        sys.exit(2)
    tools = [tool for tool in ("magick", "ffmpeg") if shutil.which(tool) is None]
    if tools:
        print(f"not on PATH: {', '.join(tools)}", file=sys.stderr)
        sys.exit(2)


def main():
    ap = argparse.ArgumentParser(description="Record the README's demo GIFs.")
    ap.add_argument("mode", choices=["hero", "tour", "agent", "all"])
    args = ap.parse_args()
    check_prerequisites()
    if args.mode == "hero":
        hero()
    if args.mode in ("tour", "all"):
        tour()
    if args.mode in ("agent", "all"):
        agent()


if __name__ == "__main__":
    main()
