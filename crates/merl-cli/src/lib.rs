//! Public command-line boundary for durable local agent messaging.
//!
//! The CLI opens only an explicit mailbox or an opt-in saved user default.
//! Unknown or retired commands
//! fail before touching runtime storage; it contains no old-product data adapter.

mod init;
mod lane;
mod mail;
mod status;
mod team;

use serde_json::{Value, json};
use std::{error::Error, fmt};

/// A CLI failure with a stable machine-readable code.
#[derive(Debug)]
pub struct CliError {
    /// Stable error identity for scripts and agents.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}
impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for CliError {}
impl CliError {
    /// Returns the versioned non-interactive error envelope.
    #[must_use]
    pub fn as_json(&self) -> String {
        format!(
            "{}\n",
            json!({"schema":"merl.error/v1","code":self.code,"message":self.message})
        )
    }
}

/// Output routing for the binary and embedded CLI callers.
#[derive(Debug)]
pub enum CliResponse {
    /// Intentional stdout output.
    Success(String),
    /// An error rendered on stderr in human mode.
    HumanError(CliError),
    /// A versioned error rendered on stdout in JSON mode.
    JsonError(CliError),
}

/// Runs the messaging CLI with the node's UTC clock.
#[must_use]
pub fn run(arguments: &[String]) -> CliResponse {
    run_with_clock(arguments, &utc_now_millis)
}

/// Runs the CLI with a caller-supplied UTC-millisecond clock.
///
/// This boundary lets acceptance tests and embedded hosts control timestamps
/// without adding a caller-controlled timestamp option to the executable.
#[must_use]
pub fn run_with_clock(
    arguments: &[String],
    clock: &impl Fn() -> Result<i64, CliError>,
) -> CliResponse {
    let mut json_output = false;
    let result = if init::handles(arguments) {
        init::execute(arguments, &mut json_output)
    } else if status::handles(arguments) {
        status::execute(arguments, &mut json_output)
    } else if team::handles(arguments) {
        team::execute(arguments, &mut json_output)
    } else if mail::handles(arguments) {
        mail::execute(arguments, &mut json_output, clock)
    } else if lane::handles(arguments) {
        lane::execute(arguments, &mut json_output, clock)
    } else {
        root(arguments, &mut json_output)
    };
    match result {
        Ok(output) => CliResponse::Success(output),
        Err(e) if json_output => CliResponse::JsonError(e),
        Err(e) => CliResponse::HumanError(e),
    }
}

fn root(arguments: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut words = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--json" => *json_output = true,
            "--format" => {
                index += 1;
                match arguments.get(index).map(String::as_str) {
                    Some("json") => *json_output = true,
                    Some("human") => *json_output = false,
                    _ => return Err(invalid_input("format must be human or json")),
                }
            }
            "--help" | "-h" => words.push("help"),
            word => words.push(word),
        }
        index += 1;
    }
    if !words.is_empty() && words != ["help"] {
        return Err(CliError{code:"UNKNOWN_COMMAND",message:"Unknown command. Run merl help for the messaging interface. Retired product commands and runtime stores are not opened or migrated.".into()});
    }
    let usage = "merl <init|status|team|mail|lane> [operation] [options]";
    let summary = "Durable local messaging, reviewer lanes and wake-up of explicitly bound existing sessions.";
    if *json_output {
        render_json(
            &json!({"schema":"merl.help/v1","command":"","kind":"group","usage":usage,"summary":summary,"description":summary,"children":["mail","lane","init","status","team"],"commands":["mail","lane","init","status","team"],"arguments":[],"outcomes":[],"errors":[],"related":["mail","lane","init","status","team"],"example":"merl help mail --json","examples":["merl help mail --json"]}),
        )
    } else {
        Ok(format!(
            "{usage}\n{summary}\nCommands: mail, lane, init, status, team\nRun merl --version or merl help <group> <operation> --json.\n"
        ))
    }
}
fn utc_now_millis() -> Result<i64, CliError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid_input("system clock is before Unix epoch"))?
        .as_millis();
    i64::try_from(now).map_err(|_| invalid_input("system clock is outside supported range"))
}
fn invalid_input(message: &str) -> CliError {
    CliError {
        code: "INVALID_INPUT",
        message: message.into(),
    }
}
fn render_json(value: &Value) -> Result<String, CliError> {
    serde_json::to_string(value)
        .map(|s| format!("{s}\n"))
        .map_err(|e| CliError {
            code: "SERIALIZATION_ERROR",
            message: e.to_string(),
        })
}
