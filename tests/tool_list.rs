//! What `tools/list` offers, and in what order.
//!
//! The order came from a `HashMap` inside rmcp's tool router, so it changed on every launch:
//! four launches gave four different orders. An agent, a client's prompt cache and an eval
//! transcript all see the list, and a list that reshuffles itself is a list nobody can diff.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// The tools as the real binary lists them over stdio.
fn listed_tools() -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_bevy-fairy"))
        .arg("--stdio")
        // Nothing listens here; listing tools does not need a game.
        .env("BEVY_BRP_PORT", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the bevy-fairy binary");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut reader = BufReader::new(child.stdout.take().expect("stdout is piped"));
    let mut send = |message: Value| {
        writeln!(stdin, "{message}").expect("write to the server");
        stdin.flush().expect("flush to the server");
    };
    let mut receive = |id: u64| -> Value {
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).expect("read from the server");
            assert!(read > 0, "the server closed stdout");
            if let Ok(reply) = serde_json::from_str::<Value>(&line) {
                if reply.get("id") == Some(&json!(id)) {
                    return reply;
                }
            }
        }
    };

    send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2024-11-05", "capabilities": {},
        "clientInfo": {"name": "order-test", "version": "1"}}}));
    receive(1);
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let reply = receive(2);
    let _ = child.kill();
    let _ = child.wait();

    reply["result"]["tools"].as_array().expect("tools/list returns tools").clone()
}

fn tool_names() -> Vec<String> {
    listed_tools()
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool has a name").to_string())
        .collect()
}

#[test]
fn tools_are_listed_by_name_and_in_the_same_order_every_launch() {
    let first = tool_names();
    let mut sorted = first.clone();
    sorted.sort();
    assert_eq!(first, sorted, "tools/list must return the tools sorted by name");

    for launch in 2..=4 {
        assert_eq!(
            tool_names(),
            first,
            "launch {launch} listed the tools in a different order than launch 1"
        );
    }
}

/// A tool that does not set `deny_unknown_fields` drops a misspelled or invented argument and
/// runs anyway, so the agent believes `{"query": "list all entities", "limit": 5}` was limited.
/// Every advertised input schema must say `additionalProperties: false`.
#[test]
fn every_tool_refuses_arguments_it_does_not_advertise() {
    let open: Vec<String> = listed_tools()
        .iter()
        .filter(|tool| tool["inputSchema"]["additionalProperties"] != json!(false))
        .map(|tool| tool["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(open.is_empty(), "these tools silently drop unknown arguments: {open:?}");
}

/// `stress_test` stays out of `tools/list` until it can report what it did. Against the fixture
/// game it answered `"success": true` while every spawn it sent failed (it used short type names
/// like `Transform` and components no game has, like `Health`), reported `16.67 + rand * 10` as
/// frame-time percentiles, and ran 30.1 s when asked for 1 s. Whoever brings it back replaces
/// this test with a ground-truth one against the fixture.
#[test]
fn stress_test_is_not_offered() {
    let names = tool_names();
    assert!(
        !names.iter().any(|name| name == "stress_test"),
        "stress_test is listed again: {names:?}"
    );
}

/// Every name a tool's description puts in backticks must exist: a field of that tool, a value its
/// schema lists, or another tool. An agent reads the description first and sends what it names.
/// `detect_anomaly` said `detect` "looks for the kind named in `detection_type`", a field its
/// request type does not have and refuses as unknown.
#[test]
fn every_name_a_tool_description_puts_in_backticks_exists() {
    let tools = listed_tools();
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    let mut missing = Vec::new();
    for tool in &tools {
        let name = tool["name"].as_str().unwrap_or_default();
        let description = tool["description"].as_str().unwrap_or_default();
        let schema = &tool["inputSchema"];
        let schema_text = schema.to_string();
        let fields: Vec<&str> = schema["properties"]
            .as_object()
            .map(|props| props.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for quoted in description.split('`').skip(1).step_by(2) {
            let word = quoted.split_whitespace().next().unwrap_or_default();
            let known = fields.contains(&word)
                || tool_names.contains(&word)
                || schema_text.contains(&format!("`{word}`"));
            if !known {
                missing.push(format!("`{name}` names `{word}`"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "tool descriptions name things the tools do not have: {missing:?}"
    );
}

/// `docs/mcp-tools.md` is generated from `tools/list`, so the reference cannot say anything the
/// server does not. Regenerate it with `UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list`.
#[test]
fn the_tool_reference_is_generated_from_tools_list() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/mcp-tools.md");
    let generated = render_tool_reference(&listed_tools());
    if std::env::var_os("UPDATE_TOOL_REFERENCE").is_some() {
        std::fs::write(&path, &generated).expect("write docs/mcp-tools.md");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/mcp-tools.md is not what tools/list says; regenerate it with \
         `UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list`"
    );
}

/// The reference page: one section per tool, in `tools/list` order, with its description and a
/// table of the arguments its schema advertises.
fn render_tool_reference(tools: &[Value]) -> String {
    let mut out = String::from(
        "# MCP tools\n\n\
         Generated from the server's `tools/list` by `tests/tool_list.rs`; do not edit by hand.\n\
         Regenerate with `UPDATE_TOOL_REFERENCE=1 cargo test --test tool_list`.\n",
    );
    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default();
        let description = tool["description"].as_str().unwrap_or_default();
        out.push_str(&format!("\n## `{name}`\n\n{description}\n"));
        let schema = &tool["inputSchema"];
        let required: Vec<&str> = schema["required"]
            .as_array()
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let Some(props) = schema["properties"].as_object().filter(|p| !p.is_empty()) else {
            out.push_str("\nNo arguments.\n");
            continue;
        };
        out.push_str("\n| Argument | Type | Required | Description |\n|---|---|---|---|\n");
        let mut names: Vec<&String> = props.keys().collect();
        names.sort();
        for arg in names {
            let prop = &props[arg.as_str()];
            let kind = match &prop["type"] {
                Value::String(s) => s.clone(),
                Value::Array(kinds) => kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|k| *k != "null")
                    .collect::<Vec<_>>()
                    .join(" or "),
                _ => "any".to_string(),
            };
            let doc = prop["description"]
                .as_str()
                .unwrap_or_default()
                .replace('\n', " ")
                .replace('|', "\\|");
            let req = if required.contains(&arg.as_str()) { "yes" } else { "no" };
            out.push_str(&format!("| `{arg}` | {kind} | {req} | {doc} |\n"));
        }
    }
    out
}

/// `observe` refuses any query its parser does not recognise, so an agent can only succeed if the
/// schema shows it the shapes. Three of five `isError` results in the 2026-10-23 evals were bare
/// nouns ("score", "crate"). Every example the `query` field advertises must be accepted by the
/// parser the handler uses.
#[test]
fn every_observe_query_example_in_the_schema_is_accepted_by_the_parser() {
    use bevy_fairy::server::query_parser::{QueryParser, RegexQueryParser};

    let tools = listed_tools();
    let observe = tools.iter().find(|t| t["name"] == "observe").expect("observe is listed");
    let description = observe["inputSchema"]["properties"]["query"]["description"]
        .as_str()
        .expect("the `query` field carries a description");
    let examples: Vec<&str> = description.split('`').skip(1).step_by(2).collect();
    assert!(examples.len() >= 5, "the `query` description gives no examples: {description}");
    let parser = RegexQueryParser::new().expect("the parser builds");
    let refused: Vec<&str> = examples
        .iter()
        .copied()
        .filter(|example| *example != "brp" && parser.parse(example).is_err())
        .collect();
    assert!(refused.is_empty(), "observe's parser refuses its own advertised examples: {refused:?}");
}
