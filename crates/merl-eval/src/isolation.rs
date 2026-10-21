//! Network-denied processes with only explicit input files and fresh writable state.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    error::Error,
    fmt, fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

/// Hard limits for one isolated adapter invocation.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLimits {
    /// Maximum serialized request bytes.
    pub request_bytes: usize,
    /// Maximum response bytes.
    pub response_bytes: usize,
    /// Maximum executable snapshot bytes, excluding model weights.
    pub program_bytes: usize,
    /// Maximum wall time before the process namespace is stopped.
    pub seconds: u16,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        // These are transport ceilings, not token estimates. Models and their
        // weights belong behind the evaluator transport, not in an executable.
        Self {
            request_bytes: 1_048_576,
            response_bytes: 262_144,
            program_bytes: 67_108_864,
            seconds: 30,
        }
    }
}

/// Content hashes for one worker invocation; no source or oracle prose is logged.
#[derive(Clone, Debug, Serialize)]
pub struct ProcessCallRecord {
    /// Exact pinned executable.
    pub program_sha256: String,
    /// Exact stdin bytes.
    pub request_sha256: String,
    /// Exact stdout bytes when the transport completed.
    pub response_sha256: Option<String>,
    /// Explicitly mounted files, including the grader's runtime answer.
    pub input_sha256: BTreeMap<String, String>,
    /// Transport completion; protocol validation is reported by its caller.
    pub transport_complete: bool,
    /// Stable transport failure class; protocol rejection is recorded by the slot.
    pub error_code: Option<&'static str>,
    /// Protocol operation, without request prose.
    pub kind: Option<String>,
    /// Approved scoring identity for answer/grader calls.
    pub unit_id: Option<String>,
    /// Paired identity for model calls.
    pub trial_id: Option<String>,
}

/// A pinned adapter executable; every call gets a separate process namespace.
pub struct IsolatedProgram {
    bytes: Vec<u8>,
    digest: String,
    limits: ProcessLimits,
}

impl fmt::Debug for IsolatedProgram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolatedProgram")
            .field("sha256", &self.digest)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl IsolatedProgram {
    /// Snapshots an adapter so replacing its path cannot change later calls.
    ///
    /// # Errors
    /// Rejects unreadable or oversized executables and zero transport limits.
    pub fn new(program: &Path, limits: ProcessLimits) -> Result<Self, IsolationError> {
        if limits.request_bytes == 0
            || limits.response_bytes == 0
            || limits.program_bytes == 0
            || limits.seconds == 0
        {
            return Err(IsolationError::InvalidLimits);
        }
        let bytes = read_bounded(program, limits.program_bytes)?;
        let digest = hex_digest(&bytes);
        Ok(Self {
            bytes,
            digest,
            limits,
        })
    }

    /// SHA-256 of the exact executable bytes used for all calls.
    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.digest
    }

    /// Runs a JSON protocol with only explicit files mounted at `/input`.
    ///
    /// Neither the host home, package, sibling stages nor ambient environment is
    /// inherited. Networking is disabled. Failure to establish isolation is an
    /// execution failure; there is no unrestricted fallback.
    ///
    /// # Errors
    /// Rejects invalid filenames, unavailable isolation, transport overruns,
    /// process failure, timeout, or a non-JSON response.
    pub fn call(
        &self,
        inputs: &BTreeMap<String, Vec<u8>>,
        request: &Value,
    ) -> Result<Value, IsolationError> {
        let bytes = serde_json::to_vec(request).map_err(|_| IsolationError::InvalidResponse)?;
        let response = self.call_bytes(inputs, &bytes)?;
        serde_json::from_slice(&response).map_err(|_| IsolationError::InvalidResponse)
    }

    /// Records hashes even when an invocation fails, then returns its JSON result.
    ///
    /// # Errors
    /// Returns the same transport errors as [`Self::call`].
    pub fn recorded_call(
        &self,
        inputs: &BTreeMap<String, Vec<u8>>,
        request: &Value,
        records: &mut Vec<ProcessCallRecord>,
    ) -> Result<Value, IsolationError> {
        let bytes = serde_json::to_vec(request).map_err(|_| IsolationError::InvalidResponse)?;
        let response = self.call_bytes(inputs, &bytes);
        records.push(ProcessCallRecord {
            program_sha256: self.digest.clone(),
            request_sha256: hex_digest(&bytes),
            response_sha256: response.as_ref().ok().map(|b| hex_digest(b)),
            input_sha256: inputs
                .iter()
                .map(|(name, bytes)| (name.clone(), hex_digest(bytes)))
                .collect(),
            transport_complete: response.is_ok(),
            error_code: response.as_ref().err().map(IsolationError::code),
            kind: request["kind"].as_str().map(ToOwned::to_owned),
            unit_id: request["unit_id"]
                .as_str()
                .or_else(|| request["question_id"].as_str())
                .map(ToOwned::to_owned),
            trial_id: request["trial"]["id"].as_str().map(ToOwned::to_owned),
        });
        serde_json::from_slice(&response?).map_err(|_| IsolationError::InvalidResponse)
    }

    /// Runs a bounded byte protocol in a fresh isolated process.
    ///
    /// # Errors
    /// Returns the same execution and transport errors as [`Self::call`].
    pub fn call_bytes(
        &self,
        inputs: &BTreeMap<String, Vec<u8>>,
        request: &[u8],
    ) -> Result<Vec<u8>, IsolationError> {
        if request.len() > self.limits.request_bytes {
            return Err(IsolationError::RequestTooLarge);
        }
        if !cfg!(target_os = "linux")
            || !Path::new("/usr/bin/bwrap").is_file()
            || !Path::new("/usr/bin/timeout").is_file()
        {
            return Err(IsolationError::Unavailable);
        }
        // A unit stage has at most six files. Bound the public staging API too.
        if inputs.len() > 6
            || inputs
                .values()
                .try_fold(0usize, |sum, b| sum.checked_add(b.len()))
                .is_none_or(|sum| sum > 67_108_864)
        {
            return Err(IsolationError::RequestTooLarge);
        }
        for name in inputs.keys() {
            let mut components = Path::new(name).components();
            if name.len() > 128
                || !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
            {
                return Err(IsolationError::InvalidInputName);
            }
        }
        let workspace = Workspace::new()?;
        let input = workspace.0.join("inputs");
        fs::create_dir(&input)?;
        for (name, bytes) in inputs {
            fs::write(input.join(name), bytes)?;
        }
        let executable = workspace.0.join("program");
        fs::write(&executable, &self.bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o500))?;
        }
        exchange(
            spawn_worker(&input, &executable, self.limits)?,
            request,
            self.limits.response_bytes,
        )
    }
}

fn spawn_worker(
    input: &Path,
    executable: &Path,
    limits: ProcessLimits,
) -> Result<std::process::Child, IsolationError> {
    // timeout bounds even an adapter that fills stdout before reading stdin.
    // Killing the namespace supervisor also stops its descendant processes.
    let child = Command::new("/usr/bin/timeout")
        .args([
            "--signal=KILL",
            &format!("{}s", limits.seconds),
            "/usr/bin/bwrap",
            "--unshare-all",
            "--unshare-user",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/bin",
            "/bin",
            "--symlink",
            "usr/lib",
            "/lib",
            "--symlink",
            "usr/lib64",
            "/lib64",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/work",
            "--chdir",
            "/work",
            "--clearenv",
            "--setenv",
            "PATH",
            "/usr/bin:/bin",
            "--ro-bind",
        ])
        .arg(input)
        .arg("/input")
        .arg("--ro-bind")
        .arg(executable)
        .arg("/runner")
        .args(["--", "/runner"])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(child)
}
fn exchange(
    mut child: std::process::Child,
    request: &[u8],
    response_limit: usize,
) -> Result<Vec<u8>, IsolationError> {
    let input_result = child
        .stdin
        .take()
        .ok_or(IsolationError::ProcessFailed)?
        .write_all(request);
    let mut response = Vec::new();
    let limit = u64::try_from(response_limit).map_err(|_| IsolationError::InvalidLimits)?;
    let read_result = child
        .stdout
        .take()
        .ok_or(IsolationError::ProcessFailed)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut response);
    if response.len() > response_limit || input_result.is_err() || read_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        if response.len() > response_limit {
            return Err(IsolationError::ResponseTooLarge);
        }
        return Err(IsolationError::ProcessFailed);
    }
    let status = child.wait()?;
    if !status.success() {
        #[cfg(unix)]
        let killed = {
            use std::os::unix::process::ExitStatusExt as _;
            status.signal() == Some(9)
        };
        #[cfg(not(unix))]
        let killed = false;
        return Err(if killed || matches!(status.code(), Some(124 | 137)) {
            IsolationError::Timeout
        } else {
            IsolationError::ProcessFailed
        });
    }
    Ok(response)
}

/// Stable execution failure categories; adapter output never appears in errors.
#[derive(Debug)]
pub enum IsolationError {
    /// Invalid transport ceilings.
    InvalidLimits,
    /// File input name is not a single safe path component.
    InvalidInputName,
    /// The runtime cannot enforce Linux filesystem and network isolation.
    Unavailable,
    /// Serialized input exceeds the configured bound.
    RequestTooLarge,
    /// Program or output exceeds its configured bound.
    ResponseTooLarge,
    /// Adapter or sandbox failed.
    ProcessFailed,
    /// Execution exceeded its time limit.
    Timeout,
    /// Protocol response is not valid JSON.
    InvalidResponse,
    /// Local runtime I/O failed.
    Io(std::io::Error),
}

impl IsolationError {
    /// Stable non-sensitive failure code for execution ledgers.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidLimits => "invalid_limits",
            Self::InvalidInputName => "invalid_input_name",
            Self::Unavailable => "isolation_unavailable",
            Self::RequestTooLarge => "request_limit",
            Self::ResponseTooLarge => "response_limit",
            Self::ProcessFailed => "process_failed",
            Self::Timeout => "timeout",
            Self::InvalidResponse => "invalid_json",
            Self::Io(_) => "io_failure",
        }
    }
}
impl fmt::Display for IsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidLimits => "invalid isolated process limits",
            Self::InvalidInputName => "invalid isolated input filename",
            Self::Unavailable => "Linux bubblewrap isolation is unavailable",
            Self::RequestTooLarge => "isolated request exceeds byte limit",
            Self::ResponseTooLarge => "isolated output exceeds byte limit",
            Self::ProcessFailed => "isolated adapter or sandbox failed",
            Self::Timeout => "isolated adapter exceeded time limit",
            Self::InvalidResponse => "invalid isolated adapter JSON",
            Self::Io(_) => "isolated process I/O failed",
        })
    }
}
impl Error for IsolationError {}
impl From<std::io::Error> for IsolationError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Result<Self, IsolationError> {
        for index in 0..1_000 {
            let path = std::env::temp_dir()
                .join(format!("merl-eval-worker-{}-{index}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                    }
                    return Ok(Self(path));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error.into()),
            }
        }
        Err(IsolationError::ProcessFailed)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, IsolationError> {
    if !fs::metadata(path)?.is_file() {
        return Err(IsolationError::ProcessFailed);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(
            u64::try_from(limit)
                .map_err(|_| IsolationError::InvalidLimits)?
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(IsolationError::ResponseTooLarge);
    }
    Ok(bytes)
}

pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for byte in Sha256::digest(bytes) {
        let _ = write!(text, "{byte:02x}");
    }
    text
}
