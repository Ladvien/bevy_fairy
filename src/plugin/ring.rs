//! `bevy_fairy/ring`: an always-on, bounded recording of a query, one sample per frame.
//!
//! A bug that lives across frames (a value that stops changing, a one-frame spike) is gone by the time
//! an agent asks about it, and polling from outside samples only the frames the poll happened to land
//! on. This runs the same `world.query` the agent would, inside the game, at the end of every frame
//! (`Last`, after the plugin's frame counter ticks), and keeps the newest `frames` results. `dump` hands
//! them back with their frame numbers, so "the frames before it went wrong" can be read after the fact.
//!
//! A `trigger` names a JSON pointer into one frame's query result and a value; the first frame where
//! the pointer resolves to that value is recorded and recording stops, so the buffer ends on the
//! interesting frame instead of scrolling past it.
//!
//! Not a frame recorder: samples are query results, not pixels (use `burst` for those), and the
//! buffer is not an input replay. It is command-level evidence about state.

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::remote::builtin_methods::process_remote_query_request;
use serde::Deserialize;
use serde_json::{json, Number, Value};

use crate::plugin::commands::{add_builtin_command, DebugCommandInfo};
use crate::plugin::ready::FrameTicks;

const DEFAULT_FRAMES: usize = 120;
const MAX_FRAMES: usize = 600;
/// A sample larger than this ends the recording rather than growing the buffer without bound.
const MAX_SAMPLE_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Start,
    Dump,
    Stop,
    Status,
}

/// Stops recording on the first frame whose query result holds `equals` at `pointer`.
#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
struct Trigger {
    pointer: String,
    equals: Value,
}

/// Params of `bevy_fairy/ring`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingParams {
    action: Action,
    query: Option<Value>,
    frames: Option<usize>,
    trigger: Option<Trigger>,
    last: Option<usize>,
}

struct Sample {
    frame: u32,
    elapsed_s: f64,
    outcome: Result<Value, String>,
}

impl Sample {
    fn to_json(&self) -> Value {
        match &self.outcome {
            Ok(result) => json!({ "frame": self.frame, "elapsed_s": self.elapsed_s, "result": result }),
            Err(error) => json!({ "frame": self.frame, "elapsed_s": self.elapsed_s, "error": error }),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct Ring {
    query: Option<Value>,
    trigger: Option<Trigger>,
    capacity: usize,
    samples: VecDeque<Sample>,
    recording: bool,
    evicted: u64,
    triggered_at: Option<u32>,
    stopped: Option<String>,
}

pub(crate) fn register(app: &mut App) {
    app.init_resource::<Ring>().add_systems(Last, record.after(crate::plugin::ready::tick_frames));
    add_builtin_command::<RingParams, _>(
        app,
        DebugCommandInfo::new(
            "bevy_fairy/ring",
            "Keep the newest N frames of a world.query result, one sample per frame, recorded inside the game. Actions: start (query, optional frames and trigger), dump (optional last), stop, status. Every sample carries its frame number and elapsed seconds; the reply says whether it is still recording, how many samples were evicted, and the trigger frame if one fired. A start over an earlier buffer discards it and the reply's replaced says what it held. A field an action does not read is refused by name. Samples are state, not pixels.",
        )
        .params(
            "action: start | dump | stop | status; start needs query (world.query params) and takes frames (1-600, default 120) and trigger {pointer: JSON pointer into one sample's result, equals: value} which stops recording on the first frame it matches (a number matches as a number: 9 matches 9.0, and 0.1 matches the f32 field that holds it); dump takes last (newest N samples)",
        )
        .example(json!({
            "action": "start",
            "query": {"data": {"components": ["my_game::Health"]}},
            "frames": 120,
        }))
        .read_only(),
        handle_ring,
    );
}

/// Whether the sampled `value` is what a trigger's `equals` asked for. `serde_json` calls `9` and
/// `9.0` different, and an `f32` component reads back as the f64 nearest to it (`0.1` is
/// `0.10000000149011612`), so a trigger written the way a person writes a number never matched a
/// float field and the buffer ran on to its end with `triggered_at_frame: null`.
fn same_value(sampled: &Value, wanted: &Value) -> bool {
    match (sampled, wanted) {
        (Value::Number(sampled), Value::Number(wanted)) => same_number(sampled, wanted),
        (Value::Array(sampled), Value::Array(wanted)) => {
            sampled.len() == wanted.len() && sampled.iter().zip(wanted).all(|(s, w)| same_value(s, w))
        }
        (Value::Object(sampled), Value::Object(wanted)) => {
            sampled.len() == wanted.len()
                && wanted.iter().all(|(key, w)| sampled.get(key).is_some_and(|s| same_value(s, w)))
        }
        _ => sampled == wanted,
    }
}

/// Two integers are equal exactly. Otherwise the numbers are equal as numbers, or the sampled one is
/// an `f32` widened to f64 and `wanted` is the shortest decimal that names that `f32`
/// (`0.10000000149011612` is the `f32` a person writes as `0.1`). A value an `f64` field holds is
/// compared exactly: `16777217` is not `16777216.0`, and `1e-50` is not `0.0`.
fn same_number(sampled: &Number, wanted: &Number) -> bool {
    if !sampled.is_f64() && !wanted.is_f64() {
        return sampled == wanted;
    }
    match (sampled.as_f64(), wanted.as_f64()) {
        (Some(sampled), Some(wanted)) => {
            sampled == wanted
                || (f64::from(sampled as f32) == sampled
                    && (sampled as f32).to_string().parse::<f64>().is_ok_and(|named| named == wanted))
        }
        _ => false,
    }
}

fn trigger_fires(trigger: &Trigger, result: &Value) -> bool {
    result.pointer(&trigger.pointer).is_some_and(|sampled| same_value(sampled, &trigger.equals))
}

/// Runs the query once for this frame and pushes the sample.
fn record(world: &mut World) {
    let ring = world.resource::<Ring>();
    if !ring.recording {
        return;
    }
    let Some(query) = ring.query.clone() else {
        return;
    };
    let frame = world.resource::<FrameTicks>().0;
    let elapsed_s = world.resource::<Time<Real>>().elapsed_secs_f64();
    let outcome = process_remote_query_request(In(Some(query)), world).map_err(|e| e.message);

    let mut ring = world.resource_mut::<Ring>();
    if let Ok(result) = &outcome {
        if result.to_string().len() > MAX_SAMPLE_BYTES {
            ring.recording = false;
            ring.stopped = Some(format!(
                "frame {frame}: the query result is over {MAX_SAMPLE_BYTES} bytes; narrow the query (fewer components or a filter) and start again"
            ));
            return;
        }
    }
    let fired = match (&outcome, &ring.trigger) {
        (Ok(result), Some(trigger)) => trigger_fires(trigger, result),
        _ => false,
    };
    ring.samples.push_back(Sample { frame, elapsed_s, outcome });
    while ring.samples.len() > ring.capacity {
        ring.samples.pop_front();
        ring.evicted += 1;
    }
    if fired {
        ring.recording = false;
        ring.triggered_at = Some(frame);
        ring.stopped = Some(format!("trigger matched on frame {frame}"));
    }
}

fn describe(ring: &Ring, now: u32) -> Value {
    json!({
        "recording": ring.recording,
        "frame": now,
        "capacity": ring.capacity,
        "buffered": ring.samples.len(),
        "oldest_frame": ring.samples.front().map(|s| s.frame),
        "newest_frame": ring.samples.back().map(|s| s.frame),
        "evicted": ring.evicted,
        "query": ring.query,
        "trigger": ring.trigger.as_ref().map(|t| json!({"pointer": t.pointer, "equals": t.equals})),
        "triggered_at_frame": ring.triggered_at,
        "stopped": ring.stopped,
    })
}

/// Refuse a field the action does not read, naming it and what the action does read. A call that is
/// refused changes nothing, so this runs before any action is applied: a `stop` that carried a stray
/// `last` must not stop a recording it was refused for.
fn refuse_unread_fields(params: &RingParams) -> Result<(), String> {
    let (action, read): (&str, &[&str]) = match params.action {
        Action::Start => ("start", &["query", "frames", "trigger"]),
        Action::Dump => ("dump", &["last"]),
        Action::Stop => ("stop", &[]),
        Action::Status => ("status", &[]),
    };
    let sent = [
        ("query", params.query.is_some()),
        ("frames", params.frames.is_some()),
        ("trigger", params.trigger.is_some()),
        ("last", params.last.is_some()),
    ];
    let Some((field, _)) = sent.iter().find(|(field, set)| *set && !read.contains(field)) else {
        return Ok(());
    };
    let reads = if read.is_empty() {
        "no fields".to_string()
    } else {
        read.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
    };
    Err(format!("`{field}` is not read by action `{action}`, so it would change nothing; `{action}` reads {reads}"))
}

/// What a `start` threw away, so an agent that starts over knows whether it lost frames it meant to
/// keep. `None` when no buffer had been started.
fn replaced(old: &Ring) -> Option<Value> {
    old.query.as_ref()?;
    Some(json!({
        "buffered": old.samples.len(),
        "oldest_frame": old.samples.front().map(|s| s.frame),
        "newest_frame": old.samples.back().map(|s| s.frame),
        "was_recording": old.recording,
        "triggered_at_frame": old.triggered_at,
        "stopped": old.stopped,
    }))
}

fn handle_ring(In(params): In<RingParams>, world: &mut World) -> Result<Value, String> {
    refuse_unread_fields(&params)?;
    let now = world.resource::<FrameTicks>().0;
    let mut replaced_buffer = None;
    match params.action {
        Action::Start => {
            let mut query = params.query.ok_or("start needs `query`: world.query params, e.g. {\"data\": {\"components\": [\"my_game::Health\"]}}")?;
            // world.query without `strict` matches nothing for a component path it cannot find, which
            // would record an empty array every frame and look like "no entity has it". Default strict;
            // the reply's `query` shows it, and `"strict": false` opts out.
            match query.as_object_mut() {
                Some(object) => {
                    object.entry("strict").or_insert(json!(true));
                }
                None => return Err("`query` must be a world.query params object".to_string()),
            }
            let frames = params.frames.unwrap_or(DEFAULT_FRAMES);
            if frames == 0 || frames > MAX_FRAMES {
                return Err(format!("frames must be 1..={MAX_FRAMES}, got {frames}"));
            }
            if let Some(trigger) = &params.trigger {
                if !trigger.pointer.is_empty() && !trigger.pointer.starts_with('/') {
                    return Err(format!(
                        "trigger.pointer `{}` is not a JSON pointer (it must be empty or start with `/`, e.g. `/0/components/my_game::Health`); it could never match",
                        trigger.pointer
                    ));
                }
            }
            // Refuse a query the game rejects now, with the game's own message, rather than record errors.
            process_remote_query_request(In(Some(query.clone())), world).map_err(|e| format!("query refused by the game: {}", e.message))?;
            let old = std::mem::replace(
                &mut *world.resource_mut::<Ring>(),
                Ring { query: Some(query), trigger: params.trigger, capacity: frames, recording: true, ..default() },
            );
            replaced_buffer = replaced(&old);
        }
        Action::Stop => {
            let mut ring = world.resource_mut::<Ring>();
            if ring.query.is_none() {
                return Err("no ring buffer has been started; call with action `start` first".to_string());
            }
            if ring.recording {
                ring.recording = false;
                ring.stopped = Some(format!("stopped by request on frame {now}"));
            }
        }
        Action::Status => {}
        Action::Dump => {
            let ring = world.resource::<Ring>();
            if ring.query.is_none() {
                return Err("no ring buffer has been started; call with action `start` first".to_string());
            }
            let skip = params.last.map_or(0, |n| ring.samples.len().saturating_sub(n));
            let samples: Vec<Value> = ring.samples.iter().skip(skip).map(Sample::to_json).collect();
            let mut reply = describe(ring, now);
            reply["returned"] = json!(samples.len());
            reply["samples"] = Value::Array(samples);
            return Ok(reply);
        }
    }
    let mut reply = describe(world.resource::<Ring>(), now);
    if let Some(replaced_buffer) = replaced_buffer {
        reply["replaced"] = replaced_buffer;
    }
    Ok(reply)
}
