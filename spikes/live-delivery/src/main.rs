use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use merl_live_delivery_spike::host::claude::{stdio_serve, tui_exec as claude_tui_exec};
use merl_live_delivery_spike::host::codex::{ExactThread, tui_exec};

#[derive(Debug, Parser)]
#[command(about = "Disposable Merl live-delivery feasibility spike")]
struct Cli {
    #[command(subcommand)]
    command: Host,
}

#[derive(Debug, Subcommand)]
enum Host {
    Claude(ClaudeArguments),
    ClaudeChannel(ClaudeChannelArguments),
    Codex(CodexArguments),
}

#[derive(Debug, Args)]
struct ClaudeArguments {
    #[arg(long, default_value = "claude")]
    program: OsString,
    #[arg(long, default_value = "merl-live-delivery")]
    server_name: String,
    #[arg(long)]
    message: String,
    #[arg(last = true, allow_hyphen_values = true)]
    arguments: Vec<OsString>,
}

#[derive(Debug, Args)]
struct ClaudeChannelArguments {
    #[arg(long)]
    message: String,
}

#[derive(Debug, Args)]
struct CodexArguments {
    #[arg(long, default_value = "codex")]
    program: OsString,
    #[arg(long)]
    thread: String,
    #[arg(last = true, allow_hyphen_values = true)]
    arguments: Vec<OsString>,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Host::Claude(arguments) => {
            let executable = match std::env::current_exe() {
                Ok(executable) => executable,
                Err(error) => {
                    eprintln!("could not resolve Claude channel executable: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let error = claude_tui_exec(
                arguments.program,
                &executable,
                &arguments.server_name,
                &arguments.message,
                &arguments.arguments,
            );
            eprintln!("could not launch stock Claude TUI with channel: {error}");
            ExitCode::FAILURE
        }
        Host::ClaudeChannel(arguments) => match stdio_serve(&arguments.message) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Claude channel stopped: {error}");
                ExitCode::FAILURE
            }
        },
        Host::Codex(arguments) => {
            let thread = match ExactThread::parse(&arguments.thread) {
                Ok(thread) => thread,
                Err(error) => {
                    eprintln!("live Codex delivery unavailable: {error}");
                    return ExitCode::from(2);
                }
            };
            let error = tui_exec(arguments.program, &thread, &arguments.arguments);
            eprintln!("could not launch stock Codex TUI for exact thread: {error}");
            ExitCode::FAILURE
        }
    }
}
