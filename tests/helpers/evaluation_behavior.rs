use merl_corpus::fixture::Fixture;
use merl_eval::{ReaderInput, ReaderMethod, prepare_reader_input};

pub struct EvaluationScenario {
    fixture: Fixture,
    raw: Option<ReaderInput>,
    recent: Option<ReaderInput>,
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
        }
    }

    pub fn when_raw_and_recent_contexts_are_prepared_at(mut self, cutoff: u64) -> Self {
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
            assert!(input.context.contains("unless the consumer moves to Parquet"));
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

    fn inputs(&self) -> [&ReaderInput; 2] {
        [
            self.raw.as_ref().expect("raw prepared"),
            self.recent.as_ref().expect("recent prepared"),
        ]
    }
}
