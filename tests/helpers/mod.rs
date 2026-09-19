use merl_corpus::corpus::{Fixture, ValidationError, validate};

pub struct CorpusFixture {
    fixture: Fixture,
    cutoff: Option<u64>,
    expected_leak: Option<(String, u64)>,
}

impl CorpusFixture {
    pub fn from_json(source: &str) -> Self {
        Self {
            fixture: serde_json::from_str(source).expect("controlled fixture should parse"),
            cutoff: None,
            expected_leak: None,
        }
    }

    pub fn given_valid_fixture(self) -> Self {
        validate(&self.fixture).expect("curated fixture should preserve causal cutoffs");
        self
    }

    pub fn at_cutoff(mut self, cutoff: u64) -> Self {
        assert!(
            self.fixture
                .gold_states
                .iter()
                .any(|state| state.source_observation_cutoff == cutoff),
            "fixture should contain cutoff {cutoff}"
        );
        self.cutoff = Some(cutoff);
        self
    }

    pub fn expects_active(self, object: &str) -> Self {
        self.expects_state(object, "active")
    }

    pub fn expects_superseded(self, object: &str) -> Self {
        self.expects_state(object, "superseded")
    }

    fn expects_state(self, object: &str, expected: &str) -> Self {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let state = self
            .fixture
            .gold_states
            .iter()
            .find(|state| state.source_observation_cutoff == cutoff)
            .expect("selected cutoff should still exist");
        assert!(
            state
                .objects
                .iter()
                .any(|item| item.key == object && item.state == expected),
            "{object} should be {expected} at cutoff {cutoff}"
        );
        self
    }

    pub fn add_support(mut self, object: &str, observation: u64) -> Self {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let state = self
            .fixture
            .gold_states
            .iter_mut()
            .find(|state| state.source_observation_cutoff == cutoff)
            .expect("selected cutoff should still exist");
        let gold_object = state
            .objects
            .iter_mut()
            .find(|item| item.key == object)
            .expect("named gold object should exist at the selected cutoff");
        gold_object.support_observations.push(observation);
        self.expected_leak = Some((object.to_owned(), observation));
        self
    }

    pub fn then_rejected_as_future_information(self) {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let (object, support_observation) = self
            .expected_leak
            .expect("future support must be added before checking rejection");
        assert_eq!(
            validate(&self.fixture),
            Err(ValidationError::FutureLeakage {
                object,
                cutoff,
                support_observation,
            })
        );
    }
}
