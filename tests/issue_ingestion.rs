use merl_core::{
    CapturePolicyVersion, CompilationMode, CoverageRequirement, ProjectId, SourceBindingId,
    SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_ingest::import_fixture;
use merl_store::{PayloadRead, SourceBinding, SourceCapture, Store};
use sha2::{Digest, Sha256};

#[test]
fn issue_edits_are_captured_once_and_keep_their_source_lineage() {
    let fixture = report_format_issue();
    let mut project = IssueHistory::new("P1");

    project
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_source_versions_keep_their_order(&fixture)
        .then_issue_is_open_without_duplicate_project_changes(&fixture)
        .then_each_edit_supersedes_the_prior_version()
        .then_edit_provenance_remains_cold()
        .then_original_issue_body_remains_available()
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes(&fixture)
        .when_recapturing_later(&fixture)
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes(&fixture);
}

#[test]
fn an_edit_keeps_its_editor_and_protected_diff() {
    IssueHistory::new("P2")
        .when_importing_an_edit_by_another_person()
        .then_keeps_the_editors_identity_and_diff();
}

#[test]
fn an_older_provider_update_cannot_reopen_a_newer_closed_issue_even_if_polled_later() {
    IssueHistory::new("P3")
        .given_the_provider_closed_the_issue()
        .when_an_older_open_snapshot_arrives_later()
        .then_keeps_the_issue_closed()
        .when_an_unversioned_open_snapshot_arrives_even_later()
        .then_keeps_the_issue_closed();
}

#[test]
fn a_github_issue_import_keeps_provider_identity_and_version_lineage() {
    let fixture = two_page_github_issue();
    let mut project = IssueHistory::new("P4");
    project
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_github_identity_and_edits_survive_import(&fixture)
        .then_provider_facts_remain_available(&fixture)
        .when_provider_snapshot_bytes_are_erased(&fixture)
        .then_provider_facts_remain_available(&fixture);
}

#[test]
fn a_retry_keeps_the_first_capture_time_and_policy() {
    IssueHistory::new("P5")
        .when_a_source_is_recaptured_under_another_policy()
        .then_first_capture_metadata_still_applies();
}

fn report_format_issue() -> merl_corpus::fixture::Fixture {
    serde_json::from_str(include_str!("../corpus/development/DEV-C3.json"))
        .expect("development fixture")
}

fn two_page_github_issue() -> merl_corpus::fixture::Fixture {
    merl_corpus::github::fixture_from_graphql_pages(
        "DEV-FAKE",
        "2026-01-02T00:00:00Z",
        include_bytes!("fixtures/github_two_page_edit.json"),
    )
    .expect("GitHub-shaped fixture")
}

fn refresh_source_digest(fixture: &mut merl_corpus::fixture::Fixture) {
    fixture.capture.source_sha256 =
        merl_corpus::fixture::source_digest(&fixture.provider_snapshot, &fixture.observations);
}

struct IssueHistory {
    store: Store,
    project: ProjectId,
    last_import_result: Option<Result<(), merl_ingest::ImportError>>,
}

impl IssueHistory {
    fn new(id: &str) -> Self {
        let project = ProjectId::try_from(id).expect("project ID");
        let mut store = Store::open_in_memory().expect("open store");
        store.create_project(&project).expect("create project");
        Self {
            store,
            project,
            last_import_result: None,
        }
    }

    fn when_importing(&mut self, fixture: &merl_corpus::fixture::Fixture) -> &mut Self {
        import_fixture(&mut self.store, &self.project, fixture).expect("import fixture");
        self
    }

    fn when_importing_an_edit_by_another_person(&mut self) -> &mut Self {
        let mut fixture = report_format_issue();
        let edit = fixture.observations[1].edit.as_mut().expect("staged edit");
        edit.editor = Some(merl_corpus::fixture::ActorRef {
            provider_id: Some("controlled:editor".to_owned()),
            login: "editor".to_owned(),
        });
        edit.diff = Some("+ unless the consumer moves to Parquet".to_owned());
        refresh_source_digest(&mut fixture);
        self.when_importing(&fixture)
    }

    fn then_keeps_the_editors_identity_and_diff(&mut self) -> &mut Self {
        let version =
            merl_ingest::fixture_version_id("controlled:DEV-C3:issue:v2").expect("edit version ID");
        let captured = self
            .store
            .source_version(&self.project, &version)
            .expect("source metadata")
            .expect("edit captured");
        assert_eq!(
            captured.provider_actor_id.as_deref(),
            Some("controlled:editor")
        );
        let diff = captured.edit_diff_payload.expect("protected edit diff");
        assert_eq!(
            self.store
                .read_payload(&self.project, &diff)
                .expect("edit evidence"),
            PayloadRead::Available(b"+ unless the consumer moves to Parquet".to_vec())
        );
        self
    }

    fn given_the_provider_closed_the_issue(&mut self) -> &mut Self {
        let mut closed = report_format_issue();
        "CLOSED".clone_into(&mut closed.provider_snapshot.state);
        closed.provider_snapshot.updated_at = Some("2026-09-20T00:40:00Z".to_owned());
        closed.provider_snapshot.closed_at = Some("2026-09-20T00:30:00Z".to_owned());
        "2026-09-20T01:00:00Z".clone_into(&mut closed.capture.captured_at);
        refresh_source_digest(&mut closed);
        self.when_importing(&closed);
        "2026-09-20T02:00:00Z".clone_into(&mut closed.capture.captured_at);
        self.when_importing(&closed)
    }

    fn when_an_older_open_snapshot_arrives_later(&mut self) -> &mut Self {
        let mut stale = report_format_issue();
        stale.provider_snapshot.updated_at = Some("2026-09-19T00:00:00Z".to_owned());
        "2026-09-20T03:00:00Z".clone_into(&mut stale.capture.captured_at);
        refresh_source_digest(&mut stale);
        self.last_import_result =
            Some(import_fixture(&mut self.store, &self.project, &stale).map(|_| ()));
        self
    }

    fn when_an_unversioned_open_snapshot_arrives_even_later(&mut self) -> &mut Self {
        let mut stale = report_format_issue();
        "2026-09-20T04:00:00Z".clone_into(&mut stale.capture.captured_at);
        self.last_import_result =
            Some(import_fixture(&mut self.store, &self.project, &stale).map(|_| ()));
        self
    }

    fn then_keeps_the_issue_closed(&mut self) -> &mut Self {
        assert!(matches!(
            self.last_import_result.as_ref(),
            Some(Err(merl_ingest::ImportError::Store(
                merl_store::StoreError::StaleProviderObservation
            )))
        ));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        let issue = merl_ingest::fixture_issue_id(&report_format_issue()).expect("Issue ID");
        let head = self
            .store
            .provider_issue_head(&self.project, &issue)
            .expect("provider head")
            .expect("accepted provider observation");
        assert_eq!(head.input.state.as_str(), "closed");
        assert_eq!(head.revision.get(), 1);
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

    fn then_has_four_source_versions(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .source_observation_head(&self.project)
                .expect("head"),
            4
        );
        self
    }

    fn then_source_versions_keep_their_order(
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

    fn then_each_edit_supersedes_the_prior_version(&mut self) -> &mut Self {
        self.then_version_supersedes("controlled:DEV-C3:issue:v2", "controlled:DEV-C3:issue:v1")
            .then_version_supersedes("controlled:DEV-C3:issue:v3", "controlled:DEV-C3:issue:v2")
    }

    fn then_edit_provenance_remains_cold(&mut self) -> &mut Self {
        self.then_version_has_cold_metadata("controlled:DEV-C3:issue:v2")
    }

    fn then_original_issue_body_remains_available(&mut self) -> &mut Self {
        self.then_version_body_is(
            "controlled:DEV-C3:issue:v1",
            b"For report A, export CSV with a sample_id column.",
        )
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

    fn then_issue_is_open_without_duplicate_project_changes(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
    ) -> &mut Self {
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue identity");
        let mirror = self
            .store
            .object(&self.project, &issue)
            .expect("provider mirror")
            .expect("observed Issue");
        assert_eq!(mirror.project_revision.get(), 1);
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

    fn then_github_identity_and_edits_survive_import(
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

    fn then_provider_facts_remain_available(
        &mut self,
        fixture: &merl_corpus::fixture::Fixture,
    ) -> &mut Self {
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
        assert_eq!(
            head.input.label_provider_ids,
            Some(vec!["label-1".to_owned()])
        );
        assert_eq!(
            head.input.assignee_provider_ids,
            Some(vec!["user-2".to_owned()])
        );
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
