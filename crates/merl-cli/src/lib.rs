//! The public command-line boundary for local Merl projects.

use std::{error::Error, fmt, path::Path};

use merl_core::ProjectId;
use merl_store::{Store, StoreError};
use serde_json::{Value, json};

/// A CLI failure with a stable machine-readable code.
#[derive(Debug)]
pub struct CliError {
    /// Stable error identity for scripts and agents.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for CliError {}

impl CliError {
    /// Returns one versioned error envelope for a non-interactive caller.
    #[must_use]
    pub fn as_json(&self) -> String {
        format!(
            "{}\n",
            json!({
                "schema": "merl.error/v1",
                "code": self.code,
                "message": self.message
            })
        )
    }
}

impl From<StoreError> for CliError {
    fn from(error: StoreError) -> Self {
        let code = match error {
            StoreError::UnsupportedSchema(_) => "UNSUPPORTED_SCHEMA",
            StoreError::ProjectMissing => "PROJECT_NOT_FOUND",
            StoreError::InvalidBatch => "INVALID_INPUT",
            StoreError::PayloadMissing => "PAYLOAD_NOT_FOUND",
            StoreError::CorruptHistory => "CORRUPT_HISTORY",
            StoreError::Storage(_) => "STORAGE_ERROR",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

/// The command result and the output mode selected by the CLI parser.
#[derive(Debug)]
pub enum CliResponse {
    /// Text for standard output after a successful command.
    Success(String),
    /// An error for standard error in human mode.
    HumanError(CliError),
    /// An error for standard output in JSON mode.
    JsonError(CliError),
}

/// Parses and runs one command without network access.
///
/// The parser keeps the chosen output mode with an error, so the executable
/// does not have to inspect the arguments again after a failure.
#[must_use]
pub fn run(arguments: &[String]) -> CliResponse {
    let mut json_output = false;
    match execute(arguments, &mut json_output) {
        Ok(output) => CliResponse::Success(output),
        Err(error) if json_output => CliResponse::JsonError(error),
        Err(error) => CliResponse::HumanError(error),
    }
}

fn execute(arguments: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut positional = Vec::new();
    let mut database = None;
    let mut id = None;
    let mut project = None;
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--format" => {
                index += 1;
                let format = arguments.get(index).ok_or_else(missing_value)?;
                *json_output = match format.as_str() {
                    "json" => true,
                    "human" => false,
                    _ => return Err(invalid_input("format must be human or json")),
                };
            }
            "--json" => *json_output = true,
            "--database" => {
                index += 1;
                database = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--id" => {
                index += 1;
                id = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--project" => {
                index += 1;
                project = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--help" | "-h" => positional.push("help"),
            option if option.starts_with('-') => {
                return Err(invalid_input(&format!("unknown option {option}")));
            }
            word => positional.push(word),
        }
        index += 1;
    }

    match positional.as_slice() {
        [] | ["help"] => help("", *json_output),
        ["help", "project"] => help("project", *json_output),
        ["help", "project", "init"] => help("project init", *json_output),
        ["help", "project", "revision"] => help("project revision", *json_output),
        ["project", "init"] => {
            let id = parse_project(id.ok_or_else(|| invalid_input("--id is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let mut store = Store::open(Path::new(path))?;
            store.create_project(&id)?;
            result("project.init", &id, 0, *json_output)
        }
        ["project", "revision"] => {
            let id = parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let store = Store::open(Path::new(path))?;
            let revision = store.project_revision(&id)?;
            result("project.revision", &id, revision.get(), *json_output)
        }
        _ => Err(invalid_input("unknown command; run `merl help`")),
    }
}

fn missing_value() -> CliError {
    invalid_input("option needs a value")
}

fn invalid_input(message: &str) -> CliError {
    CliError {
        code: "INVALID_INPUT",
        message: message.to_owned(),
    }
}

fn parse_project(value: &str) -> Result<ProjectId, CliError> {
    ProjectId::try_from(value).map_err(|error| CliError {
        code: "INVALID_ID",
        message: error.to_string(),
    })
}

fn result(
    action: &str,
    project: &ProjectId,
    revision: u64,
    json_output: bool,
) -> Result<String, CliError> {
    if json_output {
        render_json(&json!({
            "schema": "merl.result/v1",
            "action": action,
            "outcome": "accepted",
            "project": project.as_str(),
            "revision": revision,
            "objects": [],
            "warnings": []
        }))
    } else {
        Ok(format!("{action}: {project} at revision {revision}\n"))
    }
}

fn help(command: &str, json_output: bool) -> Result<String, CliError> {
    let (usage, summary, related) = match command {
        "" => (
            "merl <group> <command>",
            "Groups: project. Run `merl help project` for commands.",
            vec!["project"],
        ),
        "project" => (
            "merl project <command>",
            "Commands: init, revision.",
            vec!["project init", "project revision"],
        ),
        "project init" => (
            "merl project init --id <id> --database <path> [--format json]",
            "Create a local project at revision zero.",
            vec!["project revision"],
        ),
        "project revision" => (
            "merl project revision --project <id> --database <path> [--format json]",
            "Read the accepted project revision.",
            vec!["project init"],
        ),
        _ => return Err(invalid_input("unknown help topic")),
    };
    if json_output {
        render_json(&json!({
            "schema": "merl.help/v1",
            "command": command,
            "usage": usage,
            "summary": summary,
            "related": related,
            "errors": ["INVALID_INPUT", "INVALID_ID", "PROJECT_NOT_FOUND", "UNSUPPORTED_SCHEMA", "STORAGE_ERROR"]
        }))
    } else {
        Ok(format!("{usage}\n{summary}\n"))
    }
}

fn render_json(value: &Value) -> Result<String, CliError> {
    serde_json::to_string(&value)
        .map(|encoded| format!("{encoded}\n"))
        .map_err(|error| CliError {
            code: "SERIALIZATION_ERROR",
            message: error.to_string(),
        })
}
