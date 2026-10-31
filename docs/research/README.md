# Why it is built this way

The crate has one non-negotiable rule and a set of smaller ones, and every one of them came from a
measurement or a paper rather than a preference. This page is the short version; the loop's own
critique (`../loop/2026-10-21-bevy-fairy-loop-literature-critique.md`) is the long one, with a verdict
per concept and the cited work.

## Nothing here may touch the operating system

Capturing a window requires raising it — stealing focus, possibly switching workspaces — and driving
the OS keyboard sends keystrokes to whatever actually has focus. Measured on the game this was built
for: **7,188 distinct colours** captured with the window focused, **1** with another app in front.

So captures come from the game's own pixels (an `Image` a camera renders to), input is written into
Bevy's own input messages, and `tests/plugin_leaf.rs` fails the build if the plugin's dependency graph
ever gains a crate that could synthesise an OS event. Widening that boundary costs a deliberate
argument, not a commit.

## A tool never lies

An agent that receives fabricated data has no way to tell. So a value that cannot be measured is
refused with a reason, never answered with a plausible number: no frame time, CPU or memory, because
BRP does not report them. A tool that misleads an agent is a must-fix bug, not a rough edge.

The contract is generated from the handlers: every tool's advertised schema comes from the same Rust
type its handler deserialises, and `tests/tool_contract.rs` sends every advertised field and refuses
an unknown one, so the two cannot drift apart.

## Evidence over time

Many game bugs exist only across frames. Watching a value over N frames, and comparing a sequence of
frames against a reference, are worth more than another one-shot query — which is why `watch`, `burst`
and the in-game ring exist, and why every reply carries the frame it was observed on. Stale evidence
is worse than none, because it reads as fresh.

## Pixels and state together

Every capture is paired with a compact state snapshot taken on the same frame, and both carry frame
numbers and timestamps, so the agent can tell which picture goes with which state.

## Documentation is checked, not trusted

`tests/docs_drift.rs` reads the README, `docs/`, `book/` and the shipped scripts and fails when a doc
tells a reader to do something the code does not support — add a second `RemotePlugin`, register a
`bevy_fairy/*` method by hand, run a subcommand the binary lacks, name the old crate, or link a file
that is not there. `docs/mcp-tools.md` is written from the server's own `tools/list`, and
`tests/tool_list.rs` fails when it drifts.

Stale docs are the normal state of a project — in the top 1,000 GitHub projects, 28.9% carry an
outdated code reference right now, and have for 4.7 years on average (Tan, Wagner & Treude 2023,
[10.1007/s10664-023-10397-6](https://doi.org/10.1007/s10664-023-10397-6)) — so the guard is a test
rather than a promise.
