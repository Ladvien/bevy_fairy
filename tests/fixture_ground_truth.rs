//! Ground truth: the server's tools, checked against a game whose state is known.
//!
//! The game is `examples/fixture_game.rs`, run in-process on a free port. Every expected value
//! comes from that file's constants -- [`CRATE_WEIGHTS`], [`SCORE_POINTS`] -- and never from the
//! tool under test. The tools are called the way an agent's call arrives: as JSON, deserialised
//! into the tool's own request struct, over a real BRP connection to a real Bevy app.

#[path = "../examples/fixture_game.rs"]
mod fixture_game;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::mcp_tools::{
    BevyDebuggerTools, BrpCallRequest, GuideRequest, NoArgs, ScreenshotRequest, WatchRequest,
};
use fixture_game::{CRATE_WEIGHTS, SCORE_POINTS};
use rmcp::handler::server::tool::Parameters;
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use tokio::sync::RwLock;

/// A port that was free a moment ago. The listener is dropped before the fixture binds, so two
/// tests can race for the same port; if that happens the loser's fixture fails to bind and the
/// test talks to the winner's. The fixtures are identical and nothing here writes to them, so
/// the assertions stay honest -- but the race is real, and this is not a guarantee.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the ephemeral port")
        .port()
}

/// Start the fixture game on its own thread. It runs until the test process exits.
fn start_fixture() -> u16 {
    let port = free_port();
    std::thread::spawn(move || {
        fixture_game::build_app(port, fixture_game::DEFAULT_FPS).run();
    });
    port
}

fn tools_for(port: u16) -> BevyDebuggerTools {
    let config = Config {
        bevy_brp_host: "127.0.0.1".to_string(),
        bevy_brp_port: port,
        ..Config::default()
    };
    BevyDebuggerTools::new(Arc::new(RwLock::new(BrpClient::new(&config))))
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|item| item.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Call the `brp` tool the way an agent does, and parse the JSON it returns.
async fn brp(tools: &BevyDebuggerTools, request: Value) -> Value {
    let params: BrpCallRequest = serde_json::from_value(request).expect("the request deserialises");
    let result = tools.brp(Parameters(params)).await.expect("the brp tool returns a result");
    let text = text_of(&result);
    assert_ne!(
        result.is_error,
        Some(true),
        "the brp tool reported an error against the fixture: {text}"
    );
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("brp returned non-JSON ({e}): {text}"))
}

/// Wait until the `status` tool says the fixture is ready. This is a gate, not the `status`
/// check -- `ready` is set at once and any answer already has `frame >= 1`. The check is
/// [`status_reports_a_frame_count_that_advances`].
async fn wait_until_ready(tools: &BevyDebuggerTools) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let result = tools.status(Parameters(NoArgs {})).await.expect("status returns a result");
        let text = text_of(&result);
        if result.is_error != Some(true) {
            if let Ok(status) = serde_json::from_str::<Value>(&text) {
                if status.get("ready") == Some(&json!(true))
                    && status.get("frame").and_then(Value::as_u64).unwrap_or(0) > 0
                {
                    return status;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "the fixture never reported ready within 60 s; last status: {text}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The `brp` tool's `world.query` returns exactly the crates the fixture spawned, with exactly
/// their weights -- including the deliberately odd one.
#[tokio::test(flavor = "multi_thread")]
async fn brp_query_returns_exactly_the_crates_the_fixture_spawned() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let rows = brp(
        &tools,
        json!({
            "method": "world.query",
            "params": {"data": {"components": ["fixture_game::Crate"]}}
        }),
    )
    .await;
    let rows = rows.as_array().expect("world.query returns an array of rows");

    let mut weights: Vec<f64> = rows
        .iter()
        .map(|row| {
            row["components"]["fixture_game::Crate"]["weight"]
                .as_f64()
                .unwrap_or_else(|| panic!("a crate row has no weight: {row}"))
        })
        .collect();
    weights.sort_by(f64::total_cmp);

    let mut expected: Vec<f64> = CRATE_WEIGHTS.iter().map(|w| f64::from(*w)).collect();
    expected.sort_by(f64::total_cmp);

    assert_eq!(
        weights, expected,
        "the crates the tool reports must be the crates the fixture spawned"
    );
    assert!(
        weights.iter().any(|w| *w < 0.0),
        "the odd value (a negative weight) must come back as it is, not cleaned up"
    );
}

/// The `brp` tool's `world.get_resources` returns the resource's real value.
#[tokio::test(flavor = "multi_thread")]
async fn brp_get_resources_returns_the_fixtures_score() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let reply = brp(
        &tools,
        json!({
            "method": "world.get_resources",
            "params": {"resource": "fixture_game::Score"}
        }),
    )
    .await;

    assert_eq!(
        reply["value"]["points"].as_u64(),
        Some(u64::from(SCORE_POINTS)),
        "the score the tool reports must be the fixture's: {reply}"
    );
}

/// Read every row of one fixture component through the `brp` tool.
async fn rows_of(tools: &BevyDebuggerTools, component: &str) -> Vec<Value> {
    let rows = brp(
        tools,
        json!({"method": "world.query", "params": {"data": {"components": [component]}}}),
    )
    .await;
    rows.as_array().expect("world.query returns an array of rows").clone()
}

/// The planted bug is real, and the tool shows it: once the spinner has run more than
/// [`SPINNER_STOPS_AFTER_FRAMES`] frames, its angle is frozen at exactly that many steps while
/// its frame counter keeps climbing. An eval task's answer depends on this, so the fixture's
/// claim about its own bug is checked, not assumed.
#[tokio::test(flavor = "multi_thread")]
async fn the_planted_spinner_bug_is_visible_through_the_brp_tool() {
    use fixture_game::{SPINNER_STEP, SPINNER_STOPS_AFTER_FRAMES};

    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let deadline = Instant::now() + Duration::from_secs(60);
    let spinner = loop {
        let rows = rows_of(&tools, "fixture_game::Spinner").await;
        assert_eq!(rows.len(), 1, "the fixture spawns one spinner: {rows:?}");
        let spinner = rows[0]["components"]["fixture_game::Spinner"].clone();
        let frames = spinner["frames"].as_u64().expect("the spinner reports its frames");
        if frames > u64::from(SPINNER_STOPS_AFTER_FRAMES) + 10 {
            break spinner;
        }
        assert!(Instant::now() < deadline, "the spinner never passed its stop frame");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };

    let angle = spinner["angle"].as_f64().expect("the spinner reports its angle");
    let frozen = f64::from(SPINNER_STEP) * f64::from(SPINNER_STOPS_AFTER_FRAMES);
    assert!(
        (angle - frozen).abs() < 1e-3,
        "the angle must be frozen at {frozen} after the stop frame; the tool reports {angle} ({spinner})"
    );

    // And the freeze is a freeze, not a stale read: a later read shows the counter climbing
    // while the angle stays put.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let rows = rows_of(&tools, "fixture_game::Spinner").await;
    let later = &rows[0]["components"]["fixture_game::Spinner"];
    assert!(
        later["frames"].as_u64() > spinner["frames"].as_u64(),
        "the spinner's frame counter must keep climbing: {spinner} then {later}"
    );
    assert_eq!(
        later["angle"].as_f64(),
        Some(angle),
        "the spinner's angle must stay frozen while its counter climbs: {spinner} then {later}"
    );
}

/// The `status` tool reports the game's real frame count: two reads a moment apart must show
/// it advancing. A `status` that answered a fixed `{"ready": true, "frame": 1}` would pass a
/// readiness check and fail this one.
#[tokio::test(flavor = "multi_thread")]
async fn status_reports_a_frame_count_that_advances() {
    let port = start_fixture();
    let tools = tools_for(port);
    let first = wait_until_ready(&tools).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let result = tools.status(Parameters(NoArgs {})).await.expect("status returns a result");
    let second: Value = serde_json::from_str(&text_of(&result)).expect("status returns JSON");

    assert!(
        second["frame"].as_u64() > first["frame"].as_u64(),
        "a running game's frame count must advance between reads: {first} then {second}"
    );
}

/// The mover's x grows, and no faster than [`fixture_game::MOVER_SPEED`] allows. Game time
/// cannot outrun the wall clock, so between two reads the mover can travel at most
/// `MOVER_SPEED` times the wall-clock time that elapsed around them. A tool that returns a
/// stale snapshot fails the lower bound; one that scales or invents x fails the upper.
#[tokio::test(flavor = "multi_thread")]
async fn the_mover_travels_at_no_more_than_its_speed() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let x_now = |rows: Vec<Value>| -> f64 {
        assert_eq!(rows.len(), 1, "the fixture spawns one mover: {rows:?}");
        rows[0]["components"]["bevy_transform::components::transform::Transform"]["translation"][0]
            .as_f64()
            .unwrap_or_else(|| panic!("the mover's transform has no x: {}", rows[0]))
    };
    let request = json!({
        "method": "world.query",
        "params": {
            "data": {"components": ["bevy_transform::components::transform::Transform"]},
            "filter": {"with": ["fixture_game::Mover"]}
        }
    });
    let started = Instant::now();
    let first = x_now(brp(&tools, request.clone()).await.as_array().cloned().unwrap_or_default());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = x_now(brp(&tools, request).await.as_array().cloned().unwrap_or_default());
    let elapsed = started.elapsed().as_secs_f64();

    let travelled = second - first;
    // The first read is served on a frame that may have begun before `started`, so the game
    // time between the two reads can exceed the wall time by up to one frame -- and Bevy clamps
    // one frame's delta to `Time<Virtual>`'s default maximum of 250 ms. That bound is sound on
    // a stalled host, and still catches a tool that doubles x.
    let most = f64::from(fixture_game::MOVER_SPEED) * (elapsed + 0.25);
    assert!(
        travelled > 0.0,
        "a later read must show the mover further along +x: {first} then {second}"
    );
    assert!(
        travelled <= most,
        "the mover travelled {travelled} in {elapsed:.3} s of wall time; at {} units/s it can \
         travel at most {most:.3}",
        fixture_game::MOVER_SPEED
    );
}

/// The text an agent sees, whether the tool answered with an error result or an RPC error.
fn agent_visible_text(outcome: Result<CallToolResult, rmcp::Error>) -> String {
    match outcome {
        Ok(result) => text_of(&result),
        Err(error) => error.message.to_string(),
    }
}

/// The fixture is built with the plugin but without `plugin-guide`, so `bevy_fairy/guide` is
/// not registered. The error must name that cause. It used to tell the agent "the game needs
/// `--features debugger`; the editor has it on by default" -- a feature this crate does not
/// have, for an editor this crate does not ship.
// With `plugin-guide` on, the fixture registers the method and there is nothing to refuse.
#[cfg(not(feature = "plugin-guide"))]
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_guide_method_names_the_feature_that_registers_it() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let request: GuideRequest =
        serde_json::from_value(json!({"read": true})).expect("the request deserialises");
    let text = agent_visible_text(tools.guide(Parameters(request)).await);

    assert!(
        text.contains("plugin-guide"),
        "the error must name the `plugin-guide` feature that registers the method; got: {text}"
    );
    assert!(
        !text.contains("--features debugger") && !text.contains("editor"),
        "the error must not prescribe a feature or an editor this crate does not have; got: {text}"
    );
}

/// Same for capture: the fixture is built without `plugin-screenshot`. The error used to be
/// the bare "Method `bevy_fairy/screenshot` not found", which names no cause and no fix.
// With `plugin-screenshot` on, the fixture registers the method and there is nothing to refuse.
#[cfg(not(feature = "plugin-screenshot"))]
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_screenshot_method_names_the_feature_that_registers_it() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let request: ScreenshotRequest =
        serde_json::from_value(json!({})).expect("the request deserialises");
    let text = agent_visible_text(tools.screenshot(Parameters(request)).await);

    assert!(
        text.contains("plugin-screenshot"),
        "the error must name the `plugin-screenshot` feature that registers the method; got: {text}"
    );
}

/// A watching method (`world.get_components+watch`) streams a new answer every time the value
/// changes; `brp` returns one answer per call. It used to time out reading the stream and report
/// "no game answering" while the fixture was answering every other call -- an eval agent read that
/// and concluded the game was down. The refusal must say what the method is and what to call.
#[tokio::test(flavor = "multi_thread")]
async fn a_watching_method_through_brp_is_refused_for_what_it_is() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let request: BrpCallRequest = serde_json::from_value(json!({
        "method": "world.get_components+watch",
        "params": {"entity": 0, "components": ["fixture_game::Spinner"]},
    }))
    .expect("the request deserialises");
    let text = agent_visible_text(tools.brp(Parameters(request)).await);

    assert!(
        !text.contains("no game answering"),
        "the fixture is answering, so the refusal must not say it is not; got: {text}"
    );
    assert!(
        text.contains("+watch") && text.contains("world.get_components`"),
        "the refusal must name the watching method and the one-shot method to call instead; got: {text}"
    );
}

const SPINNER_ANGLE: &str = "/0/components/fixture_game::Spinner/angle";
const SPINNER_FRAMES: &str = "/0/components/fixture_game::Spinner/frames";

/// Call `watch` the way an agent does and parse its JSON.
async fn watch(tools: &BevyDebuggerTools, request: Value) -> Result<Value, String> {
    let params: WatchRequest = serde_json::from_value(request).map_err(|e| format!("schema: {e}"))?;
    let result = tools.watch(Parameters(params)).await.expect("watch returns a result");
    let text = text_of(&result);
    if result.is_error == Some(true) {
        return Err(text);
    }
    Ok(serde_json::from_str(&text).unwrap_or_else(|e| panic!("watch returned non-JSON ({e}): {text}")))
}

fn spinner_query() -> Value {
    json!({"data": {"components": ["fixture_game::Spinner"]}})
}

/// One call to `watch` finds the planted bug: the spinner's angle moves and then freezes at
/// exactly `SPINNER_STOPS_AFTER_FRAMES` steps. The series must say the value changed, and from
/// which frame it stopped changing -- the question an agent otherwise answers with dozens of
/// repeated `brp` polls.
#[tokio::test(flavor = "multi_thread")]
async fn watch_reports_where_the_planted_spinner_bug_stops_the_angle() {
    use fixture_game::{SPINNER_STEP, SPINNER_STOPS_AFTER_FRAMES};
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let deadline = Instant::now() + Duration::from_secs(90);
    let report = loop {
        let report = watch(
            &tools,
            json!({"method": "world.query", "params": spinner_query(), "pointer": SPINNER_ANGLE,
                   "samples": 20, "interval_ms": 200}),
        )
        .await
        .expect("watch succeeds against the fixture");
        if report["constant_since_frame"].is_u64() {
            break report;
        }
        assert!(Instant::now() < deadline, "the angle never froze: {report}");
    };
    let frozen = f64::from(SPINNER_STEP) * f64::from(SPINNER_STOPS_AFTER_FRAMES);
    let samples = report["samples"].as_array().expect("samples");
    let last = samples.last().expect("at least one sample");
    assert!((last["value"].as_f64().unwrap() - frozen).abs() < 1e-3, "frozen angle is {frozen}: {report}");
    assert!(report["distinct_values"].as_u64().unwrap() >= 1);
    // The sample at which the angle froze is a frame at or after the frame the fixture stops on.
    let since = report["constant_since_frame"].as_u64().unwrap();
    assert!(
        since >= u64::from(SPINNER_STOPS_AFTER_FRAMES) - 1,
        "the angle cannot freeze before frame {SPINNER_STOPS_AFTER_FRAMES}; reported {since}: {report}"
    );
}

/// Each sample's `frame`..`frame_until` brackets the frame its value was read on. The spinner
/// counts its own frames, so with the constant offset `c` between the two counters every sample
/// must satisfy `frame <= value - c <= frame_until` for one `c`. A stale frame read or a frame stamped once per series
/// breaks that within a few samples at 100 ms spacing. (bevy_remote serves a batch's requests
/// on separate frames, so the bracket -- not equality -- is the honest claim.)
#[tokio::test(flavor = "multi_thread")]
async fn watch_brackets_the_frame_every_sample_was_read_on() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let report = watch(
        &tools,
        json!({"method": "world.query", "params": spinner_query(), "pointer": SPINNER_FRAMES,
               "samples": 12, "interval_ms": 100}),
    )
    .await
    .expect("watch succeeds against the fixture");
    let samples = report["samples"].as_array().expect("samples");
    assert_eq!(samples.len(), 12, "{report}");
    // The counters differ by one constant `c` (the true one is unknown here). Sample i says
    // `from_i <= value_i - c <= until_i`, so `c` lies in [value_i - until_i, value_i - from_i] for
    // every i; the claim holds only if those intervals have a common point.
    let mut lowest = i64::MIN;
    let mut highest = i64::MAX;
    for s in samples {
        let (from, until) = (s["frame"].as_i64().unwrap(), s["frame_until"].as_i64().unwrap());
        let value = s["value"].as_i64().unwrap();
        assert!(from <= until, "bracket reversed: {s}");
        lowest = lowest.max(value - until);
        highest = highest.min(value - from);
    }
    assert!(lowest <= highest, "no constant offset fits every bracket ({lowest} > {highest}): {report}");
    let frames: Vec<u64> = samples.iter().map(|s| s["frame"].as_u64().unwrap()).collect();
    assert!(frames.windows(2).all(|w| w[0] < w[1]), "frames must increase: {frames:?}");
    assert_eq!(report["changed"], json!(true));
}

/// Refusals: a pointer that matches nothing is an error, not a series of nulls; a streaming
/// method and out-of-range counts are refused with the valid range.
#[tokio::test(flavor = "multi_thread")]
async fn watch_refuses_what_it_cannot_measure() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let miss = watch(
        &tools,
        json!({"method": "world.query", "params": spinner_query(), "pointer": "/0/components/nope", "samples": 2}),
    )
    .await
    .expect_err("a pointer matching nothing is an error");
    assert!(miss.contains("/0/components/nope"), "names the pointer: {miss}");

    let stream = watch(&tools, json!({"method": "world.get_components+watch"})).await.expect_err("refused");
    assert!(stream.contains("world.get_components"), "names the one-shot method: {stream}");

    let few = watch(&tools, json!({"method": "world.query", "samples": 1})).await.expect_err("refused");
    assert!(few.contains("2") && few.contains("100"), "states the valid range: {few}");

    let long = watch(&tools, json!({"method": "world.query", "samples": 100, "interval_ms": 5000}))
        .await
        .expect_err("refused");
    assert!(long.contains("30"), "states the time cap: {long}");
}

/// Every params example the `brp` tool's schema shows an agent is accepted by a real game. The
/// examples are read out of the advertised schema, not copied here, so an example that goes
/// stale (or an edit that drops them) fails this test. Examples that need an entity id are
/// skipped: the fixture's ids are not known to a description.
#[tokio::test(flavor = "multi_thread")]
async fn the_params_examples_in_the_brp_schema_are_accepted_by_the_game() {
    let schema = schemars::schema_for!(BrpCallRequest);
    let schema = serde_json::to_value(&schema).expect("the schema serialises");
    let description = schema["properties"]["params"]["description"]
        .as_str()
        .expect("the brp `params` field must describe its shapes for an agent");

    let mut examples: Vec<(String, Value)> = Vec::new();
    // schemars folds a doc comment's lines into one paragraph, so list items split on " - `".
    for item in description.split(" - `").skip(1) {
        let line = format!("`{item}");
        let mut spans = line.split('`').skip(1).step_by(2);
        let Some(method) = spans.next() else { continue };
        let Some(json_span) = spans.next().filter(|s| s.starts_with('{')) else { continue };
        if json_span.contains("entity") {
            continue;
        }
        let params: Value = serde_json::from_str(json_span)
            .unwrap_or_else(|e| panic!("the example for {method} is not JSON ({e}): {json_span}"));
        examples.push((method.to_string(), params));
    }
    let methods: Vec<&str> = examples.iter().map(|(m, _)| m.as_str()).collect();
    assert!(
        methods.contains(&"world.query") && methods.contains(&"registry.schema"),
        "the schema must show params for world.query and registry.schema; found {methods:?}"
    );

    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    for (method, params) in examples {
        // `brp` asserts the call was not an error.
        brp(&tools, json!({"method": method, "params": params})).await;
    }
}
