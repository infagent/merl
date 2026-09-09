use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("could not execute the Merl board command: {0}")]
    Execute(#[from] std::io::Error),
    #[error("Merl results query failed: {0}")]
    Query(String),
    #[error("Merl results query returned malformed structured output: {0}")]
    Malformed(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResultItem {
    pub request_id: String,
    pub requester_session_id: String,
    pub answer: Value,
}

impl ResultItem {
    #[must_use]
    pub fn synthetic(request_id: &str, requester_session_id: &str, summary: &str) -> Self {
        Self {
            request_id: request_id.to_owned(),
            requester_session_id: requester_session_id.to_owned(),
            answer: json!({"summary": summary}),
        }
    }
}

pub trait Board {
    /// Returns the canonical unread results visible to a Merl session.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError`] when canonical state cannot be queried or decoded.
    fn unread_results(&mut self, session_id: &str) -> Result<Vec<ResultItem>, BoardError>;
}

#[derive(Debug)]
pub struct CliBoard {
    program: OsString,
    prefix_arguments: Vec<OsString>,
    working_directory: PathBuf,
    home: PathBuf,
}

impl CliBoard {
    pub fn new<I, S>(
        program: impl AsRef<OsStr>,
        prefix_arguments: I,
        working_directory: impl AsRef<Path>,
        home: impl AsRef<Path>,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            program: program.as_ref().to_owned(),
            prefix_arguments: prefix_arguments
                .into_iter()
                .map(|argument| argument.as_ref().to_owned())
                .collect(),
            working_directory: working_directory.as_ref().to_owned(),
            home: home.as_ref().to_owned(),
        }
    }
}

impl Board for CliBoard {
    fn unread_results(&mut self, session_id: &str) -> Result<Vec<ResultItem>, BoardError> {
        let output = Command::new(&self.program)
            .current_dir(&self.working_directory)
            .env("UV_CACHE_DIR", self.home.join(".uv-cache"))
            .args(&self.prefix_arguments)
            .arg("--home")
            .arg(&self.home)
            .args(["results", "--session", session_id, "--json"])
            .output()?;
        if !output.status.success() {
            return Err(BoardError::Query(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| BoardError::Malformed(error.to_string()))
    }
}

#[derive(Debug)]
pub struct MemoryBoard {
    results: Vec<ResultItem>,
}

impl MemoryBoard {
    #[must_use]
    pub fn new(results: Vec<ResultItem>) -> Self {
        Self { results }
    }
}

impl Board for MemoryBoard {
    fn unread_results(&mut self, _session_id: &str) -> Result<Vec<ResultItem>, BoardError> {
        Ok(self.results.clone())
    }
}
