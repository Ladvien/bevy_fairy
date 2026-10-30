/*
 * bevy-fairy: the MCP server half of an agentic debugger for Bevy games
 * Copyright (C) 2025 ladvien
 *
 * Licensed under either of MIT (LICENSE-MIT) or Apache-2.0 (LICENSE-APACHE), at your option.
 *
 * Relicensed from GPL-3.0 when this crate was adopted into Ladvien/foundation_vs_slop: a GPL
 * crate in the Bevy ecosystem cannot be adopted, and being adoptable is why it is published.
 */

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use is_terminal::IsTerminal;

// Modules are defined in lib.rs, no need to redeclare them here

use bevy_fairy::server::brp_client::BrpClient;
use bevy_fairy::server::config::Config;
use bevy_fairy::server::error::Result;
use bevy_fairy::server::mcp_server_v2;


#[tokio::main]
async fn main() -> Result<()> {
    // Set up global panic handler that logs before exit
    std::panic::set_hook(Box::new(|panic_info| {
        let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "Unknown panic payload".to_string()
        };
        
        let location = if let Some(location) = panic_info.location() {
            format!("{}:{}:{}", location.file(), location.line(), location.column())
        } else {
            "Unknown location".to_string()
        };
        
        eprintln!("FATAL PANIC in bevy-fairy: {} at {}", payload, location);
        eprintln!("This indicates a critical bug that should be reported.");
        eprintln!("Stack trace should appear above this message.");
        
        // Ensure logs are flushed
        std::io::Write::flush(&mut std::io::stderr()).unwrap_or(());
    }));
    let args: Vec<String> = std::env::args().collect();

    // Every argument this binary understands. Anything else is refused by name: it used to
    // fall through to "start the MCP server", so `--version` or a typo looked like a server
    // crash ("connection closed: initialized request") rather than a usage error.
    //
    // `stdio` and `serve` are legacy spellings of `--stdio`. Earlier installers wrote them into
    // MCP client configs (`"args": ["stdio"]`, `"args": ["serve"]`), and those configs only
    // worked because unknown words fell through to stdio mode. They are accepted explicitly so
    // a config already on someone's machine keeps working; the installers now write `--stdio`.
    const LEGACY_STDIO_ALIASES: &[&str] = &["stdio", "serve"];
    const KNOWN_ARGS: &[&str] = &["--stdio", "--help", "-h", "--version", "-V"];
    if let Some(unknown) = args.iter().skip(1).find(|arg| {
        !KNOWN_ARGS.contains(&arg.as_str()) && !LEGACY_STDIO_ALIASES.contains(&arg.as_str())
    }) {
        eprintln!("bevy-fairy: unknown argument `{unknown}`");
        eprintln!("valid arguments: {}", KNOWN_ARGS.join(", "));
        eprintln!("try `bevy-fairy --help`");
        std::process::exit(2);
    }

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("bevy-fairy {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Check for help flag
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("bevy-fairy {}", env!("CARGO_PKG_VERSION"));
        println!("\nUsage: {} [OPTIONS]", args[0]);
        println!("\nOptions:");
        println!("  --stdio              Run in stdio mode (default for Claude Code)");
        println!("  --help, -h           Show this help message");
        println!("  --version, -V        Print the version and exit");
        println!("\nEnvironment variables:");
        println!("  BEVY_BRP_HOST        Bevy Remote Protocol host (default: localhost)");
        println!("  BEVY_BRP_PORT        Bevy Remote Protocol port (default: 15702)");
        println!("  RUST_LOG             Logging level (default: info)");
        return Ok(());
    }
    
    // stdout is the MCP stream: logs go to stderr, uncoloured.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let config = Config::from_env()?;

    // stdio is the only transport. A person at a terminal gets a usage error instead of a
    // server waiting on a keyboard for JSON-RPC; an MCP client (piped stdin), `--stdio`, a
    // legacy alias or `MCP_TRANSPORT=stdio` starts it.
    let explicit = args.iter().skip(1).any(|arg| arg == "--stdio" || LEGACY_STDIO_ALIASES.contains(&arg.as_str()))
        || std::env::var("MCP_TRANSPORT").map(|t| t == "stdio").unwrap_or(false);
    if !explicit && std::io::stdin().is_terminal() {
        eprintln!("bevy-fairy speaks MCP over stdio and was started from a terminal.");
        eprintln!("Register it in an MCP client as `bevy-fairy --stdio`, or pass --stdio to run it by hand.");
        std::process::exit(2);
    }

    info!("Starting bevy-fairy in stdio mode");
    run_stdio_mode(config).await
}

async fn run_stdio_mode(config: Config) -> Result<()> {
    let brp_client = Arc::new(RwLock::new(BrpClient::new(&config)));
    {
        let client = brp_client.read().await;
        client.init().await?;
    }
    
    let server = mcp_server_v2::McpServerV2::new(config, brp_client)?;
    server.run_stdio().await
}
