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
    BevyDebuggerTools, BrpCallRequest, GuideRequest, NoArgs, ObserveRequest, ScreenshotRequest, WatchRequest,
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

/// The entry of a watch reply's `fields` for one pointer.
fn field<'a>(report: &'a Value, pointer: &str) -> &'a Value {
    report["fields"]
        .as_array()
        .unwrap_or_else(|| panic!("the reply has no `fields` array: {report}"))
        .iter()
        .find(|f| f["pointer"] == pointer)
        .unwrap_or_else(|| panic!("`fields` does not list {pointer}: {report}"))
}

/// Watching the whole spinner, with no `pointer`, says which of its fields froze. The planted bug
/// stops `angle` while `frames` keeps counting, so the component as a whole differs at every
/// sample and its own `constant_since_frame` stays null: it cannot say that anything stopped.
/// An agent that watched the whole component read "changing" and then spent a dozen calls
/// finding the field. `fields` gives each value its own summary.
#[tokio::test(flavor = "multi_thread")]
async fn watch_without_a_pointer_says_which_spinner_field_froze() {
    use fixture_game::SPINNER_STOPS_AFTER_FRAMES;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let deadline = Instant::now() + Duration::from_secs(90);
    let report = loop {
        let report = watch(
            &tools,
            json!({"method": "world.query", "params": spinner_query(), "samples": 20, "interval_ms": 200}),
        )
        .await
        .expect("watch succeeds against the fixture");
        if field(&report, SPINNER_ANGLE)["constant_since_frame"].is_u64() {
            break report;
        }
        assert!(Instant::now() < deadline, "the angle never froze: {report}");
    };

    // The whole component still changes at the last sample, because the counter runs.
    assert_eq!(report["changed"], json!(true), "{report}");
    assert!(report["constant_since_frame"].is_null(), "the whole value never settles: {report}");

    let counter = field(&report, SPINNER_FRAMES);
    assert_eq!(counter["changed"], json!(true), "{report}");
    assert!(counter["constant_since_frame"].is_null(), "the counter is still counting: {report}");

    let angle = field(&report, SPINNER_ANGLE);
    let since = angle["constant_since_frame"].as_u64().unwrap();
    assert!(
        since >= u64::from(SPINNER_STOPS_AFTER_FRAMES) - 1,
        "the angle cannot freeze before frame {SPINNER_STOPS_AFTER_FRAMES}; reported {since}: {report}"
    );
    if angle["changed"] == json!(true) {
        // The window saw the angle turn and then stop: the frame it stopped on is the one the
        // fixture stops on, to within the frames one 200 ms interval spans.
        assert!(since <= u64::from(SPINNER_STOPS_AFTER_FRAMES) + 40, "froze at frame {since}: {report}");
        // A field that moved and then stopped is listed ahead of the one still counting.
        assert_eq!(report["fields"][0]["pointer"], json!(SPINNER_ANGLE), "{report}");
    }

    // Fields that changed are listed before fields that did not: the entity id never changes.
    let listed: Vec<&str> = report["fields"].as_array().unwrap().iter().map(|f| f["pointer"].as_str().unwrap()).collect();
    assert_eq!(listed.last(), Some(&"/0/entity"), "{listed:?}");
    assert_eq!(report["fields"].as_array().unwrap().len(), 3, "{report}");
    assert!(report.get("fields_omitted").is_none(), "{report}");
}

/// With a `pointer` the sampled value is the spinner object, and every field's pointer is still
/// a pointer into the method's whole result, so it can be handed straight back to `watch`.
#[tokio::test(flavor = "multi_thread")]
async fn watch_field_pointers_are_into_the_whole_result_and_can_be_passed_back() {
    const SPINNER: &str = "/0/components/fixture_game::Spinner";
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let report = watch(
        &tools,
        json!({"method": "world.query", "params": spinner_query(), "pointer": SPINNER, "samples": 4, "interval_ms": 100}),
    )
    .await
    .expect("watch succeeds against the fixture");
    let pointers: Vec<&str> = report["fields"].as_array().expect("fields").iter().map(|f| f["pointer"].as_str().unwrap()).collect();
    assert_eq!(pointers.len(), 2, "{report}");
    assert!(pointers.contains(&SPINNER_ANGLE) && pointers.contains(&SPINNER_FRAMES), "{pointers:?}");

    // Hand one of the listed pointers back, as an agent narrowing its watch would.
    let counter = pointers.iter().find(|p| **p == SPINNER_FRAMES).expect("listed");
    let again = watch(
        &tools,
        json!({"method": "world.query", "params": spinner_query(), "pointer": counter, "samples": 3, "interval_ms": 100}),
    )
    .await
    .expect("a field's pointer is accepted as `pointer`");
    assert_eq!(again["changed"], json!(true), "the frame counter moves: {again}");
    assert!(again.get("fields").is_none(), "a lone value has no breakdown: {again}");
}

/// A result with more values than `fields` lists is cut, and says how many it left out: a
/// silent cut would read as a complete breakdown. Every entity with no component asked for is a
/// row of two values (its id and an empty `components`), so the fixture's world is well over 40.
#[tokio::test(flavor = "multi_thread")]
async fn watch_caps_a_long_fields_list_and_counts_what_it_left_out() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let report = watch(
        &tools,
        json!({"method": "world.query", "params": {"data": {}}, "samples": 3, "interval_ms": 100}),
    )
    .await
    .expect("watch succeeds against the fixture");
    let listed = report["fields"].as_array().expect("fields").len();
    assert_eq!(listed, 40, "{listed} fields listed");
    let left_out = report["fields_omitted"].as_u64().unwrap_or_else(|| panic!("a cut list says what it cut: {report}"));
    // Every row holds an entity id and an empty `components`, so the rows number half the values.
    let rows = report["samples"][0]["value"].as_array().expect("rows").len() as u64;
    assert_eq!(left_out + 40, rows * 2, "{report}");
}

/// The score is 42 and nothing changes it, so a watch must say the value never changed and that
/// it has been constant since the first sample, and a value that is one number has no `fields`.
/// (The other `watch` tests only ever watch values that move.)
#[tokio::test(flavor = "multi_thread")]
async fn watch_says_a_value_that_never_moves_never_changed() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    for pointer in [Some("/value/points"), None] {
        let mut request = json!({"method": "world.get_resources", "params": {"resource": "fixture_game::Score"},
                                 "samples": 4, "interval_ms": 100});
        if let Some(pointer) = pointer {
            request["pointer"] = json!(pointer);
        }
        let report = watch(&tools, request).await.expect("watch succeeds against the fixture");
        assert_eq!(report["changed"], json!(false), "{report}");
        assert_eq!(report["distinct_values"], json!(1), "{report}");
        assert!(report["first_change_frame"].is_null(), "{report}");
        assert_eq!(report["constant_since_frame"], report["samples"][0]["frame"], "{report}");
        assert!(report.get("fields").is_none(), "one number has no breakdown: {report}");
        let last = &report["samples"][3]["value"];
        let points = if pointer.is_some() { last } else { &last["value"]["points"] };
        assert_eq!(points.as_u64(), Some(u64::from(SCORE_POINTS)), "{report}");
    }
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

/// Call `observe` the way an agent does and return the raw text and the error flag.
async fn observe(tools: &BevyDebuggerTools, query: &str) -> (String, bool) {
    let request = ObserveRequest { query: query.to_string(), diff: false };
    let result = tools.observe(Parameters(request)).await.expect("observe returns a result");
    (text_of(&result), result.is_error == Some(true))
}

/// `observe` read the game, not its own memory: the mover is further along +x on the second read
/// of the very same query. A 5-minute result cache answered the second read with the first
/// read's rows under a fresh timestamp.
#[tokio::test(flavor = "multi_thread")]
async fn observe_reads_the_live_game_every_time_it_is_asked() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let query = "find entities with component bevy_transform::components::transform::Transform, fixture_game::Mover";
    let x_of = |text: &str| -> f64 {
        let value: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
        value["result"][0]["components"]["bevy_transform::components::transform::Transform"]["translation"][0]
            .as_f64()
            .unwrap_or_else(|| panic!("no mover x in: {text}"))
    };
    let (first, failed) = observe(&tools, query).await;
    assert!(!failed, "{first}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let (second, failed) = observe(&tools, query).await;
    assert!(!failed, "{second}");
    assert!(
        x_of(&second) > x_of(&first),
        "the mover moves along +x, so a later read must be further along: {} then {}",
        x_of(&first),
        x_of(&second)
    );
}

/// A query that names a component the game does not register has no answer; `[]` would say
/// "no such entities" for what is really "no such component". The error names the type and the
/// registered one it is probably short for.
#[tokio::test(flavor = "multi_thread")]
async fn observe_refuses_a_component_name_the_game_does_not_register() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let (text, failed) = observe(&tools, "find entities with component Crate").await;
    assert!(failed, "an unregistered component must be an error, got: {text}");
    assert!(text.contains("Crate"), "the error names the bad type: {text}");
    assert!(text.contains("fixture_game::Crate"), "the error offers the registered path: {text}");

    let (text, failed) = observe(&tools, "find entities with component fixture_game::Crate").await;
    assert!(!failed, "the registered path is accepted: {text}");
}

/// Bevy's `Mesh3d` has a digit in its registered path (`bevy_mesh::components::Mesh3d`), and the
/// unregistered-name error tells an agent to use exactly such a path. The parser refused it
/// ("Unrecognized query pattern"), so every component of `Mesh3d`, `Camera3d` or `avian3d` was
/// unnameable (issue #14). The fixture registers `Mesh3d` and spawns none, so the true answers are
/// known: no entity has one, and every entity lacks one.
#[tokio::test(flavor = "multi_thread")]
async fn observe_names_a_registered_component_whose_path_has_a_digit() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    let rows_of = |text: &str| -> usize {
        let reply: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
        reply["result"].as_array().unwrap_or_else(|| panic!("no rows in: {reply}")).len()
    };

    let mesh3d = "bevy_mesh::components::Mesh3d";
    let (text, failed) = observe(&tools, &format!("find entities with component {mesh3d}")).await;
    assert!(!failed, "a registered path with a digit must parse: {text}");
    assert_eq!(rows_of(&text), 0, "the fixture spawns no mesh: {text}");

    let (all, failed) = observe(&tools, "list all entities").await;
    assert!(!failed, "{all}");
    let (without, failed) = observe(&tools, &format!("find entities without component {mesh3d}")).await;
    assert!(!failed, "{without}");
    assert_eq!(rows_of(&without), rows_of(&all), "every entity lacks a Mesh3d: {without}");

    // A generic path: `Time<Real>` is one resource, so exactly one entity holds it.
    let (text, failed) = observe(&tools, "find entities with component bevy_time::time::Time<bevy_time::real::Real>").await;
    assert!(!failed, "a registered generic path must parse: {text}");
    assert_eq!(rows_of(&text), 1, "one `Time<Real>` resource: {text}");
}

/// `find N entities with component Y` parsed N and dropped it, so `find 2 entities` answered all
/// six of the fixture's crates under an `entity_count` of 6 (B-079). The game's `world.query` has no
/// limit, so the server cuts the rows, and says how many matched so the cut is visible.
#[tokio::test(flavor = "multi_thread")]
async fn observe_find_n_entities_returns_n_and_says_how_many_matched() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let (text, failed) = observe(&tools, "find 2 entities with component fixture_game::Crate").await;
    assert!(!failed, "{text}");
    let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
    let rows = reply["result"].as_array().unwrap_or_else(|| panic!("no rows in: {reply}"));
    assert_eq!(rows.len(), 2, "two were asked for: {reply}");
    assert!(rows.iter().all(|row| row["components"].get("fixture_game::Crate").is_some()), "{reply}");
    assert_eq!(reply["metadata"]["entity_count"], 2, "{reply}");
    assert_eq!(reply["metadata"]["limit"], 2, "{reply}");
    assert_eq!(reply["metadata"]["matched"], CRATE_WEIGHTS.len(), "the fixture spawned every crate: {reply}");

    // A limit above the match count cuts nothing.
    let (text, failed) = observe(&tools, "find 50 entities with component fixture_game::Crate").await;
    assert!(!failed, "{text}");
    let reply: Value = serde_json::from_str(&text).expect("observe answers JSON");
    assert_eq!(reply["result"].as_array().map(Vec::len), Some(CRATE_WEIGHTS.len()), "{reply}");
    assert_eq!(reply["metadata"]["matched"], CRATE_WEIGHTS.len(), "{reply}");
}

/// `observe` with `reflection: true` answered type questions the server never put to the game. Every
/// component came back `is_reflected: false`, `type_category: "Value"` with no `fields`, and each
/// value's type was guessed from its JSON: the fixture's `Crate` is a reflected struct whose `weight`
/// is an `f32`, and it was reported as an unreflected value holding an `f64`. The server has no type
/// registry of its own, so the honest answer is no answer: the field is refused the way an agent's
/// call with an unknown field is, and the game's own types are a `brp` `registry.schema` call away.
#[tokio::test(flavor = "multi_thread")]
async fn observe_reports_no_type_information_it_did_not_read_from_the_game() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    let arguments = json!({"query": "find entities with component fixture_game::Crate", "reflection": true});
    match serde_json::from_value::<ObserveRequest>(arguments) {
        Err(refusal) => assert!(refusal.to_string().contains("reflection"), "the refusal names the field: {refusal}"),
        Ok(request) => {
            let result = tools.observe(Parameters(request)).await.expect("observe returns a result");
            let reply: Value = serde_json::from_str(&text_of(&result)).expect("observe answers JSON");
            // Compact, so the claims below are matched whatever the reply's layout.
            let reflection = serde_json::to_string(&reply["result"]["reflection_data"]).expect("re-serialise");
            assert!(!reflection.contains(r#""is_reflected":false"#), "Crate is a reflected component: {reflection}");
            assert!(!reflection.contains(r#""type_category":"Value""#), "Crate is a struct: {reflection}");
            assert!(!reflection.contains(r#""f64""#), "Crate.weight is an f32: {reflection}");
        }
    }
}

/// The mover's entity id, read from the game.
async fn mover_id(tools: &BevyDebuggerTools) -> u64 {
    let (text, failed) = observe(tools, "find entities with component fixture_game::Mover").await;
    assert!(!failed, "{text}");
    let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
    reply["result"][0]["entity"].as_u64().unwrap_or_else(|| panic!("no mover in: {text}"))
}

/// `observe` keeps one process-wide "previous `diff` read", so the tests that diff take turns: a
/// diff read by one test in between another's two reads would make the second one a new baseline.
static OBSERVE_DIFF_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Call `observe` with `diff: true`, as an agent does, and return the parsed reply.
async fn observe_diff(tools: &BevyDebuggerTools, query: &str) -> Value {
    let request = ObserveRequest { query: query.to_string(), diff: true };
    let result = tools.observe(Parameters(request)).await.expect("observe returns a result");
    let text = text_of(&result);
    assert!(result.is_error != Some(true), "{text}");
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"))
}

/// `diff: true` compares a read with the previous `diff: true` read. It compared each read with
/// itself -- the new read became "the previous one" before the comparison ran -- and answered "No
/// changes" while the mover moved. The mover moves along +x, so two reads of its `Transform` half a
/// second apart differ, and the change is the mover's: a `world.get_components` reply does not name
/// its entity, and the snapshot was once keyed on entity 0 (B-062).
#[tokio::test(flavor = "multi_thread")]
async fn observe_diff_reports_the_mover_moving_between_two_reads() {
    let _turn = OBSERVE_DIFF_TURN.lock().await;
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    let mover = mover_id(&tools).await;

    let transform = "bevy_transform::components::transform::Transform";
    let query = format!("show entity {mover} components {transform}");
    let first = observe_diff(&tools, &query).await;
    assert_eq!(first["diff"]["baseline"], true, "the first diff read has nothing to compare with: {first}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let second = observe_diff(&tools, &query).await;

    let changes = second["diff"]["changes"].as_array().unwrap_or_else(|| panic!("no diff in: {second}"));
    assert!(
        changes.iter().any(|c| c["entity_id"] == mover && c["component_type"] == transform),
        "the mover moved {} between the reads, so its Transform changed: {second}",
        fixture_game::MOVER_SPEED * 0.5
    );
    assert!(changes.iter().all(|c| c["entity_id"] == mover), "only the mover was read: {second}");
    // The summary counts what `changes` lists: one entity, the mover, had a component change. It
    // said `entities_modified: 0` beside `components_modified: 1`, because it counted only a change
    // kind the diff never produces (B-078).
    let summary = &second["diff"]["summary"];
    assert_eq!(summary["entities_modified"], 1, "the mover is the one entity that changed: {second}");
    assert_eq!(summary["components_modified"], changes.len(), "every change listed is a modified component: {second}");

    // A read of another query covers other components, so it is not compared with this one: the
    // mover's Name and Mover are outside the Transform-only query, not removed from the game.
    for query in [format!("show entity {mover}"), query] {
        let switched = observe_diff(&tools, &query).await;
        assert_eq!(switched["diff"]["baseline"], true, "a new query starts a new baseline: {switched}");
        assert!(switched["diff"].get("changes").is_none(), "nothing is compared across queries: {switched}");
    }
}

/// `find N` cuts the returned rows, but a diff must compare every match. The game answers
/// `world.query` in table order and a despawn moves the last row into the freed slot, so a diff of
/// the cut alone reported a crate that had been there all along as spawned (review of 1032672).
#[tokio::test(flavor = "multi_thread")]
async fn observe_diff_with_a_limit_reports_only_what_the_game_changed() {
    let _turn = OBSERVE_DIFF_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let query = "find 2 entities with component fixture_game::Crate";
    let first = observe_diff(&tools, query).await;
    assert_eq!(first["diff"]["baseline"], true, "{first}");
    let shown = first["result"][0]["entity"].as_u64().unwrap_or_else(|| panic!("no crate row in: {first}"));
    brp(&tools, json!({"method": "world.despawn_entity", "params": {"entity": shown}})).await;

    let second = observe_diff(&tools, query).await;
    let summary = &second["diff"]["summary"];
    assert_eq!(summary["entities_removed"], 1, "one crate was despawned: {second}");
    assert_eq!(summary["entities_added"], 0, "no crate was spawned: {second}");
    assert_eq!(second["metadata"]["matched"], CRATE_WEIGHTS.len() - 1, "{second}");
    assert_eq!(second["result"].as_array().map(Vec::len), Some(2), "the rows are still cut to 2: {second}");
}

/// `show entity N` shows that entity: the mover has a `Transform` and a `fixture_game::Mover`. It
/// asked `world.get_components` for no components and answered `{"components": {}}` with an
/// entity_count of 1, so every entity read as one with no components at all.
#[tokio::test(flavor = "multi_thread")]
async fn observe_show_entity_lists_the_entitys_components() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    let mover = mover_id(&tools).await;

    let (text, failed) = observe(&tools, &format!("show entity {mover}")).await;
    assert!(!failed, "{text}");
    let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
    let components = &reply["result"]["components"];
    assert!(components.get("fixture_game::Mover").is_some(), "the mover's own component: {text}");
    assert!(
        components.get("bevy_transform::components::transform::Transform").is_some(),
        "the mover's Transform: {text}"
    );
    // Every registered component is asked for, so the ones the mover lacks must not come back as
    // errors about the mover.
    let not_present = i64::from(bevy_remote::error_codes::COMPONENT_NOT_PRESENT);
    let errors = reply["result"]["errors"].as_object().unwrap_or_else(|| panic!("no errors map: {text}"));
    assert!(errors.values().all(|e| e["code"].as_i64() != Some(not_present)), "noise about absent components: {text}");

    // An entity the game does not have is an error, not an entity with nothing on it. The fixture
    // spawns a dozen entities; index 999999 is none of them.
    let absent = (1u64 << 32) | 999_999;
    let (text, failed) = observe(&tools, &format!("show entity {absent}")).await;
    assert!(failed, "an entity the game does not have must be an error: {text}");
}

/// The semantic phrases claimed selections the server never made. Their value thresholds were
/// never sent to the game (B-077), so `find stuck entities` would have listed every entity with a
/// velocity, and `find memory leaks` answered the fixture's mover -- its one entity with a
/// `Transform`, which leaks nothing -- as a leak candidate at "confidence" 0.6. They
/// are not query shapes any more: each is refused, and the refusal lists the shapes that are.
#[tokio::test(flavor = "multi_thread")]
async fn observe_refuses_the_semantic_phrases_it_never_evaluated() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;

    for query in [
        "find memory leaks",
        "find stuck entities",
        "show fast moving objects",
        "find overlapping colliders",
        "find inconsistent state",
        "find physics violations",
        "find stuck and fast entities",
    ] {
        let (text, failed) = observe(&tools, query).await;
        assert!(failed, "`{query}` was never evaluated, so it cannot be answered: {text}");
        assert!(!text.contains("semantic_analysis"), "`{query}` must not claim an analysis: {text}");
        assert!(text.contains("find entities with component"), "the refusal lists the shapes that work: {text}");
    }
}

/// `experiment` keeps one process-wide history, so the tests that read it take turns.
static EXPERIMENT_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Call the `experiment` tool the way an agent does and parse what it returns.
async fn experiment(tools: &BevyDebuggerTools, request: Value) -> Value {
    let params: bevy_fairy::server::mcp_tools::ExperimentRequest =
        serde_json::from_value(request).expect("the request deserialises");
    let result = tools
        .experiment(Parameters(params))
        .await
        .expect("the experiment tool returns a result");
    let text = text_of(&result);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("experiment returned non-JSON ({e}): {text}"))
}

/// The weight of every fixture crate, keyed by entity id.
async fn crate_weights(tools: &BevyDebuggerTools) -> std::collections::BTreeMap<u64, f64> {
    rows_of(tools, "fixture_game::Crate")
        .await
        .iter()
        .map(|row| {
            (
                row["entity"].as_u64().expect("a row names its entity"),
                row["components"]["fixture_game::Crate"]["weight"].as_f64().expect("a crate has a weight"),
            )
        })
        .collect()
}

/// `experiment` modify says `rollback_available: true` and records the original value, so `undo`
/// must put that value back. Before the fix the claim was a constant: the history entry had no
/// reverse action, `undo` answered "No reverse action available", and it had already popped the
/// entry, so a second `undo` found nothing to undo either.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_undo_restores_a_modified_component_when_it_claims_rollback() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;

    let before = crate_weights(&tools).await;
    let (&entity, &original) = before.iter().next().expect("the fixture spawns crates");
    assert!(CRATE_WEIGHTS.iter().any(|w| (f64::from(*w) - original).abs() < 1e-9));

    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{
            "type": "modify", "entity_id": entity,
            "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]
        }]}),
    )
    .await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");
    assert_eq!(crate_weights(&tools).await[&entity], 99.0, "the modify must reach the game");
    let claimed = run["results"][0]["rollback_available"] == json!(true);
    assert!(claimed, "the crate's original weight was readable, so the modify is reversible: {run}");

    let undo = experiment(&tools, json!({"action": "undo"})).await;
    let restored = crate_weights(&tools).await[&entity];
    if claimed {
        assert_eq!(undo["result"]["success"], json!(true), "rollback was claimed, undo said: {undo}");
        assert_eq!(restored, original, "undo must put the original weight back");
    } else {
        assert_eq!(undo["result"]["success"], json!(false), "{undo}");
    }

    let history = experiment(&tools, json!({"action": "history"})).await;
    if !claimed {
        assert_eq!(history["count"], json!(1), "a refused undo must not drop the entry: {history}");
    }
}

/// A delete cannot be put back (a respawned entity is a different entity), so it must not claim
/// `rollback_available`, and the failed `undo` must leave the entry in `history`.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_delete_does_not_claim_a_rollback_undo_cannot_perform() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;

    let entity = *crate_weights(&tools).await.keys().next().expect("the fixture spawns crates");
    let run = experiment(&tools, json!({"action": "execute", "actions": [{"type": "delete", "entity_id": entity}]})).await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");
    assert!(!crate_weights(&tools).await.contains_key(&entity), "the delete must reach the game");

    let undo = experiment(&tools, json!({"action": "undo"})).await;
    let claimed = run["results"][0]["rollback_available"] == json!(true);
    assert_eq!(
        undo["result"]["success"] == json!(true),
        claimed,
        "rollback_available={claimed} but undo said: {undo}"
    );
    if !claimed {
        let history = experiment(&tools, json!({"action": "history"})).await;
        assert_eq!(history["count"], json!(1), "a refused undo must not drop the entry: {history}");
    }
}

/// A spawn of one more crate, as an `experiment` action.
fn spawn_crate(weight: f64) -> Value {
    json!({"type": "spawn", "components": [{"type_id": "fixture_game::Crate", "value": {"weight": weight}}]})
}

/// A spawn the server refuses before it reaches the game: its component has no type.
fn refused_spawn() -> Value {
    json!({"type": "spawn", "components": [{"type_id": "", "value": 1}]})
}

/// An action the server refuses is a failed action, reported beside the ones that ran. Before the
/// fix the refusal was an `Err` that escaped the loop: the reply was one tool error, the spawn
/// that had already happened was not in it, and for an atomic batch the `?` skipped the
/// transaction's rollback and its pop, so the first spawn stayed in the game and no later action
/// in the process was recorded for `undo`.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_reports_a_refused_action_beside_the_ones_that_ran() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;
    let crates = CRATE_WEIGHTS.len();

    // Not atomic: the spawn before the refusal stays, and the reply says so.
    let run = experiment(&tools, json!({"action": "execute", "actions": [spawn_crate(9.0), refused_spawn()]})).await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");
    assert!(run["results"][0]["entity_id"].is_u64(), "the spawned entity is named: {run}");
    assert_eq!(run["results"][1]["success"], json!(false), "{run}");
    assert!(run["results"][1]["error"].as_str().is_some_and(|e| e.contains("empty")), "{run}");
    assert_eq!(crate_weights(&tools).await.len(), crates + 1, "the first spawn reached the game");
    experiment(&tools, json!({"action": "undo"})).await;
    assert_eq!(crate_weights(&tools).await.len(), crates, "and it was recorded, so undo removes it");

    // Atomic: the refusal rolls the whole batch back.
    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{"type": "batch", "atomic": true, "actions": [spawn_crate(9.0), refused_spawn()]}]}),
    )
    .await;
    assert_eq!(run["results"][0]["success"], json!(false), "{run}");
    assert_eq!(crate_weights(&tools).await.len(), crates, "the atomic batch must leave no spawn behind: {run}");

    // And history still works afterwards: an open transaction would have disabled it.
    let entity = *crate_weights(&tools).await.keys().next().expect("the fixture spawns crates");
    let original = crate_weights(&tools).await[&entity];
    let modify = json!({"type": "modify", "entity_id": entity,
        "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]});
    experiment(&tools, json!({"action": "execute", "actions": [modify]})).await;
    let undo = experiment(&tools, json!({"action": "undo"})).await;
    assert_eq!(undo["result"]["success"], json!(true), "{undo}");
    assert_eq!(crate_weights(&tools).await[&entity], original, "undo must still restore after a failed batch");
}

/// A batch that succeeds reports what each action did (the spawned entities are the only way to
/// target them afterwards), and `undo` steps back through the actions it ran. Before the fix the
/// reply was "Successfully executed all 2 actions" with no entity, and the batch itself was the
/// history entry: a batch has no reverse, so `undo` answered "`batch` cannot be undone" forever
/// and every earlier entry was out of reach.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_batch_names_what_it_spawned_and_can_be_undone() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;
    let before = crate_weights(&tools).await;

    for atomic in [true, false] {
        let run = experiment(
            &tools,
            json!({"action": "execute", "actions": [{"type": "batch", "atomic": atomic, "actions": [spawn_crate(8.0), spawn_crate(8.5)]}]}),
        )
        .await;
        assert_eq!(run["results"][0]["success"], json!(true), "atomic {atomic}: {run}");
        let inner = run["results"][0]["results"].as_array().unwrap_or_else(|| panic!("the batch lists its actions: {run}"));
        let named: Vec<u64> = inner.iter().filter_map(|r| r["entity_id"].as_u64()).collect();
        assert_eq!(named.len(), 2, "each spawn names its entity: {run}");
        let now = crate_weights(&tools).await;
        assert!(named.iter().all(|id| now.contains_key(id)), "the named entities are in the game: {run}");

        for _ in 0..2 {
            let undo = experiment(&tools, json!({"action": "undo"})).await;
            assert_eq!(undo["result"]["success"], json!(true), "atomic {atomic}: {undo}");
        }
        assert_eq!(crate_weights(&tools).await, before, "atomic {atomic}: two undos remove the two spawns");
    }
}

/// `experiment` advertised four archetypes (player, enemy, projectile, static_mesh) and could
/// spawn them, but each was a hard-coded component list in short names (`Transform`, `Health`)
/// and `{x, y, z}` objects. BRP wants full type paths and arrays, and no game is guaranteed to
/// have a `Health`, so no spawn from the list could work against any game: the listing was
/// false. The action is gone. Fields the server does not read are refused rather than dropped.
/// A `path` on a component, which nothing reads, used to be dropped without a word and the whole
/// component overwritten.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_offers_no_archetypes_and_refuses_fields_it_does_not_read() {
    use bevy_fairy::server::mcp_tools::ExperimentRequest;
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let reply = experiment(&tools, json!({"action": "archetypes"})).await;
    assert_eq!(reply["error"], json!("Unknown action"), "archetypes must not be an action: {reply}");
    let valid = reply["available_actions"].as_array().unwrap_or_else(|| panic!("the refusal lists the actions: {reply}"));
    assert!(valid.contains(&json!("execute")) && !valid.contains(&json!("archetypes")), "{reply}");

    for (request, field) in [
        (json!({"action": "execute", "actions": [{"type": "spawn", "archetype": "player", "components": []}]}), "archetype"),
        (
            json!({"action": "execute", "actions": [{"type": "modify", "entity_id": 1,
                "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 5}, "path": "weight"}]}]}),
            "path",
        ),
        (json!({"action": "execute", "actions": [{"type": "batch", "atomic": true, "atomc": true, "actions": []}]}), "atomc"),
    ] {
        let error = serde_json::from_value::<ExperimentRequest>(request.clone())
            .expect_err(&format!("{request} carries a field nothing reads, so it must be refused"))
            .to_string();
        assert!(error.contains(field), "the refusal names `{field}`: {error}");
    }
}

/// An atomic batch rolls back what it ran. Rollback skipped every failed result without looking
/// inside it, so a batch inside an atomic batch that half-succeeded kept its spawn in the game
/// while the reply said "rolled back", and nothing recorded it for `undo`. A batch inside a batch
/// is refused, before anything runs.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_refuses_a_batch_inside_a_batch_instead_of_half_reversing_it() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;
    let before = crate_weights(&tools).await;

    for outer_atomic in [true, false] {
        let inner = json!({"type": "batch", "actions": [spawn_crate(7.0), {"type": "delete", "entity_id": 999_999}]});
        let run = experiment(
            &tools,
            json!({"action": "execute", "actions": [{"type": "batch", "atomic": outer_atomic, "actions": [spawn_crate(6.0), inner]}]}),
        )
        .await;
        assert_eq!(run["results"][0]["success"], json!(false), "outer atomic {outer_atomic}: {run}");
        let error = run["results"][0]["error"].as_str().unwrap_or_else(|| panic!("no error: {run}"));
        assert!(error.contains("batch cannot contain a batch") && error.contains("Nothing was run"), "{error}");
        assert_eq!(crate_weights(&tools).await, before, "outer atomic {outer_atomic}: nothing may have run");
    }
    let history = experiment(&tools, json!({"action": "history"})).await;
    assert_eq!(history["count"], json!(0), "nothing ran, so nothing is recorded: {history}");
}

/// `modify` says what the game holds after the write. The game's answer to the write is "accepted",
/// and `Successfully modified` after it was all the reply said, so a value that a game system sets
/// again on the next frame still read as a success. The reply now carries a read-back, taken after
/// the write. The spinner's `frames` counts up every frame, so a write of 1000000 cannot still be
/// there when it is read; a crate's weight is not touched by any system, so a write of 99 is.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_modify_reads_the_entity_back_and_says_when_it_does_not_hold_the_value() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;

    let crate_entity = *crate_weights(&tools).await.keys().next().expect("the fixture spawns crates");
    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{"type": "modify", "entity_id": crate_entity,
            "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]}]}),
    )
    .await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");
    assert_eq!(run["results"][0]["read_back"]["matches"], json!(true), "nothing changes a crate's weight: {run}");
    assert_eq!(run["results"][0]["read_back"]["components"]["fixture_game::Crate"]["weight"], json!(99.0), "{run}");

    let spinner = rows_of(&tools, "fixture_game::Spinner").await;
    let spinner_entity = spinner[0]["entity"].as_u64().expect("a row names its entity");
    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{"type": "modify", "entity_id": spinner_entity,
            "components": [{"type_id": "fixture_game::Spinner", "value": {"angle": 0.0, "frames": 1_000_000}}]}]}),
    )
    .await;
    let result = &run["results"][0];
    assert_eq!(result["success"], json!(true), "the game accepted the write: {run}");
    assert_eq!(result["read_back"]["matches"], json!(false), "the spinner counts frames, so 1000000 is gone: {run}");
    let held = result["read_back"]["components"]["fixture_game::Spinner"]["frames"].as_u64();
    assert!(held.is_some_and(|frames| frames > 1_000_000), "the read-back carries what the game holds: {run}");
    let message = result["message"].as_str().expect("a message");
    assert!(message.contains("read back") && message.contains("accepted"), "the message says both halves: {message}");
}

/// `undo` and `redo` answer as what they are. The reply was the reverse operation's own result, so
/// undoing a spawn came back as `action_type: "delete"`. And a redo of a spawn makes a different
/// entity (a despawned id is not reused), so the entries behind it, which name the old one, would
/// have written to an entity that no longer exists; they follow the new one now.
#[tokio::test(flavor = "multi_thread")]
async fn experiment_undo_and_redo_say_what_they_did_and_redo_follows_a_respawned_entity() {
    let _turn = EXPERIMENT_TURN.lock().await;
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    experiment(&tools, json!({"action": "clear_history"})).await;
    let base = crate_weights(&tools).await;

    let run = experiment(&tools, json!({"action": "execute", "actions": [spawn_crate(9.0)]})).await;
    let spawned = run["results"][0]["entity_id"].as_u64().unwrap_or_else(|| panic!("no entity: {run}"));
    let run = experiment(
        &tools,
        json!({"action": "execute", "actions": [{"type": "modify", "entity_id": spawned,
            "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]}]}),
    )
    .await;
    assert_eq!(run["results"][0]["success"], json!(true), "{run}");

    for undone in ["modify", "spawn"] {
        let undo = experiment(&tools, json!({"action": "undo"})).await;
        assert_eq!(undo["result"]["success"], json!(true), "{undo}");
        assert_eq!(undo["result"]["action_type"], json!("undo"), "{undo}");
        assert_eq!(undo["result"]["of_action"], json!(undone), "{undo}");
    }
    assert_eq!(crate_weights(&tools).await, base, "both undone");

    let redo = experiment(&tools, json!({"action": "redo"})).await;
    assert_eq!(redo["result"]["success"], json!(true), "{redo}");
    assert_eq!(redo["result"]["action_type"], json!("redo"), "{redo}");
    assert_eq!(redo["result"]["of_action"], json!("spawn"), "{redo}");
    let respawned = redo["result"]["entity_id"].as_u64().unwrap_or_else(|| panic!("no entity: {redo}"));
    assert_ne!(respawned, spawned, "a despawned id is not reused, so the redo made a different entity");

    let redo = experiment(&tools, json!({"action": "redo"})).await;
    assert_eq!(redo["result"]["success"], json!(true), "the modify must follow the respawned entity: {redo}");
    assert_eq!(redo["result"]["of_action"], json!("modify"), "{redo}");
    let weights = crate_weights(&tools).await;
    assert_eq!(weights.len(), base.len() + 1, "{weights:?}");
    assert_eq!(weights.get(&respawned), Some(&99.0), "the redone modify reached the respawned crate: {weights:?}");
}

/// Call the `hypothesis` tool the way an agent does: the reply, and whether the tool called it an error.
async fn hypothesis(tools: &BevyDebuggerTools, request: Value) -> (Value, bool) {
    let params: bevy_fairy::server::mcp_tools::HypothesisRequest =
        serde_json::from_value(request).expect("the request deserialises");
    let result = tools
        .hypothesis(Parameters(params))
        .await
        .expect("the hypothesis tool returns a result");
    let text = text_of(&result);
    let failed = result.is_error == Some(true);
    // A refusal is plain text; anything else is the JSON reply.
    let reply = match serde_json::from_str(&text) {
        Ok(reply) => reply,
        Err(_) if failed => Value::String(text),
        Err(e) => panic!("hypothesis returned non-JSON ({e}): {text}"),
    };
    (reply, failed)
}

/// A claim is checked only when its actions ran. When the game refused an action, nothing was
/// checked, and the reply has to say so, with the game's own words. Before the fix every refusal
/// counted as a violation: the verdict was `REJECTED`, the success rate 0, and the only error the
/// agent got was the constant "Action execution failed", so a typo in a component path read as
/// "your claim is false".
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_whose_actions_the_game_refused_is_not_evaluated_not_rejected() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let entity = *crate_weights(&tools).await.keys().next().expect("the fixture spawns crates");

    let (reply, failed) = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "a crate still exists after the modify",
            "success_condition": "entity_exists:fixture_game::Crate",
            "actions": [{"type": "modify", "entity_id": entity,
                "components": [{"type_id": "fixture_game::NoSuchComponent", "value": {"weight": 1.0}}]}]
        }),
    )
    .await;

    assert!(!failed, "a claim that could not be checked is a result, not a tool failure: {reply}");
    assert_eq!(reply["verdict"], json!("NOT_EVALUATED"), "the condition was never read: {reply}");
    assert_eq!(reply["success_rate"], Value::Null, "no iteration evaluated the claim, so it has no rate: {reply}");
    let stats = &reply["statistics"];
    assert_eq!(
        (stats["held"].as_u64(), stats["violated"].as_u64()),
        (Some(0), Some(0)),
        "an iteration that never evaluated the claim neither held nor violated it: {reply}"
    );
    assert_eq!(stats["not_evaluated"], stats["iterations_run"], "every iteration failed before the condition: {reply}");
    let error = reply["examples"][0]["error"].as_str().unwrap_or_else(|| panic!("no example error: {reply}"));
    assert!(error.contains("NoSuchComponent"), "the example must carry the game's own message, not a constant: {error}");
    assert_eq!(reply["examples"][0]["kind"], json!("not_evaluated"), "{reply}");
}

/// Zero iterations evaluated nothing. Before the fix `iterations: 0` under a varied strategy
/// answered `REJECTED` (rate 0 of 0), and with `threshold: 0` it answered `CONFIRMED`.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_that_ran_no_iterations_is_not_evaluated() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    for threshold in [0.95, 0.0] {
        let (reply, failed) = hypothesis(
            &tools,
            json!({
                "action": "test",
                "description": "there is a crate",
                "success_condition": "entity_exists:fixture_game::Crate",
                "variation_type": "random",
                "iterations": 0,
                "threshold": threshold
            }),
        )
        .await;
        assert!(failed || reply["verdict"] == json!("NOT_EVALUATED"), "threshold {threshold}: zero iterations evaluated nothing: {reply}");
    }
}

/// The control for the two tests above: a claim the fixture makes true is confirmed, and one it
/// makes false is rejected, so `NOT_EVALUATED` is not simply what every answer became. The
/// fixture spawns exactly `CRATE_WEIGHTS.len()` crates.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_about_the_crate_count_is_confirmed_or_rejected_by_the_fixture() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    for (count, verdict) in [(CRATE_WEIGHTS.len(), "CONFIRMED"), (CRATE_WEIGHTS.len() + 1, "REJECTED")] {
        let (reply, failed) = hypothesis(
            &tools,
            json!({
                "action": "test",
                "description": "the fixture has this many crates",
                "success_condition": {"type": "entity_exists", "component_types": ["fixture_game::Crate"], "count": count}
            }),
        )
        .await;
        assert!(!failed, "{reply}");
        assert_eq!(reply["verdict"], json!(verdict), "{count} crates: {reply}");
    }
}

/// One iteration cannot support certainty. The interval was a Wald interval, whose width is zero
/// at a rate of 0 or 1 for any number of trials, so a claim read once came back as
/// `confidence_interval: [1.0, 1.0]` and `confidence_level: "100.0%"`; the level was also a
/// linear remap of the interval's bound, not a probability. The Wilson interval of 1 of 1 at 95%
/// is about [0.21, 1.0], and the reply carries no `confidence_level`.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_read_once_does_not_claim_certainty() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let (reply, failed) = hypothesis(
        &tools,
        json!({"action": "test", "description": "there is a crate", "success_condition": "entity_exists:fixture_game::Crate"}),
    )
    .await;
    assert!(!failed, "{reply}");
    assert_eq!(reply["statistics"]["iterations_run"], json!(1), "no variation is one iteration: {reply}");
    let lower = reply["confidence_interval"]["lower"].as_f64().unwrap_or_else(|| panic!("no interval: {reply}"));
    assert!((0.2..0.22).contains(&lower), "Wilson 95% lower bound of 1 of 1 is 0.2065, not {lower}: {reply}");
    assert_eq!(reply["confidence_interval"]["upper"], json!(1.0), "{reply}");
    assert!(reply.get("confidence_level").is_none(), "a made-up percentage must not be reported: {reply}");
}

/// `threshold` is a share of iterations, so a value outside 0..=1 can never be met (or is always
/// met). It is refused before the actions run, because they change the live game.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_refuses_a_threshold_outside_zero_to_one_before_touching_the_game() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let before = crate_weights(&tools).await;
    let entity = *before.keys().next().expect("the fixture spawns crates");

    for threshold in [1.5, -0.1, 0.0] {
        let (reply, failed) = hypothesis(
            &tools,
            json!({
                "action": "test",
                "description": "a crate weighs 99 after the modify",
                "success_condition": "entity_exists:fixture_game::Crate",
                "actions": [{"type": "modify", "entity_id": entity,
                    "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]}],
                "threshold": threshold
            }),
        )
        .await;
        assert!(failed, "threshold {threshold} must be refused: {reply}");
        assert!(reply.to_string().contains("threshold"), "the refusal names the argument: {reply}");
    }
    assert_eq!(crate_weights(&tools).await, before, "a refused request must not have changed the game");
}

/// The variation generators call `random_range` on a range the caller chose, and it panics on an
/// empty one: `numeric_min` at or above `numeric_max`, or `max_mutations` of 0. A panic in the
/// handler is a tool call that never answers. Each is refused naming the argument, before any
/// action runs.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_refuses_variation_settings_that_would_panic_the_generator() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let entity = *crate_weights(&tools).await.keys().next().expect("the fixture spawns crates");

    for (settings, argument) in [
        (json!({"variation_type": "random", "numeric_min": 5.0, "numeric_max": 5.0}), "numeric_max"),
        (json!({"variation_type": "random", "numeric_min": 9.0, "numeric_max": 1.0}), "numeric_max"),
        (json!({"variation_type": "random", "bool_probability": -1.0}), "bool_probability"),
        (json!({"variation_type": "fuzz", "max_mutations": 0}), "max_mutations"),
        (json!({"variation_type": "fuzz", "mutation_probability": 1.5}), "mutation_probability"),
    ] {
        let mut request = json!({
            "action": "test",
            "description": "a crate weighs something",
            "success_condition": "entity_exists:fixture_game::Crate",
            "actions": [{"type": "modify", "entity_id": entity,
                "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 99.0}}]}],
            "iterations": 3
        });
        request.as_object_mut().expect("an object").extend(settings.as_object().expect("an object").clone());
        let (reply, failed) = hypothesis(&tools, request).await;
        assert!(failed, "{settings} must be refused: {reply}");
        assert!(reply.to_string().contains(argument), "the refusal names `{argument}`: {reply}");
    }
}

/// A condition the server cannot read as written must be refused, never read as "nothing to
/// check". Before the fix each of these parsed to a condition that is true of any game with an
/// entity -- a string where a list belongs became an empty list, a string inside `all` was
/// skipped, a count that was not an integer became "any count" -- and the verdict was CONFIRMED.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_refuses_a_condition_it_cannot_read_instead_of_confirming_it() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let crate_type = "fixture_game::Crate";
    for (condition, names) in [
        (json!({"type": "entity_exists", "component_types": crate_type}), "component_types"),
        (json!({"type": "entity_exists", "component_types": [crate_type, 3]}), "component_types"),
        (json!({"type": "entity_exists", "component_types": [crate_type], "count": "6"}), "count"),
        (json!({"type": "entity_exists", "component_types": []}), "component_types"),
        (json!({"type": "entity_exists"}), "component_types"),
        (json!({"type": "all", "assertions": [format!("entity_exists:{crate_type}")]}), "assertions"),
        (json!({"type": "all", "assertions": []}), "assertions"),
        (json!({"type": "any", "assertions": []}), "assertions"),
        (json!({"type": "component_equals", "entity_id": "12", "component_type": crate_type, "expected_value": 1}), "entity_id"),
        (json!({"type": "entity_exists", "component_types": [crate_type], "cuont": 6}), "cuont"),
        (json!({"type": "entity_exist", "component_types": [crate_type]}), "entity_exists"),
        (json!("entity_exists:"), "component"),
        (json!({"type": "performance_within", "metric": "frame_time", "max_value": 20.0}), "frame time"),
        (json!({"type": "performance_within", "metric": "frame_tim", "max_value": 20.0}), "frame_time"),
        (
            json!({"type": "action_succeeds", "action": {"type": "spawn", "components": [{"type_id": "", "value": 1}]}}),
            "type_id",
        ),
    ] {
        for action in ["test", "validate"] {
            let (reply, failed) = hypothesis(
                &tools,
                json!({"action": action, "description": "a claim", "success_condition": condition}),
            )
            .await;
            let refused = failed || reply["valid"] == json!(false);
            assert!(refused, "{action} must refuse {condition}, got: {reply}");
            assert!(reply.get("verdict").is_none(), "{action} of {condition} must not carry a verdict: {reply}");
            assert!(
                reply.to_string().contains(names),
                "{action} of {condition}: the refusal must name `{names}`: {reply}"
            );
        }
    }
}

/// The string `action_succeeds` was a placeholder: it carried no action, so the server made one
/// up -- spawn an empty entity -- ran it every iteration, and answered CONFIRMED. It left an
/// entity in the game each time.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_action_succeeds_without_an_action_is_refused_and_spawns_nothing() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let count = |rows: Value| rows.as_array().map_or(0, Vec::len);
    let query = json!({"method": "world.query", "params": {"data": {"components": []}}});
    let before = count(brp(&tools, query.clone()).await);

    let (reply, failed) = hypothesis(
        &tools,
        json!({"action": "test", "description": "an action succeeds", "success_condition": "action_succeeds"}),
    )
    .await;
    assert!(failed, "a string cannot carry an action, so it must be refused: {reply}");
    assert!(reply.to_string().contains("action"), "{reply}");
    assert_eq!(count(brp(&tools, query).await), before, "the placeholder must not spawn anything");
}

/// The game reports a crate's weight as `3.0`; an agent writes the whole number `3`. They are the
/// same number, and the fixture has a crate that weighs 3.0 (`CRATE_WEIGHTS`). Before the fix the
/// comparison was `serde_json::Value` equality, where an integer and a float differ, so a true
/// claim was REJECTED.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_component_equals_does_not_tell_3_from_3_point_0() {
    assert!(CRATE_WEIGHTS.contains(&3.0) && !CRATE_WEIGHTS.contains(&3.5));
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    for (weight, verdict) in [(json!(3), "CONFIRMED"), (json!(3.0), "CONFIRMED"), (json!(3.5), "REJECTED")] {
        let (reply, failed) = hypothesis(
            &tools,
            json!({
                "action": "test",
                "description": "some crate weighs this much",
                "success_condition": {"type": "component_equals", "component_type": "fixture_game::Crate",
                    "expected_value": {"weight": weight}}
            }),
        )
        .await;
        assert!(!failed, "{reply}");
        assert_eq!(reply["verdict"], json!(verdict), "weight {weight}: {reply}");
    }
}

/// A component name the game never registered cannot be held by any entity, so a claim about it
/// is false by construction, and its negation true -- whatever the game does. That is a typo
/// read as a measurement: before the fix `entity_exists:fixture_game::Crat` was REJECTED and
/// `not entity_exists Crat` was CONFIRMED. A registered component that no entity holds is a real
/// answer and stays one.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_refuses_a_component_the_game_never_registered_instead_of_judging_the_claim() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    for condition in [
        json!("entity_exists:fixture_game::Crat"),
        json!({"type": "not", "assertion": {"type": "entity_exists", "component_types": ["fixture_game::Crat"]}}),
        json!({"type": "component_equals", "component_type": "Crat", "expected_value": {"weight": 3.0}}),
        json!({"type": "any", "assertions": [
            {"type": "entity_exists", "component_types": ["fixture_game::Crate"]},
            {"type": "entity_exists", "component_types": ["fixture_game::Crat"]}]}),
        json!({"type": "not", "assertion": {"type": "action_succeeds",
            "action": {"type": "spawn", "components": [{"type_id": "fixture_game::Crat", "value": {"weight": 1.0}}]}}}),
    ] {
        let (reply, failed) = hypothesis(
            &tools,
            json!({"action": "test", "description": "a claim", "success_condition": condition}),
        )
        .await;
        assert!(failed, "{condition} names a component the game does not have: {reply}");
        let text = reply.to_string();
        assert!(text.contains("Crat") && text.contains("not registered"), "the refusal names it: {text}");
        assert!(text.contains("fixture_game::Crate"), "and offers the registered name it is close to: {text}");
    }

    // The control: a registered component that no entity holds.
    let registered = brp(&tools, json!({"method": "world.list_components"})).await;
    let mut unheld = None;
    for path in registered.as_array().expect("world.list_components returns an array") {
        let path = path.as_str().expect("a component path is a string");
        let rows = brp(&tools, json!({"method": "world.query", "params": {"data": {"components": [path]}}})).await;
        if rows.as_array().is_some_and(Vec::is_empty) {
            unheld = Some(path.to_string());
            break;
        }
    }
    let unheld = unheld.expect("the fixture registers at least one component that no entity holds");
    let (reply, failed) = hypothesis(
        &tools,
        json!({"action": "test", "description": "an entity holds it", "success_condition": format!("entity_exists:{unheld}")}),
    )
    .await;
    assert!(!failed, "{unheld} is registered, so the claim is answerable: {reply}");
    assert_eq!(reply["verdict"], json!("REJECTED"), "no entity holds {unheld}: {reply}");
}

/// The crates a game test leaves behind or takes away, as a count.
async fn crate_count(tools: &BevyDebuggerTools) -> usize {
    crate_weights(tools).await.len()
}

/// `hypothesis` varies the numbers, strings and booleans inside a component's value. A Bevy
/// component is nearly always a struct, so its value is an object, and `grid`, `boundary` and
/// `fuzz` only looked at a bare number, string or boolean: every iteration ran the base action
/// unchanged while the reply counted them as separate trials. The grid below has two numbers, so
/// two iterations: one writes weight 10, the other weight 20, and the claim "the crate weighs 20"
/// holds in exactly one of them.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_variation_reaches_the_fields_inside_a_component() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let before = crate_weights(&tools).await;
    let entity = *before.keys().next().expect("the fixture spawns crates");

    let (reply, failed) = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "the crate can be made to weigh 20",
            "success_condition": {"type": "component_equals", "entity_id": entity,
                "component_type": "fixture_game::Crate", "expected_value": {"weight": 20.0}},
            "actions": [{"type": "modify", "entity_id": entity,
                "components": [{"type_id": "fixture_game::Crate", "value": {"weight": 5.0}}]}],
            "variation_type": "grid",
            "numeric_values": [10.0, 20.0],
            "bool_values": [true]
        }),
    )
    .await;

    assert!(!failed, "{reply}");
    let stats = &reply["statistics"];
    assert_eq!(stats["iterations_run"], json!(2), "{reply}");
    assert_eq!(
        (stats["held"].as_u64(), stats["violated"].as_u64()),
        (Some(1), Some(1)),
        "the grid wrote weight 10 and weight 20, so the claim held once: {reply}"
    );
    assert_eq!(stats["varied_iterations"], json!(2), "both iterations ran a changed action: {reply}");
    assert_eq!(crate_weights(&tools).await, before, "the modifies must have been reversed: {reply}");
}

/// A variation changes the values inside `actions`. With nothing there to change, N iterations are
/// N identical reads of the game: each one narrows the confidence interval and none of them tests
/// anything. Before the fix `grid` over a bare condition ran its six combinations and answered
/// CONFIRMED. Refused before anything runs, naming the fix.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_refuses_a_variation_that_would_change_nothing() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let before = crate_weights(&tools).await;
    let entity = *before.keys().next().expect("the fixture spawns crates");
    let delete = json!({"type": "delete", "entity_id": entity});

    for kind in ["random", "grid", "boundary", "fuzz"] {
        // No actions at all, and an action with no values in it: neither has anything to vary.
        for actions in [json!([]), json!([delete])] {
            let (reply, failed) = hypothesis(
                &tools,
                json!({
                    "action": "test",
                    "description": "there is a crate",
                    "success_condition": "entity_exists:fixture_game::Crate",
                    "actions": actions,
                    "variation_type": kind,
                    "iterations": 6
                }),
            )
            .await;
            assert!(failed, "{kind} over {actions} changes nothing and must be refused: {reply}");
            let text = reply.to_string();
            assert!(text.contains(kind) && text.contains("actions"), "the refusal names the variation and the fix: {text}");
            assert!(reply.get("verdict").is_none(), "{reply}");
        }
    }
    assert_eq!(crate_weights(&tools).await, before, "a refused request must not have changed the game");
}

/// Every iteration starts from the game the last one found. The claim below ("after the spawn
/// there are seven crates") is true of one spawn, but the spawns used to accumulate: iteration 2
/// saw eight crates and the claim was violated by the previous iterations' leftovers, not by
/// anything this iteration did. Each iteration's spawns and modifies are now reversed, and the
/// reply counts what was reversed.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_puts_the_game_back_after_every_iteration() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let crates = crate_count(&tools).await;
    assert_eq!(crates, CRATE_WEIGHTS.len());

    let (reply, failed) = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "one spawn makes exactly one more crate",
            "success_condition": {"type": "entity_exists", "component_types": ["fixture_game::Crate"], "count": crates + 1},
            "actions": [spawn_crate(1.0)],
            "variation_type": "random",
            "iterations": 5
        }),
    )
    .await;

    assert!(!failed, "{reply}");
    assert_eq!(reply["statistics"]["held"], json!(5), "each iteration starts from {crates} crates: {reply}");
    assert_eq!(reply["verdict"], json!("CONFIRMED"), "{reply}");
    assert_eq!(reply["game_state"]["actions_reversed"], json!(5), "{reply}");
    assert_eq!(reply["game_state"]["left_applied"], json!(0), "{reply}");
    assert_eq!(crate_count(&tools).await, crates, "the game is as it was found: {reply}");
}

/// What cannot be put back is said, not hidden. A despawned entity returns as a different one, so
/// a `delete` stays applied and is counted; and a condition that is itself an action
/// (`action_succeeds`) is reversed too, which used to leave one more crate per iteration.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_says_what_it_could_not_put_back() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let before = crate_weights(&tools).await;
    let entity = *before.keys().next().expect("the fixture spawns crates");

    let (reply, failed) = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "a crate can be deleted",
            "success_condition": "entity_exists:fixture_game::Mover",
            "actions": [{"type": "delete", "entity_id": entity}]
        }),
    )
    .await;
    assert!(!failed, "{reply}");
    assert_eq!(reply["game_state"]["left_applied"], json!(1), "the delete cannot be undone: {reply}");
    assert_eq!(reply["game_state"]["actions_reversed"], json!(0), "{reply}");
    assert_eq!(crate_count(&tools).await, before.len() - 1, "the delete reached the game and stayed");

    let after_delete = crate_count(&tools).await;
    let (reply, failed) = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "a crate can be spawned",
            "success_condition": {"type": "action_succeeds", "action": spawn_crate(2.0)},
            "variation_type": "none"
        }),
    )
    .await;
    assert!(!failed, "{reply}");
    assert_eq!(reply["verdict"], json!("CONFIRMED"), "{reply}");
    assert_eq!(reply["game_state"]["actions_reversed"], json!(1), "the probe's spawn was reversed: {reply}");
    assert_eq!(crate_count(&tools).await, after_delete, "the probe must not leave a crate: {reply}");
}

/// `timeout` bounds the run, and the client is free between iterations. Before the fix it bounded
/// one action: 400 iterations of a spawn and a read took as long as they took, and the BRP client
/// lock was held throughout, so any other tool (here `status`) waited for the whole run.
#[tokio::test(flavor = "multi_thread")]
async fn hypothesis_timeout_bounds_the_whole_run_and_leaves_the_client_free() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;
    let crates = crate_count(&tools).await;
    let started = Instant::now();

    let run = hypothesis(
        &tools,
        json!({
            "action": "test",
            "description": "crates stay countable",
            "success_condition": "entity_exists:fixture_game::Crate",
            "actions": [spawn_crate(1.0)],
            "variation_type": "random",
            "iterations": 400,
            "timeout": 1
        }),
    );
    let probe = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let asked = Instant::now();
        tools.status(Parameters(NoArgs {})).await.expect("status returns a result");
        asked.elapsed()
    };
    let ((reply, failed), waited) = tokio::join!(run, probe);

    assert!(!failed, "{reply}");
    assert!(started.elapsed() < Duration::from_secs(8), "a 1 s timeout bounds the run, took {:?}", started.elapsed());
    assert!(waited < Duration::from_secs(2), "`status` waited {waited:?} for a run that holds the client throughout");
    let stats = &reply["statistics"];
    assert_eq!(stats["iterations_planned"], json!(400), "{reply}");
    let run_count = stats["iterations_run"].as_u64().expect("iterations_run is a number");
    assert!(run_count < 400, "the 1 s timeout must have cut the run: {reply}");
    assert!(
        reply["stopped_early"].as_str().is_some_and(|why| why.contains("timeout")),
        "the reply says why it stopped: {reply}"
    );
    assert_ne!(reply["verdict"], json!("CONFIRMED"), "iterations that never ran cannot confirm the claim: {reply}");
    assert_eq!(crate_count(&tools).await, crates, "the run stopped between iterations and left the game as it found it: {reply}");
}

/// The real `bevy-fairy` binary over stdio, pointed at a fixture: a tool is reached by name, the
/// way an agent's client reaches it, so a tool the server does not offer is answered as such.
struct StdioServer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    replies: std::sync::mpsc::Receiver<Value>,
    next_id: u64,
}

impl StdioServer {
    fn start(port: u16) -> Self {
        use std::io::BufRead;
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_bevy-fairy"))
            .arg("--stdio")
            .env("BEVY_BRP_HOST", "127.0.0.1")
            .env("BEVY_BRP_PORT", port.to_string())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("start the bevy-fairy binary");
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (send, replies) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    if send.send(message).is_err() {
                        break;
                    }
                }
            }
        });
        let mut server = Self { child, stdin, replies, next_id: 0 };
        server.request(
            "initialize",
            json!({"protocolVersion": "2024-11-05", "capabilities": {},
                   "clientInfo": {"name": "fixture-ground-truth", "version": "1"}}),
        );
        server.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }

    fn send(&mut self, message: Value) {
        use std::io::Write;
        writeln!(self.stdin, "{message}").expect("write to the server");
        self.stdin.flush().expect("flush to the server");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let reply = self
                .replies
                .recv_timeout(left)
                .unwrap_or_else(|e| panic!("no reply to {method} within 60 s: {e}"));
            if reply.get("id") == Some(&json!(id)) {
                return reply;
            }
        }
    }

    /// Call a tool by name. `Err` is what the agent sees when the call is refused, whether as an
    /// error result or as an RPC error; `Ok` is the tool's JSON answer.
    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let reply = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        if let Some(error) = reply.get("error") {
            return Err(error.to_string());
        }
        let result = &reply["result"];
        let text = result["content"]
            .as_array()
            .map(|items| items.iter().filter_map(|item| item["text"].as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if result["isError"] == json!(true) {
            Err(text)
        } else {
            Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
    }
}

impl Drop for StdioServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The mover's x, read from the game.
async fn mover_x(tools: &BevyDebuggerTools) -> f64 {
    let rows = brp(
        tools,
        json!({"method": "world.query", "params": {
            "data": {"components": ["bevy_transform::components::transform::Transform"]},
            "filter": {"with": ["fixture_game::Mover"]}
        }}),
    )
    .await;
    rows[0]["components"]["bevy_transform::components::transform::Transform"]["translation"][0]
        .as_f64()
        .unwrap_or_else(|| panic!("the mover's transform has no x: {rows}"))
}

/// `replay` answered "Playback started" -- and "Stepped forward", "Seeked to frame N" -- while
/// nothing reached the game: the sync that was to write each recorded frame back was a `TODO`
/// that returned `Ok`, so the mover carried on from wherever it was. A tool that says it is
/// playing a recording back must put the game in the recorded states, and one that cannot must
/// refuse.
#[tokio::test(flavor = "multi_thread")]
async fn no_tool_says_it_plays_a_recording_back_without_the_game_seeing_it() {
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    let mut server = StdioServer::start(port);

    let x_when_recording_began = mover_x(&tools).await;
    if server.call_tool("replay", json!({"action": "record", "sample_rate": 20.0})).is_err() {
        return; // Refused: nothing was claimed about the game.
    }
    tokio::time::sleep(Duration::from_millis(1000)).await;
    if server.call_tool("replay", json!({"action": "stop"})).is_err() {
        return;
    }
    let x_when_recording_stopped = mover_x(&tools).await;
    // Let the mover travel well past every recorded state before asking for them back.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let Ok(claim) = server.call_tool("replay", json!({"action": "play"})) else {
        return;
    };
    tokio::time::sleep(Duration::from_millis(500)).await;

    let x_now = mover_x(&tools).await;
    // Played back, the mover is somewhere the recording saw it, and moves on at MOVER_SPEED from
    // there; allow half a second of that travel either side.
    let slack = f64::from(fixture_game::MOVER_SPEED) * 0.5;
    assert!(
        x_now >= x_when_recording_began - slack && x_now <= x_when_recording_stopped + slack,
        "replay answered {claim}, but the mover is at x = {x_now}: the recording saw it only \
         between x = {x_when_recording_began} and x = {x_when_recording_stopped}"
    );
}

/// `detect_anomaly` keeps one process-wide set of detectors, so the tests that read it take turns.
static ANOMALY_TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn detect_anomaly(tools: &BevyDebuggerTools, request: Value) -> Value {
    let params: bevy_fairy::server::mcp_tools::AnomalyRequest =
        serde_json::from_value(request).expect("the request deserialises");
    let result = tools
        .detect_anomaly(Parameters(params))
        .await
        .expect("the detect_anomaly tool returns a result");
    let text = text_of(&result);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("detect_anomaly returned non-JSON ({e}): {text}"))
}

/// `detect` returned `anomalies: []` and `entities_analyzed: N` for a game in which no detector
/// found anything to read, which an agent takes for a clean bill of health. The fixture has no
/// `Velocity` and no `Health`/`Alive`, and a constant number of entities, so the truth is that the
/// physics and consistency detectors evaluated nothing and the performance detector holds exactly
/// as many samples as `detect` has been called.
#[tokio::test(flavor = "multi_thread")]
async fn detect_anomaly_reports_what_the_detectors_could_not_read_in_the_fixture() {
    let _turn = ANOMALY_TURN.lock().await;
    let port = start_fixture();
    let tools = tools_for(port);
    wait_until_ready(&tools).await;
    let everything = brp(&tools, json!({"method": "world.query", "params": {"data": {"components": []}}})).await;
    let entities_in_the_game = everything.as_array().expect("world.query returns rows").len() as u64;

    let mut held = Vec::new();
    let mut frames_seen: Vec<u64> = Vec::new();
    for call in 1..=2u64 {
        let reply = detect_anomaly(&tools, json!({"action": "detect"})).await;
        assert_eq!(reply["summary"]["entities_read"], entities_in_the_game, "{reply}");
        let (from, until) = (reply["frame"].as_u64(), reply["frame_until"].as_u64());
        assert!(from.is_some() && until >= from, "the detection says which frames it was read between: {reply}");
        if let Some(previous) = frames_seen.last() {
            assert!(from.unwrap() > *previous, "a later call is on a later frame: {reply}");
        }
        frames_seen.push(from.unwrap());
        assert_eq!(reply["anomalies"], json!([]), "the fixture is steady: {reply}");
        let detectors = reply["detectors"].as_array().expect("coverage per detector");
        let named = |name: &str| detectors.iter().find(|d| d["detector"] == name).unwrap_or_else(|| panic!("{name}: {reply}"));
        for name in ["PhysicsDetector", "ConsistencyDetector"] {
            assert_eq!(named(name)["examined"], 0, "the fixture has nothing {name} reads: {reply}");
            assert!(named(name)["status"].as_str().unwrap().starts_with("not evaluated"), "{reply}");
        }
        let performance = named("PerformanceDetector")["status"].as_str().unwrap().to_string();
        assert!(performance.contains(&format!("{call} of the 10 samples")), "call {call}: {performance}");
        held.push(performance);
        assert!(reply["note"].as_str().unwrap().contains("says nothing about the game"), "{reply}");
    }
    assert_ne!(held[0], held[1], "each call adds one sample");
}

/// An `observe` reply carries no frame, so an agent that reads twice cannot tell fresh evidence
/// from stale, or how far apart two reads were. Each reply says which frames the read lies
/// between. The spinner counts its own frames, so with the constant offset `c` between the two
/// counters every read must satisfy `frame <= frames - c <= frame_until` for one `c`
/// (`watch_brackets_the_frame_every_sample_was_read_on` argues the same bracket).
#[tokio::test(flavor = "multi_thread")]
async fn observe_brackets_the_frame_every_read_was_taken_on() {
    let tools = tools_for(start_fixture());
    wait_until_ready(&tools).await;

    let (mut lowest, mut highest) = (i64::MIN, i64::MAX);
    let mut firsts = Vec::new();
    for _ in 0..10 {
        let (text, failed) = observe(&tools, "find entities with component fixture_game::Spinner").await;
        assert!(!failed, "{text}");
        let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("observe returned non-JSON ({e}): {text}"));
        let counted = reply["result"][0]["components"]["fixture_game::Spinner"]["frames"]
            .as_i64()
            .unwrap_or_else(|| panic!("no spinner frame counter in: {text}"));
        let from = reply["frame"].as_i64().unwrap_or_else(|| panic!("the reply carries no frame: {text}"));
        let until = reply["frame_until"].as_i64().unwrap_or_else(|| panic!("the reply carries no frame_until: {text}"));
        assert!(from <= until, "bracket reversed: {text}");
        lowest = lowest.max(counted - until);
        highest = highest.min(counted - from);
        firsts.push(from);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(lowest <= highest, "no constant offset fits every bracket ({lowest} > {highest})");
    assert!(firsts.windows(2).all(|w| w[0] < w[1]), "reads 100 ms apart are on later frames: {firsts:?}");
}

/// The MCP client starts the server with the agent's session, and the game starts later. The
/// background heartbeat that refreshes the connection state runs every 30 s, and `experiment`
/// and `hypothesis` read that state without asking the game, so for up to 30 s after the game
/// came up each answered "not connected to Bevy game" about a game that was answering. They
/// now probe once before giving that answer. (`observe` and `detect_anomaly` already did, by
/// asking the game for its frame first; they are here so that stays true.)
#[tokio::test(flavor = "multi_thread")]
async fn tools_answer_as_soon_as_the_game_is_up_not_after_the_next_heartbeat() {
    let port = free_port();
    // Clients that have never seen the game, as in a server started before it.
    let tools = [tools_for(port), tools_for(port), tools_for(port), tools_for(port)];

    // Before the game exists the refusal is true.
    let (text, _) = observe(&tools[0], "find entities with component fixture_game::Crate").await;
    assert!(text.contains("not connected"), "no game is running yet: {text}");

    std::thread::spawn(move || {
        fixture_game::build_app(port, fixture_game::DEFAULT_FPS).run();
    });
    // A fifth client waits until the game answers; the four above are never asked before the check.
    wait_until_ready(&tools_for(port)).await;

    let asked = [
        text_of(
            &tools[0]
                .observe(Parameters(ObserveRequest {
                    query: "find entities with component fixture_game::Crate".to_string(),
                    diff: false,
                }))
                .await
                .expect("observe returns a result"),
        ),
        text_of(
            &tools[1]
                .detect_anomaly(Parameters(
                    serde_json::from_value(json!({"action": "detect"})).expect("the request deserialises"),
                ))
                .await
                .expect("detect_anomaly returns a result"),
        ),
        text_of(
            &tools[2]
                .experiment(Parameters(
                    serde_json::from_value(json!({"action": "undo"})).expect("the request deserialises"),
                ))
                .await
                .expect("experiment returns a result"),
        ),
        text_of(
            &tools[3]
                .hypothesis(Parameters(
                    serde_json::from_value(json!({
                        "action": "test", "description": "a crate exists",
                        "success_condition": "entity_exists:fixture_game::Crate"
                    }))
                    .expect("the request deserialises"),
                ))
                .await
                .expect("hypothesis returns a result"),
        ),
    ];
    for (tool, text) in ["observe", "detect_anomaly", "experiment", "hypothesis"].iter().zip(&asked) {
        assert!(!text.contains("not connected"), "{tool} said the game was not connected while it answered: {text}");
    }
}
