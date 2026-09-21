use merl_corpus::fixture::{
    EvidenceRole, Fixture, GoldObjectState, RelationKind, SupportStatus, TaskCommitment,
    TaskExecution, TaskScheduling, ValidationError, source_digest, validate,
};

pub struct CorpusFixture {
    fixture: Fixture,
    cutoff: Option<u64>,
    expected_leak: Option<(String, u64)>,
    future_support_validation: Option<Result<(), ValidationError>>,
    invalid_variant_results: Vec<Result<(), ValidationError>>,
    deferred_task_validation: Vec<Result<(), ValidationError>>,
}

impl CorpusFixture {
    pub fn from_json(source: &str) -> Self {
        Self {
            fixture: serde_json::from_str(source).expect("controlled fixture should parse"),
            cutoff: None,
            expected_leak: None,
            future_support_validation: None,
            invalid_variant_results: Vec::new(),
            deferred_task_validation: Vec::new(),
        }
    }

    pub fn given_valid_fixture(self) -> Self {
        validate(&self.fixture).expect("curated fixture should preserve causal cutoffs");
        self
    }

    pub fn when_viewed_at_cutoff(mut self, cutoff: u64) -> Self {
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

    pub fn then_is_active(self, object: &str) -> Self {
        self.expects_state(object, GoldObjectState::Active)
    }

    pub fn then_is_superseded(self, object: &str) -> Self {
        self.expects_state(object, GoldObjectState::Superseded)
    }

    pub fn then_is_candidate(self, object: &str) -> Self {
        self.expects_state(object, GoldObjectState::Candidate)
    }

    pub fn then_is_open(self, object: &str) -> Self {
        self.expects_state(object, GoldObjectState::Open)
    }

    pub fn then_has_no_active_decision(self) -> Self {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let state = self
            .fixture
            .gold_states
            .iter()
            .find(|state| state.source_observation_cutoff == cutoff)
            .expect("gold state");
        assert!(!state.objects.iter().any(|object| {
            object.kind == "decision" && object.lifecycle == GoldObjectState::Active
        }));
        self
    }

    pub fn then_task_is_reviewed_at(self, task: &str, timestamp: &str) -> Self {
        let review = self
            .object(task)
            .task_plan
            .as_ref()
            .unwrap()
            .review_at
            .as_deref();
        assert_eq!(review, Some(timestamp));
        self
    }

    pub fn then_has_current_support(self, object: &str) -> Self {
        let gold = self.object(object);
        assert_eq!(gold.support_status, SupportStatus::Current);
        self
    }

    pub fn then_revalidation_is_pending(self, object: &str) -> Self {
        let gold = self.object(object);
        assert_eq!(gold.support_status, SupportStatus::RevalidationPending);
        self
    }

    pub fn then_evidence_changed_by(self, object: &str, observation: u64) -> Self {
        let gold = self.object(object);
        assert!(gold.evidence.iter().any(|item| {
            item.observation == observation && matches!(item.role, EvidenceRole::EvidenceChanged)
        }));
        self
    }

    pub fn then_source_supersedes(self, newer: u64, older: u64) -> Self {
        let observation = self
            .fixture
            .observations
            .iter()
            .find(|item| item.sequence == newer)
            .expect("newer observation should exist");
        let prior = self
            .fixture
            .observations
            .iter()
            .find(|item| item.sequence == older)
            .expect("older observation should exist");
        assert_eq!(observation.supersedes, Some(older));
        assert_eq!(observation.provider_id, prior.provider_id);
        self
    }

    pub fn then_task_awaits_acceptance(self, task: &str) -> Self {
        let plan = self.object(task).task_plan.as_ref().unwrap();
        assert_eq!(plan.commitment, TaskCommitment::Pending);
        assert_eq!(plan.scheduling, TaskScheduling::Unscheduled);
        assert_eq!(plan.execution, TaskExecution::NotStarted);
        self
    }

    pub fn then_task_starts_after_pr_merge(self, task: &str, pull_request: &str) -> Self {
        let predicate = self
            .object(task)
            .task_plan
            .as_ref()
            .unwrap()
            .start_after
            .as_ref()
            .unwrap();
        assert_eq!(predicate.subject, pull_request);
        assert_eq!(predicate.predicate, "merged");
        assert!(predicate.expected);
        self
    }

    pub fn then_task_is_accepted_but_deferred_until_pr_merge(
        self,
        task: &str,
        pull_request: &str,
    ) -> Self {
        let plan = self.object(task).task_plan.as_ref().unwrap();
        assert_eq!(plan.commitment, TaskCommitment::Accepted);
        assert_eq!(plan.scheduling, TaskScheduling::Deferred);
        assert_eq!(plan.execution, TaskExecution::NotStarted);
        assert!(
            plan.deferral_reason
                .as_deref()
                .is_some_and(|reason| !reason.is_empty())
        );
        self.then_task_starts_after_pr_merge(task, pull_request)
    }

    pub fn when_deferred_task_lacks_reason_or_condition(mut self, task: &str) -> Self {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        for field in ["deferral_reason", "start_after"] {
            let mut changed = self.fixture.clone();
            let state = changed
                .gold_states
                .iter_mut()
                .find(|state| state.source_observation_cutoff == cutoff)
                .unwrap();
            let plan = state
                .objects
                .iter_mut()
                .find(|object| object.key == task)
                .unwrap()
                .task_plan
                .as_mut()
                .unwrap();
            if field == "deferral_reason" {
                plan.deferral_reason = None;
            } else {
                plan.start_after = None;
            }
            self.deferred_task_validation.push(validate(&changed));
        }
        self
    }

    pub fn then_rejects_deferred_task_without_reason_or_condition(self, task: &str) {
        assert_eq!(self.deferred_task_validation.len(), 2);
        for result in &self.deferred_task_validation {
            assert_eq!(
                result,
                &Err(ValidationError::InvalidTaskPlan(task.to_owned()))
            );
        }
    }

    pub fn then_has_partial_support(self, object: &str) -> Self {
        let gold = self.object(object);
        assert_eq!(gold.support_status, SupportStatus::PartiallySupported);
        self
    }

    pub fn then_is_disputed_by(self, object: &str, observation: u64) -> Self {
        let gold = self.object(object);
        assert!(gold.evidence.iter().any(|item| {
            item.observation == observation && matches!(item.role, EvidenceRole::Disputes)
        }));
        self
    }

    pub fn then_supersedes(self, newer: &str, older: &str, observation: u64) -> Self {
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

    fn object(&self, key: &str) -> &merl_corpus::fixture::GoldObject {
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

    fn expects_state(self, object: &str, expected: GoldObjectState) -> Self {
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

    pub fn when_future_support_is_added(mut self, object: &str, observation: u64) -> Self {
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
        gold_object
            .evidence
            .push(merl_corpus::fixture::GoldEvidence {
                observation,
                role: merl_corpus::fixture::EvidenceRole::Supports,
            });
        self.expected_leak = Some((object.to_owned(), observation));
        self.future_support_validation = Some(validate(&self.fixture));
        self
    }

    pub fn then_rejected_as_future_information(self) {
        let cutoff = self.cutoff.expect("a cutoff must be selected first");
        let (object, support_observation) = self
            .expected_leak
            .expect("future support must be added before checking rejection");
        assert_eq!(
            self.future_support_validation
                .expect("future support must be validated first"),
            Err(ValidationError::FutureLeakage {
                object,
                cutoff,
                support_observation,
            })
        );
    }

    pub fn when_invalid_variants_are_validated(mut self) -> Self {
        let mut changed = self.fixture.clone();
        "2025-12-31T09:00:00Z".clone_into(&mut changed.observations[0].occurred_at);
        refresh_digest(&mut changed);
        self.invalid_variant_results.push(validate(&changed));

        let mut changed = self.fixture.clone();
        "2026-09-19T00:00:00Z".clone_into(&mut changed.observations[3].occurred_at);
        refresh_digest(&mut changed);
        self.invalid_variant_results.push(validate(&changed));

        let mut changed = self.fixture.clone();
        let (first, rest) = changed.observations.split_at_mut(1);
        rest[0].provider_id.clone_from(&first[0].provider_id);
        rest[0].version_id.clone_from(&first[0].version_id);
        changed.observations[1].supersedes = Some(1);
        refresh_digest(&mut changed);
        self.invalid_variant_results.push(validate(&changed));

        let mut changed = self.fixture.clone();
        changed.gold_states.push(changed.gold_states[0].clone());
        self.invalid_variant_results.push(validate(&changed));

        let mut changed = self.fixture.clone();
        let duplicate = changed.gold_states[0].objects[0].clone();
        changed.gold_states[0].objects.push(duplicate);
        self.invalid_variant_results.push(validate(&changed));
        self
    }

    pub fn then_rejects_an_observation_before_creation(self) -> Self {
        assert_eq!(
            self.invalid_variant_results[0],
            Err(ValidationError::ObservationBeforeCreation(1))
        );
        self
    }

    pub fn then_rejects_an_observation_after_capture(self) -> Self {
        assert_eq!(
            self.invalid_variant_results[1],
            Err(ValidationError::ObservationAfterCapture(4))
        );
        self
    }

    pub fn then_rejects_a_duplicate_source_version(self) -> Self {
        assert_eq!(
            self.invalid_variant_results[2],
            Err(ValidationError::DuplicateVersionIdentity(2))
        );
        self
    }

    pub fn then_rejects_a_duplicate_gold_cutoff(self) -> Self {
        assert_eq!(
            self.invalid_variant_results[3],
            Err(ValidationError::DuplicateGoldCutoff(1))
        );
        self
    }

    pub fn then_rejects_a_duplicate_gold_object(self) {
        assert_eq!(
            self.invalid_variant_results[4],
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
