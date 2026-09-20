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
}

impl GithubCapture {
    pub fn two_pages_with_issue_edit() -> Self {
        Self {
            source: include_str!("../fixtures/github_two_page_edit.json").to_owned(),
            captured_at: "2026-01-02T00:00:00Z",
            fixture: None,
        }
    }

    pub fn with_capture_time(mut self, captured_at: &'static str) -> Self {
        self.captured_at = captured_at;
        self
    }

    pub fn with_issue_edit_tied_to_last_comment(mut self) -> Self {
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

    pub fn with_offset_issue_creation_time(mut self) -> Self {
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
        validate(fixture).expect("source digest should validate");
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
        self
    }

    pub fn then_remains_valid_without_new_provider_facts(self) -> Self {
        validate(self.fixture()).expect("the old schema remains readable");
        self
    }

    pub fn then_does_not_invent_missing_provider_facts(self) -> Self {
        let fixture = self.fixture();
        let project = merl_core::ProjectId::try_from("legacy").expect("project ID");
        let mut store = merl_store::Store::open_in_memory().expect("store");
        store.create_project(&project).expect("project");
        merl_ingest::import_fixture(&mut store, &project, fixture).expect("legacy import");
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

    pub fn then_cannot_claim_the_new_schema_without_them(self) {
        let mut fixture = self.fixture().clone();
        "merl.corpus-fixture/v2".clone_into(&mut fixture.schema);
        assert_eq!(
            validate(&fixture),
            Err(ValidationError::InvalidProviderSnapshot)
        );
    }

    pub fn then_rejects_exact_replay_across_the_gap(self) {
        assert_eq!(
            require_exact_source_bodies_through(self.fixture(), 2),
            Err(ValidationError::MissingHistoricalBody(1))
        );
    }

    pub fn then_capture_fails_for_timestamp(self) {
        assert!(
            fixture_from_graphql_pages("DEV-FAKE", self.captured_at, self.source.as_bytes())
                .is_err_and(|error| error.contains("RFC 3339"))
        );
    }

    pub fn then_reports_ambiguous_cutoff(self) {
        let fixture = self.fixture();
        assert!(fixture.observations[3].ambiguous_order_with_previous);
        assert_eq!(
            merl_corpus::fixture::require_unambiguous_order_through(fixture, 3),
            Err(ValidationError::AmbiguousCausalOrder(4))
        );
        assert_eq!(
            merl_corpus::fixture::require_unambiguous_order_through(fixture, 4),
            Err(ValidationError::AmbiguousCausalOrder(4))
        );
    }

    pub fn then_rejects_truncated_labels(mut self) -> Self {
        self.expect_truncated_snapshot("labels");
        self
    }

    pub fn then_rejects_truncated_assignees(mut self) -> Self {
        self.expect_truncated_snapshot("assignees");
        self
    }

    fn expect_truncated_snapshot(&mut self, field: &str) {
        let mut pages: serde_json::Value = serde_json::from_str(&self.source).unwrap();
        for page in pages.as_array_mut().unwrap() {
            page["data"]["repository"]["issue"][field]["pageInfo"]["hasNextPage"] = true.into();
        }
        let source = serde_json::to_vec(&pages).unwrap();
        assert!(
            fixture_from_graphql_pages("DEV-FAKE", self.captured_at, &source)
                .is_err_and(|error| error.contains(field) && error.contains("incomplete"))
        );
    }

    pub fn then_rejects_mismatched_edit_identity(self) -> Self {
        self.expect_invalid_edit(
            |edit| "another-edit".clone_into(&mut edit.provider_id),
            ValidationError::InvalidContentEdit(3),
        );
        self
    }

    pub fn then_rejects_mismatched_edit_time(self) -> Self {
        self.expect_invalid_edit(
            |edit| "2026-01-01T11:30:00Z".clone_into(&mut edit.edited_at),
            ValidationError::InvalidContentEdit(3),
        );
        self.expect_invalid_edit(
            |edit| "not-a-time".clone_into(&mut edit.edited_at),
            ValidationError::InvalidTimestamp("not-a-time".to_owned()),
        );
        self
    }

    pub fn then_rejects_invalid_deletion_time(self) {
        self.expect_invalid_edit(
            |edit| edit.deleted_at = Some("not-a-time".to_owned()),
            ValidationError::InvalidTimestamp("not-a-time".to_owned()),
        );
        for deleted_at in ["2026-01-01T10:30:00Z", "2026-01-03T00:00:00Z"] {
            self.expect_invalid_edit(
                |edit| edit.deleted_at = Some(deleted_at.to_owned()),
                ValidationError::InvalidContentEdit(3),
            );
        }
    }

    fn expect_invalid_edit(
        &self,
        change: impl FnOnce(&mut merl_corpus::fixture::ContentEdit),
        expected: ValidationError,
    ) {
        let mut fixture = self.fixture().clone();
        change(fixture.observations[2].edit.as_mut().unwrap());
        fixture.capture.source_sha256 =
            source_digest(&fixture.provider_snapshot, &fixture.observations);
        assert_eq!(validate(&fixture), Err(expected));
    }
}
