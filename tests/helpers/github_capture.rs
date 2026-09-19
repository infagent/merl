use corpus::{
    fixture::{Fixture, ValidationError, require_exact_history, source_digest, validate},
    github::fixture_from_graphql_pages,
};

pub struct GithubCapture {
    source: &'static str,
    captured_at: &'static str,
    fixture: Option<Fixture>,
}

impl GithubCapture {
    pub fn two_pages_with_issue_edit() -> Self {
        Self {
            source: include_str!("../fixtures/github_two_page_edit.json"),
            captured_at: "2026-01-02T00:00:00Z",
            fixture: None,
        }
    }

    pub fn with_capture_time(mut self, captured_at: &'static str) -> Self {
        self.captured_at = captured_at;
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
            require_exact_history(self.fixture(), 2),
            Err(ValidationError::MissingHistoricalBody(1))
        );
    }

    pub fn then_capture_fails_for_timestamp(self) {
        assert!(
            fixture_from_graphql_pages("DEV-FAKE", self.captured_at, self.source.as_bytes())
                .is_err_and(|error| error.contains("RFC 3339"))
        );
    }
}
