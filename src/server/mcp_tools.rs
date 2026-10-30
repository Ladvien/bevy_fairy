/*
 * bevy-fairy - Centralized Tool Definitions
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

use rmcp::{model::*, tool, tool_router, handler::server::{ServerHandler, router::tool::ToolRouter, tool::Parameters}, schemars, Error as McpError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, future::Future};
use tokio::sync::RwLock;
use tracing::{error, info, debug};
use schemars::JsonSchema;

use crate::server::brp_client::BrpClient;
use crate::server::tools::{observe, experiment, guide, hypothesis, anomaly, replay};
use crate::server::experiment_system::Action;
use crate::server::brp::{BrpPayload, BrpRequest};
use base64::Engine;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Parameter structures for tools
//
// **Every one of these must be named in a `#[tool]` signature, not deserialized from
// `Parameters<Value>` inside the body.** `schemars` renders `serde_json::Value` as
// `{"$schema": …, "title": "AnyValue"}` — a schema with no `type` and no `properties` — and an MCP
// client rejects the whole tool list with `expected "object" (at tools.N.inputSchema.type)`. The
// tools were unreachable in Claude Code for exactly that reason.

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveRequest {
    /// A phrase in one of the shapes the parser accepts; a bare word such as "score" or "crate" is
    /// refused. Shapes, each a complete example: `list all entities`, `list components`,
    /// `show entity 4294967298`, `find entities with component bevy_ecs::name::Name`,
    /// `find entities without component bevy_ecs::name::Name`, `find 5 entities with component bevy_ecs::name::Name`.
    /// Component names are full type paths; a name the game does not register is an error that lists the registered paths ending in it. The metadata carries entity_count (component_type_count for the list-components query) and read_at; find N entities returns at most N rows and adds limit and matched, the number that matched before the cut. For any other read, such as one
    /// resource, use `brp`.
    pub query: String,
    /// Compare this read with the previous `diff: true` read of the same query in this server
    /// session. The first such read is the baseline and says so.
    #[serde(default)]
    pub diff: bool,
}

/// What `guide` accepts. One tool, four verbs, because they are four things you do to one script
/// and splitting them would make an agent choose a tool before it knows what it wants.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuideRequest {
    /// The steps to post, replacing whatever is up. Each step is an object:
    /// `label` (required), `goal`, `do` (array of 2-4 short imperatives), `checkpoint`
    /// (a condition name the APP registered, or null when only a person can judge it),
    /// and `recovery` (what to do when it does not work).
    #[serde(default)]
    pub steps: Option<Vec<Value>>,
    /// Return the transcript and where the person is, changing nothing. Safe to call at any time,
    /// and the way to find out whether the current step is waiting on them.
    #[serde(default)]
    pub read: bool,
    /// Move past the current step WITHOUT its condition passing. This is how a step only a person
    /// can judge gets advanced -- after they have told you, never before.
    #[serde(default)]
    pub skip: bool,
    /// Take the script down. The transcript survives, because the transcript is the result.
    #[serde(default)]
    pub clear: bool,
    /// Show or hide the card without discarding the script.
    #[serde(default)]
    pub visible: Option<bool>,
}

/// `experiment`: run actions against the game. The fields here are the ones the handler reads.
/// `action` picks the operation, `actions` is what it acts on. Before this struct was written the
/// tool advertised `type`/`params`/`duration` and the handler read `action`/`actions`, so every
/// call ran the default branch and the caller's intent was dropped.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExperimentRequest {
    /// The operation to run. One of `execute`, `undo`, `redo`, `history`, `clear_history`,
    /// `archetypes`.
    pub action: String,
    /// The actions to carry out. Required by `execute`; ignored by the other operations.
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// How `hypothesis` varies a test between iterations: the strategies the handler runs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariationType {
    None,
    Random,
    Grid,
    Boundary,
    Fuzz,
}

/// `hypothesis`: state a claim and check it against the game. The handler reads `description` and
/// `success_condition`; the tool used to send `hypothesis`/`confidence`/`context`, so the handler
/// fell through to its default and then failed on a `description` the schema never advertised.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HypothesisRequest {
    /// The operation to run. One of `test`, `quick_test` (10 iterations, 10 s), `stress_test` (fuzz
    /// variation, 1000 iterations unless `iterations` or `variation_type` say otherwise), `validate`
    /// (checks the claim parses; runs nothing).
    pub action: String,
    /// What the claim is, in words. `test` and `quick_test` require it.
    #[serde(default)]
    pub description: Option<String>,
    /// The condition that decides whether the claim held. `test` and `quick_test` require it.
    #[serde(default)]
    pub success_condition: Option<Value>,
    /// The actions to try while testing the claim.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// How to vary the test between iterations. Default `none`. `random` varies within
    /// `numeric_min`..`numeric_max`, `string_pool` and `bool_probability`; `grid` walks
    /// `numeric_values`, `string_values` and `bool_values`; `boundary` tries edge values chosen by
    /// the `include_*` flags; `fuzz` mutates the actions up to `max_mutations` times, each with
    /// probability `mutation_probability`.
    #[serde(default)]
    pub variation_type: Option<VariationType>,
    /// For `random`: the low end of the numeric range to vary over. Default 0.
    #[serde(default)]
    pub numeric_min: Option<f64>,
    /// For `random`: the high end of the numeric range to vary over. Default 100.
    #[serde(default)]
    pub numeric_max: Option<f64>,
    /// For `random`: the strings to vary over. Default none.
    #[serde(default)]
    pub string_pool: Option<Vec<String>>,
    /// For `random`: the probability a varied boolean is true. Default 0.5.
    #[serde(default)]
    pub bool_probability: Option<f64>,
    /// For `grid`: the numbers to try. Default 0, 50, 100.
    #[serde(default)]
    pub numeric_values: Option<Vec<f64>>,
    /// For `grid`: the strings to try. Default `default`.
    #[serde(default)]
    pub string_values: Option<Vec<String>>,
    /// For `grid`: the booleans to try. Default true and false.
    #[serde(default)]
    pub bool_values: Option<Vec<bool>>,
    /// For `boundary`: try zero. Default true.
    #[serde(default)]
    pub include_zero: Option<bool>,
    /// For `boundary`: try negative values. Default true.
    #[serde(default)]
    pub include_negative: Option<bool>,
    /// For `boundary`: try the largest values. Default true.
    #[serde(default)]
    pub include_max: Option<bool>,
    /// For `boundary`: try empty strings and collections. Default true.
    #[serde(default)]
    pub include_empty: Option<bool>,
    /// For `fuzz`: the most mutations applied to one run. Default 3.
    #[serde(default)]
    pub max_mutations: Option<u64>,
    /// For `fuzz`: the probability each mutation applies. Default 0.5.
    #[serde(default)]
    pub mutation_probability: Option<f64>,
    /// How many times to run the test.
    #[serde(default)]
    pub iterations: Option<u64>,
    /// How long the test may run, in seconds.
    #[serde(default)]
    pub timeout: Option<u64>,
    /// The seed, so a run can be repeated exactly.
    #[serde(default)]
    pub seed: Option<u64>,
    /// The confidence a result needs before it counts as confirmed.
    #[serde(default)]
    pub threshold: Option<f64>,
}

/// `detect_anomaly`: look for anomalies in the running game. The handler reads the thresholds below;
/// the tool used to send `type`/`sensitivity`/`window`, none of which the handler reads, so every
/// call ran with the default configuration and the caller's intent was dropped.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnomalyRequest {
    /// The operation to run. One of `detect`, `configure`, `status`.
    pub action: String,
    /// How many samples the window covers.
    #[serde(default)]
    pub window_size: Option<u64>,
    /// The z-score above which a sample counts as anomalous.
    #[serde(default)]
    pub z_score_threshold: Option<f64>,
    /// The IQR multiplier above which a sample counts as anomalous.
    #[serde(default)]
    pub iqr_multiplier: Option<f64>,
    /// The fewest samples needed before a verdict is possible.
    #[serde(default)]
    pub min_samples: Option<u64>,
    /// The frame time, in milliseconds, above which performance counts as anomalous.
    #[serde(default)]
    pub performance_threshold: Option<f64>,
    /// The entity growth per sample above which the count counts as anomalous.
    #[serde(default)]
    pub entity_growth_threshold: Option<f64>,
    /// The lowest severity to report. The handler reads this as a number.
    #[serde(default)]
    pub min_severity: Option<f64>,
    /// How many anomalies to report at most.
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    /// What to do. Recording: `record`, `stop`, `status`, `marker`, `save`, `load`, `stats`.
    /// Playback: `play`, `pause`, `seek`, `step`, `set_speed`, `playback_status`. Branches:
    /// `create_branch`, `list_branches`, `switch_branch`, `add_modification`, `merge_branch`,
    /// `compare_branches`, `delete_branch`, `branch_tree`. Each action reads only the arguments
    /// named for it below.
    pub action: ReplayAction,
    /// `record`: samples per second, above 0 (default 30). Each sample is a BRP round trip, so the rate reached is lower when the game answers slower; `stop` reports the frames and seconds actually recorded.
    #[serde(default)]
    pub sample_rate: Option<f32>,
    /// `record`: the most frames kept in memory.
    #[serde(default)]
    pub max_buffer_size: Option<u64>,
    /// `record`: compress the recording.
    #[serde(default)]
    pub compression: Option<bool>,
    /// `record`: checksum each frame.
    #[serde(default)]
    pub checksums: Option<bool>,
    /// `record`: record only these component type paths.
    #[serde(default)]
    pub component_filter: Option<Vec<String>>,
    /// `record`: record only these event types.
    #[serde(default)]
    pub event_filter: Option<Vec<String>>,
    /// `marker` and `create_branch`: the name. `marker` defaults to `marker`; `create_branch` requires it.
    #[serde(default)]
    pub name: Option<String>,
    /// `marker`: a free-text note.
    /// `add_modification`: what the modification is for.
    #[serde(default)]
    pub description: Option<String>,
    /// `save` and `load`: the file, on this machine. `save` defaults to `recording.bevy`.
    #[serde(default)]
    pub filename: Option<String>,
    /// `seek`: the frame number to go to. Give this or `marker`.
    #[serde(default)]
    pub frame: Option<u64>,
    /// `seek`: the marker name to go to. Give this or `frame`.
    #[serde(default)]
    pub marker: Option<String>,
    /// `step`: `forward` (the default) or `backward`.
    #[serde(default)]
    pub direction: Option<StepDirection>,
    /// `set_speed`: the playback speed, required (1.0 is real time).
    #[serde(default)]
    pub speed: Option<f32>,
    /// `create_branch`: the branch this one forks from.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// `create_branch`: the frame it forks at. Default 0.
    #[serde(default)]
    pub branch_point_frame: Option<u64>,
    /// `switch_branch`, `add_modification`, `merge_branch`, `delete_branch`, `branch_tree`: the branch.
    #[serde(default)]
    pub branch_id: Option<String>,
    /// `add_modification`: the frame the modification applies at.
    #[serde(default)]
    pub frame_number: Option<u64>,
    /// `add_modification`: set one component of one entity. Give this or `entity_removal`.
    #[serde(default)]
    pub entity_modification: Option<EntityModification>,
    /// `add_modification`: remove one entity. Give this or `entity_modification`.
    #[serde(default)]
    pub entity_removal: Option<EntityRemoval>,
    /// `merge_branch`: `take_source` (the default), `take_target` or `manual`.
    #[serde(default)]
    pub strategy: Option<MergeStrategyArg>,
    /// `compare_branches`: the first branch.
    #[serde(default)]
    pub branch_a: Option<String>,
    /// `compare_branches`: the second branch.
    #[serde(default)]
    pub branch_b: Option<String>,
    /// `compare_branches`: the first frame compared. Default 0.
    #[serde(default)]
    pub start_frame: Option<u64>,
    /// `compare_branches`: the frame the comparison stops before. Default 10.
    #[serde(default)]
    pub end_frame: Option<u64>,
}

/// What `replay` does.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplayAction {
    Record, Stop, Status, Marker, Save, Load, Stats,
    Play, Pause, Seek, Step, SetSpeed, PlaybackStatus,
    CreateBranch, ListBranches, SwitchBranch, AddModification, MergeBranch, CompareBranches,
    DeleteBranch, BranchTree,
}

/// The direction of a `step`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepDirection { Forward, Backward }

/// How `merge_branch` resolves conflicts.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategyArg { TakeSource, TakeTarget, Manual }

/// Set one component of one entity.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityModification {
    /// The entity's id.
    pub entity_id: u64,
    /// The component's type path.
    pub component_name: String,
    /// The component's new value, as JSON.
    pub new_value: Value,
}

/// Remove one entity.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityRemoval {
    /// The entity's id.
    pub entity_id: u64,
}

/// A tool that takes no arguments. It is a named, empty struct for the reason given at the top of this file.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// `run`: run one command the game registered. `commands` lists them with their parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// The command name, `<game>/<verb>`, exactly as `commands` lists it.
    pub command: String,
    /// The command's parameters as a JSON object. Omit it for a command that takes none.
    #[serde(default)]
    pub params: Option<Value>,
}

/// `brp`: a raw Bevy Remote Protocol call, for any method the game serves.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrpCallRequest {
    /// The BRP method, for example `world.query` or `registry.schema`.
    pub method: String,
    /// The method's params as JSON, omitted for a method that takes none. Shapes, one per line:
    /// - `world.query`: `{"data":{"components":["bevy_ecs::name::Name"]},"filter":{"with":[],"without":[]}}` (full type paths, never short names: take the game's own from `world.list_components`; `filter` is optional)
    /// - `registry.schema`: `{"with_crates":["bevy_transform"],"type_limit":{"with":["Component"]}}` (every field optional and a list; omit one rather than sending null)
    /// - `world.list_components`: no params for every registered component, or `{"entity":4294967296}` for one entity
    /// - `rpc.discover`: no params; lists every method the game serves with its param schema
    #[serde(default)]
    pub params: Option<Value>,
}

/// A rectangle of the captured image, in its pixels.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// `screenshot`: capture the game to a PNG on this machine.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenshotRequest {
    /// File to write on this machine. Default: `$BEVY_FAIRY_CAPTURE_DIR/capture-<unix_ms>.png`,
    /// or `./debug_captures/` when that variable is unset.
    #[serde(default)]
    pub save_to: Option<String>,
    /// `offscreen` (the default) renders the game's own image and leaves out UI. `window` captures
    /// the game window, UI included.
    #[serde(default)]
    pub source: Option<String>,
    /// Crop to this rectangle before saving.
    #[serde(default)]
    pub region: Option<CaptureRegion>,
    /// Magnify the captured image by this factor. Default 1.
    #[serde(default)]
    pub zoom: Option<f32>,
    /// Also return the PNG inline, so the agent sees the image as well as the saved file.
    #[serde(default)]
    pub inline: Option<bool>,
}

/// What `scene` does with the commands recorded in this game session.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SceneAction {
    /// Write the recorded commands to `path` as a replayable scene file.
    Save,
    /// Return the recorded commands.
    Show,
    /// Forget the recorded commands.
    Clear,
}

/// `scene`: save, show or clear the recorded commands.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneRequest {
    /// `save`, `show` or `clear`.
    pub action: SceneAction,
    /// Required for `save`: the file to write, on this machine.
    #[serde(default)]
    pub path: Option<String>,
}

/// `watch`: sample one BRP method over time, each sample bracketed by the game frames it was read between.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchRequest {
    /// The one-shot BRP method to sample, for example `world.query` or `world.get_resources`.
    /// Names ending in `+watch` are refused.
    pub method: String,
    /// The method's params, exactly as for `brp`.
    #[serde(default)]
    pub params: Option<Value>,
    /// A JSON pointer (RFC 6901) into each result, for example
    /// `/0/components/my_game::Health/current`. Without it each sample carries the whole result,
    /// so use it for anything larger than a few values. A pointer that matches nothing is an error.
    #[serde(default)]
    pub pointer: Option<String>,
    /// How many samples to take, 2 to 100. Default 10.
    #[serde(default)]
    pub samples: Option<u32>,
    /// Milliseconds between samples, 20 to 5000. Default 100. `samples` x `interval_ms` may not
    /// exceed 30000.
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

const WATCH_MAX_TOTAL_MS: u64 = 30_000;
const WATCH_MAX_REPLY_BYTES: usize = 60_000;

// Default value functions

/// Centralized tool schema definitions for better discoverability
#[derive(Clone)]
pub struct BevyDebuggerTools {
    brp_client: Arc<RwLock<BrpClient>>,
    tool_router: ToolRouter<Self>,
}

impl BevyDebuggerTools {
    pub fn new(brp_client: Arc<RwLock<BrpClient>>) -> Self {
        Self {
            brp_client,
            tool_router: Self::tool_router(),
        }
    }

    /// The names of the tools this server registers.
    ///
    /// The instructions and the docs name tools, and a name that is not registered sends an agent
    /// to nothing. This is how a check finds out.
    pub fn tool_names() -> Vec<String> {
        Self::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }
}

/// One entry of `bevy_fairy/commands`, as the game lists it.
#[derive(Debug, Deserialize)]
struct CommandListing {
    commands: Vec<CommandEntry>,
}

#[derive(Debug, Deserialize)]
struct CommandEntry {
    name: String,
    description: String,
    #[serde(default)]
    params: String,
    #[serde(default)]
    example: Value,
}

const STATUS_METHOD: &str = "bevy_fairy/status";
const COMMANDS_METHOD: &str = "bevy_fairy/commands";
const TIME_COMMAND: &str = "bevy_fairy/time";
const SCREENSHOT_METHOD: &str = "bevy_fairy/screenshot";
const CAPTURE_METHOD: &str = "bevy_fairy/capture";
const SCENE_METHOD: &str = "bevy_fairy/scene";
const QUIT_METHOD: &str = "bevy_fairy/quit";

/// The text an agent reads when nothing answers. Kept exact, because agents match on it.
fn no_game_text(url: &str) -> String {
    format!("bevy-fairy: no game answering at {url} — start the game with DebuggerPlugin and RemoteHttpPlugin, or set BEVY_BRP_HOST/BEVY_BRP_PORT")
}

fn tool_ok(text: impl Into<String>) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(text.into())]))
}

fn tool_err(text: impl Into<String>) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![Content::text(text.into())]))
}

/// A request as the handler's argument object. Serialising the request struct, not listing its
/// fields by hand, means a field added to the schema reaches the handler; a hand-written list was
/// the second place a field could be added and missed (B-024).
fn request_args<T: Serialize>(req: &T) -> Result<Value, Result<CallToolResult, McpError>> {
    serde_json::to_value(req).map_err(|e| tool_err(format!("the request could not be encoded: {e}")))
}

/// A tool handler's answer as a tool result. The handlers report a refusal -- an unknown action,
/// a query they cannot parse, a game that said no -- as `Ok` with a top-level `"error"` key. That
/// is a failed call, so it goes back with `isError` set: a client that branches on `isError`
/// would otherwise take "Query parsing failed" for an answer.
fn handler_result(result: Value) -> Result<CallToolResult, McpError> {
    if result.get("error").is_some_and(|error| !error.is_null()) {
        tool_err(result.to_string())
    } else {
        tool_ok(result.to_string())
    }
}

fn command_block(entry: &CommandEntry) -> String {
    let params = if entry.params.is_empty() { "(none)" } else { entry.params.as_str() };
    format!("{} — {}\n  params: {}\n  example: {}", entry.name, entry.description, params, entry.example)
}

/// Name the closest registered command, and list them all, when `asked` is not one of them.
fn unknown_command_text(asked: &str, entries: &[CommandEntry]) -> String {
    let known: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
    let closest = known
        .iter()
        .copied()
        .max_by(|a, b| strsim::jaro_winkler(asked, a).total_cmp(&strsim::jaro_winkler(asked, b)));
    match closest {
        Some(closest) => format!("unknown command `{asked}`; did you mean `{closest}`? known: {}", known.join(", ")),
        None => format!("unknown command `{asked}`; the game has registered no commands"),
    }
}

fn write_file_creating_dirs(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)
}

/// `$BEVY_FAIRY_CAPTURE_DIR/capture-<unix_ms>.png`, or under `./debug_captures` when unset.
fn default_capture_path() -> PathBuf {
    let dir = std::env::var_os("BEVY_FAIRY_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("debug_captures"));
    let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    dir.join(format!("capture-{unix_ms}.png"))
}

fn default_burst_dir() -> PathBuf {
    let dir = std::env::var_os("BEVY_FAIRY_CAPTURE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("debug_captures"));
    let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    dir.join(format!("burst-{unix_ms}"))
}

fn truncate_for_error(value: &Value) -> String {
    let text = value.to_string();
    if text.len() <= 300 {
        text
    } else {
        format!("{}…", text.chars().take(300).collect::<String>())
    }
}

/// `burst`: a short sequence of captures as one labelled contact sheet.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BurstRequest {
    /// Frames to capture after the reference, 1 to 7. Default 4.
    #[serde(default)]
    pub frames: Option<u32>,
    /// Requested captures per second, 0.5 to 10. Default 4. The real spacing is in the labels.
    #[serde(default)]
    pub fps: Option<f32>,
    /// `offscreen` (the default) renders the game's own image and leaves out UI. `window` captures
    /// the game window, UI included.
    #[serde(default)]
    pub source: Option<String>,
    /// Directory to write sheet.png and ref.png into. Default: a new `burst-<unix_ms>` directory
    /// under `$BEVY_FAIRY_CAPTURE_DIR`, or `./debug_captures/` when that variable is unset.
    #[serde(default)]
    pub save_dir: Option<String>,
    /// Also save every frame at full resolution as `frame-<n>.png`. Default false.
    #[serde(default)]
    pub full_frames: Option<bool>,
    /// Also return the contact sheet inline, so the agent sees it in the reply.
    #[serde(default)]
    pub inline: Option<bool>,
}

/// One finished capture as the plugin delivered it.
struct Captured {
    width: u64,
    height: u64,
    png_base64: String,
    png: Vec<u8>,
    source: Option<String>,
    frame_requested: Option<u64>,
    frame_captured: Option<u64>,
}

impl Captured {
    /// What the plugin said about the capture, or that it said nothing.
    fn provenance(&self) -> String {
        match (&self.source, self.frame_requested, self.frame_captured) {
            (Some(source), Some(requested), Some(taken)) => {
                format!("source {source}, requested at frame {requested}, delivered at frame {taken}")
            }
            _ => "source and frame unavailable: this game's plugin does not report them".to_string(),
        }
    }
}

impl BevyDebuggerTools {
    /// Ask the game for one capture and wait for it (15 s at most).
    async fn capture_once(&self, params: serde_json::Map<String, Value>) -> Result<Captured, String> {
        let started = self.call_game(SCREENSHOT_METHOD, Some(Value::Object(params))).await?;
        let Some(id) = started.get("id").and_then(Value::as_u64) else {
            return Err(format!("{SCREENSHOT_METHOD} did not return a capture id: {started}"));
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let state = loop {
            let state = self.call_game(CAPTURE_METHOD, Some(serde_json::json!({"id": id}))).await?;
            match state.get("status").and_then(Value::as_str) {
                Some("done") => break state,
                Some("failed") => {
                    return Err(state.get("error").and_then(Value::as_str).unwrap_or("the capture failed").to_string());
                }
                Some("pending") => {}
                _ => return Err(format!("{CAPTURE_METHOD} returned an unexpected reply: {state}")),
            }
            if Instant::now() >= deadline {
                return Err(format!("capture {id} still pending after 15 s"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        let dimension = |key: &str| {
            state.get(key).and_then(Value::as_u64).ok_or_else(|| format!("{CAPTURE_METHOD} returned no {key}"))
        };
        let (width, height) = (dimension("width")?, dimension("height")?);
        let Some(png_base64) = state.get("png_base64").and_then(Value::as_str).map(str::to_owned) else {
            return Err(format!("{CAPTURE_METHOD} returned no png_base64"));
        };
        let png = base64::engine::general_purpose::STANDARD
            .decode(&png_base64)
            .map_err(|e| format!("the capture was not valid base64: {e}"))?;
        Ok(Captured {
            width,
            height,
            png_base64,
            png,
            source: state.get("source").and_then(Value::as_str).map(str::to_owned),
            frame_requested: state.get("frame_requested").and_then(Value::as_u64),
            frame_captured: state.get("frame_captured").and_then(Value::as_u64),
        })
    }
}

impl BevyDebuggerTools {
    /// Read `method` between two reads of the game's frame counter, in one HTTP round trip. The
    /// game serves a batch's requests one after another, each possibly on a later frame, so the
    /// value is known only to lie between the two frames. Returns `(frame_before, frame_after, result)`.
    async fn sample_with_frame(&self, method: &str, params: Option<Value>) -> Result<(u64, u64, Value), String> {
        let requests = [
            BrpRequest { method: STATUS_METHOD.to_string(), id: Some(Value::from(1)), params: None },
            BrpRequest { method: method.to_string(), id: Some(Value::from(2)), params },
            BrpRequest { method: STATUS_METHOD.to_string(), id: Some(Value::from(3)), params: None },
        ];
        let mut client = self.brp_client.write().await;
        let answers = match client.send_batch(&requests).await {
            Ok(answers) => answers,
            Err(e) if client.is_connected() => {
                return Err(format!(
                    "bevy-fairy: the game at {} answered, but not with a batch reply this server can read: {e}",
                    client.url()
                ))
            }
            Err(e) => {
                debug!("BRP batch did not reach the game: {e}");
                return Err(no_game_text(client.url()));
            }
        };
        let mut values = Vec::with_capacity(3);
        for (request, answer) in requests.iter().zip(answers) {
            match answer.payload {
                BrpPayload::Result(value) => values.push(value),
                BrpPayload::Error(err) => return Err(crate::server::brp::refusal_text(&request.method, &err)),
            }
        }
        let after = values.pop().unwrap_or(Value::Null);
        let result = values.pop().unwrap_or(Value::Null);
        let before = values.pop().unwrap_or(Value::Null);
        let frame_of = |status: &Value| {
            status
                .get("frame")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{STATUS_METHOD} returned no frame counter: {status}"))
        };
        Ok((frame_of(&before)?, frame_of(&after)?, result))
    }
}

impl BevyDebuggerTools {
    /// One BRP call to the game. The `Err` is the text an agent reads: the no-game message when
    /// nothing answers, the game's own error message verbatim, or -- when the game answered with
    /// something this server cannot read as one JSON-RPC reply -- a message that says exactly that.
    async fn call_game(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let request = BrpRequest { method: method.to_string(), id: Some(Value::from(1)), params };
        let mut client = self.brp_client.write().await;
        match client.send_request(&request).await {
            Ok(response) => match response.payload {
                BrpPayload::Result(value) => Ok(value),
                BrpPayload::Error(err) => Err(crate::server::brp::refusal_text(method, &err)),
            },
            // The client marks itself connected once the game has answered with a success
            // status; a reply it then fails to read is still an answer, and "no game answering"
            // would send the agent off to restart a game that is running.
            Err(e) if client.is_connected() => Err(format!(
                "bevy-fairy: the game at {} answered `{method}`, but not with one JSON-RPC reply \
                 this server can read: {e}",
                client.url()
            )),
            Err(e) => {
                debug!("BRP call {method} did not reach the game: {e}");
                Err(no_game_text(client.url()))
            }
        }
    }

    async fn game_url(&self) -> String {
        self.brp_client.read().await.url().to_string()
    }

    /// Poll `bevy_fairy/status` until the game reports ready, for at most 120 s.
    async fn wait_until_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let status = self.call_game(STATUS_METHOD, None).await?;
            if status.get("ready").and_then(Value::as_bool) == Some(true) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("game not ready after 120 s".to_string());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Every command the game registered, plus `bevy_fairy/time`, with the built-ins last.
    async fn command_list(&self) -> Result<Vec<CommandEntry>, String> {
        let value = self.call_game(COMMANDS_METHOD, None).await?;
        let listing: CommandListing = serde_json::from_value(value)
            .map_err(|e| format!("{COMMANDS_METHOD} returned an unexpected reply: {e}"))?;
        let mut entries = listing.commands;
        if !entries.iter().any(|entry| entry.name == TIME_COMMAND) {
            entries.push(CommandEntry {
                name: TIME_COMMAND.to_string(),
                description: "Pause or slow game time (Time<Virtual>); returns the paused flag and the speed.".to_string(),
                params: "pause: bool (optional); speed: f32 (optional, must be > 0)".to_string(),
                example: serde_json::json!({"pause": true}),
            });
        }
        entries.sort_by(|a, b| {
            (a.name.starts_with("bevy_fairy/"), &a.name).cmp(&(b.name.starts_with("bevy_fairy/"), &b.name))
        });
        Ok(entries)
    }
}

/// **How an agent is expected to work with a human through this server.**
///
/// One string, served by this server, because it is a rule and a rule with two copies is two
/// rules.
///
/// It is here rather than in a doc comment because it is the one thing every agent reads before it
/// touches a tool, and the half worth spending words on is the half agents get wrong. `guide` is the
/// only tool in this server that involves a person, and the failure it invites is silence: post a
/// script, go quiet, and somebody is left in front of a card that will never move. That happened
/// during the tool's own development, to the person it was built for, who asked the reasonable
/// question -- "how am I supposed to tell you whether it worked? Am I supposed to come back to the
/// chat?" The answer is yes, and nothing anywhere said so.
pub const COLLABORATION_PROTOCOL: &str =
                "Debugging tools for a RUNNING Bevy game, over the Bevy Remote Protocol.\n\n\
                 DRIVING A GAME THAT REGISTERS ITS OWN COMMANDS: call `status` first (is the game up and \
                 ready?), then `commands` (what can be run, with each one's parameters and an example), then \
                 `run` for each step. `screenshot` saves a PNG on this machine, `scene save` writes the \
                 commands you ran to a replayable file, `brp` calls any Bevy Remote Protocol method, \
                 `watch` samples one over N frames (a value that stops, a speed), and \
                 `quit` closes the game.\n\n\
                 INSPECTING THE GAME: `observe` reads the game's entities from a short phrase `query`. \
                 `detect_anomaly` looks for anomalies in them. `hypothesis` tests a claim about the game \
                 -- it needs a `description` of the claim and a `success_condition` that decides it, or it \
                 cannot run. `experiment` changes the game (spawn, modify, delete) and needs the `actions` \
                 to carry out. `replay` records and replays what it holds.\n\n\
                 `detect_anomaly`, `hypothesis`, `experiment` and `replay` each take an `action` that picks \
                 what to do, and each has its own action names -- read them from the tool's schema. A tool \
                 asked for something it cannot do says so; it does not answer with a default.\n\n\
                 WORKING WITH THE PERSON AT THE KEYBOARD:\n\
                 `guide` is different from the rest -- it talks to the human, not the game. It \
                 draws one instruction at a time on their own window and watches for the state it \
                 asked for. Reach for it when you would otherwise type a numbered list into the \
                 chat and hope they map it onto an interface you cannot see: reproducing a bug, \
                 walking an acceptance test, or checking something only a person can look at.\n\n\
                 Three rules, all learned the hard way:\n\
                 - AFTER POSTING A SCRIPT, SAY SO IN CHAT. Nothing in the app gets their attention. \
                 An agent that posts and goes quiet has hidden the instructions rather than \
                 delivered them.\n\
                 - A step with `checkpoint: null` NEVER ADVANCES ON ITS OWN. It is the question a \
                 machine cannot answer, so ask it in the conversation, by name, and wait for a real \
                 answer before calling `skip`. The card promises them 'tell the agent'; you are the \
                 agent.\n\
                 - WHEN THEY SAY A STEP MADE NO SENSE, THAT IS THE FINDING. Do not work around it. \
                 Most people follow a confusing instruction rather than report it, so the ones you \
                 do get are worth more than a passing run.\n\n\
                 What comes back is k/n per step -- times passed over times attempted -- never a \
                 boolean, because 'it worked' is precisely the judgement people get wrong.\n\n\
                 CAPTURES: the default offscreen capture renders the game's own image and leaves out UI. \
                 `screenshot` with source \"window\" captures the game window with its UI included, without \
                 raising or focusing it. A covered or minimised window can give a stale frame, so use it only \
                 when the UI is what you need to see.";

#[tool_router]
impl BevyDebuggerTools {
    /// Read the game's entities with a phrase query
    #[tool(description = "Read the game's entities with a short phrase query and return them with their component values: every entity, one entity by id, the entities with or without a component, or the registered component types. `diff` compares a read with the previous `diff` read of the same query. Entities only: for a resource use `brp` with world.get_resources, and for a component's fields and their types use `brp` with registry.schema.")]
    pub async fn observe(&self, Parameters(req): Parameters<ObserveRequest>) -> Result<CallToolResult, McpError> {
        debug!("Executing observe query: {}", req.query);
        
        let arguments = serde_json::json!({
            "query": req.query,
            "diff": req.diff,
        });
        
        match observe::handle(arguments, self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Observe tool error: {}", e);
                tool_err(format!("Observe tool error: {}", e))
            }
        }
    }

    /// Guide a human through the running app
    #[tool(description = "Guide the HUMAN through something in the running app, and get back what actually happened. This is the only tool here that talks to the person rather than the game: you post a short script, the app draws ONE step at a time over their own window, and it advances itself when a named condition arrives. Use it to walk somebody through reproducing a bug, to hand over an acceptance test, or any time you would otherwise type a numbered list into the chat and hope they map it onto an interface you cannot see.\n\nHOW TO COLLABORATE (this part is not optional -- getting it wrong strands them):\n1. Post steps, then TELL THEM IN CHAT that the guide is up and to look at the app window. They will not know otherwise; nothing pops up to get their attention.\n2. Poll with {read:true}. When it answers waiting_on_a_person:true, that step has NO machine check and will never advance on its own. STOP AND ASK THEM IN CHAT, in words, naming the step. The card tells them 'tell the agent' -- that promise is yours to keep. Only after they answer, call {skip:true}.\n3. When they say a step made no sense or did not work, that is DATA, not a detour. Record it and fix the step or the app. The transcript is k/n per step -- passes over attempts -- because a boolean 'it worked' is the thing people get wrong.\n4. Never post a step whose checkpoint the app has not registered. The app refuses it by name and lists what would have worked, but the person is already standing there by then.\n\nWrite steps the way you would brief a colleague: say WHY before what (people ignore a suggestion whose reason they cannot see), two to four short imperatives, and always fill in recovery -- what to do when the step does not work is the field that makes this better than a list in a chat window.")]
    pub async fn guide(&self, Parameters(req): Parameters<GuideRequest>) -> Result<CallToolResult, McpError> {
        let mut arguments = serde_json::Map::new();
        if let Some(steps) = req.steps {
            arguments.insert("steps".to_string(), Value::Array(steps));
        }
        if req.read {
            arguments.insert("read".to_string(), Value::Bool(true));
        }
        if req.skip {
            arguments.insert("skip".to_string(), Value::Bool(true));
        }
        if req.clear {
            arguments.insert("clear".to_string(), Value::Bool(true));
        }
        if let Some(visible) = req.visible {
            arguments.insert("visible".to_string(), Value::Bool(visible));
        }
        debug!("Guide request: {} key(s)", arguments.len());

        match guide::handle(Value::Object(arguments), self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Guide tool error: {}", e);
                tool_err(format!("Guide tool error: {}", e))
            }
        }
    }

    /// Run controlled experiments on game state
    #[tool(description = "Run controlled experiments on your Bevy game: spawn, modify or delete entities and read back what changed. `action` picks the operation; `execute` carries the `actions` to run.")]
    pub async fn experiment(&self, Parameters(req): Parameters<ExperimentRequest>) -> Result<CallToolResult, McpError> {
        debug!("Running experiment action: {}", req.action);

        let arguments = match request_args(&req) {
            Ok(arguments) => arguments,
            Err(refusal) => return refusal,
        };

        match experiment::handle(arguments, self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Experiment tool error: {}", e);
                tool_err(format!("Experiment tool error: {}", e))
            }
        }
    }

    /// Test hypotheses about game behavior
    #[tool(description = "Test a claim about your game against the running game. `action` picks the operation; `test` needs a `description` of the claim and a `success_condition` that decides it.")]
    pub async fn hypothesis(&self, Parameters(req): Parameters<HypothesisRequest>) -> Result<CallToolResult, McpError> {
        debug!("Testing hypothesis action: {}", req.action);

        let arguments = match request_args(&req) {
            Ok(arguments) => arguments,
            Err(refusal) => return refusal,
        };

        match hypothesis::handle(arguments, self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Hypothesis tool error: {}", e);
                tool_err(format!("Hypothesis tool error: {}", e))
            }
        }
    }

    /// Detect anomalies in game behavior
    #[tool(description = "Look for anomalies in the game's entities. `action` picks the operation; `detect` reads every entity once and runs the detectors over them with the thresholds below. A detection is a lead to check, not a verdict.")]
    pub async fn detect_anomaly(&self, Parameters(req): Parameters<AnomalyRequest>) -> Result<CallToolResult, McpError> {
        debug!("Running anomaly action: {}", req.action);

        let arguments = match request_args(&req) {
            Ok(arguments) => arguments,
            Err(refusal) => return refusal,
        };

        match anomaly::handle(arguments, self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Anomaly detection error: {}", e);
                tool_err(format!("Anomaly detection error: {}", e))
            }
        }
    }

    /// Record and replay game state
    #[tool(description = "Record and replay game state for time-travel debugging. Capture game state at specific points and replay to understand how bugs occur.")]
    pub async fn replay(&self, Parameters(req): Parameters<ReplayRequest>) -> Result<CallToolResult, McpError> {
        info!("Replay action: {:?}", req.action);
        
        let arguments = match serde_json::to_value(&req) {
            Ok(Value::Object(mut map)) => {
                map.retain(|_, value| !value.is_null());
                Value::Object(map)
            }
            Ok(_) | Err(_) => return tool_err("replay arguments could not be encoded"),
        };
        
        match replay::handle(arguments, self.brp_client.clone()).await {
            Ok(result) => handler_result(result),
            Err(e) => {
                error!("Replay tool error: {}", e);
                tool_err(format!("Replay tool error: {}", e))
            }
        }
    }

    /// Report whether the game is answering and ready for commands
    #[tool(description = "Report whether the game is answering and ready for commands: its frame, how many commands it registered, where captures come from, and replay progress. The reply also carries the URL this server talks to. Call this first.")]
    pub async fn status(&self, Parameters(_req): Parameters<NoArgs>) -> Result<CallToolResult, McpError> {
        let url = self.game_url().await;
        match self.call_game(STATUS_METHOD, None).await {
            Ok(mut value) => {
                if let Some(fields) = value.as_object_mut() {
                    fields.insert("url".to_string(), Value::from(url));
                }
                tool_ok(value.to_string())
            }
            Err(text) => tool_err(text),
        }
    }

    /// List the commands the game registered
    #[tool(description = "List the commands the game registered, each with its parameters and a working example. Call this before `run`. The built-in bevy_fairy/time is listed last.")]
    pub async fn commands(&self, Parameters(_req): Parameters<NoArgs>) -> Result<CallToolResult, McpError> {
        match self.command_list().await {
            Ok(entries) => {
                let blocks: Vec<String> = entries.iter().map(command_block).collect();
                tool_ok(blocks.join("\n\n"))
            }
            Err(text) => tool_err(text),
        }
    }

    /// Run one command the game registered
    #[tool(description = "Run one command the game registered, by name, with its parameters. Waits up to 120 s for the game to be ready. An unknown name is refused with the closest known one. Every command that runs is recorded, so `scene save` can write the run out as a replayable file.")]
    pub async fn run(&self, Parameters(req): Parameters<RunRequest>) -> Result<CallToolResult, McpError> {
        if let Err(text) = self.wait_until_ready().await {
            return tool_err(text);
        }
        let entries = match self.command_list().await {
            Ok(entries) => entries,
            Err(text) => return tool_err(text),
        };
        if !entries.iter().any(|entry| entry.name == req.command) {
            return tool_err(unknown_command_text(&req.command, &entries));
        }
        match self.call_game(&req.command, req.params).await {
            Ok(value) => tool_ok(value.to_string()),
            Err(text) => tool_err(text),
        }
    }

    /// Call any Bevy Remote Protocol method the game serves
    #[tool(description = "Call any Bevy Remote Protocol method the game serves, for example world.query or registry.schema, and return its result. Use `run` for the game's own commands. Watching methods (names ending in +watch) are refused: they stream, and this returns one answer per call.")]
    pub async fn brp(&self, Parameters(req): Parameters<BrpCallRequest>) -> Result<CallToolResult, McpError> {
        // A watching method answers with a stream that never ends; reading it as one reply timed
        // out and was reported as "no game answering". Say what it is and what to call instead.
        if let Some(one_shot) = req.method.strip_suffix("+watch") {
            return tool_err(format!(
                "`{}` is a watching method: the game streams a new answer every time the value \
                 changes, and `brp` returns one answer per call, so it cannot read it. Call \
                 `{one_shot}` again for a later value, or `watch` with method `{one_shot}` for a series over frames.",
                req.method
            ));
        }
        match self.call_game(&req.method, req.params).await {
            Ok(value) => tool_ok(value.to_string()),
            Err(text) => tool_err(text),
        }
    }

    /// Capture the game to a PNG file
    #[tool(description = "Capture the game to a PNG file on this machine and return its path, size, the source actually read (offscreen or window) and the game frames the request and the delivery happened on (compare with `status`'s frame to judge freshness). The default offscreen capture leaves out UI; source 'window' captures the game window with its UI included. Pass inline true to also get the image in the reply.")]
    pub async fn screenshot(&self, Parameters(req): Parameters<ScreenshotRequest>) -> Result<CallToolResult, McpError> {
        let mut params = serde_json::Map::new();
        if let Some(source) = req.source {
            params.insert("source".to_string(), Value::from(source));
        }
        if let Some(region) = req.region {
            params.insert(
                "region".to_string(),
                serde_json::json!({"x": region.x, "y": region.y, "width": region.width, "height": region.height}),
            );
        }
        if let Some(zoom) = req.zoom {
            params.insert("zoom".to_string(), Value::from(zoom));
        }
        let captured = match self.capture_once(params).await {
            Ok(captured) => captured,
            Err(text) => return tool_err(text),
        };
        let (width, height, png_base64, png) = (captured.width, captured.height, captured.png_base64.clone(), &captured.png);
        let path = req.save_to.map(PathBuf::from).unwrap_or_else(default_capture_path);
        if let Err(e) = write_file_creating_dirs(&path, png) {
            return tool_err(format!("could not write {}: {e}", path.display()));
        }
        let absolute = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        let provenance = captured.provenance();
        let mut content = vec![Content::text(format!("saved {} ({width}x{height}); {provenance}", absolute.display()))];
        if req.inline.unwrap_or(false) {
            content.push(Content::image(png_base64.to_string(), "image/png"));
        }
        Ok(CallToolResult::success(content))
    }

    /// Capture a short sequence as a labelled contact sheet
    #[tool(description = "Capture a short sequence of frames and return it as one contact sheet (a PNG, at most 2 rows x 4 columns), so a change over time can be seen: a sprite that stops moving, a flash, a transition. The first cell is a reference frame taken just before the sequence (labelled REF); the rest are `frames` captures at `fps` (default 4, 0.5-10), 1-7 frames. Every cell is labelled with the game frame the capture was delivered on (F<frame>) and the milliseconds since the reference was requested (+<ms>MS), both as reported, so two cells with the same F<frame> are the same game frame (the game is slower than `fps`). Each frame needs a round trip to the game, so the real spacing can exceed 1/fps: read the labels, not the request. Saves sheet.png and ref.png in a new directory (or full-resolution frame-<n>.png files too with full_frames true) and returns their paths. The default offscreen source leaves out UI; source 'window' includes it. Fails, saving nothing, when any capture fails or the frame size changes mid-sequence.")]
    pub async fn burst(&self, Parameters(req): Parameters<BurstRequest>) -> Result<CallToolResult, McpError> {
        let frames = req.frames.unwrap_or(4);
        if !(1..=7).contains(&frames) {
            return tool_err(format!("`frames` must be between 1 and 7 (the reference takes the eighth cell), got {frames}. Example: {{\"frames\": 6, \"fps\": 4}}"));
        }
        let fps = req.fps.unwrap_or(4.0);
        if !(0.5..=10.0).contains(&fps) {
            return tool_err(format!("`fps` must be between 0.5 and 10, got {fps}. Example: {{\"frames\": 6, \"fps\": 4}}"));
        }
        let mut params = serde_json::Map::new();
        if let Some(source) = &req.source {
            params.insert("source".to_string(), Value::from(source.clone()));
        }
        let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
        let started = Instant::now();
        let mut shots: Vec<(Captured, u128)> = Vec::with_capacity(frames as usize + 1);
        for n in 0..=frames {
            if n > 0 {
                tokio::time::sleep_until((started + interval.mul_f64(f64::from(n))).into()).await;
            }
            match self.capture_once(params.clone()).await {
                Ok(captured) => shots.push((captured, started.elapsed().as_millis())),
                Err(text) => return tool_err(format!("burst stopped at capture {n} of {frames}, nothing saved: {text}")),
            }
        }

        let mut cells = Vec::with_capacity(shots.len());
        for (n, (shot, at_ms)) in shots.iter().enumerate() {
            let image = match image::load_from_memory_with_format(&shot.png, image::ImageFormat::Png) {
                Ok(image) => image.to_rgba8(),
                Err(e) => return tool_err(format!("capture {n} was not a readable PNG: {e}")),
            };
            let frame = shot.frame_captured.map_or_else(|| "-".to_string(), |f| f.to_string());
            let label = if n == 0 { format!("REF F{frame}") } else { format!("F{frame} +{at_ms}MS") };
            cells.push(crate::server::contact_sheet::Cell { image, label });
        }
        let sheet = match crate::server::contact_sheet::compose(&cells) {
            Ok(sheet) => sheet,
            Err(text) => return tool_err(format!("{text}; nothing saved")),
        };
        let mut sheet_png = Vec::new();
        if let Err(e) = sheet.write_to(&mut std::io::Cursor::new(&mut sheet_png), image::ImageFormat::Png) {
            return tool_err(format!("could not encode the contact sheet: {e}"));
        }

        let dir = req.save_dir.map(PathBuf::from).unwrap_or_else(default_burst_dir);
        let mut files: Vec<(PathBuf, &[u8])> = vec![(dir.join("sheet.png"), &sheet_png), (dir.join("ref.png"), &shots[0].0.png)];
        let frame_names: Vec<PathBuf> = (1..shots.len()).map(|n| dir.join(format!("frame-{n}.png"))).collect();
        if req.full_frames.unwrap_or(false) {
            for (name, (shot, _)) in frame_names.iter().zip(shots.iter().skip(1)) {
                files.push((name.clone(), &shot.png));
            }
        }
        for (path, bytes) in &files {
            if let Err(e) = write_file_creating_dirs(path, bytes) {
                return tool_err(format!("could not write {}: {e}", path.display()));
            }
        }

        let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        let mut lines = vec![format!(
            "saved contact sheet {} ({}x{}) and reference {}",
            absolute(&files[0].0).display(),
            sheet.width(),
            sheet.height(),
            absolute(&files[1].0).display()
        )];
        if req.full_frames.unwrap_or(false) {
            lines.push(format!("full-resolution frames: {}", frame_names.iter().map(|p| absolute(p).display().to_string()).collect::<Vec<_>>().join(", ")));
        }
        for (n, (shot, at_ms)) in shots.iter().enumerate() {
            let name = if n == 0 { "ref".to_string() } else { format!("frame {n}") };
            lines.push(format!("{name}: +{at_ms} ms, {}", shot.provenance()));
        }
        let distinct: std::collections::BTreeSet<u64> = shots.iter().filter_map(|(s, _)| s.frame_captured).collect();
        lines.push(format!("{} distinct game frames across {} captures", distinct.len(), shots.len()));
        let mut content = vec![Content::text(lines.join("\n"))];
        if req.inline.unwrap_or(false) {
            content.push(Content::image(base64::engine::general_purpose::STANDARD.encode(&sheet_png), "image/png"));
        }
        Ok(CallToolResult::success(content))
    }

    /// Save, show or clear the recorded commands
    #[tool(description = "Work with the commands recorded in this game session. save writes them to a replayable scene file on this machine (path required); show returns them; clear forgets them.")]
    pub async fn scene(&self, Parameters(req): Parameters<SceneRequest>) -> Result<CallToolResult, McpError> {
        match req.action {
            SceneAction::Save => {
                let Some(path) = req.path else {
                    return tool_err("scene save needs a path");
                };
                let scene = match self.call_game(SCENE_METHOD, Some(serde_json::json!({"action": "get"}))).await {
                    Ok(value) => value,
                    Err(text) => return tool_err(text),
                };
                let steps = scene.get("steps").and_then(Value::as_array).map_or(0, Vec::len);
                let text = match serde_json::to_string_pretty(&scene) {
                    Ok(text) => text,
                    Err(e) => return tool_err(format!("could not format the scene: {e}")),
                };
                let path = PathBuf::from(path);
                if let Err(e) = write_file_creating_dirs(&path, text.as_bytes()) {
                    return tool_err(format!("could not write {}: {e}", path.display()));
                }
                let absolute = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
                tool_ok(format!("saved {} ({steps} steps)", absolute.display()))
            }
            SceneAction::Show => {
                let scene = match self.call_game(SCENE_METHOD, Some(serde_json::json!({"action": "get"}))).await {
                    Ok(value) => value,
                    Err(text) => return tool_err(text),
                };
                let steps = scene.get("steps").cloned().unwrap_or_else(|| Value::Array(Vec::new()));
                tool_ok(steps.to_string())
            }
            SceneAction::Clear => match self.call_game(SCENE_METHOD, Some(serde_json::json!({"action": "clear"}))).await {
                Ok(value) => tool_ok(value.to_string()),
                Err(text) => tool_err(text),
            },
        }
    }

    /// Sample a BRP method over time
    #[tool(description = "Sample one BRP method repeatedly and return the series, each sample stamped with the game frames it was read between (\"frame\" to \"frame_until\", usually one apart or equal) and the milliseconds since the first sample. Use it for anything that happens over time: a value that stops changing, a position's speed, a counter that drifts. The reply says whether the value changed, how many distinct values were seen, the frame of the first change, and \"constant_since_frame\" (the frame from which the last value never changed, null when it changed at the last sample). Samples are polled over HTTP at `interval_ms` (20-5000), so changes between samples are not seen; 2-100 samples, 30 s in total at most. Pass `pointer` (JSON pointer into each result) to keep samples small; a pointer that matches nothing is an error. Streaming `+watch` methods are refused.")]
    pub async fn watch(&self, Parameters(req): Parameters<WatchRequest>) -> Result<CallToolResult, McpError> {
        if let Some(one_shot) = req.method.strip_suffix("+watch") {
            return tool_err(format!(
                "`{}` is a streaming method; `watch` polls a one-shot method. Call \
                 `watch` with method `{one_shot}`.",
                req.method
            ));
        }
        let samples = req.samples.unwrap_or(10);
        if !(2..=100).contains(&samples) {
            return tool_err(format!("`samples` must be between 2 and 100, got {samples}. Example: {{\"method\": \"world.query\", \"samples\": 10}}"));
        }
        let interval_ms = req.interval_ms.unwrap_or(100);
        if !(20..=5000).contains(&interval_ms) {
            return tool_err(format!("`interval_ms` must be between 20 and 5000, got {interval_ms}."));
        }
        if u64::from(samples) * interval_ms > WATCH_MAX_TOTAL_MS {
            return tool_err(format!(
                "{samples} samples x {interval_ms} ms is {} s; the most `watch` runs is {} s. Lower `samples` or `interval_ms`.",
                u64::from(samples) * interval_ms / 1000,
                WATCH_MAX_TOTAL_MS / 1000
            ));
        }

        let started = Instant::now();
        let mut series: Vec<(u64, u64, u128, Value)> = Vec::with_capacity(samples as usize);
        let mut stopped: Option<String> = None;
        let mut reply_bytes = 0usize;
        for n in 0..samples {
            if n > 0 {
                tokio::time::sleep(Duration::from_millis(interval_ms)).await;
            }
            let (frame, frame_until, result) = match self.sample_with_frame(&req.method, req.params.clone()).await {
                Ok(pair) => pair,
                Err(text) if series.is_empty() => return tool_err(text),
                Err(text) => {
                    stopped = Some(text);
                    break;
                }
            };
            let value = match &req.pointer {
                None => result,
                Some(pointer) => match result.pointer(pointer) {
                    Some(value) => value.clone(),
                    None if series.is_empty() => {
                        return tool_err(format!(
                            "pointer `{pointer}` matches nothing in the result of `{}`; the result was: {}",
                            req.method,
                            truncate_for_error(&result)
                        ));
                    }
                    None => {
                        stopped = Some(format!("pointer `{pointer}` stopped matching at sample {}", n + 1));
                        break;
                    }
                },
            };
            reply_bytes += value.to_string().len();
            if reply_bytes > WATCH_MAX_REPLY_BYTES {
                return tool_err(format!(
                    "the series passed {WATCH_MAX_REPLY_BYTES} bytes after {n} samples. Pass `pointer` to pick the value to watch, or lower `samples`."
                ));
            }
            series.push((frame, frame_until, started.elapsed().as_millis(), value));
        }

        let first = &series[0].3;
        let first_change = series.iter().find(|(_, _, _, v)| v != first).map(|(frame, _, _, _)| *frame);
        let mut distinct: Vec<&Value> = Vec::new();
        for (_, _, _, value) in &series {
            if !distinct.contains(&value) {
                distinct.push(value);
            }
        }
        let last = &series[series.len() - 1].3;
        let run_start = series.iter().rposition(|(_, _, _, v)| v != last).map_or(0, |i| i + 1);
        let constant_since = (run_start + 1 < series.len()).then(|| series[run_start].0);
        let body = serde_json::json!({
            "method": req.method,
            "source": "polled over HTTP. Each sample is bracketed by two bevy_fairy/status reads: the value was read on a frame from `frame` to `frame_until`",
            "interval_ms": interval_ms,
            "samples_taken": series.len(),
            "changed": first_change.is_some(),
            "distinct_values": distinct.len(),
            "first_change_frame": first_change,
            "constant_since_frame": constant_since,
            "stopped_early": stopped,
            "samples": series
                .iter()
                .map(|(frame, frame_until, t_ms, value)| serde_json::json!({"frame": frame, "frame_until": frame_until, "t_ms": t_ms, "value": value}))
                .collect::<Vec<_>>(),
        });
        tool_ok(body.to_string())
    }

    /// Ask the game to quit
    #[tool(description = "Ask the game to quit cleanly.")]
    pub async fn quit(&self, Parameters(_req): Parameters<NoArgs>) -> Result<CallToolResult, McpError> {
        match self.call_game(QUIT_METHOD, None).await {
            Ok(value) => tool_ok(value.to_string()),
            Err(text) => tool_err(text),
        }
    }
}

// `#[tool_handler]` is not used: the `list_tools` it generates returns the router's `HashMap`
// values, so the order changed on every launch. The two methods it would generate are written
// out below, and `list_tools` sorts by name.
impl ServerHandler for BevyDebuggerTools {
    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router.call(call).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut tools = self.tool_router.list_all();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ListToolsResult::with_all_items(tools))
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "bevy-fairy".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            // **The collaboration protocol lives here, because this is the one string every agent
            // reads before it touches a tool.** It was one sentence describing the server, which is
            // the least useful thing it could say: a tool list already describes the server.
            //
            // The half worth spending words on is the half agents get wrong. `guide` is the only
            // tool here that involves a person, and the failure it invites is silence -- post a
            // script, go quiet, and somebody is left in front of a card that will never move. That
            // happened during the tool's own development, to the person it was built for, who asked
            // the reasonable question: "how am I supposed to tell you whether it worked? Am I
            // supposed to come back to the chat?" The answer is yes, and nothing said so.
            instructions: Some(COLLABORATION_PROTOCOL.to_string()),
        }
    }
}