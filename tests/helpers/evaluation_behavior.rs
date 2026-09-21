use merl_corpus::fixture::Fixture;
use merl_eval::{
    AnswerAction, AnswerRequest, AnswerResponse, BenchmarkConfig, BenchmarkMethod, BenchmarkReport,
    BenchmarkRunner, EvaluationQuestion, MerlPrepared, MerlSurface, ModelAdapter, ReaderInput,
    ReaderMethod, Score, Scorer, SummaryRequest, SummaryResponse, TokenUsage, prepare_reader_input,
};

pub struct EvaluationScenario {
    fixture: Fixture,
    raw: Option<ReaderInput>,
    recent: Option<ReaderInput>,
    report: Option<BenchmarkReport>,
    observed_questions: Vec<String>,
    summary_calls: usize,
}

impl EvaluationScenario {
    pub fn given_the_gain_discussion() -> Self {
        Self::from_fixture(include_str!("../../corpus/development/DEV-C1.json"))
    }

    pub fn given_the_edited_export_discussion() -> Self {
        Self::from_fixture(include_str!("../../corpus/development/DEV-C3.json"))
    }

    fn from_fixture(json: &str) -> Self {
        Self {
            fixture: serde_json::from_str(json).expect("controlled fixture"),
            raw: None,
            recent: None,
            report: None,
            observed_questions: Vec::new(),
            summary_calls: 0,
        }
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

    pub fn when_five_methods_answer_two_paired_trials_at(mut self, cutoff: u64) -> Self {
        let mut model = FakeModel::default();
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
                        trials: 2,
                        recent_window: 2,
                        max_tool_rounds: 2,
                        search_results: 2,
                        temperature: Some(0.0),
                        seed: Some(7),
                    },
                )
                .expect("benchmark runs"),
        );
        self.observed_questions = model.questions;
        self.summary_calls = model.summaries;
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
    summaries: usize,
}

impl ModelAdapter for FakeModel {
    fn summarize(&mut self, request: SummaryRequest<'_>) -> Result<SummaryResponse, String> {
        self.summaries += 1;
        Ok(SummaryResponse {
            text: format!("{}\n{}", request.previous, request.source.body),
            usage: TokenUsage {
                input: 1,
                output: 1,
            },
        })
    }

    fn answer(&mut self, request: AnswerRequest<'_>) -> Result<AnswerResponse, String> {
        self.questions.push(request.question.to_owned());
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
