use std::ffi::OsString;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use merl_live_delivery_spike::host::codex::{ExactThread, tui_exec};

#[derive(Debug, Parser)]
#[command(about = "Disposable Merl live-delivery feasibility spike")]
struct Cli {
    #[command(subcommand)]
    command: Host,
}

#[derive(Debug, Subcommand)]
enum Host {
    Codex(CodexArguments),
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
    let Cli {
        command: Host::Codex(arguments),
    } = Cli::parse();
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
