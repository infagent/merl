use merl_core::ProjectId;
use merl_ingest::import_fixture;
use merl_store::{PayloadRead, Store};

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
fn an_older_provider_snapshot_cannot_reopen_a_newer_closed_issue() {
    let earlier: merl_corpus::fixture::Fixture =
        serde_json::from_str(include_str!("../corpus/development/DEV-C3.json"))
            .expect("development fixture");
    let mut later = earlier.clone();
    later.provider_snapshot.state = "CLOSED".to_owned();
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
    stale.capture.captured_at = "2026-09-20T01:30:00Z".to_owned();
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
}
