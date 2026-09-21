//! Causal inputs for comparing Merl with ordinary Issue-reading methods.

use std::{collections::HashMap, error::Error, fmt, fmt::Write as _};

use merl_corpus::fixture::{
    Fixture, HistoryFidelity, Observation, ObservationKind, ValidationError,
    require_exact_source_bodies_through, require_unambiguous_order_through, validate,
};

mod adapters;
mod runner;

pub use adapters::{CliMerlSurface, ProcessModelAdapter, ProcessScorer};

pub use runner::{
    AnswerAction, AnswerRequest, AnswerResponse, BenchmarkConfig, BenchmarkError, BenchmarkMethod,
    BenchmarkReport, BenchmarkRunner, BreakEven, EvaluationQuestion, MerlPrepared, MerlSurface,
    MethodStats, ModelAdapter, Score, Scorer, SummaryRequest, SummaryResponse, SummarySource,
    TokenUsage, TrialReport,
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
/// Rejects invalid fixtures, a missing historical body, ambiguous source order,
/// insufficient history fidelity, or a zero recent-window size.
pub fn prepare_reader_input(
    fixture: &Fixture,
    cutoff: u64,
    method: ReaderMethod,
    recent_window: usize,
) -> Result<ReaderInput, InputError> {
    validate(fixture)?;
    require_exact_source_bodies_through(fixture, cutoff)?;
    require_unambiguous_order_through(fixture, cutoff)?;
    if !matches!(
        fixture.capture.history_fidelity,
        HistoryFidelity::ExactObserved
            | HistoryFidelity::DiffReconstructable
            | HistoryFidelity::StagedExact
    ) {
        return Err(InputError::NonCausalHistory);
    }
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
    for (index, observation) in visible.into_iter().enumerate() {
        let body = observation.body.as_deref().ok_or(InputError::Fixture(
            ValidationError::MissingHistoricalBody(observation.sequence),
        ))?;
        if included.contains(&index) {
            let _ = write!(
                context,
                "[observation {} | source {} | authored {} | version {}]",
                observation.sequence,
                observation.provider_id,
                observation.created_at,
                observation.version_id
            );
            context.push('\n');
            context.push_str(body);
            context.push_str("\n\n");
        }
        if method == ReaderMethod::RawHistory || !included.contains(&index) {
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
    })
}
