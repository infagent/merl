use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use uuid::Uuid;

use crate::{DeliveryError, HostDelivery};

#[derive(Debug)]
pub struct ExactThread {
    value: String,
}

impl ExactThread {
    /// Accepts only a concrete Codex UUID; names, repository lookup, and recency are excluded.
    ///
    /// # Errors
    ///
    /// Returns a description when `value` is not a UUID.
    pub fn parse(value: &str) -> Result<Self, String> {
        Uuid::parse_str(value)
            .map_err(|error| format!("Codex thread must be an exact UUID: {error}"))?;
        Ok(Self {
            value: value.to_owned(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

#[derive(Debug)]
pub struct QueueDelivery {
    program: OsString,
    thread: ExactThread,
}

impl QueueDelivery {
    pub fn new(program: impl AsRef<OsStr>, thread: ExactThread) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            thread,
        }
    }
}

impl HostDelivery for QueueDelivery {
    fn deliver(&mut self, message: &str) -> Result<(), DeliveryError> {
        let output = Command::new(&self.program)
            .args([
                "queue",
                "--thread",
                self.thread.as_str(),
                "--message",
                message,
            ])
            .stdout(Stdio::null())
            .output()
            .map_err(|error| DeliveryError::Unavailable(error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }
        Err(DeliveryError::Unavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

/// Replaces the wrapper process with the stock Codex TUI resumed by exact UUID.
///
/// `exec` leaves terminal file descriptors, environment, signal handling, terminal sizing, and
/// final exit status under Codex's direct control. This function returns only when `exec` fails.
pub fn tui_exec(
    program: impl AsRef<OsStr>,
    thread: &ExactThread,
    user_arguments: &[OsString],
) -> io::Error {
    Command::new(program)
        .arg("resume")
        .arg(thread.as_str())
        .args(user_arguments)
        .exec()
}
