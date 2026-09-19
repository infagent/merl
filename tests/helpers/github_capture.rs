use corpus::{
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
            corpus::fixture::require_unambiguous_order_through(fixture, 3),
            Err(ValidationError::AmbiguousCausalOrder(4))
        );
        assert_eq!(
            corpus::fixture::require_unambiguous_order_through(fixture, 4),
            Err(ValidationError::AmbiguousCausalOrder(4))
        );
    }
}
