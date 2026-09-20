use merl_core::{
    CapturePolicyVersion, CompilationMode, CoverageRequirement, ProjectId, SourceBindingId,
    SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_ingest::import_fixture;
use merl_store::{PayloadRead, SourceBinding, SourceCapture, Store};
use sha2::{Digest, Sha256};

#[test]
fn issue_edits_are_captured_once_and_keep_their_source_lineage() {
    let fixture = serde_json::from_str(include_str!("../corpus/development/DEV-C3.json"))
        .expect("development fixture");
    let mut project = IssueHistory::new("P1");

    project
        .when_importing(&fixture)
        .then_observation_head_is(4)
        .then_observations_follow_the_fixture(&fixture)
        .then_provider_issue_is_open_at_revision(&fixture, 1)
        .then_version_supersedes("controlled:DEV-C3:issue:v2", "controlled:DEV-C3:issue:v1")
        .then_version_supersedes("controlled:DEV-C3:issue:v3", "controlled:DEV-C3:issue:v2")
        .then_version_has_cold_metadata("controlled:DEV-C3:issue:v2")
        .then_version_body_is(
            "controlled:DEV-C3:issue:v1",
            b"For report A, export CSV with a sample_id column.",
        )
        .when_importing(&fixture)
        .then_observation_head_is(4)
        .then_provider_issue_is_open_at_revision(&fixture, 1)
        .when_recapturing_later(&fixture)
        .then_observation_head_is(4)
        .then_provider_issue_is_open_at_revision(&fixture, 1);
}

#[test]
fn an_edit_keeps_its_editor_and_protected_diff() {
    let mut fixture: merl_corpus::fixture::Fixture =
        serde_json::from_str(include_str!("../corpus/development/DEV-C3.json"))
            .expect("development fixture");
    let edit = fixture.observations[1].edit.as_mut().expect("staged edit");
    edit.editor = Some(merl_corpus::fixture::ActorRef {
        provider_id: Some("controlled:editor".to_owned()),
        login: "editor".to_owned(),
    });
    edit.diff = Some("+ unless the consumer moves to Parquet".to_owned());
    fixture.capture.source_sha256 =
        merl_corpus::fixture::source_digest(&fixture.provider_snapshot, &fixture.observations);

    let mut project = IssueHistory::new("P2");
    project.when_importing(&fixture);
    let version = merl_ingest::fixture_version_id(&fixture.observations[1].version_id)
        .expect("edit version ID");
    let captured = project
        .store
        .source_version(&project.project, &version)
        .expect("source metadata")
        .expect("edit captured");
    assert_eq!(
        captured.provider_actor_id.as_deref(),
        Some("controlled:editor")
    );
    let diff = captured.edit_diff_payload.expect("protected edit diff");
    assert_eq!(
        project
            .store
            .read_payload(&project.project, &diff)
            .expect("edit evidence"),
        PayloadRead::Available(b"+ unless the consumer moves to Parquet".to_vec())
    );
}

#[test]
fn an_older_provider_update_cannot_reopen_a_newer_closed_issue_even_if_polled_later() {
    let earlier: merl_corpus::fixture::Fixture =
        serde_json::from_str(include_str!("../corpus/development/DEV-C3.json"))
            .expect("development fixture");
    let mut later = earlier.clone();
    later.provider_snapshot.state = "CLOSED".to_owned();
    later.provider_snapshot.updated_at = Some("2026-09-20T00:40:00Z".to_owned());
    later.provider_snapshot.closed_at = Some("2026-09-20T00:30:00Z".to_owned());
    later.capture.captured_at = "2026-09-20T01:00:00Z".to_owned();
    later.capture.source_sha256 =
        merl_corpus::fixture::source_digest(&later.provider_snapshot, &later.observations);

    let mut project = IssueHistory::new("P3");
    project.when_importing(&later);
    let mut confirmed = later.clone();
    confirmed.capture.captured_at = "2026-09-20T02:00:00Z".to_owned();
    project.when_importing(&confirmed);
    let mut stale = earlier;
    stale.provider_snapshot.updated_at = Some("2026-09-19T00:00:00Z".to_owned());
    stale.capture.captured_at = "2026-09-20T03:00:00Z".to_owned();
    stale.capture.source_sha256 =
        merl_corpus::fixture::source_digest(&stale.provider_snapshot, &stale.observations);
    let result = import_fixture(&mut project.store, &project.project, &stale);
    assert!(matches!(
        result,
        Err(merl_ingest::ImportError::Store(
            merl_store::StoreError::StaleProviderObservation
        ))
    ));
    assert_eq!(
        project
            .store
            .project_revision(&project.project)
            .expect("revision")
            .get(),
        1
    );
    let issue = merl_ingest::fixture_issue_id(&later).expect("Issue ID");
    let head = project
        .store
        .provider_issue_head(&project.project, &issue)
        .expect("provider head")
        .expect("accepted provider observation");
    assert_eq!(head.input.state.as_str(), "closed");
    assert_eq!(head.revision.get(), 1);
}

#[test]
fn a_github_issue_import_keeps_provider_identity_and_version_lineage() {
    let fixture = merl_corpus::github::fixture_from_graphql_pages(
        "DEV-FAKE",
        "2026-01-02T00:00:00Z",
        include_bytes!("fixtures/github_two_page_edit.json"),
    )
    .expect("GitHub-shaped fixture");
    let mut project = IssueHistory::new("P4");
    project
        .when_importing(&fixture)
        .then_observation_head_is(4)
        .then_github_binding_and_versions(&fixture)
        .then_github_provider_facts(&fixture)
        .when_provider_snapshot_bytes_are_erased(&fixture)
        .then_github_provider_facts(&fixture);
}

#[test]
fn a_retry_keeps_the_first_capture_time_and_policy() {
    IssueHistory::new("P5")
        .when_a_source_is_recaptured_under_another_policy()
        .then_first_capture_metadata_still_applies();
}

struct IssueHistory {
    store: Store,
    project: ProjectId,
}

impl IssueHistory {
    fn new(id: &str) -> Self {
        let project = ProjectId::try_from(id).expect("project ID");
        let mut store = Store::open_in_memory().expect("open store");
        store.create_project(&project).expect("create project");
        Self { store, project }
    }

    fn when_importing(&mut self, fixture: &merl_corpus::fixture::Fixture) -> &mut Self {
        import_fixture(&mut self.store, &self.project, fixture).expect("import fixture");
        self
    }

    fn when_recapturing_later(&mut self, fixture: &merl_corpus::fixture::Fixture) -> &mut Self {
        let mut later = fixture.clone();
        "2026-09-20T00:00:00Z".clone_into(&mut later.capture.captured_at);
        self.when_importing(&later)
    }

    fn when_a_source_is_recaptured_under_another_policy(&mut self) -> &mut Self {
        let binding = SourceBinding {
            id: SourceBindingId::try_from("binding").unwrap(),
            provider: SourceProvider::try_from("github").unwrap(),
            provider_namespace_id: "repo-1".to_owned(),
            namespace_digest: Sha256::digest(b"repo-1").into(),
        };
        let first = SourceCapture {
            binding,
            source: SourceId::try_from("source").unwrap(),
            provider_entity_id: "issue-1",
            version: SourceVersionId::try_from("version").unwrap(),
            provider_version_id: "issue-1:initial",
            kind: SourceKind::try_from("issue").unwrap(),
            supersedes: None,
            ambiguous_order_with_previous: false,
            created_at_millis: 1,
            occurred_at_millis: 1,
            observed_at_millis: 2,
            actor: None,
            provider_actor_id: None,
            body: Some(b"Use fixed gain."),
            edit_diff: None,
            edit_deleted_at_millis: None,
            missing_body_reason: None,
            compilation_mode: CompilationMode::Eager,
            coverage_requirement: CoverageRequirement::Required,
            policy_version: CapturePolicyVersion::try_from("initial").unwrap(),
        };
        assert!(
            self.store
                .capture_source_version(&self.project, &first)
                .unwrap()
        );
        let retry = SourceCapture {
            observed_at_millis: 3,
            compilation_mode: CompilationMode::CaptureOnly,
            coverage_requirement: CoverageRequirement::Optional,
            policy_version: CapturePolicyVersion::try_from("new_policy").unwrap(),
            ..first
        };
        assert!(
            !self
                .store
                .capture_source_version(&self.project, &retry)
                .unwrap()
        );
        self
    }

    fn then_first_capture_metadata_still_applies(&mut self) -> &mut Self {
        let captured = self
            .store
            .source_version(
                &self.project,
                &SourceVersionId::try_from("version").unwrap(),
            )
            .unwrap()
            .expect("first version");
        assert_eq!(captured.observed_at_millis, 2);
        assert_eq!(captured.compilation_mode, CompilationMode::Eager);
        assert_eq!(captured.coverage_requirement, CoverageRequirement::Required);
        assert_eq!(captured.policy_version.as_str(), "initial");
        assert_eq!(
            self.store.source_observation_head(&self.project).unwrap(),
            1
        );
        self
    }

    fn then_observation_head_is(&mut self, expected: u64) -> &mut Self {
        assert_eq!(
            self.store
                .source_observation_head(&self.project)
                .expect("head"),
            expected
        );
        self
    }

    fn then_observations_follow_the_fixture(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
    ) -> &mut Self {
        for observation in &fixture.observations {
            let captured = self
                .store
                .source_version_at(&self.project, observation.sequence)
                .expect("observation lookup")
                .expect("captured observation");
            assert_eq!(captured.provider_version_id, observation.version_id);
            assert_eq!(captured.sequence, observation.sequence);
        }
        self
    }

    fn then_version_supersedes(&mut self, newer: &str, older: &str) -> &mut Self {
        let newer = merl_ingest::fixture_version_id(newer).expect("version ID");
        let older = merl_ingest::fixture_version_id(older).expect("version ID");
        let version = self
            .store
            .source_version(&self.project, &newer)
            .expect("version")
            .expect("captured version");
        assert_eq!(version.supersedes, Some(older));
        self
    }

    fn then_version_body_is(&mut self, version: &str, expected: &[u8]) -> &mut Self {
        let version = merl_ingest::fixture_version_id(version).expect("version ID");
        let version = self
            .store
            .source_version(&self.project, &version)
            .expect("version")
            .expect("captured version");
        let payload = version.payload.expect("protected body");
        assert_eq!(
            self.store
                .read_payload(&self.project, &payload)
                .expect("body"),
            PayloadRead::Available(expected.to_vec())
        );
        self
    }

    fn then_version_has_cold_metadata(&mut self, provider_version: &str) -> &mut Self {
        let version = merl_ingest::fixture_version_id(provider_version).expect("version ID");
        let capture = self
            .store
            .source_version(&self.project, &version)
            .expect("source metadata")
            .expect("captured version");
        assert_eq!(capture.provider_version_id, provider_version);
        assert_eq!(capture.kind.as_str(), "controlled");
        assert_eq!(capture.compilation_mode.as_str(), "eager");
        assert_eq!(capture.coverage_requirement.as_str(), "required");
        assert!(capture.provider_actor_id.is_some());
        self
    }

    fn then_provider_issue_is_open_at_revision(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
        revision: u64,
    ) -> &mut Self {
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue identity");
        let mirror = self
            .store
            .object(&self.project, &issue)
            .expect("provider mirror")
            .expect("observed Issue");
        assert_eq!(mirror.project_revision.get(), revision);
        let payload = mirror.payload.expect("provider snapshot payload");
        let PayloadRead::Available(bytes) = self
            .store
            .read_payload(&self.project, &payload)
            .expect("provider snapshot")
        else {
            panic!("provider snapshot was unexpectedly erased");
        };
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot JSON");
        assert_eq!(snapshot["state"], "OPEN");
        self
    }

    fn then_github_binding_and_versions(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
    ) -> &mut Self {
        let kinds: Vec<_> = (1..=4)
            .map(|sequence| {
                self.store
                    .source_version_at(&self.project, sequence)
                    .expect("source lookup")
                    .expect("captured source")
            })
            .collect();
        assert_eq!(kinds[0].kind.as_str(), "issue");
        assert_eq!(kinds[1].kind.as_str(), "issue_comment");
        assert_eq!(kinds[2].kind.as_str(), "issue");
        assert_eq!(kinds[3].kind.as_str(), "issue_comment");
        assert_eq!(
            kinds[0].provider_entity_id,
            fixture.source.issue_provider_id
        );
        assert_eq!(kinds[2].supersedes, Some(kinds[0].id.clone()));
        assert_eq!(kinds[0].binding, kinds[1].binding);
        assert_eq!(kinds[1].binding, kinds[2].binding);
        assert_eq!(kinds[0].provider_actor_id.as_deref(), Some("user-1"));
        assert_eq!(kinds[2].provider_actor_id.as_deref(), Some("user-2"));
        assert_eq!(kinds[1].provider_actor_id.as_deref(), Some("user-3"));
        assert_eq!(
            kinds[0].policy_version.as_str(),
            merl_ingest::FIXTURE_CAPTURE_POLICY_VERSION
        );
        self
    }

    fn then_github_provider_facts(&mut self, fixture: &merl_corpus::fixture::Fixture) -> &mut Self {
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue identity");
        let head = self
            .store
            .provider_issue_head(&self.project, &issue)
            .expect("provider head")
            .expect("accepted observation");
        assert_eq!(head.input.state.as_str(), "open");
        assert_eq!(
            head.input.upstream_updated_at_millis,
            Some(1_767_265_200_000)
        );
        assert_eq!(head.input.closed_at_millis, None);
        assert_eq!(head.input.label_provider_ids, ["label-1"]);
        assert_eq!(head.input.assignee_provider_ids, ["user-2"]);
        self
    }

    fn when_provider_snapshot_bytes_are_erased(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
    ) -> &mut Self {
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue identity");
        let head = self
            .store
            .provider_issue_head(&self.project, &issue)
            .expect("provider head")
            .expect("accepted observation");
        self.store
            .erase_payload(&self.project, &head.input.snapshot_payload)
            .expect("erase protected snapshot");
        self
    }
}
