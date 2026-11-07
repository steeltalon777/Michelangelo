//! `michelangelo mcp` subcommands: the MCP adapter (Stage D2).
//!
//! The command serves the assistant surface over the newline-delimited
//! JSON-RPC 2.0 stdio transport. Stdout carries the JSON-RPC responses only,
//! stderr carries the diagnostics, and the end of the input terminates the
//! server with the exit code 0.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use michelangelo_assistant::mcp::McpServer;
use michelangelo_assistant::policy::load_policy;
use michelangelo_assistant::SystemClock;

/// Exit code: the server terminated cleanly.
pub const EXIT_OK: u8 = 0;
/// Exit code: usage or IO error.
pub const EXIT_USAGE: u8 = 2;

/// Subcommands of `michelangelo mcp`.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Serve the assistant surface over the stdio transport.
    Serve {
        /// The workspace that owns the run store (default: the current
        /// directory).
        #[arg(long, value_name = "DIR")]
        workspace: Option<PathBuf>,
    },
}

/// Execute an `mcp` subcommand.
pub fn execute(command: &McpCommand) -> ExitCode {
    match command {
        McpCommand::Serve { workspace } => serve(workspace.as_deref()),
    }
}

/// Serve the assistant surface over the stdio transport.
fn serve(workspace: Option<&std::path::Path>) -> ExitCode {
    let root = match workspace {
        Some(path) => path.to_path_buf(),
        None => match std::env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                eprintln!("[michelangelo] the current directory cannot be read: {error}");
                return ExitCode::from(EXIT_USAGE);
            }
        },
    };
    let policy = match load_policy(&root) {
        Ok(policy) => policy,
        Err(error) => {
            eprintln!("[michelangelo] {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let mut server = match McpServer::new(root, policy, Box::new(SystemClock)) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("[michelangelo] {error}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    match server.serve(&mut input, &mut output) {
        Ok(()) => ExitCode::from(EXIT_OK),
        Err(error) => {
            eprintln!("[michelangelo] the MCP server stopped: {error}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}
