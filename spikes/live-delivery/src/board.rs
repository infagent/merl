use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use wait_timeout::ChildExt;

use crate::SessionId;

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("could not execute the Merl board command: {0}")]
    Execute(#[from] std::io::Error),
    #[error("Merl results query timed out after {0:?}")]
    Timeout(Duration),
    #[error("Merl results query failed: {0}")]
    Query(String),
    #[error("Merl results query returned malformed structured output: {0}")]
    Malformed(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResultItem {
    pub request_id: String,
    pub requester_session_id: SessionId,
    pub answer: Value,
}

pub trait Board {
    /// Returns the canonical unread results visible to a Merl session.
    ///
    /// # Errors
    ///
    /// Returns [`BoardError`] when canonical state cannot be queried or decoded.
    fn unread_results(&mut self, session_id: &SessionId) -> Result<Vec<ResultItem>, BoardError>;
}

#[derive(Debug)]
pub struct CliBoard {
    program: OsString,
    prefix_arguments: Vec<OsString>,
    working_directory: PathBuf,
    home: PathBuf,
    timeout: Duration,
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
        Self::with_timeout(
            program,
            prefix_arguments,
            working_directory,
            home,
            Duration::from_secs(10),
        )
    }

    pub fn with_timeout<I, S>(
        program: impl AsRef<OsStr>,
        prefix_arguments: I,
        working_directory: impl AsRef<Path>,
        home: impl AsRef<Path>,
        timeout: Duration,
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
            timeout,
        }
    }
}

impl Board for CliBoard {
    fn unread_results(&mut self, session_id: &SessionId) -> Result<Vec<ResultItem>, BoardError> {
        let mut command = Command::new(&self.program);
        command
            .current_dir(&self.working_directory)
            .env("UV_CACHE_DIR", self.home.join(".uv-cache"))
            .args(&self.prefix_arguments)
            .arg("--home")
            .arg(&self.home)
            .args(["results", "--session", session_id.as_str(), "--json"]);
        let output = output_with_timeout(&mut command, self.timeout)?
            .ok_or(BoardError::Timeout(self.timeout))?;
        if !output.status.success() {
            return Err(BoardError::Query(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| BoardError::Malformed(error.to_string()))
    }
}

pub(crate) fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<Option<Output>, std::io::Error> {
    // Regular files avoid blocking when a killed child leaves a grandchild holding inherited
    // stdout or stderr descriptors open.
    let mut stdout_file = tempfile::tempfile()?;
    let mut stderr_file = tempfile::tempfile()?;
    let mut child = command
        .stdout(Stdio::from(stdout_file.try_clone()?))
        .stderr(Stdio::from(stderr_file.try_clone()?))
        .spawn()?;
    let status = child.wait_timeout(timeout)?;
    if status.is_none() {
        if let Err(error) = child.kill() {
            if error.kind() != std::io::ErrorKind::InvalidInput {
                return Err(error);
            }
        }
        child.wait()?;
    }
    stdout_file.seek(SeekFrom::Start(0))?;
    stderr_file.seek(SeekFrom::Start(0))?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    stdout_file.read_to_end(&mut stdout)?;
    stderr_file.read_to_end(&mut stderr)?;
    Ok(status.map(|status| Output {
        status,
        stdout,
        stderr,
    }))
}
