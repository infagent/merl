#[path = "board.rs"]
pub mod board;
pub mod host;

use std::collections::HashSet;

use board::{Board, BoardError, ResultItem};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

const ENVELOPE_HEADER: &str =
    "MERL_LIVE_RESULT_V1\nprovenance=untrusted-agent-result\nauthorization=none\n";

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error("live delivery unavailable: {0}")]
    Unavailable(String),
}

pub trait HostDelivery {
    /// Attempts to inject one fixed Merl envelope at a host boundary.
    ///
    /// # Errors
    ///
    /// Returns [`DeliveryError`] when the host cannot accept the envelope.
    fn deliver(&mut self, message: &str) -> Result<(), DeliveryError>;
}

#[derive(Debug, Eq, PartialEq)]
pub struct DeliveryFailure {
    pub request_id: String,
    pub error: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Reconciliation {
    pub delivered: usize,
    pub failed: Vec<DeliveryFailure>,
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Board(#[from] BoardError),
}

pub struct DeliveryCore<B> {
    board: B,
    attempted_request_ids: HashSet<String>,
}

impl<B: Board> DeliveryCore<B> {
    pub fn new(board: B) -> Self {
        Self {
            board,
            attempted_request_ids: HashSet::new(),
        }
    }

    /// Re-reads canonical board state and attempts each matching request once.
    ///
    /// # Errors
    ///
    /// Returns [`ReconcileError`] when the board cannot provide canonical state. Host delivery
    /// failures are reported in the returned [`Reconciliation`] and deliberately fail toward the
    /// durable pull path.
    pub fn reconcile<H: HostDelivery>(
        &mut self,
        bound_session_id: &SessionId,
        host: &mut H,
    ) -> Result<Reconciliation, ReconcileError> {
        let results = self.board.unread_results(bound_session_id)?;
        let mut reconciliation = Reconciliation::default();
        for result in results {
            if &result.requester_session_id != bound_session_id
                || !self.attempted_request_ids.insert(result.request_id.clone())
            {
                continue;
            }

            match host.deliver(&envelope_render(&result)) {
                Ok(()) => reconciliation.delivered += 1,
                Err(error) => reconciliation.failed.push(DeliveryFailure {
                    request_id: result.request_id,
                    error: error.to_string(),
                }),
            }
        }
        Ok(reconciliation)
    }
}

fn envelope_render(result: &ResultItem) -> String {
    let payload = json!({
        "answer": result.answer,
        "request_id": result.request_id,
        "requester_session_id": result.requester_session_id,
    })
    .to_string();
    format!(
        "{ENVELOPE_HEADER}request_id={}\nrequester_session_id={}\ncontent_type=application/json\ncontent_length={}\n\n{payload}",
        result.request_id,
        result.requester_session_id,
        payload.len()
    )
}
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
