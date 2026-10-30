//! The server's instructions must match the tools it registers.
//!
//! The instructions are the first thing an agent reads, and they used to say "hypothesis/experiment/
//! stress_test/replay re-run it" without saying how to call any of them. An agent then guessed the
//! argument names, and the tools silently ran a default instead of what was asked. The instructions
//! have to name the fields a tool actually needs, and they must not point at a tool that cannot do
//! what the sentence claims.
//!
//! This is the drift check the loop's rules ask for: a test that fails when a doc names something
//! the code does not have.

use bevy_fairy::server::mcp_tools::COLLABORATION_PROTOCOL;

/// Every tool the instructions point at must be a real tool. A name in the instructions that is
/// not a tool sends the agent to nothing.
#[test]
fn every_tool_named_in_the_instructions_exists() {
    let tools = bevy_fairy::server::mcp_tools::BevyDebuggerTools::tool_names();
    for name in [
        "status",
        "commands",
        "run",
        "screenshot",
        "scene",
        "brp",
        "quit",
        "observe",
        "detect_anomaly",
        "hypothesis",
        "experiment",
        "guide",
    ] {
        assert!(
            tools.contains(&name.to_string()),
            "the instructions name `{name}`, which is not a registered tool; the tools are {tools:?}"
        );
    }
}

/// The instructions must name the field a tool needs, or the agent guesses and the tool runs a
/// default instead of what was asked.
#[test]
fn the_instructions_name_the_fields_the_tools_need() {
    // `hypothesis` cannot run without these two.
    assert!(
        COLLABORATION_PROTOCOL.contains("description"),
        "the instructions must say that `hypothesis` needs a `description` of the claim"
    );
    assert!(
        COLLABORATION_PROTOCOL.contains("success_condition"),
        "the instructions must say that `hypothesis` needs a `success_condition`"
    );
    // `experiment` needs the actions to carry out.
    assert!(
        COLLABORATION_PROTOCOL.contains("actions"),
        "the instructions must say that `experiment` needs the `actions` to carry out"
    );
    // Every one of these picks its operation with an `action`.
    assert!(
        COLLABORATION_PROTOCOL.contains("`action`"),
        "the instructions must say that these tools pick what to do with an `action`"
    );
}
