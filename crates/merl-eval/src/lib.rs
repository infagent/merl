//! Causal inputs for comparing Merl with ordinary Issue-reading methods.

use std::{collections::HashMap, error::Error, fmt, fmt::Write as _};

use merl_corpus::fixture::{
    Fixture, HistoryFidelity, Observation, ObservationKind, ValidationError, available_body_at,
    require_exact_source_bodies_through, require_unambiguous_order_through, validate,
};
use serde::Serialize;

mod adapters;
mod preparation;
mod runner;

pub use adapters::{CliMerlSurface, MerlTrialArtifact, ProcessModelAdapter, ProcessScorer};
pub use preparation::{
    CompilerArtifacts, MerlPreparationRecord, PreparationExpectation, verify_preparation,
};

pub use runner::{
    AnswerAction, AnswerRequest, AnswerResponse, BenchmarkConfig, BenchmarkError, BenchmarkMethod,
    BenchmarkReport, BenchmarkRunner, BreakEven, EvaluationQuestion, MerlPrepared, MerlSurface,
    MethodStats, ModelAdapter, RandomnessControl, Score, Scorer, SummaryRequest, SummaryResponse,
    SummarySource, TokenUsage, TrialIdentity, TrialReport,
};

/// One of the reading methods in the first-release comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderMethod {
    /// Give the reader the entire visible Issue thread.
    RawHistory,
    /// Give the opening source and a bounded recent window, with older sources searchable.
    RecentRetrieval,
}

/// A source the reader can request through the retrieval tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchableSource {
    /// Stable provider entity identity.
    pub provider_id: String,
    /// Latest version visible at the chosen cutoff.
    pub version_id: String,
    /// Observation sequence for that version.
    pub observation: u64,
    /// Exact source body, withheld until a search result is requested.
    pub body: String,
}

/// Causal input for one reader at one source-observation cutoff.
#[derive(Clone, Debug)]
pub struct ReaderInput {
    /// Reading method that produced this input.
    pub method: ReaderMethod,
    /// Inclusive observation cutoff.
    pub cutoff: u64,
    /// Exact text placed into the initial reader context.
    pub context: String,
    /// Older versions available only through explicit retrieval.
    pub searchable: Vec<SearchableSource>,
    /// Declared limits on what the reader could know at this cutoff.
    pub fidelity: ReaderFidelity,
}

/// Source gaps carried into the reader and benchmark report.
#[derive(Clone, Debug, Serialize)]
pub struct ReaderFidelity {
    /// Corpus-level history class; it does not grant access to any particular body.
    pub history: HistoryFidelity,
    /// Observations whose bytes were unavailable at this cutoff.
    pub missing_bodies: Vec<u64>,
    /// Whether the fixture can reproduce the full source history exactly.
    pub exact_replay: bool,
}

/// A fixture cannot support the requested causal reader input.
#[derive(Debug)]
pub enum InputError {
    /// The fixture violates its own schema or ordering contract.
    Fixture(ValidationError),
    /// Source history cannot establish the exact bytes available at this cutoff.
    NonCausalHistory,
    /// A retrieval window must contain at least one recent source.
    EmptyRecentWindow,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fixture(error) => write!(f, "invalid evaluation fixture: {error}"),
            Self::NonCausalHistory => f.write_str("exact causal source history is unavailable"),
            Self::EmptyRecentWindow => f.write_str("recent retrieval needs a nonzero window"),
        }
    }
}

impl Error for InputError {}

impl From<ValidationError> for InputError {
    fn from(error: ValidationError) -> Self {
        Self::Fixture(error)
    }
}

/// Builds a reader context without exposing later edits or comments.
///
/// # Errors
/// Rejects invalid fixtures, ambiguous source order, or a zero recent-window
/// size. Unavailable bytes appear as gaps; terminal bodies never fill them.
#[expect(
    clippy::too_many_lines,
    reason = "one causal reader selection keeps source availability and disclosure together"
)]
pub fn prepare_reader_input(
    fixture: &Fixture,
    cutoff: u64,
    method: ReaderMethod,
    recent_window: usize,
) -> Result<ReaderInput, InputError> {
    validate(fixture)?;
    require_unambiguous_order_through(fixture, cutoff)?;
    if method == ReaderMethod::RecentRetrieval && recent_window == 0 {
        return Err(InputError::EmptyRecentWindow);
    }

    let mut visible = Vec::<&Observation>::new();
    let mut entity_position = HashMap::<&str, usize>::new();
    for observation in fixture
        .observations
        .iter()
        .filter(|item| item.sequence <= cutoff)
    {
        if let Some(&index) = entity_position.get(observation.provider_id.as_str()) {
            visible[index] = observation;
        } else {
            entity_position.insert(observation.provider_id.as_str(), visible.len());
            visible.push(observation);
        }
    }

    let opening = visible
        .iter()
        .position(|item| matches!(item.kind, ObservationKind::Issue))
        .unwrap_or(0);
    let included = match method {
        ReaderMethod::RawHistory => (0..visible.len()).collect::<Vec<_>>(),
        ReaderMethod::RecentRetrieval => {
            let mut positions = vec![opening];
            let mut recent = visible
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != opening)
                .collect::<Vec<_>>();
            recent.sort_by_key(|(_, observation)| observation.sequence);
            positions.extend(
                recent
                    .into_iter()
                    .rev()
                    .take(recent_window)
                    .map(|(index, _)| index),
            );
            positions.sort_unstable();
            positions
        }
    };

    let mut context = String::new();
    let mut searchable = Vec::new();
    let missing_bodies: Vec<u64> = fixture
        .observations
        .iter()
        .filter(|observation| observation.sequence <= cutoff)
        .filter(|observation| available_body_at(fixture, observation, cutoff).is_none())
        .map(|observation| observation.sequence)
        .collect();
    for (index, observation) in visible.into_iter().enumerate() {
        let body = available_body_at(fixture, observation, cutoff);
        if included.contains(&index) {
            let _ = write!(
                context,
                "[observation {} | source {} | author {} | authored {} | version {}]",
                observation.sequence,
                observation.provider_id,
                observation
                    .author
                    .as_ref()
                    .and_then(|actor| actor.provider_id.as_deref())
                    .unwrap_or("unknown"),
                observation.created_at,
                observation.version_id
            );
            context.push('\n');
            context.push_str(body.unwrap_or("[body unavailable at this cutoff]"));
            context.push_str("\n\n");
        }
        if let Some(body) = body
            && (method == ReaderMethod::RawHistory || !included.contains(&index))
        {
            searchable.push(SearchableSource {
                provider_id: observation.provider_id.clone(),
                version_id: observation.version_id.clone(),
                observation: observation.sequence,
                body: body.to_owned(),
            });
        }
    }

    Ok(ReaderInput {
        method,
        cutoff,
        context,
        searchable,
        fidelity: ReaderFidelity {
            history: fixture.capture.history_fidelity,
            exact_replay: missing_bodies.is_empty()
                && matches!(
                    fixture.capture.history_fidelity,
                    HistoryFidelity::ExactObserved
                        | HistoryFidelity::DiffReconstructable
                        | HistoryFidelity::StagedExact
                ),
            missing_bodies,
        },
    })
}

/// Builds the stricter exact-history input used for staged or fully observed replay.
///
/// # Errors
/// Rejects any unavailable body, unsuitable history class, or ambiguous order.
pub fn prepare_exact_reader_input(
    fixture: &Fixture,
    cutoff: u64,
    method: ReaderMethod,
    recent_window: usize,
) -> Result<ReaderInput, InputError> {
    validate(fixture)?;
    require_exact_source_bodies_through(fixture, cutoff)?;
    if !matches!(
        fixture.capture.history_fidelity,
        HistoryFidelity::ExactObserved
            | HistoryFidelity::DiffReconstructable
            | HistoryFidelity::StagedExact
    ) {
        return Err(InputError::NonCausalHistory);
    }
    prepare_reader_input(fixture, cutoff, method, recent_window)
}
