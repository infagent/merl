use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use merl_live_delivery_spike::host::claude::{
    SessionBinding, session_initialize, stdio_live_serve, stdio_serve, tui_exec as claude_tui_exec,
};
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
    #[arg(long, default_value = "merl")]
    merl_program: OsString,
    #[arg(long)]
    merl_home: Option<PathBuf>,
    #[arg(long)]
    message: Option<String>,
    #[arg(last = true, allow_hyphen_values = true)]
    arguments: Vec<OsString>,
}

#[derive(Debug, Args)]
struct ClaudeChannelArguments {
    #[arg(long)]
    message: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    merl_program: Option<OsString>,
    #[arg(long)]
    merl_home: Option<PathBuf>,
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
            if let Some(message) = arguments.message {
                let error = merl_live_delivery_spike::host::claude::tui_message_exec(
                    arguments.program,
                    &executable,
                    &arguments.server_name,
                    &message,
                    &arguments.arguments,
                );
                eprintln!("could not launch stock Claude TUI with channel: {error}");
                return ExitCode::FAILURE;
            }
            let Some(home) = arguments.merl_home else {
                eprintln!(
                    "live delivery unavailable; pass --merl-home or use the existing Merl pull commands"
                );
                return ExitCode::from(2);
            };
            let binding = match session_initialize(&arguments.merl_program, &home) {
                Ok(binding) => binding,
                Err(error) => {
                    eprintln!(
                        "live delivery unavailable; starting stock Claude without a channel; use the existing Merl pull commands: {error}"
                    );
                    let error = merl_live_delivery_spike::host::claude::tui_plain_exec(
                        arguments.program,
                        &arguments.arguments,
                    );
                    eprintln!("could not launch stock Claude TUI: {error}");
                    return ExitCode::FAILURE;
                }
            };
            let error = claude_tui_exec(
                arguments.program,
                &executable,
                &arguments.server_name,
                &binding,
                &arguments.merl_program,
                &home,
                &arguments.arguments,
            );
            eprintln!("could not launch stock Claude TUI with channel: {error}");
            ExitCode::FAILURE
        }
        Host::ClaudeChannel(arguments) => match channel_serve(arguments) {
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

fn channel_serve(arguments: ClaudeChannelArguments) -> Result<(), String> {
    match (
        arguments.message,
        arguments.session,
        arguments.merl_program,
        arguments.merl_home,
    ) {
        (Some(message), None, None, None) => stdio_serve(&message),
        (None, Some(session), Some(program), Some(home)) => stdio_live_serve(
            &SessionBinding {
                session_id: session,
            },
            &program,
            &home,
        ),
        _ => Err(
            "provide either --message or all of --session, --merl-program, and --merl-home"
                .to_owned(),
        ),
    }
}
