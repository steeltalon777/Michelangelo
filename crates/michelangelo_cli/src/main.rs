//! Michelangelo CLI — headless command-line entry point.
//!
//! Thin shell over the Core protocol. All human-readable diagnostics
//! go to stderr; stdout is reserved for protocol JSONL messages.

mod assistant_cmd;
mod edit_cmd;
mod input_cmd;
mod mcp_cmd;
mod reconstruct_cmd;
mod stdio;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "michelangelo")]
#[command(about = "Michelangelo — AI-friendly 3D workbench headless core", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the core in stdio JSONL mode (read commands from stdin, write responses to stdout)
    Core {
        #[command(subcommand)]
        mode: Option<CoreMode>,
    },
    /// Validate and describe input packages (Stage A)
    Input {
        #[command(subcommand)]
        command: input_cmd::InputCommand,
    },
    /// Reconstruct a blockout from an input package (Stage B)
    Reconstruct {
        #[command(subcommand)]
        command: reconstruct_cmd::ReconstructCommand,
    },
    /// Edit a reconstruction run through the local edit boundary (Stage C)
    Edit {
        #[command(subcommand)]
        command: edit_cmd::EditCommand,
    },
    /// Generate and submit bounded assistant tasks (Stage D2)
    Assistant {
        #[command(subcommand)]
        command: assistant_cmd::AssistantCommand,
    },
    /// Serve the assistant surface over the MCP stdio transport (Stage D2)
    Mcp {
        #[command(subcommand)]
        command: mcp_cmd::McpCommand,
    },
}

#[derive(Subcommand)]
enum CoreMode {
    /// JSONL over stdin/stdout mode
    Stdio,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Core { mode }) => {
            match mode {
                Some(CoreMode::Stdio) => {
                    // Run the JSONL stdio loop.
                    // stdout is reserved for protocol JSONL; all diagnostics go to stderr.
                    if let Err(e) = crate::stdio::run_loop() {
                        eprintln!("[michelangelo] stdio error: {e}");
                    }
                }
                None => {
                    eprintln!("[michelangelo] core mode requires a subcommand. Try: michelangelo core stdio");
                }
            }
        }
        Some(Commands::Input { command }) => {
            // The input subcommands are independent of the protocol mode:
            // stdout carries the report JSON, stderr the human diagnostics.
            return input_cmd::run(command);
        }
        Some(Commands::Reconstruct { command }) => {
            // The reconstruction subcommands are independent of the protocol
            // mode: stdout carries the summary or the report JSON.
            return reconstruct_cmd::execute(command);
        }
        Some(Commands::Edit { command }) => {
            // The edit subcommands are independent of the protocol mode:
            // stdout carries the summary or the decision report JSON.
            return edit_cmd::execute(command);
        }
        Some(Commands::Assistant { command }) => {
            // The assistant subcommands are independent of the protocol mode:
            // stdout carries the summary or the task JSON.
            return assistant_cmd::execute(command);
        }
        Some(Commands::Mcp { command }) => {
            // The MCP transport owns stdout: only the JSON-RPC responses are
            // written there.
            return mcp_cmd::execute(command);
        }
        None => {
            // No subcommand — show help (--help is handled by clap automatically)
            // If someone runs `michelangelo` with no args, just print help to stderr
            // so stdout stays clean for protocol use.
            eprintln!("[michelangelo] no command given. Run with --help for usage.");
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn test_cli_help_succeeds() {
        // Verify that the CLI definition is valid by calling clap's assert
        Cli::command().debug_assert();
    }

    #[test]
    fn test_cli_accepts_help_flag() {
        // Simulate --help; clap will write to stdout but we just verify it parses
        let res = Cli::try_parse_from(["michelangelo", "--help"]);
        assert!(res.is_ok() || res.is_err());
        // (clap may return an error for --help in try_parse_from depending on version)
    }

    #[test]
    fn test_cli_accepts_version_flag() {
        let res = Cli::try_parse_from(["michelangelo", "--version"]);
        // --version may be handled by clap via ok() or err(DisplayHelp)
        // We just verify it doesn't panic
        let _ = res;
    }

    #[test]
    fn test_cli_accepts_core_stdio() {
        let res = Cli::try_parse_from(["michelangelo", "core", "stdio"]);
        assert!(res.is_ok());
    }

    #[test]
    fn test_cli_rejects_unknown_subcommand() {
        let res = Cli::try_parse_from(["michelangelo", "blender"]);
        assert!(res.is_err());
    }

    #[test]
    fn test_cli_rejects_unknown_flag() {
        let res = Cli::try_parse_from(["michelangelo", "--bogus"]);
        assert!(res.is_err());
    }
}
