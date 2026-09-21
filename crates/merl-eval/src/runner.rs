//! Paired reader trials with model usage reported by the configured adapter.

use std::{error::Error, fmt, fmt::Write as _};

use merl_corpus::fixture::{BodyAvailability, Fixture, available_body_at, body_availability};
use serde::{Deserialize, Serialize};

use crate::{InputError, ReaderFidelity, ReaderMethod, SearchableSource, prepare_reader_input};

/// One method in the five-way Issue comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMethod {
    /// Entire visible Issue history at the question cutoff.
    RawHistory,
    /// Summary updated in source-observation order.
    RollingSummary,
    /// Opening source and recent window, with older content searchable.
    RecentRetrieval,
    /// Rolling summary with searchable source evidence.
    SummaryRetrieval,
    /// Accepted Merl state with explicit expansion.
    Merl,
}

impl BenchmarkMethod {
    /// All methods in the frozen first-release comparison.
    pub const ALL: [Self; 5] = [
        Self::RawHistory,
        Self::RollingSummary,
        Self::RecentRetrieval,
        Self::SummaryRetrieval,
        Self::Merl,
    ];
}

/// Provider-reported model usage. The harness never estimates this from text length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    /// Input tokens billed for a model call.
    pub input: u64,
    /// Output tokens billed for a model call.
    pub output: u64,
}

impl TokenUsage {
    /// Total reported tokens.
    #[must_use]
    pub fn total(self) -> u128 {
        u128::from(self.input) + u128::from(self.output)
    }

    fn add(&mut self, other: Self) -> Result<(), BenchmarkError> {
        self.input = self
            .input
            .checked_add(other.input)
            .ok_or(BenchmarkError::UsageOverflow)?;
        self.output = self
            .output
            .checked_add(other.output)
            .ok_or(BenchmarkError::UsageOverflow)?;
        Ok(())
    }
}

/// Public question; the answer key stays in the scorer, never in a model request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationQuestion {
    /// Stable evaluator-owned question identity.
    pub id: String,
    /// Highest source observation available to any reader.
    pub cutoff: u64,
    /// Identical task wording given to all five methods.
    pub text: String,
}

/// One frozen model and tool configuration for all paired trials.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkConfig {
    /// Model name exposed by the adapter.
    pub model: String,
    /// Exact model version where the provider exposes one.
    pub model_version: String,
    /// Reasoning effort or an explicit `none` value.
    pub effort: String,
    /// Shared system instruction.
    pub system_prompt: String,
    /// Shared task instruction prepended to each question.
    pub task_prompt: String,
    /// Stable paired-trial identities and provider randomness controls.
    pub trials: Vec<TrialIdentity>,
    /// Number of recent sources visible before search.
    pub recent_window: usize,
    /// Maximum search or expansion rounds per answer.
    pub max_tool_rounds: usize,
    /// Maximum search results disclosed per query.
    pub search_results: usize,
    /// Provider sampling temperature, when supported.
    pub temperature: Option<f64>,
}

/// One independently prepared five-method trial.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrialIdentity {
    /// Stable ID shared by every method in this pair.
    pub id: String,
    /// Per-trial seed, or an explicit statement that the provider exposes none.
    pub randomness: RandomnessControl,
}

/// Provider randomness control for a paired trial.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RandomnessControl {
    /// Seed submitted to the provider for every method in the pair.
    Seed(u64),
    /// The provider exposes no usable seed; the reason remains in the report.
    Unavailable { reason: String },
}

/// One model call to update the rolling summary after a source observation.
#[derive(Debug)]
pub struct SummarySource<'a> {
    /// Observation available at this position.
    pub sequence: u64,
    /// Earlier version replaced by this source, if any.
    pub supersedes: Option<u64>,
    /// Stable source entity identity.
    pub provider_id: &'a str,
    /// Stable author identity known at this position.
    pub author_id: Option<&'a str>,
    /// Original authored time of the source entity.
    pub created_at: &'a str,
    /// Time this version became visible.
    pub occurred_at: &'a str,
    /// Exact body visible at this position.
    pub body: Option<&'a str>,
    /// True when the body was first available in the terminal capture.
    pub disclosed_at_capture: bool,
    /// This visible source belongs to a timestamp-tied group with no proven upstream order.
    pub upstream_order_unresolved: bool,
}

/// One model call to update the rolling summary after a source observation.
#[derive(Debug)]
pub struct SummaryRequest<'a> {
    /// The same frozen model configuration used for answer calls.
    pub config: &'a BenchmarkConfig,
    /// Paired identity and randomness for this summary preparation.
    pub trial: &'a TrialIdentity,
    /// Previous summary, empty for the first observation.
    pub previous: &'a str,
    /// Source version newly available at this position.
    pub source: SummarySource<'a>,
}

/// Bounded summary and its reported generation cost.
#[derive(Clone, Debug)]
pub struct SummaryResponse {
    /// Current-state summary after the new observation.
    pub text: String,
    /// Provider-reported usage for this call.
    pub usage: TokenUsage,
}

/// One answer call. A tool result, if any, is already appended to `context`.
#[derive(Debug)]
pub struct AnswerRequest<'a> {
    /// Shared model and prompt configuration.
    pub config: &'a BenchmarkConfig,
    /// Paired identity and randomness shared by all five methods.
    pub trial: &'a TrialIdentity,
    /// Reader method, for audit and tool eligibility.
    pub method: BenchmarkMethod,
    /// Stable question identity.
    pub question_id: &'a str,
    /// Identical question text for each method.
    pub question: &'a str,
    /// Context currently disclosed to this reader.
    pub context: &'a str,
    /// Whether source search is available in this method.
    pub can_search: bool,
    /// Whether Merl object expansion is available in this method.
    pub can_expand: bool,
}

/// One reader response or a request for bounded additional context.
#[derive(Clone, Debug)]
pub enum AnswerAction {
    /// Final answer with source or object references supplied by the reader.
    Final {
        /// Reader's answer.
        answer: String,
        /// References used by the answer.
        citations: Vec<String>,
    },
    /// Search earlier source content without exposing the whole pool.
    Search(String),
    /// Expand one object or source reference from Merl.
    Expand(String),
}

/// Adapter response and provider-reported cost.
#[derive(Clone, Debug)]
pub struct AnswerResponse {
    /// Reader answer or tool request.
    pub action: AnswerAction,
    /// Tokens billed for this model call.
    pub usage: TokenUsage,
}

/// Model-facing adapter. It never receives the evaluator's answer key.
pub trait ModelAdapter {
    /// Update a rolling summary once from one newly visible source version.
    ///
    /// # Errors
    /// Returns a provider or response-validation failure.
    fn summarize(&mut self, request: SummaryRequest<'_>) -> Result<SummaryResponse, String>;

    /// Answer from the disclosed context or request an allowed tool.
    ///
    /// # Errors
    /// Returns a provider or response-validation failure.
    fn answer(&mut self, request: AnswerRequest<'_>) -> Result<AnswerResponse, String>;
}

/// Causal Merl view prepared independently of the scorer's answer key.
#[derive(Clone, Debug)]
pub struct MerlPrepared {
    /// Exact source cutoff used to construct this accepted view.
    pub source_cutoff: u64,
    /// Role-appropriate view text, without raw source payloads by default.
    pub view: String,
    /// Measured compiler and correction usage incurred before downstream reads.
    pub preparation_usage: TokenUsage,
    /// Whether required sources through the cutoff were semantically processed.
    pub required_coverage_complete: bool,
    /// True only for a causal live or replay derivation, not hindsight.
    pub causal: bool,
}

/// Supplies accepted Merl state and explicit expansions from a prepared authority.
pub trait MerlSurface {
    /// Read the causal Merl view for this question.
    ///
    /// # Errors
    /// Returns a project, coverage, or view failure.
    fn prepare(&mut self, question: &EvaluationQuestion) -> Result<MerlPrepared, String>;

    /// Expand an object or source reference at the same accepted cutoff.
    ///
    /// # Errors
    /// Returns an unknown, purged, or unauthorized reference failure.
    fn expand(&mut self, reference: &str) -> Result<String, String>;
}

/// Correctness and provenance grades assigned after a reader answers.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Score {
    /// Correctness from zero to one; `None` means a human must adjudicate.
    pub correctness: Option<f64>,
    /// Citation quality from zero to one; `None` means unadjudicated.
    pub provenance: Option<f64>,
    /// Reader relied on state superseded before the cutoff.
    pub stale_state_error: bool,
    /// Reader missed a blocker required by the question.
    pub missed_blocker: bool,
}

/// Evaluator-only scoring boundary. The candidate adapter cannot inspect it.
pub trait Scorer {
    /// Score one answer and its cited evidence.
    ///
    /// # Errors
    /// Returns a missing key or scoring failure.
    fn score(
        &mut self,
        question_id: &str,
        answer: &str,
        citations: &[String],
    ) -> Result<Score, String>;
}

/// One paired answer and its measured downstream consumption.
#[derive(Clone, Debug, Serialize)]
pub struct TrialReport {
    /// Reading method.
    pub method: BenchmarkMethod,
    /// Zero-based paired trial number.
    pub trial: usize,
    /// Stable paired-trial identity.
    pub trial_id: String,
    /// Question answered in this paired trial.
    pub question_id: String,
    /// Reader answer.
    pub answer: String,
    /// Reader-supplied citations.
    pub citations: Vec<String>,
    /// Independent correctness and provenance grades.
    pub score: Score,
    /// Preparation cost incurred once for this independent trial.
    pub preparation_usage: TokenUsage,
    /// Provider-reported answer, search, and expansion token usage.
    pub read_usage: TokenUsage,
    /// Number of explicit search or expansion rounds.
    pub tool_rounds: usize,
    /// Required semantic coverage for the Merl arm; other methods make no such claim.
    pub merl_required_coverage_complete: Option<bool>,
}

/// Summary statistics over independently prepared paired trials.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MethodStats {
    /// Reading method.
    pub method: BenchmarkMethod,
    /// Exact preparation cost across independent trials.
    pub total_preparation_tokens: u128,
    /// Mean preparation cost per trial.
    pub mean_preparation_tokens: f64,
    /// Sample variance of preparation cost.
    pub preparation_token_variance: f64,
    /// Exact total across paired reads, used for break-even arithmetic.
    pub total_read_tokens: u128,
    /// Mean per-read token cost across paired trials.
    pub mean_read_tokens: f64,
    /// Sample variance of per-read token cost.
    pub read_token_variance: f64,
    /// Mean adjudicated correctness, if every trial has a grade.
    pub mean_correctness: Option<f64>,
    /// Sample variance of correctness, if every trial has a grade.
    pub correctness_variance: Option<f64>,
    /// Mean provenance grade, if every trial has a grade.
    pub mean_provenance: Option<f64>,
    /// Sample variance of provenance, if every trial has a grade.
    pub provenance_variance: Option<f64>,
    /// Trials that relied on superseded state.
    pub stale_state_errors: usize,
    /// Trials that missed a required blocker.
    pub missed_blockers: usize,
    /// Answers whose correctness still needs human adjudication.
    pub unadjudicated_correctness: usize,
    /// Answers whose citations still need human adjudication.
    pub unadjudicated_provenance: usize,
}

/// First read count where a method costs fewer total tokens than repeated raw reads.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct BreakEven {
    /// Comparison method; raw history is the reference and has no entry.
    pub method: BenchmarkMethod,
    /// None when measured per-read cost cannot amortize preparation.
    pub first_cheaper_read: Option<u64>,
}

/// One paired, five-method benchmark result.
#[derive(Clone, Debug, Serialize)]
pub struct BenchmarkReport {
    /// Corpus fixture ID.
    pub fixture_id: String,
    /// History class and bodies unavailable at the question cutoff.
    pub source_fidelity: Vec<CutoffFidelity>,
    /// Evaluator questions, each paired across all five methods.
    pub questions: Vec<EvaluationQuestion>,
    /// Exact model and prompt configuration used for every reader.
    pub config: BenchmarkConfig,
    /// Individual answers and costs.
    pub trials: Vec<TrialReport>,
    /// Per-method preparation and trial statistics.
    pub methods: Vec<MethodStats>,
    /// Four comparisons against repeated raw-history reads.
    pub break_even: Vec<BreakEven>,
}

/// Source evidence available at one question cutoff.
#[derive(Clone, Debug, Serialize)]
pub struct CutoffFidelity {
    /// Observation cutoff.
    pub cutoff: u64,
    /// History class and unavailable bodies at that cutoff.
    pub fidelity: ReaderFidelity,
}

/// A benchmark cannot be run or audited under its declared contract.
#[derive(Debug)]
pub enum BenchmarkError {
    /// Source history is not exact and causally ordered at the requested cutoff.
    Input(InputError),
    /// Trial count, retrieval budget, or configuration is invalid.
    InvalidConfig(&'static str),
    /// The model adapter failed.
    Model(String),
    /// Merl view or expansion failed.
    Merl(String),
    /// Evaluator-only scoring failed.
    Scoring(String),
    /// Merl view was prepared against another or hindsight cutoff.
    InvalidMerlSurface,
    /// Reader requested a tool that this method does not provide.
    ToolNotAllowed,
    /// Reader exceeded the frozen expansion/search budget.
    ToolBudget,
    /// Reported token usage overflowed the counter.
    UsageOverflow,
    /// A scorer returned a grade outside zero to one.
    InvalidScore,
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Input(error) => write!(f, "{error}"),
            Self::InvalidConfig(reason) => write!(f, "invalid benchmark configuration: {reason}"),
            Self::Model(reason) => write!(f, "model adapter failed: {reason}"),
            Self::Merl(reason) => write!(f, "Merl surface failed: {reason}"),
            Self::Scoring(reason) => write!(f, "scoring failed: {reason}"),
            Self::InvalidMerlSurface => {
                f.write_str("Merl view is noncausal, incomplete, or at the wrong cutoff")
            }
            Self::ToolNotAllowed => {
                f.write_str("reader requested a tool unavailable to its method")
            }
            Self::ToolBudget => f.write_str("reader exceeded the tool-round budget"),
            Self::UsageOverflow => f.write_str("reported token usage overflowed"),
            Self::InvalidScore => f.write_str("scorer returned a grade outside zero to one"),
        }
    }
}

impl Error for BenchmarkError {}

impl From<InputError> for BenchmarkError {
    fn from(error: InputError) -> Self {
        Self::Input(error)
    }
}

/// Runs the five methods with one question and one frozen model configuration.
pub struct BenchmarkRunner<'a, M, S, J> {
    /// Candidate-facing model adapter.
    pub model: &'a mut M,
    /// Merl authority view and expansion adapter.
    pub merl: &'a mut S,
    /// Evaluator-only answer scorer.
    pub scorer: &'a mut J,
}

impl<M: ModelAdapter, S: MerlSurface, J: Scorer> BenchmarkRunner<'_, M, S, J> {
    /// Runs independently prepared paired trials. Each trial pays preparation once;
    /// the break-even calculation then models repeated reads of that prepared state.
    ///
    /// # Errors
    /// Rejects noncausal inputs, mismatched Merl views, invalid adapter replies,
    /// unavailable tools, or invalid scorer output.
    pub fn run(
        &mut self,
        fixture: &Fixture,
        question: EvaluationQuestion,
        config: BenchmarkConfig,
    ) -> Result<BenchmarkReport, BenchmarkError> {
        self.run_suite(fixture, vec![question], config)
    }

    /// Runs every question with shared per-Issue preparation charged only once
    /// per paired trial and source cutoff.
    ///
    /// # Errors
    /// Rejects invalid questions, noncausal sources, or adapter failures.
    #[expect(
        clippy::too_many_lines,
        reason = "paired-trial validation and five-method execution share one public boundary"
    )]
    pub fn run_suite(
        &mut self,
        fixture: &Fixture,
        questions: Vec<EvaluationQuestion>,
        config: BenchmarkConfig,
    ) -> Result<BenchmarkReport, BenchmarkError> {
        if questions.is_empty() {
            return Err(BenchmarkError::InvalidConfig("questions must be nonempty"));
        }
        let mut question_ids = std::collections::HashSet::new();
        let mut cutoffs = Vec::new();
        for question in &questions {
            if question.id.is_empty() || !question_ids.insert(question.id.as_str()) {
                return Err(BenchmarkError::InvalidConfig(
                    "question IDs must be nonempty and unique",
                ));
            }
            if !cutoffs.contains(&question.cutoff) {
                cutoffs.push(question.cutoff);
            }
        }
        if config.trials.is_empty() {
            return Err(BenchmarkError::InvalidConfig("trials must be positive"));
        }
        let mut ids = std::collections::HashSet::new();
        let mut seeds = std::collections::HashSet::new();
        let seeded = matches!(config.trials[0].randomness, RandomnessControl::Seed(_));
        for trial in &config.trials {
            if trial.id.is_empty() || !ids.insert(trial.id.as_str()) {
                return Err(BenchmarkError::InvalidConfig(
                    "paired trial IDs must be nonempty and unique",
                ));
            }
            match &trial.randomness {
                RandomnessControl::Seed(seed) if seeded && seeds.insert(*seed) => {}
                RandomnessControl::Unavailable { reason }
                    if !seeded && !reason.trim().is_empty() => {}
                _ => {
                    return Err(BenchmarkError::InvalidConfig(
                        "trials need one supported randomness mode and distinct seeds",
                    ));
                }
            }
        }
        if config.recent_window == 0 || config.search_results == 0 {
            return Err(BenchmarkError::InvalidConfig(
                "retrieval limits must be positive",
            ));
        }
        if config.model.is_empty() || config.model_version.is_empty() || config.effort.is_empty() {
            return Err(BenchmarkError::InvalidConfig(
                "model identity and effort are required",
            ));
        }

        let mut inputs = Vec::new();
        for cutoff in cutoffs {
            inputs.push((
                cutoff,
                prepare_reader_input(
                    fixture,
                    cutoff,
                    ReaderMethod::RawHistory,
                    config.recent_window,
                )?,
                prepare_reader_input(
                    fixture,
                    cutoff,
                    ReaderMethod::RecentRetrieval,
                    config.recent_window,
                )?,
            ));
        }
        let mut trials =
            Vec::with_capacity(config.trials.len() * questions.len() * BenchmarkMethod::ALL.len());
        for (trial_index, trial) in config.trials.iter().enumerate() {
            for (cutoff, raw, recent) in &inputs {
                let first_question = questions
                    .iter()
                    .find(|question| question.cutoff == *cutoff)
                    .ok_or(BenchmarkError::InvalidConfig("cutoff has no question"))?;
                let merl = self
                    .merl
                    .prepare(first_question)
                    .map_err(BenchmarkError::Merl)?;
                if merl.source_cutoff != *cutoff || !merl.causal {
                    return Err(BenchmarkError::InvalidMerlSurface);
                }
                let (summary, summary_usage) =
                    self.prepare_summary(fixture, *cutoff, &config, trial)?;
                for (question_index, question) in questions
                    .iter()
                    .filter(|question| question.cutoff == *cutoff)
                    .enumerate()
                {
                    for method in BenchmarkMethod::ALL {
                        let (context, searchable, preparation_usage) = match method {
                            BenchmarkMethod::RawHistory => (
                                raw.context.as_str(),
                                raw.searchable.as_slice(),
                                TokenUsage::default(),
                            ),
                            BenchmarkMethod::RollingSummary | BenchmarkMethod::SummaryRetrieval => {
                                (
                                    summary.as_str(),
                                    raw.searchable.as_slice(),
                                    if question_index == 0 {
                                        summary_usage
                                    } else {
                                        TokenUsage::default()
                                    },
                                )
                            }
                            BenchmarkMethod::RecentRetrieval => (
                                recent.context.as_str(),
                                recent.searchable.as_slice(),
                                TokenUsage::default(),
                            ),
                            BenchmarkMethod::Merl => (
                                merl.view.as_str(),
                                &[][..],
                                if question_index == 0 {
                                    merl.preparation_usage
                                } else {
                                    TokenUsage::default()
                                },
                            ),
                        };
                        let mut result = self.read_once(
                            method,
                            trial_index,
                            trial,
                            question,
                            &config,
                            context,
                            searchable,
                        )?;
                        result.preparation_usage = preparation_usage;
                        if method == BenchmarkMethod::Merl {
                            result.merl_required_coverage_complete =
                                Some(merl.required_coverage_complete);
                        }
                        trials.push(result);
                    }
                }
            }
        }

        let methods = BenchmarkMethod::ALL
            .into_iter()
            .map(|method| method_stats(method, &trials))
            .collect::<Vec<_>>();
        let raw_cost = methods[0].total_read_tokens;
        let break_even = methods[1..]
            .iter()
            .map(|stats| BreakEven {
                method: stats.method,
                first_cheaper_read: break_even_reads(
                    raw_cost,
                    stats.total_read_tokens,
                    stats.total_preparation_tokens,
                ),
            })
            .collect();

        Ok(BenchmarkReport {
            fixture_id: fixture.id.clone(),
            source_fidelity: inputs
                .into_iter()
                .map(|(cutoff, raw, _)| CutoffFidelity {
                    cutoff,
                    fidelity: raw.fidelity,
                })
                .collect(),
            questions,
            config,
            trials,
            methods,
            break_even,
        })
    }

    fn prepare_summary(
        &mut self,
        fixture: &Fixture,
        cutoff: u64,
        config: &BenchmarkConfig,
        trial: &TrialIdentity,
    ) -> Result<(String, TokenUsage), BenchmarkError> {
        let mut summary = String::new();
        let mut usage = TokenUsage::default();
        for source in fixture
            .observations
            .iter()
            .filter(|item| item.sequence <= cutoff)
        {
            let response = self
                .model
                .summarize(SummaryRequest {
                    config,
                    trial,
                    previous: &summary,
                    source: SummarySource {
                        sequence: source.sequence,
                        supersedes: source.supersedes,
                        provider_id: &source.provider_id,
                        author_id: source
                            .author
                            .as_ref()
                            .and_then(|actor| actor.provider_id.as_deref()),
                        created_at: &source.created_at,
                        occurred_at: &source.occurred_at,
                        body: (body_availability(fixture, source)
                            == Some(BodyAvailability::AtObservation))
                        .then(|| available_body_at(fixture, source, source.sequence))
                        .flatten(),
                        disclosed_at_capture: false,
                        upstream_order_unresolved: source.ambiguous_order_with_previous
                            || fixture
                                .observations
                                .get(usize::try_from(source.sequence).unwrap_or(usize::MAX))
                                .is_some_and(|next| {
                                    next.sequence <= cutoff && next.ambiguous_order_with_previous
                                }),
                    },
                })
                .map_err(BenchmarkError::Model)?;
            if response.text.is_empty() {
                return Err(BenchmarkError::Model("empty rolling summary".to_owned()));
            }
            usage.add(response.usage)?;
            summary = response.text;
        }
        if cutoff == fixture.observations.last().map_or(0, |last| last.sequence) {
            for source in fixture
                .observations
                .iter()
                .filter(|item| item.sequence <= cutoff)
                .filter(|item| {
                    body_availability(fixture, item) == Some(BodyAvailability::AtCapture)
                })
            {
                let response = self
                    .model
                    .summarize(SummaryRequest {
                        config,
                        trial,
                        previous: &summary,
                        source: SummarySource {
                            sequence: source.sequence,
                            supersedes: source.supersedes,
                            provider_id: &source.provider_id,
                            author_id: source
                                .author
                                .as_ref()
                                .and_then(|actor| actor.provider_id.as_deref()),
                            created_at: &source.created_at,
                            occurred_at: &source.occurred_at,
                            body: source.body.as_deref(),
                            disclosed_at_capture: true,
                            upstream_order_unresolved: source.ambiguous_order_with_previous,
                        },
                    })
                    .map_err(BenchmarkError::Model)?;
                if response.text.is_empty() {
                    return Err(BenchmarkError::Model("empty rolling summary".to_owned()));
                }
                usage.add(response.usage)?;
                summary = response.text;
            }
        }
        Ok((summary, usage))
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "reader identity, paired trial, question, and disclosed context stay explicit"
    )]
    fn read_once(
        &mut self,
        method: BenchmarkMethod,
        trial_index: usize,
        trial: &TrialIdentity,
        question: &EvaluationQuestion,
        config: &BenchmarkConfig,
        initial_context: &str,
        searchable: &[SearchableSource],
    ) -> Result<TrialReport, BenchmarkError> {
        let mut context = initial_context.to_owned();
        let mut usage = TokenUsage::default();
        for round in 0..=config.max_tool_rounds {
            let can_search = matches!(
                method,
                BenchmarkMethod::RecentRetrieval | BenchmarkMethod::SummaryRetrieval
            );
            let can_expand = method == BenchmarkMethod::Merl;
            let response = self
                .model
                .answer(AnswerRequest {
                    config,
                    trial,
                    method,
                    question_id: &question.id,
                    question: &question.text,
                    context: &context,
                    can_search,
                    can_expand,
                })
                .map_err(BenchmarkError::Model)?;
            usage.add(response.usage)?;
            match response.action {
                AnswerAction::Final { answer, citations } => {
                    let score = self
                        .scorer
                        .score(&question.id, &answer, &citations)
                        .map_err(BenchmarkError::Scoring)?;
                    for grade in [score.correctness, score.provenance].into_iter().flatten() {
                        if !(0.0..=1.0).contains(&grade) {
                            return Err(BenchmarkError::InvalidScore);
                        }
                    }
                    return Ok(TrialReport {
                        method,
                        trial: trial_index,
                        trial_id: trial.id.clone(),
                        question_id: question.id.clone(),
                        answer,
                        citations,
                        score,
                        preparation_usage: TokenUsage::default(),
                        read_usage: usage,
                        tool_rounds: round,
                        merl_required_coverage_complete: None,
                    });
                }
                AnswerAction::Search(query) if can_search => {
                    if round == config.max_tool_rounds {
                        return Err(BenchmarkError::ToolBudget);
                    }
                    let matches = searchable
                        .iter()
                        .rev()
                        .filter(|source| {
                            source
                                .provider_id
                                .to_lowercase()
                                .contains(&query.to_lowercase())
                                || source.body.to_lowercase().contains(&query.to_lowercase())
                        })
                        .take(config.search_results);
                    context.push_str("\n[search results]\n");
                    for source in matches {
                        let _ = write!(
                            context,
                            "[source {} version {} observation {}]",
                            source.provider_id, source.version_id, source.observation
                        );
                        context.push('\n');
                        context.push_str(&source.body);
                        context.push('\n');
                    }
                }
                AnswerAction::Expand(reference) if can_expand => {
                    if round == config.max_tool_rounds {
                        return Err(BenchmarkError::ToolBudget);
                    }
                    let detail = self.merl.expand(&reference).map_err(BenchmarkError::Merl)?;
                    context.push_str("\n[expanded ");
                    context.push_str(&reference);
                    context.push_str("]\n");
                    context.push_str(&detail);
                }
                AnswerAction::Search(_) | AnswerAction::Expand(_) => {
                    return Err(BenchmarkError::ToolNotAllowed);
                }
            }
        }
        Err(BenchmarkError::ToolBudget)
    }
}

fn method_stats(method: BenchmarkMethod, trials: &[TrialReport]) -> MethodStats {
    let reads = trials
        .iter()
        .filter(|trial| trial.method == method)
        .collect::<Vec<_>>();
    // Provider usage is exact in the report; floating point is only for display statistics.
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let count = reads.len() as f64;
    let mut preparation_by_trial = std::collections::BTreeMap::<&str, u128>::new();
    for read in &reads {
        *preparation_by_trial
            .entry(read.trial_id.as_str())
            .or_default() += read.preparation_usage.total();
    }
    let total_preparation_tokens = preparation_by_trial.values().sum();
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let preparation_count = preparation_by_trial.len() as f64;
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let mean_preparation_tokens = total_preparation_tokens as f64 / preparation_count;
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let preparation_token_variance = if preparation_by_trial.len() > 1 {
        preparation_by_trial
            .values()
            .map(|cost| (*cost as f64 - mean_preparation_tokens).powi(2))
            .sum::<f64>()
            / (preparation_count - 1.0)
    } else {
        0.0
    };
    let total_read_tokens = reads.iter().map(|trial| trial.read_usage.total()).sum();
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let mean_read_tokens = reads
        .iter()
        .map(|trial| trial.read_usage.total() as f64)
        .sum::<f64>()
        / count;
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let read_token_variance = if reads.len() > 1 {
        reads
            .iter()
            .map(|trial| (trial.read_usage.total() as f64 - mean_read_tokens).powi(2))
            .sum::<f64>()
            / (count - 1.0)
    } else {
        0.0
    };
    let correctness = reads
        .iter()
        .map(|trial| trial.score.correctness)
        .collect::<Option<Vec<_>>>();
    let provenance = reads
        .iter()
        .map(|trial| trial.score.provenance)
        .collect::<Option<Vec<_>>>();
    let (mean_correctness, correctness_variance) = grade_stats(correctness.as_deref());
    let (mean_provenance, provenance_variance) = grade_stats(provenance.as_deref());
    MethodStats {
        method,
        total_preparation_tokens,
        mean_preparation_tokens,
        preparation_token_variance,
        total_read_tokens,
        mean_read_tokens,
        read_token_variance,
        mean_correctness,
        correctness_variance,
        mean_provenance,
        provenance_variance,
        stale_state_errors: reads
            .iter()
            .filter(|trial| trial.score.stale_state_error)
            .count(),
        missed_blockers: reads
            .iter()
            .filter(|trial| trial.score.missed_blocker)
            .count(),
        unadjudicated_correctness: reads
            .iter()
            .filter(|trial| trial.score.correctness.is_none())
            .count(),
        unadjudicated_provenance: reads
            .iter()
            .filter(|trial| trial.score.provenance.is_none())
            .count(),
    }
}

fn grade_stats(grades: Option<&[f64]>) -> (Option<f64>, Option<f64>) {
    let Some(grades) = grades else {
        return (None, None);
    };
    #[expect(clippy::cast_precision_loss, reason = "display-only grade statistics")]
    let count = grades.len() as f64;
    let mean = grades.iter().sum::<f64>() / count;
    let variance = if grades.len() > 1 {
        grades
            .iter()
            .map(|grade| (grade - mean).powi(2))
            .sum::<f64>()
            / (count - 1.0)
    } else {
        0.0
    };
    (Some(mean), Some(variance))
}

fn break_even_reads(raw_total: u128, other_total: u128, preparation_total: u128) -> Option<u64> {
    let saving = raw_total.checked_sub(other_total)?;
    if saving == 0 {
        return None;
    }
    u64::try_from(preparation_total / saving + 1).ok()
}
