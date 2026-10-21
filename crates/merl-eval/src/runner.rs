//! Paired reader trials with model usage reported by the configured adapter.

use std::{error::Error, fmt, fmt::Write as _};

use merl_corpus::fixture::{BodyAvailability, Fixture, available_body_at, body_availability};
use serde::{Deserialize, Serialize};

use crate::grading::{Grader, TrialGrade};
use crate::{InputError, ReaderFidelity, ReaderMethod, SearchableSource, prepare_reader_input_at};

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
    /// Whether the terminal provider capture is available after the last observation.
    #[serde(default)]
    pub capture_phase: bool,
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
pub struct TrialReport<G = Score> {
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
    pub score: G,
    /// Preparation cost incurred once for this independent trial.
    pub preparation_usage: TokenUsage,
    /// Provider-reported answer, search, and expansion token usage.
    pub read_usage: TokenUsage,
    /// Usage of each answer call, including tool-request rounds.
    pub answer_calls: Vec<AnswerCallRecord>,
    /// Number of explicit search or expansion rounds.
    pub tool_rounds: usize,
    /// Required semantic coverage for the Merl arm; other methods make no such claim.
    pub merl_required_coverage_complete: Option<bool>,
}

/// Provider-reported cost for one answer or tool-request round.
#[derive(Clone, Debug, Serialize)]
pub struct AnswerCallRecord {
    /// Zero-based round within one reader visit.
    pub round: usize,
    /// Whether the model answered, searched, or expanded.
    pub action: AnswerCallAction,
    /// Provider-reported usage for this call.
    pub usage: TokenUsage,
}

/// Action charged to one answer call.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerCallAction {
    /// The reader returned an answer.
    Final,
    /// The reader requested source search.
    Search,
    /// The reader requested Merl expansion.
    Expand,
}

/// One summary update; both summary-based methods reuse its result.
#[derive(Clone, Debug, Serialize)]
pub struct SummaryCallRecord {
    /// Paired trial identity.
    pub trial_id: String,
    /// Observation cutoff of the prepared summary.
    pub cutoff: u64,
    /// Whether terminal capture was available.
    pub capture_phase: bool,
    /// Source observation whose evidence was summarized.
    pub source_observation: u64,
    /// The call disclosed terminal-capture text after the observations.
    pub disclosed_at_capture: bool,
    /// Provider-reported usage for this update.
    pub usage: TokenUsage,
}

/// Summary statistics over independently prepared paired trials.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MethodStats {
    /// Reading method.
    pub method: BenchmarkMethod,
    /// Known preparation cost across independent trials, including failed slots.
    pub total_preparation_tokens: u128,
    /// Mean known preparation cost per trial.
    pub mean_preparation_tokens: f64,
    /// Sample variance of preparation cost.
    pub preparation_token_variance: f64,
    /// Known total across paired reads, including calls retained by failed slots.
    pub total_read_tokens: u128,
    /// Additional provider usage may be unknown; known totals are lower bounds.
    pub usage_incomplete: bool,
    /// Mean known token cost per recorded visit, including failed slots.
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
pub struct BenchmarkReport<G = Score> {
    /// Corpus fixture ID.
    pub fixture_id: String,
    /// History class and bodies unavailable at the question cutoff.
    pub source_fidelity: Vec<CutoffFidelity>,
    /// Evaluator questions, each paired across all five methods.
    pub questions: Vec<EvaluationQuestion>,
    /// Exact model and prompt configuration used for every reader.
    pub config: BenchmarkConfig,
    /// Individual answers and costs.
    pub trials: Vec<TrialReport<G>>,
    /// Shared summary calls retained separately from per-arm attributable cost.
    pub summary_calls: Vec<SummaryCallRecord>,
    /// Failed slots; these are never scored as correct or excluded.
    pub failures: Vec<TrialFailure>,
    /// Shared costs survive failed or missing answers.
    pub preparation: Vec<PreparationRecord>,
    /// Per-method preparation and trial statistics.
    pub methods: Vec<MethodStats>,
    /// Four comparisons against repeated raw-history reads.
    pub break_even: Vec<BreakEven>,
}

/// A failed scoring slot, retained separately from all categorical outcomes.
#[derive(Clone, Debug, Serialize)]
pub struct TrialFailure {
    /// Exact question-cutoff identity.
    pub unit_id: String,
    /// Comparison arm that could not complete.
    pub method: BenchmarkMethod,
    /// Independent paired trial.
    pub trial_id: String,
    /// Stable failure class, without source or grader output excerpts.
    pub code: &'static str,
    /// Successfully reported calls before failure; never estimated as zero usage.
    pub answer_calls: Vec<AnswerCallRecord>,
    /// Plain answer retained when grading failed.
    pub answer: Option<String>,
    /// Citations retained when grading failed.
    pub citations: Vec<String>,
    /// True when a failed model call may have incurred unreported usage.
    pub usage_incomplete: bool,
}

/// Preparation shared by distinct scoring units at one cutoff and trial.
#[derive(Clone, Debug, Serialize)]
pub struct PreparationRecord {
    /// Observation cutoff.
    pub cutoff: u64,
    /// Independent paired trial.
    pub trial_id: String,
    /// Arm to which this cost is attributable. The two summary arms reuse calls.
    pub method: BenchmarkMethod,
    /// Reported cost, including known calls before a failure.
    pub known_usage: TokenUsage,
    /// A failed preparation can leave provider usage unreported.
    pub complete: bool,
}

#[derive(Default)]
struct ReadProgress {
    calls: Vec<AnswerCallRecord>,
    answer: Option<String>,
    citations: Vec<String>,
}

/// Source evidence available at one question cutoff.
#[derive(Clone, Debug, Serialize)]
pub struct CutoffFidelity {
    /// Observation cutoff.
    pub cutoff: u64,
    /// Whether the cutoff includes terminal capture.
    pub capture_phase: bool,
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

impl<M: ModelAdapter, S: MerlSurface, J: Grader> BenchmarkRunner<'_, M, S, J> {
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
    ) -> Result<BenchmarkReport<J::Grade>, BenchmarkError> {
        self.run_suite(fixture, vec![question], config)
    }

    /// Runs every question with shared per-Issue preparation charged only once
    /// per paired trial and source cutoff.
    ///
    /// # Errors
    /// Rejects invalid questions, noncausal sources, or adapter failures.
    pub fn run_suite(
        &mut self,
        fixture: &Fixture,
        questions: Vec<EvaluationQuestion>,
        config: BenchmarkConfig,
    ) -> Result<BenchmarkReport<J::Grade>, BenchmarkError> {
        self.run_suite_inner(fixture, questions, config, false)
    }

    /// Continues independent slots after adapter or preparation failures.
    /// Failed slots and known preparation/read usage remain explicit in the report.
    ///
    /// # Errors
    /// Rejects invalid configuration or source inputs before starting any trials.
    pub fn run_suite_recording(
        &mut self,
        fixture: &Fixture,
        questions: Vec<EvaluationQuestion>,
        config: BenchmarkConfig,
    ) -> Result<BenchmarkReport<J::Grade>, BenchmarkError> {
        self.run_suite_inner(fixture, questions, config, true)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "validation precedes paired execution and shared preparation accounting"
    )]
    fn run_suite_inner(
        &mut self,
        fixture: &Fixture,
        questions: Vec<EvaluationQuestion>,
        config: BenchmarkConfig,
        record_failures: bool,
    ) -> Result<BenchmarkReport<J::Grade>, BenchmarkError> {
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
            if let Some((_, capture_phase)) = cutoffs
                .iter()
                .find(|(cutoff, _)| *cutoff == question.cutoff)
                && *capture_phase != question.capture_phase
            {
                return Err(BenchmarkError::InvalidConfig(
                    "questions at one cutoff must share the capture phase",
                ));
            }
            if !cutoffs.iter().any(|(cutoff, _)| *cutoff == question.cutoff) {
                cutoffs.push((question.cutoff, question.capture_phase));
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
        for (cutoff, capture_phase) in cutoffs {
            inputs.push((
                cutoff,
                capture_phase,
                prepare_reader_input_at(
                    fixture,
                    cutoff,
                    capture_phase,
                    ReaderMethod::RawHistory,
                    config.recent_window,
                )?,
                prepare_reader_input_at(
                    fixture,
                    cutoff,
                    capture_phase,
                    ReaderMethod::RecentRetrieval,
                    config.recent_window,
                )?,
            ));
        }
        let mut trials =
            Vec::with_capacity(config.trials.len() * questions.len() * BenchmarkMethod::ALL.len());
        let mut summary_calls = Vec::new();
        let mut failures = Vec::new();
        let mut preparation = Vec::new();
        for (trial_index, trial) in config.trials.iter().enumerate() {
            for (cutoff, capture_phase, raw, recent) in &inputs {
                let first_question = questions
                    .iter()
                    .find(|question| question.cutoff == *cutoff)
                    .ok_or(BenchmarkError::InvalidConfig("cutoff has no question"))?;
                let merl = self
                    .merl
                    .prepare(first_question)
                    .map_err(BenchmarkError::Merl)
                    .and_then(|prepared| {
                        if prepared.source_cutoff != *cutoff || !prepared.causal {
                            Err(BenchmarkError::InvalidMerlSurface)
                        } else {
                            Ok(prepared)
                        }
                    });
                let merl = strict_or_recorded(merl, record_failures)?;
                let before = summary_calls.len();
                let summary = self.prepare_summary(
                    fixture,
                    *cutoff,
                    *capture_phase,
                    &config,
                    trial,
                    &mut summary_calls,
                );
                let summary = strict_or_recorded(summary, record_failures)?;
                let mut summary_usage = TokenUsage::default();
                for call in &summary_calls[before..] {
                    summary_usage.add(call.usage)?;
                }
                for method in [
                    BenchmarkMethod::RollingSummary,
                    BenchmarkMethod::SummaryRetrieval,
                    BenchmarkMethod::Merl,
                ] {
                    preparation.push(PreparationRecord {
                        cutoff: *cutoff,
                        trial_id: trial.id.clone(),
                        method,
                        known_usage: if method == BenchmarkMethod::Merl {
                            merl.as_ref()
                                .map_or(TokenUsage::default(), |m| m.preparation_usage)
                        } else {
                            summary_usage
                        },
                        complete: if method == BenchmarkMethod::Merl {
                            merl.is_ok()
                        } else {
                            summary.is_ok()
                        },
                    });
                }
                for (question_index, question) in questions
                    .iter()
                    .filter(|question| question.cutoff == *cutoff)
                    .enumerate()
                {
                    for method in BenchmarkMethod::ALL {
                        let preparation_error = match method {
                            BenchmarkMethod::Merl => merl.as_ref().err(),
                            BenchmarkMethod::RollingSummary | BenchmarkMethod::SummaryRetrieval => {
                                summary.as_ref().err()
                            }
                            _ => None,
                        };
                        if let Some(error) = preparation_error {
                            failures.push(failed_slot(
                                question,
                                method,
                                trial,
                                error,
                                ReadProgress::default(),
                            ));
                            continue;
                        }
                        let (context, searchable, preparation_usage) = match method {
                            BenchmarkMethod::RawHistory => (
                                raw.context.as_str(),
                                raw.searchable.as_slice(),
                                TokenUsage::default(),
                            ),
                            BenchmarkMethod::RollingSummary | BenchmarkMethod::SummaryRetrieval => {
                                (
                                    summary.as_ref().map_or("", String::as_str),
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
                                merl.as_ref().map_or("", |m| m.view.as_str()),
                                &[][..],
                                if question_index == 0 {
                                    merl.as_ref()
                                        .map_or(TokenUsage::default(), |m| m.preparation_usage)
                                } else {
                                    TokenUsage::default()
                                },
                            ),
                        };
                        let mut progress = ReadProgress::default();
                        let result = self.read_once(
                            method,
                            trial_index,
                            trial,
                            question,
                            &config,
                            context,
                            searchable,
                            &mut progress,
                        );
                        let mut result = match result {
                            Ok(result) => result,
                            Err(error) if record_failures => {
                                failures
                                    .push(failed_slot(question, method, trial, &error, progress));
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                        result.preparation_usage = preparation_usage;
                        if method == BenchmarkMethod::Merl {
                            result.merl_required_coverage_complete =
                                merl.as_ref().ok().map(|m| m.required_coverage_complete);
                        }
                        trials.push(result);
                    }
                }
            }
        }

        let methods = BenchmarkMethod::ALL
            .into_iter()
            .map(|method| method_stats(method, &trials, &failures, &preparation))
            .collect::<Vec<_>>();
        let raw_cost = methods[0].total_read_tokens;
        let break_even = methods[1..]
            .iter()
            .map(|stats| BreakEven {
                method: stats.method,
                first_cheaper_read: if failures.is_empty() {
                    break_even_reads(
                        raw_cost,
                        stats.total_read_tokens,
                        stats.total_preparation_tokens,
                    )
                } else {
                    None
                },
            })
            .collect();

        Ok(BenchmarkReport {
            fixture_id: fixture.id.clone(),
            source_fidelity: inputs
                .into_iter()
                .map(|(cutoff, capture_phase, raw, _)| CutoffFidelity {
                    cutoff,
                    capture_phase,
                    fidelity: raw.fidelity,
                })
                .collect(),
            questions,
            config,
            trials,
            summary_calls,
            failures,
            preparation,
            methods,
            break_even,
        })
    }

    fn prepare_summary(
        &mut self,
        fixture: &Fixture,
        cutoff: u64,
        capture_phase: bool,
        config: &BenchmarkConfig,
        trial: &TrialIdentity,
        calls: &mut Vec<SummaryCallRecord>,
    ) -> Result<String, BenchmarkError> {
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
                        .then(|| available_body_at(fixture, source, source.sequence, false))
                        .flatten(),
                        disclosed_at_capture: false,
                        upstream_order_unresolved: source_order_unresolved(
                            fixture,
                            source.sequence,
                            cutoff,
                        ),
                    },
                })
                .map_err(BenchmarkError::Model)?;
            record_summary_call(
                &mut summary,
                &mut usage,
                calls,
                response,
                trial,
                cutoff,
                capture_phase,
                source.sequence,
                false,
            )?;
        }
        if capture_phase && cutoff == fixture.observations.last().map_or(0, |last| last.sequence) {
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
                            upstream_order_unresolved: source_order_unresolved(
                                fixture,
                                source.sequence,
                                cutoff,
                            ),
                        },
                    })
                    .map_err(BenchmarkError::Model)?;
                record_summary_call(
                    &mut summary,
                    &mut usage,
                    calls,
                    response,
                    trial,
                    cutoff,
                    capture_phase,
                    source.sequence,
                    true,
                )?;
            }
        }
        Ok(summary)
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
        progress: &mut ReadProgress,
    ) -> Result<TrialReport<J::Grade>, BenchmarkError> {
        let mut context = initial_context.to_owned();
        let mut usage = TokenUsage::default();
        let mut answer_calls = Vec::new();
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
            answer_calls.push(AnswerCallRecord {
                round,
                action: match &response.action {
                    AnswerAction::Final { .. } => AnswerCallAction::Final,
                    AnswerAction::Search(_) => AnswerCallAction::Search,
                    AnswerAction::Expand(_) => AnswerCallAction::Expand,
                },
                usage: response.usage,
            });
            progress.calls.clone_from(&answer_calls);
            match response.action {
                AnswerAction::Final { answer, citations } => {
                    progress.answer = Some(answer.clone());
                    progress.citations.clone_from(&citations);
                    let score = self
                        .scorer
                        .grade(&question.id, &answer, &citations)
                        .map_err(BenchmarkError::Scoring)?;
                    score.validate(&question.id)?;
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
                        answer_calls,
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

#[expect(
    clippy::too_many_arguments,
    reason = "the summary call identity must stay explicit in the audit ledger"
)]
fn record_summary_call(
    summary: &mut String,
    usage: &mut TokenUsage,
    calls: &mut Vec<SummaryCallRecord>,
    response: SummaryResponse,
    trial: &TrialIdentity,
    cutoff: u64,
    capture_phase: bool,
    source_observation: u64,
    disclosed_at_capture: bool,
) -> Result<(), BenchmarkError> {
    let empty = response.text.is_empty();
    usage.add(response.usage)?;
    calls.push(SummaryCallRecord {
        trial_id: trial.id.clone(),
        cutoff,
        capture_phase,
        source_observation,
        disclosed_at_capture,
        usage: response.usage,
    });
    if empty {
        return Err(BenchmarkError::Model("empty rolling summary".to_owned()));
    }
    *summary = response.text;
    Ok(())
}

fn source_order_unresolved(fixture: &Fixture, sequence: u64, cutoff: u64) -> bool {
    usize::try_from(sequence.saturating_sub(1))
        .ok()
        .and_then(|index| fixture.observations.get(index))
        .is_some_and(|source| source.ambiguous_order_with_previous)
        || usize::try_from(sequence)
            .ok()
            .and_then(|index| fixture.observations.get(index))
            .is_some_and(|next| next.sequence <= cutoff && next.ambiguous_order_with_previous)
}

fn strict_or_recorded<T>(
    result: Result<T, BenchmarkError>,
    record: bool,
) -> Result<Result<T, BenchmarkError>, BenchmarkError> {
    match result {
        Err(error) if !record => Err(error),
        result => Ok(result),
    }
}

fn failed_slot(
    question: &EvaluationQuestion,
    method: BenchmarkMethod,
    trial: &TrialIdentity,
    error: &BenchmarkError,
    progress: ReadProgress,
) -> TrialFailure {
    let code = match error {
        BenchmarkError::Model(_) => "model_failure",
        BenchmarkError::Scoring(_) | BenchmarkError::InvalidScore => "grading_failure",
        BenchmarkError::Merl(_) | BenchmarkError::InvalidMerlSurface => "merl_failure",
        BenchmarkError::ToolNotAllowed | BenchmarkError::ToolBudget => "tool_failure",
        BenchmarkError::UsageOverflow => "usage_overflow",
        BenchmarkError::Input(_) | BenchmarkError::InvalidConfig(_) => "input_failure",
    };
    TrialFailure {
        unit_id: question.id.clone(),
        method,
        trial_id: trial.id.clone(),
        code,
        answer_calls: progress.calls,
        answer: progress.answer,
        citations: progress.citations,
        usage_incomplete: matches!(
            error,
            BenchmarkError::Model(_)
                | BenchmarkError::Merl(_)
                | BenchmarkError::InvalidMerlSurface
                | BenchmarkError::UsageOverflow
        ),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "legacy display statistics retain their native scalar grade contract"
)]
fn method_stats<G: TrialGrade>(
    method: BenchmarkMethod,
    trials: &[TrialReport<G>],
    failures: &[TrialFailure],
    preparation: &[PreparationRecord],
) -> MethodStats {
    let reads = trials
        .iter()
        .filter(|trial| trial.method == method)
        .collect::<Vec<_>>();
    let failed = failures
        .iter()
        .filter(|failure| failure.method == method)
        .collect::<Vec<_>>();
    let preparation = preparation
        .iter()
        .filter(|record| record.method == method)
        .collect::<Vec<_>>();
    let costs = reads
        .iter()
        .map(|read| read.read_usage.total())
        .chain(failed.iter().map(|failure| {
            failure
                .answer_calls
                .iter()
                .map(|call| call.usage.total())
                .sum()
        }))
        .collect::<Vec<u128>>();
    // Floating point is only for display; totals retain every reported call.
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let count = costs.len() as f64;
    let mut preparation_by_trial = std::collections::BTreeMap::<&str, u128>::new();
    for trial_id in reads
        .iter()
        .map(|read| read.trial_id.as_str())
        .chain(failed.iter().map(|failure| failure.trial_id.as_str()))
    {
        preparation_by_trial.entry(trial_id).or_default();
    }
    // The ledger owns shared costs even when the first attributable answer fails.
    for record in &preparation {
        *preparation_by_trial
            .entry(record.trial_id.as_str())
            .or_default() += record.known_usage.total();
    }
    let total_preparation_tokens = preparation_by_trial.values().sum();
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let preparation_count = preparation_by_trial.len() as f64;
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let mean_preparation_tokens = total_preparation_tokens as f64 / preparation_count.max(1.0);
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
    let total_read_tokens = costs.iter().sum();
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let mean_read_tokens = costs.iter().map(|cost| *cost as f64).sum::<f64>() / count.max(1.0);
    #[expect(clippy::cast_precision_loss, reason = "display-only mean and variance")]
    let read_token_variance = if costs.len() > 1 {
        costs
            .iter()
            .map(|cost| (*cost as f64 - mean_read_tokens).powi(2))
            .sum::<f64>()
            / (count - 1.0)
    } else {
        0.0
    };
    let correctness = reads
        .iter()
        .map(|trial| trial.score.scalar().and_then(|grade| grade.correctness))
        .collect::<Option<Vec<_>>>();
    let provenance = reads
        .iter()
        .map(|trial| trial.score.scalar().and_then(|grade| grade.provenance))
        .collect::<Option<Vec<_>>>();
    let (mean_correctness, correctness_variance) = if failed.is_empty() {
        grade_stats(correctness.as_deref())
    } else {
        (None, None)
    };
    let (mean_provenance, provenance_variance) = if failed.is_empty() {
        grade_stats(provenance.as_deref())
    } else {
        (None, None)
    };
    MethodStats {
        method,
        total_preparation_tokens,
        mean_preparation_tokens,
        preparation_token_variance,
        total_read_tokens,
        usage_incomplete: failed.iter().any(|f| f.usage_incomplete)
            || preparation.iter().any(|p| !p.complete),
        mean_read_tokens,
        read_token_variance,
        mean_correctness,
        correctness_variance,
        mean_provenance,
        provenance_variance,
        stale_state_errors: reads
            .iter()
            .filter(|trial| {
                trial
                    .score
                    .scalar()
                    .is_some_and(|grade| grade.stale_state_error)
            })
            .count(),
        missed_blockers: reads
            .iter()
            .filter(|trial| {
                trial
                    .score
                    .scalar()
                    .is_some_and(|grade| grade.missed_blocker)
            })
            .count(),
        unadjudicated_correctness: reads
            .iter()
            .filter(|trial| {
                trial
                    .score
                    .scalar()
                    .is_some_and(|grade| grade.correctness.is_none())
            })
            .count(),
        unadjudicated_provenance: reads
            .iter()
            .filter(|trial| {
                trial
                    .score
                    .scalar()
                    .is_some_and(|grade| grade.provenance.is_none())
            })
            .count(),
    }
}

fn grade_stats(grades: Option<&[f64]>) -> (Option<f64>, Option<f64>) {
    let Some(grades) = grades.filter(|g| !g.is_empty()) else {
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
