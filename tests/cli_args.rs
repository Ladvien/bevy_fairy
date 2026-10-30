//! The binary's command line answers what it is asked, and refuses what it does not know.
//!
//! `bevy-fairy --version` used to start the MCP server in stdio mode, wait for a client that
//! was never coming, and exit with "connection closed: initialized request". Any argument the
//! binary did not recognise did the same, so a typo looked like a server crash rather than a
//! typo. The loop's release check runs `bevy-fairy --version` to confirm the installed binary
//! matches the release, and it could not.

use std::process::{Command, Stdio};

fn bevy_fairy(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_bevy-fairy"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("running the bevy-fairy binary")
}

#[test]
fn version_prints_the_crate_version_and_exits_cleanly() {
    for flag in ["--version", "-V"] {
        let out = bevy_fairy(&[flag]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "`bevy-fairy {flag}` must exit 0; got {:?}, stderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            stdout.trim(),
            format!("bevy-fairy {}", env!("CARGO_PKG_VERSION")),
            "`bevy-fairy {flag}` must print its name and version"
        );
    }
}

#[test]
fn an_unknown_argument_is_refused_by_name_rather_than_starting_the_server() {
    let out = bevy_fairy(&["--verison"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unknown argument is a usage error (exit 2), not a server run; stderr: {stderr}"
    );
    assert!(
        stderr.contains("--verison"),
        "the error must name the argument it did not recognise; got: {stderr}"
    );
    assert!(
        stderr.contains("--version") && stderr.contains("--help"),
        "the error must list the arguments that do exist; got: {stderr}"
    );
}

/// MCP client configs that earlier installers wrote say `"args": ["stdio"]` or
/// `"args": ["serve"]`. They worked because unknown words fell through to stdio mode, and they
/// are on users' machines now, so refusing unknown arguments must not refuse these two.
///
/// With stdin closed the server starts, sees the client go away, and exits; what matters is
/// that it started rather than refusing the argument as a usage error.
#[test]
fn the_legacy_stdio_spellings_installers_wrote_are_still_accepted() {
    for legacy in ["stdio", "serve"] {
        let out = bevy_fairy(&[legacy]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_ne!(
            out.status.code(),
            Some(2),
            "`bevy-fairy {legacy}` was written into client configs by earlier installers and \
             must still start the server; stderr: {stderr}"
        );
        assert!(
            !stderr.contains("unknown argument"),
            "`{legacy}` must not be refused as unknown; stderr: {stderr}"
        );
    }
}

/// `--tcp` / `--server` bound a port, accepted a connection and dropped it without reading a
/// byte, so an agent that followed `--help` held a port open and got nothing. The flags are
/// gone; stdio is the only transport, and a removed flag is refused by name like any typo.
#[test]
fn the_tcp_flags_that_never_served_anything_are_refused() {
    for flag in ["--tcp", "--server"] {
        let out = bevy_fairy(&[flag]);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "`{flag}` must be a usage error; stderr: {stderr}");
        assert!(stderr.contains(flag), "the error must name `{flag}`; got: {stderr}");
    }
    let help = String::from_utf8_lossy(&bevy_fairy(&["--help"]).stdout).into_owned();
    assert!(!help.contains("--tcp") && !help.contains("TCP"), "--help still advertises TCP: {help}");
}
