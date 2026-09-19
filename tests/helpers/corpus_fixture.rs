use corpus::fixture::{
    EvidenceRole, Fixture, ObjectLifecycle, RelationKind, SupportStatus, ValidationError,
    source_digest, validate,
};

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
        self.expects_state(object, ObjectLifecycle::Active)
    }

    pub fn expects_superseded(self, object: &str) -> Self {
        self.expects_state(object, ObjectLifecycle::Superseded)
    }

    pub fn expects_partial_support(self, object: &str) -> Self {
        let gold = self.object(object);
        assert_eq!(gold.support_status, SupportStatus::PartiallySupported);
        self
    }

    pub fn expects_disputed_by(self, object: &str, observation: u64) -> Self {
        let gold = self.object(object);
        assert!(gold.evidence.iter().any(|item| {
            item.observation == observation && matches!(item.role, EvidenceRole::Disputes)
        }));
        self
    }

    pub fn expects_supersedes(self, newer: &str, older: &str, observation: u64) -> Self {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let state = self
            .fixture
            .gold_states
            .iter()
            .find(|state| state.source_observation_cutoff == cutoff)
            .unwrap();
        assert!(state.relations.iter().any(|relation| {
            relation.from == newer
                && relation.to == older
                && relation.observation == observation
                && matches!(relation.kind, RelationKind::Supersedes)
        }));
        self
    }

    fn object(&self, key: &str) -> &corpus::fixture::GoldObject {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        self.fixture
            .gold_states
            .iter()
            .find(|state| state.source_observation_cutoff == cutoff)
            .unwrap()
            .objects
            .iter()
            .find(|item| item.key == key)
            .unwrap()
    }

    fn expects_state(self, object: &str, expected: ObjectLifecycle) -> Self {
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
                .any(|item| item.key == object && item.lifecycle == expected),
            "{object} should be {expected:?} at cutoff {cutoff}"
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
        gold_object.evidence.push(corpus::fixture::GoldEvidence {
            observation,
            role: corpus::fixture::EvidenceRole::Supports,
        });
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

    pub fn rejects_observation_before_creation(self) -> Self {
        let mut changed = self.fixture.clone();
        "2025-12-31T09:00:00Z".clone_into(&mut changed.observations[0].occurred_at);
        refresh_digest(&mut changed);
        assert_eq!(
            validate(&changed),
            Err(ValidationError::ObservationBeforeCreation(1))
        );
        self
    }

    pub fn rejects_observation_after_capture(self) -> Self {
        let mut changed = self.fixture.clone();
        "2026-09-19T00:00:00Z".clone_into(&mut changed.observations[3].occurred_at);
        refresh_digest(&mut changed);
        assert_eq!(
            validate(&changed),
            Err(ValidationError::ObservationAfterCapture(4))
        );
        self
    }

    pub fn rejects_duplicate_source_version(self) -> Self {
        let mut changed = self.fixture.clone();
        let (first, rest) = changed.observations.split_at_mut(1);
        rest[0].provider_id.clone_from(&first[0].provider_id);
        rest[0].version_id.clone_from(&first[0].version_id);
        changed.observations[1].supersedes = Some(1);
        refresh_digest(&mut changed);
        assert_eq!(
            validate(&changed),
            Err(ValidationError::DuplicateVersionIdentity(2))
        );
        self
    }

    pub fn rejects_duplicate_gold_cutoff(self) -> Self {
        let mut changed = self.fixture.clone();
        changed.gold_states.push(changed.gold_states[0].clone());
        assert_eq!(
            validate(&changed),
            Err(ValidationError::DuplicateGoldCutoff(1))
        );
        self
    }

    pub fn rejects_duplicate_gold_object(self) {
        let mut changed = self.fixture.clone();
        let duplicate = changed.gold_states[0].objects[0].clone();
        changed.gold_states[0].objects.push(duplicate);
        assert_eq!(
            validate(&changed),
            Err(ValidationError::DuplicateGoldObject(
                "gain-strategy".to_owned()
            ))
        );
    }
}

fn refresh_digest(fixture: &mut Fixture) {
    fixture.capture.source_sha256 =
        source_digest(&fixture.provider_snapshot, &fixture.observations);
}
