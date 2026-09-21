use merl_core::ProjectId;
use merl_corpus::fixture::{
    BodyAvailability, Fixture, HistoryFidelity, MissingBodyReason, Origin, source_digest,
};
use merl_eval::{
    AnswerAction, AnswerRequest, AnswerResponse, BenchmarkConfig, BenchmarkMethod, BenchmarkReport,
    BenchmarkRunner, EvaluationQuestion, InputError, MerlPrepared, MerlSurface, ModelAdapter,
    RandomnessControl, ReaderInput, ReaderMethod, Score, Scorer, SummaryRequest, SummaryResponse,
    TokenUsage, TrialIdentity, prepare_exact_reader_input, prepare_reader_input,
};
use merl_ingest::import_fixture_for_causal_replay_through;
use merl_store::Store;

pub struct EvaluationScenario {
    fixture: Fixture,
    raw: Option<ReaderInput>,
    recent: Option<ReaderInput>,
    report: Option<BenchmarkReport>,
    observed_questions: Vec<String>,
    observed_trials: Vec<String>,
    summary_calls: usize,
    summary_disclosures: Vec<(u64, bool, Option<String>)>,
    tool_result_methods: Vec<BenchmarkMethod>,
    reader_error: Option<InputError>,
    imported_terminal_payload: Option<bool>,
}

impl EvaluationScenario {
    pub fn given_the_gain_discussion() -> Self {
        Self::from_fixture(include_str!("../../corpus/development/DEV-C1.json"))
    }

    pub fn given_the_edited_export_discussion() -> Self {
        Self::from_fixture(include_str!("../../corpus/development/DEV-C3.json"))
    }

    pub fn given_the_gain_discussion_with_inexact_earlier_bodies() -> Self {
        let mut scenario = Self::given_the_gain_discussion();
        scenario.fixture.capture.history_fidelity = HistoryFidelity::DiffOnly;
        scenario
    }

    pub fn given_an_edit_with_an_unavailable_first_body() -> Self {
        let mut scenario = Self::given_the_edited_export_discussion();
        let first = &mut scenario.fixture.observations[0];
        first.body = None;
        first.body_sha256 = None;
        first.missing_body_reason = Some(MissingBodyReason::PriorVersionUnavailable);
        scenario.fixture.capture.history_fidelity = HistoryFidelity::DiffOnly;
        scenario.fixture.capture.source_sha256 = source_digest(
            &scenario.fixture.provider_snapshot,
            &scenario.fixture.observations,
        );
        scenario
    }

    pub fn given_an_opening_body_known_only_from_terminal_capture() -> Self {
        let mut scenario = Self::given_the_gain_discussion();
        "merl.corpus-fixture/v3".clone_into(&mut scenario.fixture.schema);
        scenario.fixture.capture.history_fidelity = HistoryFidelity::TerminalSnapshotOnly;
        for observation in &mut scenario.fixture.observations {
            observation.body_availability = Some(BodyAvailability::AtObservation);
        }
        scenario.fixture.observations[0].body_availability = Some(BodyAvailability::AtCapture);
        scenario.fixture.capture.source_sha256 = source_digest(
            &scenario.fixture.provider_snapshot,
            &scenario.fixture.observations,
        );
        scenario
    }

    pub fn given_two_sources_with_unresolved_order() -> Self {
        let mut scenario = Self::given_the_gain_discussion();
        scenario.fixture.origin = Origin::Natural;
        scenario.fixture.provider_snapshot.updated_at = Some("2026-01-01T12:00:00Z".to_owned());
        "2026-01-01T09:00:00Z".clone_into(&mut scenario.fixture.observations[1].created_at);
        "2026-01-01T09:00:00Z".clone_into(&mut scenario.fixture.observations[1].occurred_at);
        scenario.fixture.observations[1].ambiguous_order_with_previous = true;
        scenario.fixture.capture.source_sha256 = source_digest(
            &scenario.fixture.provider_snapshot,
            &scenario.fixture.observations,
        );
        scenario
    }

    pub fn when_a_reader_revisits_the_tied_cutoff(mut self) -> Self {
        self.raw = Some(
            prepare_reader_input(&self.fixture, 2, ReaderMethod::RawHistory, 2)
                .expect("qualified reader input"),
        );
        self
    }

    pub fn then_both_sources_are_labeled_unordered(self) -> Self {
        let input = self.raw.as_ref().expect("qualified reader input");
        assert_eq!(input.fidelity.unordered_groups, vec![vec![1, 2]]);
        assert!(!input.fidelity.exact_replay);
        assert!(input.context.contains("upstream order unresolved"));
        assert!(input.context.contains("observation 1"));
        assert!(input.context.contains("observation 2"));
        self
    }

    fn from_fixture(json: &str) -> Self {
        Self {
            fixture: serde_json::from_str(json).expect("controlled fixture"),
            raw: None,
            recent: None,
            report: None,
            observed_questions: Vec::new(),
            observed_trials: Vec::new(),
            summary_calls: 0,
            summary_disclosures: Vec::new(),
            tool_result_methods: Vec::new(),
            reader_error: None,
            imported_terminal_payload: None,
        }
    }

    pub fn when_imported_for_an_earlier_cutoff(mut self) -> Self {
        self.import_at_cutoff(1);
        self
    }

    pub fn when_imported_for_the_terminal_cutoff(mut self) -> Self {
        let cutoff = self.fixture.observations.len() as u64;
        self.import_at_cutoff(cutoff);
        self
    }

    fn import_at_cutoff(&mut self, cutoff: u64) {
        let mut store = Store::open_in_memory().expect("authority");
        let project = ProjectId::try_from("eval-partial").expect("project");
        store.create_project(&project).expect("project authority");
        import_fixture_for_causal_replay_through(&mut store, &project, &self.fixture, cutoff)
            .expect("causal prefix");
        let first = store
            .source_version_at(&project, 1)
            .expect("source lookup")
            .expect("source version");
        self.imported_terminal_payload = Some(first.payload.is_some());
    }

    pub fn then_terminal_only_bytes_are_not_in_the_authority(self) {
        assert_eq!(self.imported_terminal_payload, Some(false));
    }

    pub fn when_raw_and_recent_readers_revisit_observation(mut self, cutoff: u64) -> Self {
        self.raw = Some(
            prepare_reader_input(&self.fixture, cutoff, ReaderMethod::RawHistory, 2)
                .expect("raw context"),
        );
        self.recent = Some(
            prepare_reader_input(&self.fixture, cutoff, ReaderMethod::RecentRetrieval, 2)
                .expect("recent context"),
        );
        self
    }

    pub fn then_both_show_the_fixed_gain_decision(self) -> Self {
        for input in self.inputs() {
            assert!(input.context.contains("keep receive gain fixed at 20 dB"));
        }
        self
    }

    pub fn then_neither_shows_the_later_sweep_decision(self) {
        for input in self.inputs() {
            assert!(!input.context.contains("Sweep 10, 20, and 30 dB"));
        }
    }

    pub fn then_both_show_the_original_export_format(self) -> Self {
        for input in self.inputs() {
            assert!(
                input
                    .context
                    .contains("For report A, export CSV with a sample_id column.\n")
            );
        }
        self
    }

    pub fn then_both_show_the_edited_export_format(self) -> Self {
        for input in self.inputs() {
            assert!(
                input
                    .context
                    .contains("unless the consumer moves to Parquet")
            );
        }
        self
    }

    pub fn then_neither_shows_the_original_wording(self) {
        for input in self.inputs() {
            assert!(
                !input
                    .context
                    .contains("For report A, export CSV with a sample_id column.\n")
            );
        }
    }

    pub fn when_five_methods_answer_two_paired_trials_at(self, cutoff: u64) -> Self {
        self.run_five_methods(cutoff, false)
    }

    pub fn when_retrieval_and_merl_expansion_are_requested_at(self, cutoff: u64) -> Self {
        self.run_five_methods(cutoff, true)
    }

    pub fn when_two_questions_use_the_same_issue_cutoff(mut self) -> Self {
        let mut model = FakeModel::default();
        let mut merl = FakeMerl;
        let mut scorer = FakeScorer;
        self.report = Some(
            BenchmarkRunner {
                model: &mut model,
                merl: &mut merl,
                scorer: &mut scorer,
            }
            .run_suite(
                &self.fixture,
                vec![
                    EvaluationQuestion {
                        id: "current-gain".to_owned(),
                        cutoff: 4,
                        text: "What gain is current?".to_owned(),
                    },
                    EvaluationQuestion {
                        id: "gain-rationale".to_owned(),
                        cutoff: 4,
                        text: "Why did the gain change?".to_owned(),
                    },
                ],
                BenchmarkConfig {
                    model: "test-model".to_owned(),
                    model_version: "test-v1".to_owned(),
                    effort: "fixed".to_owned(),
                    system_prompt: "Answer from available evidence.".to_owned(),
                    task_prompt: "Identify current project state.".to_owned(),
                    trials: vec![TrialIdentity {
                        id: "pair-a".to_owned(),
                        randomness: RandomnessControl::Seed(7),
                    }],
                    recent_window: 2,
                    max_tool_rounds: 2,
                    search_results: 2,
                    temperature: Some(0.0),
                },
            )
            .expect("two-question benchmark"),
        );
        self.summary_calls = model.summaries;
        self
    }

    pub fn then_preparation_is_charged_once_per_trial_and_method(self) {
        let report = self.report.expect("report");
        assert_eq!(report.trials.len(), 10);
        assert_eq!(self.summary_calls, 4);
        assert_eq!(report.methods[1].total_preparation_tokens, 8);
        assert!((report.methods[1].mean_preparation_tokens - 8.0).abs() < f64::EPSILON);
        assert_eq!(report.methods[4].total_preparation_tokens, 30);
        assert_eq!(report.trials[0].question_id, "current-gain");
        assert_eq!(report.trials[5].question_id, "gain-rationale");
    }

    fn run_five_methods(mut self, cutoff: u64, request_details: bool) -> Self {
        let mut model = FakeModel {
            request_details,
            ..FakeModel::default()
        };
        let mut merl = FakeMerl;
        let mut scorer = FakeScorer;
        let mut runner = BenchmarkRunner {
            model: &mut model,
            merl: &mut merl,
            scorer: &mut scorer,
        };
        self.report = Some(
            runner
                .run(
                    &self.fixture,
                    EvaluationQuestion {
                        id: "gain-at-cutoff".to_owned(),
                        cutoff,
                        text: "What gain strategy is current?".to_owned(),
                    },
                    BenchmarkConfig {
                        model: "test-model".to_owned(),
                        model_version: "test-v1".to_owned(),
                        effort: "fixed".to_owned(),
                        system_prompt: "Answer from available evidence.".to_owned(),
                        task_prompt: "Identify current project state.".to_owned(),
                        trials: vec![
                            TrialIdentity {
                                id: "pair-a".to_owned(),
                                randomness: RandomnessControl::Seed(7),
                            },
                            TrialIdentity {
                                id: "pair-b".to_owned(),
                                randomness: RandomnessControl::Seed(11),
                            },
                        ],
                        recent_window: 2,
                        max_tool_rounds: 2,
                        search_results: 2,
                        temperature: Some(0.0),
                    },
                )
                .expect("benchmark runs"),
        );
        self.observed_questions = model.questions;
        self.observed_trials = model.trials;
        self.summary_calls = model.summaries;
        self.summary_disclosures = model.summary_disclosures;
        self.tool_result_methods = model.tool_result_methods;
        self
    }

    pub fn then_every_reader_received_the_same_question(self) -> Self {
        assert_eq!(self.observed_questions.len(), 10);
        assert!(
            self.observed_questions
                .iter()
                .all(|question| question == "What gain strategy is current?")
        );
        self
    }

    pub fn then_each_pair_shares_one_randomness_setting_and_repeats_differ(self) -> Self {
        assert_eq!(self.observed_trials.len(), 10);
        assert_eq!(&self.observed_trials[..5], &["pair-a:7"; 5]);
        assert_eq!(&self.observed_trials[5..], &["pair-b:11"; 5]);
        self
    }

    pub fn then_the_report_has_two_trials_per_method(self) -> Self {
        let report = self.report.as_ref().expect("report");
        assert_eq!(report.trials.len(), 10);
        for method in BenchmarkMethod::ALL {
            assert_eq!(
                report
                    .trials
                    .iter()
                    .filter(|trial| trial.method == method)
                    .count(),
                2
            );
        }
        self
    }

    pub fn then_summary_and_merl_preparation_are_counted_once_per_trial(self) -> Self {
        let report = self.report.as_ref().expect("report");
        assert_eq!(self.summary_calls, 8);
        assert_eq!(report.methods[1].total_preparation_tokens, 16);
        assert_eq!(report.methods[3].total_preparation_tokens, 16);
        assert_eq!(report.methods[4].total_preparation_tokens, 60);
        assert!(
            report
                .trials
                .iter()
                .all(|trial| trial.read_usage.total() <= 101)
        );
        self
    }

    pub fn then_correctness_and_provenance_are_reported_with_variance(self) -> Self {
        for method in &self.report.as_ref().expect("report").methods {
            assert!(method.mean_correctness.is_some());
            assert!(method.correctness_variance.is_some());
            assert!(method.mean_provenance.is_some());
            assert!(method.provenance_variance.is_some());
        }
        self
    }

    pub fn then_each_method_has_a_break_even_result(self) {
        let report = self.report.expect("report");
        assert_eq!(report.break_even.len(), 4);
        assert!(
            report
                .break_even
                .iter()
                .all(|result| result.first_cheaper_read.is_some())
        );
    }

    pub fn then_only_tool_enabled_readers_expand_details(self) -> Self {
        assert_eq!(self.tool_result_methods.len(), 6);
        for method in &self.tool_result_methods {
            assert!(matches!(
                method,
                BenchmarkMethod::RecentRetrieval
                    | BenchmarkMethod::SummaryRetrieval
                    | BenchmarkMethod::Merl
            ));
        }
        self
    }

    pub fn then_expansion_calls_are_included_in_token_cost(self) {
        let report = self.report.expect("report");
        for trial in report.trials {
            let expected = usize::from(matches!(
                trial.method,
                BenchmarkMethod::RecentRetrieval
                    | BenchmarkMethod::SummaryRetrieval
                    | BenchmarkMethod::Merl
            ));
            assert_eq!(trial.tool_rounds, expected);
            if expected == 1 {
                assert!(trial.read_usage.total() > 2);
            }
        }
    }

    pub fn when_a_causal_reader_requests_the_history(mut self) -> Self {
        self.reader_error =
            prepare_exact_reader_input(&self.fixture, 3, ReaderMethod::RawHistory, 2).err();
        self
    }

    pub fn when_a_reader_revisits_the_first_observation(mut self) -> Self {
        self.raw = Some(
            prepare_reader_input(&self.fixture, 1, ReaderMethod::RawHistory, 2)
                .expect("available evidence context"),
        );
        self
    }

    pub fn then_the_missing_body_is_visible_as_a_gap(self) -> Self {
        let input = self.raw.as_ref().expect("reader input");
        assert!(input.context.contains("body unavailable"));
        assert_eq!(input.fidelity.missing_bodies, vec![1]);
        self
    }

    pub fn then_the_later_edit_is_not_disclosed(self) {
        let input = self.raw.expect("reader input");
        assert!(
            !input
                .context
                .contains("unless the consumer moves to Parquet")
        );
    }

    pub fn then_a_causal_reader_refuses_it(self) {
        assert!(matches!(
            self.reader_error,
            Some(InputError::NonCausalHistory)
        ));
    }

    pub fn then_exact_replay_rejects_unresolved_order(self) {
        assert!(matches!(
            self.reader_error,
            Some(InputError::Fixture(
                merl_corpus::fixture::ValidationError::AmbiguousCausalOrder(_)
            ))
        ));
    }

    pub fn then_the_early_summaries_do_not_receive_the_opening_body(self) -> Self {
        assert!(self.summary_disclosures.iter().all(|(_, _, body)| {
            body.as_deref()
                .is_none_or(|text| !text.contains("Baseline capture procedure is not final"))
        }));
        self
    }

    pub fn then_the_opening_body_is_disclosed_after_the_history(self) {
        assert!(
            self.summary_disclosures
                .iter()
                .any(|(sequence, capture, body)| {
                    *sequence == 1
                        && *capture
                        && body.as_deref().is_some_and(|text| {
                            text.contains("Baseline capture procedure is not final")
                        })
                })
        );
    }

    fn inputs(&self) -> [&ReaderInput; 2] {
        [
            self.raw.as_ref().expect("raw prepared"),
            self.recent.as_ref().expect("recent prepared"),
        ]
    }
}

#[derive(Default)]
struct FakeModel {
    questions: Vec<String>,
    trials: Vec<String>,
    summaries: usize,
    summary_disclosures: Vec<(u64, bool, Option<String>)>,
    request_details: bool,
    tool_result_methods: Vec<BenchmarkMethod>,
}

impl ModelAdapter for FakeModel {
    fn summarize(&mut self, request: SummaryRequest<'_>) -> Result<SummaryResponse, String> {
        self.summaries += 1;
        self.summary_disclosures.push((
            request.source.sequence,
            request.source.disclosed_at_capture,
            request.source.body.map(str::to_owned),
        ));
        Ok(SummaryResponse {
            text: format!(
                "{}\n{}",
                request.previous,
                request.source.body.unwrap_or("[body unavailable]")
            ),
            usage: TokenUsage {
                input: 1,
                output: 1,
            },
        })
    }

    fn answer(&mut self, request: AnswerRequest<'_>) -> Result<AnswerResponse, String> {
        self.questions.push(request.question.to_owned());
        self.trials.push(format!(
            "{}:{}",
            request.trial.id,
            match &request.trial.randomness {
                RandomnessControl::Seed(seed) => seed.to_string(),
                RandomnessControl::Unavailable { .. } => "unavailable".to_owned(),
            }
        ));
        if self.request_details
            && request.can_search
            && !request.context.contains("[search results]")
        {
            return Ok(AnswerResponse {
                action: AnswerAction::Search("fixed".to_owned()),
                usage: TokenUsage {
                    input: 5,
                    output: 1,
                },
            });
        }
        if self.request_details && request.can_expand && !request.context.contains("[expanded") {
            return Ok(AnswerResponse {
                action: AnswerAction::Expand("D2".to_owned()),
                usage: TokenUsage {
                    input: 5,
                    output: 1,
                },
            });
        }
        if request.context.contains("[search results]") || request.context.contains("[expanded") {
            self.tool_result_methods.push(request.method);
        }
        let input = match request.method {
            BenchmarkMethod::RawHistory => 100,
            BenchmarkMethod::RollingSummary => 40,
            BenchmarkMethod::RecentRetrieval => 50,
            BenchmarkMethod::SummaryRetrieval => 35,
            BenchmarkMethod::Merl => 20,
        };
        Ok(AnswerResponse {
            action: AnswerAction::Final {
                answer: "Sweep gain for the baseline.".to_owned(),
                citations: vec!["O4".to_owned()],
            },
            usage: TokenUsage { input, output: 1 },
        })
    }
}

struct FakeMerl;

impl MerlSurface for FakeMerl {
    fn prepare(&mut self, question: &EvaluationQuestion) -> Result<MerlPrepared, String> {
        Ok(MerlPrepared {
            source_cutoff: question.cutoff,
            view: "Active decision: sweep gain for the baseline. (D2)".to_owned(),
            preparation_usage: TokenUsage {
                input: 20,
                output: 10,
            },
            required_coverage_complete: true,
            causal: true,
        })
    }

    fn expand(&mut self, _reference: &str) -> Result<String, String> {
        Ok("Source observation O4 supports D2.".to_owned())
    }
}

struct FakeScorer;

impl Scorer for FakeScorer {
    fn score(
        &mut self,
        _question_id: &str,
        _answer: &str,
        _citations: &[String],
    ) -> Result<Score, String> {
        Ok(Score {
            correctness: Some(1.0),
            provenance: Some(1.0),
            stale_state_error: false,
            missed_blocker: false,
        })
    }
}
