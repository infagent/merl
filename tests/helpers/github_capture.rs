use merl_corpus::{
    fixture::{
        Fixture, ValidationError, require_exact_source_bodies_through, source_digest, validate,
    },
    github::fixture_from_graphql_pages,
};

pub struct GithubCapture {
    source: String,
    captured_at: &'static str,
    fixture: Option<Fixture>,
    capture_error: Option<String>,
    capture_validation: Option<Result<(), ValidationError>>,
    exact_replay_result: Option<Result<(), ValidationError>>,
    causal_cutoff_results: Option<[Result<(), ValidationError>; 2]>,
    incomplete_capture_results: Option<[Result<(), String>; 2]>,
    invalid_edit_results: Option<InvalidEditResults>,
    legacy_validation: Option<Result<(), ValidationError>>,
    legacy_store: Option<merl_store::Store>,
    new_schema_validation: Option<Result<(), ValidationError>>,
}

struct InvalidEditResults {
    identity: Result<(), ValidationError>,
    mismatched_time: Result<(), ValidationError>,
    malformed_time: Result<(), ValidationError>,
    malformed_deletion: Result<(), ValidationError>,
    early_deletion: Result<(), ValidationError>,
    late_deletion: Result<(), ValidationError>,
}

impl GithubCapture {
    pub fn given_two_pages_with_issue_edit() -> Self {
        Self {
            source: include_str!("../fixtures/github_two_page_edit.json").to_owned(),
            captured_at: "2026-01-02T00:00:00Z",
            fixture: None,
            capture_error: None,
            capture_validation: None,
            exact_replay_result: None,
            causal_cutoff_results: None,
            incomplete_capture_results: None,
            invalid_edit_results: None,
            legacy_validation: None,
            legacy_store: None,
            new_schema_validation: None,
        }
    }

    pub fn given_capture_time(mut self, captured_at: &'static str) -> Self {
        self.captured_at = captured_at;
        self
    }

    pub fn given_issue_edit_tied_to_last_comment(mut self) -> Self {
        let mut pages: serde_json::Value = serde_json::from_str(&self.source).unwrap();
        for page in pages.as_array_mut().unwrap() {
            let issue = &mut page["data"]["repository"]["issue"];
            issue["updatedAt"] = "2026-01-01T12:00:00Z".into();
            issue["lastEditedAt"] = "2026-01-01T12:00:00Z".into();
            issue["userContentEdits"]["nodes"][0]["editedAt"] = "2026-01-01T12:00:00Z".into();
        }
        self.source = serde_json::to_string(&pages).unwrap();
        self
    }

    pub fn given_offset_issue_creation_time(mut self) -> Self {
        let mut pages: serde_json::Value = serde_json::from_str(&self.source).unwrap();
        for page in pages.as_array_mut().unwrap() {
            page["data"]["repository"]["issue"]["createdAt"] = "2026-01-01T11:00:00+02:00".into();
        }
        self.source = serde_json::to_string(&pages).unwrap();
        self
    }

    pub fn when_captured(mut self) -> Self {
        self.fixture = Some(
            fixture_from_graphql_pages("DEV-FAKE", self.captured_at, self.source.as_bytes())
                .expect("two-page example should capture"),
        );
        self.capture_validation = Some(validate(self.fixture()));
        self
    }

    pub fn when_exact_replay_is_requested(mut self) -> Self {
        self.exact_replay_result = Some(require_exact_source_bodies_through(self.fixture(), 2));
        self
    }

    pub fn when_causal_cutoffs_are_checked(mut self) -> Self {
        let fixture = self.fixture();
        self.causal_cutoff_results = Some([
            merl_corpus::fixture::require_unambiguous_order_through(fixture, 3),
            merl_corpus::fixture::require_unambiguous_order_through(fixture, 4),
        ]);
        self
    }

    pub fn when_capture_is_attempted(mut self) -> Self {
        self.capture_error =
            fixture_from_graphql_pages("DEV-FAKE", self.captured_at, self.source.as_bytes()).err();
        self
    }

    fn fixture(&self) -> &Fixture {
        self.fixture.as_ref().expect("capture should run first")
    }

    pub fn then_has_four_versions_in_order(self) -> Self {
        let observations = &self.fixture().observations;
        assert_eq!(observations.len(), 4);
        assert_eq!(
            observations
                .iter()
                .map(|item| item.provider_id.as_str())
                .collect::<Vec<_>>(),
            ["issue-1", "comment-1", "issue-1", "comment-2"]
        );
        self
    }

    pub fn then_the_early_issue_body_is_unavailable(self) -> Self {
        let observations = &self.fixture().observations;
        assert!(observations[0].body.is_none());
        assert_eq!(observations[2].body.as_deref(), Some("Use variable gain."));
        self
    }

    pub fn then_creation_edit_is_one_version(self) -> Self {
        let observations = &self.fixture().observations;
        assert_eq!(observations[1].version_id, "comment-1-created");
        assert_eq!(observations[1].supersedes, None);
        assert_eq!(observations[1].body.as_deref(), Some("What gain?"));
        self
    }

    pub fn then_edit_supersedes_the_same_issue(self) -> Self {
        let observations = &self.fixture().observations;
        assert_eq!(observations[2].supersedes, Some(1));
        assert_eq!(observations[2].version_id, "edit-1");
        self
    }

    pub fn then_preserves_stable_actor_ids(self) -> Self {
        let fixture = self.fixture();
        assert_eq!(
            fixture.observations[0]
                .author
                .as_ref()
                .unwrap()
                .provider_id
                .as_deref(),
            Some("user-1")
        );
        assert_eq!(
            fixture.observations[2]
                .edit
                .as_ref()
                .unwrap()
                .editor
                .as_ref()
                .unwrap()
                .provider_id
                .as_deref(),
            Some("user-2")
        );
        assert_eq!(
            fixture.provider_snapshot.assignees[0]
                .provider_id
                .as_deref(),
            Some("user-2")
        );
        self
    }

    pub fn then_has_verified_source_digest(self) -> Self {
        let fixture = self.fixture();
        assert_eq!(
            fixture.capture.source_sha256,
            source_digest(&fixture.provider_snapshot, &fixture.observations)
        );
        assert_eq!(
            self.capture_validation
                .as_ref()
                .expect("captured fixture was validated"),
            &Ok(())
        );
        self
    }

    pub fn then_uses_the_new_schema(self) -> Self {
        assert_eq!(self.fixture().schema, "merl.corpus-fixture/v2");
        self
    }

    pub fn when_read_as_a_legacy_capture(mut self) -> Self {
        let fixture = self.fixture.as_mut().expect("capture should run first");
        "merl.corpus-fixture/v1".clone_into(&mut fixture.schema);
        fixture.provider_snapshot.updated_at = None;
        fixture.provider_snapshot.label_refs.clear();
        for observation in &mut fixture.observations {
            observation.updated_at = None;
        }
        fixture.capture.source_sha256 =
            source_digest(&fixture.provider_snapshot, &fixture.observations);
        self.legacy_validation = Some(validate(self.fixture()));
        let project = merl_core::ProjectId::try_from("legacy").expect("project ID");
        let mut store = merl_store::Store::open_in_memory().expect("store");
        store.create_project(&project).expect("project");
        merl_ingest::import_fixture(&mut store, &project, self.fixture()).expect("legacy import");
        self.legacy_store = Some(store);
        self
    }

    pub fn then_remains_valid_without_new_provider_facts(self) -> Self {
        assert_eq!(
            self.legacy_validation
                .as_ref()
                .expect("legacy validation ran"),
            &Ok(())
        );
        self
    }

    pub fn then_does_not_invent_missing_provider_facts(self) -> Self {
        let fixture = self.fixture();
        let project = merl_core::ProjectId::try_from("legacy").expect("project ID");
        let store = self
            .legacy_store
            .as_ref()
            .expect("legacy capture was imported");
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue ID");
        let head = store
            .provider_issue_head(&project, &issue)
            .expect("provider head")
            .expect("accepted observation");
        assert_eq!(head.input.upstream_updated_at_millis, None);
        assert_eq!(head.input.label_provider_ids, None);
        assert_eq!(
            head.input.assignee_provider_ids,
            Some(vec!["user-2".to_owned()])
        );
        for sequence in 1..=4 {
            let version = store
                .source_version_at(&project, sequence)
                .expect("source lookup")
                .expect("legacy version");
            assert_eq!(version.upstream_updated_at_millis, None);
        }
        self
    }

    pub fn when_legacy_capture_claims_the_new_schema(mut self) -> Self {
        let mut fixture = self.fixture().clone();
        "merl.corpus-fixture/v2".clone_into(&mut fixture.schema);
        self.new_schema_validation = Some(validate(&fixture));
        self
    }

    pub fn then_cannot_claim_the_new_schema_without_them(self) {
        assert_eq!(
            self.new_schema_validation
                .expect("new-schema claim was validated"),
            Err(ValidationError::InvalidProviderSnapshot),
        );
    }

    pub fn then_rejects_exact_replay_across_the_gap(self) {
        assert_eq!(
            self.exact_replay_result
                .expect("exact replay was requested"),
            Err(ValidationError::MissingHistoricalBody(1))
        );
    }

    pub fn then_capture_fails_for_timestamp(self) {
        assert!(
            self.capture_error
                .as_deref()
                .is_some_and(|error| error.contains("RFC 3339"))
        );
    }

    pub fn then_reports_ambiguous_cutoff(self) {
        let fixture = self.fixture();
        assert!(fixture.observations[3].ambiguous_order_with_previous);
        let [before_tie, after_tie] = self
            .causal_cutoff_results
            .expect("causal cutoffs were checked");
        assert_eq!(before_tie, Err(ValidationError::AmbiguousCausalOrder(4)));
        assert_eq!(after_tie, Err(ValidationError::AmbiguousCausalOrder(4)));
    }

    pub fn when_incomplete_provider_snapshots_are_captured(mut self) -> Self {
        self.incomplete_capture_results = Some([
            self.capture_truncated_snapshot("labels"),
            self.capture_truncated_snapshot("assignees"),
        ]);
        self
    }

    pub fn then_truncated_labels_are_rejected(self) -> Self {
        let result = &self
            .incomplete_capture_results
            .as_ref()
            .expect("incomplete captures were attempted")[0];
        assert!(
            matches!(result, Err(error) if error.contains("labels") && error.contains("incomplete"))
        );
        self
    }

    pub fn then_truncated_assignees_are_rejected(self) {
        let result = &self
            .incomplete_capture_results
            .as_ref()
            .expect("incomplete captures were attempted")[1];
        assert!(
            matches!(result, Err(error) if error.contains("assignees") && error.contains("incomplete"))
        );
    }

    fn capture_truncated_snapshot(&self, field: &str) -> Result<(), String> {
        let mut pages: serde_json::Value = serde_json::from_str(&self.source).unwrap();
        for page in pages.as_array_mut().unwrap() {
            page["data"]["repository"]["issue"][field]["pageInfo"]["hasNextPage"] = true.into();
        }
        let source = serde_json::to_vec(&pages).unwrap();
        fixture_from_graphql_pages("DEV-FAKE", self.captured_at, &source).map(|_| ())
    }

    pub fn when_malformed_edit_metadata_is_validated(mut self) -> Self {
        self.invalid_edit_results = Some(InvalidEditResults {
            identity: self.validate_after_edit(|edit| {
                "another-edit".clone_into(&mut edit.provider_id);
            }),
            mismatched_time: self.validate_after_edit(|edit| {
                "2026-01-01T11:30:00Z".clone_into(&mut edit.edited_at);
            }),
            malformed_time: self.validate_after_edit(|edit| {
                "not-a-time".clone_into(&mut edit.edited_at);
            }),
            malformed_deletion: self.validate_after_edit(|edit| {
                edit.deleted_at = Some("not-a-time".to_owned());
            }),
            early_deletion: self.validate_after_edit(|edit| {
                edit.deleted_at = Some("2026-01-01T10:30:00Z".to_owned());
            }),
            late_deletion: self.validate_after_edit(|edit| {
                edit.deleted_at = Some("2026-01-03T00:00:00Z".to_owned());
            }),
        });
        self
    }

    pub fn then_mismatched_edit_identity_is_rejected(self) -> Self {
        let results = self
            .invalid_edit_results
            .as_ref()
            .expect("edit metadata was validated");
        assert_eq!(
            results.identity,
            Err(ValidationError::InvalidContentEdit(3))
        );
        self
    }

    pub fn then_mismatched_edit_time_is_rejected(self) -> Self {
        let results = self
            .invalid_edit_results
            .as_ref()
            .expect("edit metadata was validated");
        assert_eq!(
            results.mismatched_time,
            Err(ValidationError::InvalidContentEdit(3))
        );
        assert_eq!(
            results.malformed_time,
            Err(ValidationError::InvalidTimestamp("not-a-time".to_owned())),
        );
        self
    }

    pub fn then_invalid_deletion_time_is_rejected(self) {
        let results = self
            .invalid_edit_results
            .as_ref()
            .expect("edit metadata was validated");
        assert_eq!(
            results.malformed_deletion,
            Err(ValidationError::InvalidTimestamp("not-a-time".to_owned()))
        );
        assert_eq!(
            results.early_deletion,
            Err(ValidationError::InvalidContentEdit(3))
        );
        assert_eq!(
            results.late_deletion,
            Err(ValidationError::InvalidContentEdit(3))
        );
    }

    fn validate_after_edit(
        &self,
        change: impl FnOnce(&mut merl_corpus::fixture::ContentEdit),
    ) -> Result<(), ValidationError> {
        let mut fixture = self.fixture().clone();
        change(fixture.observations[2].edit.as_mut().unwrap());
        fixture.capture.source_sha256 =
            source_digest(&fixture.provider_snapshot, &fixture.observations);
        validate(&fixture)
    }
}
