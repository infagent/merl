//! Durable reviewer-lane discovery, activity checkpoints and immutable delivery intent.
//!
//! A lane selects current open work in one repository. It never changes GitHub or
//! infers a host session. Metadata progress commits with intent before mail I/O.
//! Same-user configuration and the configured executables are trusted capabilities.

mod engine;
mod github;
mod outbox;
mod store;

use merl_mail::{Draft, Key, Mailbox};
use serde_json::Value;
use std::{error::Error, fmt, path::PathBuf, str::FromStr};

pub use store::Store;

/// Maximum required labels; selectors are a small conjunction, not a query language.
pub const MAX_LABELS: usize = 8;
/// Maximum provider requests in one pass, limiting work before mail dispatch resumes.
pub const MAX_REQUESTS: usize = 16;
/// Default minimum time between complete scans of one unit of work.
pub const POLL_MILLIS: i64 = 60_000;

/// A stable failure code and optional server-enforced retry deadline.
#[derive(Debug)]
pub struct LaneError {
    /// Machine-readable recovery identity.
    pub code: &'static str,
    /// Earliest retry time, in UTC milliseconds, when supplied by the provider.
    pub retry_at: Option<i64>,
    detail: &'static str,
}
impl fmt::Display for LaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl Error for LaneError {}
impl LaneError {
    /// Reports that a caller-supplied clock cannot provide a valid timestamp.
    #[must_use]
    pub fn clock_unavailable() -> Self {
        failure(
            "LANE_CLOCK",
            "UTC clock unavailable; no further source work attempted",
        )
    }
}
impl From<rusqlite::Error> for LaneError {
    fn from(_: rusqlite::Error) -> Self {
        failure("LANE_STORAGE", "lane storage operation failed")
    }
}
impl From<std::io::Error> for LaneError {
    fn from(_: std::io::Error) -> Self {
        failure("LANE_IO", "lane filesystem or process operation failed")
    }
}
fn failure(code: &'static str, detail: &'static str) -> LaneError {
    LaneError {
        code,
        detail,
        retry_at: None,
    }
}
fn input(detail: &'static str) -> LaneError {
    failure("LANE_INPUT", detail)
}
fn protocol() -> LaneError {
    failure(
        "LANE_GITHUB_PROTOCOL",
        "incomplete or invalid GitHub metadata; checkpoint retained",
    )
}

/// A durable lane name small enough to fit an idempotent mail ID with its sequence.
#[derive(Clone, Debug)]
pub struct Name(Key);
impl Name {
    /// Returns the validated stable name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl FromStr for Name {
    type Err = LaneError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() > 32 {
            return Err(input("lane names require 1..32 key characters"));
        }
        Ok(Self(s.parse().map_err(|_| {
            input("lane names require 1..32 key characters")
        })?))
    }
}

/// An explicit GitHub owner/repository pair; no URLs or arbitrary API endpoints.
#[derive(Clone, Debug)]
pub struct Repository(String);
impl Repository {
    /// Returns the validated owner/repository text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn parts(&self) -> (&str, &str) {
        self.0.split_once('/').expect("validated repository pair")
    }
}
impl FromStr for Repository {
    type Err = LaneError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts = s.split('/').collect::<Vec<_>>();
        if parts.len() != 2
            || parts.iter().any(|p| {
                p.is_empty()
                    || p.len() > 100
                    || *p == "."
                    || *p == ".."
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
        {
            return Err(input(
                "repository must be an explicit bounded owner/repository pair",
            ));
        }
        Ok(Self(s.into()))
    }
}

/// Required label names, matched as an AND selector without author suppression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels(Vec<String>);
impl Labels {
    /// Validates and sorts one to eight nonempty label names.
    ///
    /// ASCII case is ignored, matching ordinary GitHub label spelling. Other UTF-8
    /// bytes compare exactly. Control characters and names over 64 bytes are rejected.
    ///
    /// # Errors
    /// Returns `LANE_INPUT` for empty, duplicate or unbounded selectors.
    pub fn new(mut labels: Vec<String>) -> Result<Self, LaneError> {
        if !(1..=MAX_LABELS).contains(&labels.len())
            || labels
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 64 || s.chars().any(char::is_control))
        {
            return Err(input(
                "provide one to eight nonempty --label names, each up to 64 bytes",
            ));
        }
        for label in &mut labels {
            label.make_ascii_lowercase();
        }
        labels.sort();
        if labels.windows(2).any(|w| w[0] == w[1]) {
            return Err(input("duplicate required label"));
        }
        Ok(Self(labels))
    }
    /// Returns the canonical required label names.
    #[must_use]
    pub fn as_slice(&self) -> &[String] {
        &self.0
    }
}

/// Immutable repository/recipient identity and an explicitly validated selector.
#[derive(Clone, Debug)]
pub struct Configuration {
    /// Selected GitHub repository.
    pub repository: Repository,
    /// Durable registered Merl identity; session binding remains in the mailbox.
    pub agent: Key,
    /// Required labels.
    pub labels: Labels,
}

/// One fixed metadata query. Opaque connection cursors are never user query text.
#[derive(Clone, Debug)]
pub enum Query {
    /// Discover a page of current open matching work and its initial timeline tail.
    Discover {
        repository: Repository,
        labels: Labels,
        after: Option<String>,
    },
    /// Observe one PR's independent timeline and recurring inline-thread sweep.
    Observe {
        repository: Repository,
        number: i64,
        after: Option<String>,
        threads_after: Option<String>,
        resync: bool,
    },
}

/// Metadata from the fixed GitHub schema, deliberately represented as JSON.
///
/// The engine validates every required field and connection before committing
/// progress. Discussion bodies and author names are absent from the requests.
#[derive(Debug)]
pub struct Reply {
    /// Fixed-query GraphQL response document.
    pub document: Value,
    /// Provider-enforced cooldown even when a request returned usable data.
    pub not_before: i64,
}

/// Read-only GitHub capability; synthetic implementations control I/O and time.
pub trait GitHub {
    /// Fetches one bounded metadata page, sampling the clock after response I/O.
    ///
    /// # Errors
    /// Returns explicit provider/protocol/rate/cursor failures without advancing state.
    fn fetch(
        &mut self,
        query: &Query,
        clock: &impl Fn() -> Result<i64, LaneError>,
    ) -> Result<Reply, LaneError>;
}

/// Idempotent immutable mail publication, separate from source progress and host wake.
pub trait MailSink {
    /// Publishes the frozen ID and exact body or leaves intent retryable.
    ///
    /// # Errors
    /// Errors may follow an ambiguous publication; retry must use identical bytes.
    fn publish(&mut self, id: &Key, recipient: &Key, body: &str, now: i64)
    -> Result<(), LaneError>;
}
impl MailSink for Mailbox {
    fn publish(
        &mut self,
        id: &Key,
        recipient: &Key,
        body: &str,
        now: i64,
    ) -> Result<(), LaneError> {
        let from: Key = "merl-watch".parse().map_err(|_| protocol())?;
        self.register(&from).map_err(|_| {
            failure(
                "LANE_DELIVERY",
                "watcher sender registration failed; intent retained",
            )
        })?;
        let draft = Draft::new(
            id.clone(),
            from,
            recipient.clone(),
            "review",
            body.into(),
            None,
        )
        .map_err(|_| failure("LANE_DELIVERY", "invalid frozen mail; intent retained"))?;
        self.send(&draft, now).map_err(|_| {
            failure(
                "LANE_DELIVERY",
                "mail publication failed or conflicted; exact intent retained",
            )
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Lane {
    name: String,
    configuration: Configuration,
    generation: i64,
}

/// Filesystem configuration for the single supported read-only GitHub adapter.
#[derive(Debug)]
pub struct Gh {
    executable: PathBuf,
    home: PathBuf,
}
