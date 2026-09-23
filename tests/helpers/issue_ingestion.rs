use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, ProjectId,
    SourceBindingId, SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_ingest::import_fixture;
use merl_store::{
    PayloadRead, SourceBinding, SourceCapture, Store, StoreError, StoredSourceVersion,
};
use sha2::{Digest, Sha256};

pub fn report_format_issue() -> merl_corpus::fixture::Fixture {
    serde_json::from_str(include_str!("../../corpus/development/DEV-C3.json"))
        .expect("development fixture")
}

pub fn two_page_github_issue() -> merl_corpus::fixture::Fixture {
    merl_corpus::github::fixture_from_graphql_pages(
        "DEV-FAKE",
        "2026-01-02T00:00:00Z",
        include_bytes!("../fixtures/github_two_page_edit.json"),
    )
    .expect("GitHub-shaped fixture")
}

fn refresh_source_digest(fixture: &mut merl_corpus::fixture::Fixture) {
    fixture.capture.source_sha256 =
        merl_corpus::fixture::source_digest(&fixture.provider_snapshot, &fixture.observations);
}

pub struct IssueHistory {
    store: Store,
    project: ProjectId,
    last_import_result: Option<Result<(), merl_ingest::ImportError>>,
    fixture: Option<merl_corpus::fixture::Fixture>,
    retry_capture: Option<SourceCapture<'static>>,
    retry_was_new: Option<bool>,
    author_retry_result: Option<Result<bool, StoreError>>,
    first_capture: Option<StoredSourceVersion>,
}

impl IssueHistory {
    pub fn new(id: &str) -> Self {
        let project = ProjectId::try_from(id).expect("project ID");
        let mut store = Store::open_in_memory().expect("open store");
        store.create_project(&project).expect("create project");
        Self {
            store,
            project,
            last_import_result: None,
            fixture: None,
            retry_capture: None,
            retry_was_new: None,
            author_retry_result: None,
            first_capture: None,
        }
    }

    pub fn given_a_report_format_issue(&mut self) -> &mut Self {
        self.fixture = Some(report_format_issue());
        self
    }

    pub fn given_a_two_page_github_issue(&mut self) -> &mut Self {
        self.fixture = Some(two_page_github_issue());
        self
    }

    pub fn given_a_github_issue_with_an_edit(&mut self) -> &mut Self {
        self.fixture = Some(two_page_github_issue());
        self
    }

    pub fn when_the_issue_is_imported(&mut self) -> &mut Self {
        import_fixture(
            &mut self.store,
            &self.project,
            self.fixture.as_ref().expect("GitHub issue fixture"),
        )
        .expect("import GitHub issue");
        self
    }

    pub fn then_current_issue_and_comment_updates_are_queryable(&mut self) -> &mut Self {
        for (sequence, expected) in [
            (2, 1_767_261_600_000),
            (3, 1_767_265_200_000),
            (4, 1_767_268_800_000),
        ] {
            let version = self
                .store
                .source_version_at(&self.project, sequence)
                .expect("source lookup")
                .expect("captured source");
            assert_eq!(version.upstream_updated_at_millis, Some(expected));
        }
        self
    }

    pub fn then_the_earlier_issue_version_has_no_future_update_time(&mut self) -> &mut Self {
        let version = self
            .store
            .source_version_at(&self.project, 1)
            .expect("source lookup")
            .expect("first Issue version");
        assert_eq!(version.upstream_updated_at_millis, None);
        self
    }

    pub fn when_importing(&mut self, fixture: &merl_corpus::fixture::Fixture) -> &mut Self {
        import_fixture(&mut self.store, &self.project, fixture).expect("import fixture");
        self
    }

    pub fn given_an_edit_by_another_person(&mut self) -> &mut Self {
        let mut fixture = report_format_issue();
        let edit = fixture.observations[1].edit.as_mut().expect("staged edit");
        edit.editor = Some(merl_corpus::fixture::ActorRef {
            provider_id: Some("controlled:editor".to_owned()),
            login: "editor".to_owned(),
        });
        edit.diff = Some("+ unless the consumer moves to Parquet".to_owned());
        refresh_source_digest(&mut fixture);
        self.fixture = Some(fixture);
        self
    }

    pub fn then_keeps_the_editors_identity_and_diff(&mut self) -> &mut Self {
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

    pub fn given_the_provider_closed_the_issue(&mut self) -> &mut Self {
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

    pub fn when_an_older_open_snapshot_arrives_later(&mut self) -> &mut Self {
        let mut stale = report_format_issue();
        stale.provider_snapshot.updated_at = Some("2026-09-19T00:00:00Z".to_owned());
        "2026-09-20T03:00:00Z".clone_into(&mut stale.capture.captured_at);
        refresh_source_digest(&mut stale);
        self.last_import_result =
            Some(import_fixture(&mut self.store, &self.project, &stale).map(|_| ()));
        self
    }

    pub fn when_an_unversioned_open_snapshot_arrives_even_later(&mut self) -> &mut Self {
        let mut stale = report_format_issue();
        "2026-09-20T04:00:00Z".clone_into(&mut stale.capture.captured_at);
        self.last_import_result =
            Some(import_fixture(&mut self.store, &self.project, &stale).map(|_| ()));
        self
    }

    pub fn then_keeps_the_issue_closed(&mut self) -> &mut Self {
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

    pub fn when_recapturing_later(&mut self) -> &mut Self {
        let mut later = self.fixture.as_ref().expect("Issue fixture").clone();
        "2026-09-20T00:00:00Z".clone_into(&mut later.capture.captured_at);
        self.when_importing(&later)
    }

    pub fn given_a_captured_source(&mut self) -> &mut Self {
        self.given_a_source_with_author(None)
    }

    pub fn given_a_source_authored_by_alice_and_edited_by_bob(&mut self) -> &mut Self {
        self.given_a_source_with_author(Some("alice"))
    }

    fn given_a_source_with_author(&mut self, author: Option<&'static str>) -> &mut Self {
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
            context_scope_id: "issue-1",
            version: SourceVersionId::try_from("version").unwrap(),
            provider_version_id: "issue-1:initial",
            kind: SourceKind::try_from("issue").unwrap(),
            supersedes: None,
            ambiguous_order_with_previous: false,
            created_at_millis: 1,
            occurred_at_millis: 1,
            upstream_updated_at_millis: Some(1),
            observed_at_millis: 2,
            actor: Some(ActorId::try_from("bob").unwrap()),
            provider_actor_id: Some("bob"),
            source_author: author.map(|id| ActorId::try_from(id).unwrap()),
            provider_source_author_id: author,
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
        self.first_capture = self
            .store
            .source_version(&self.project, &first.version)
            .expect("captured source metadata");
        let retry = SourceCapture {
            observed_at_millis: 3,
            upstream_updated_at_millis: Some(2),
            compilation_mode: CompilationMode::CaptureOnly,
            coverage_requirement: CoverageRequirement::Optional,
            policy_version: CapturePolicyVersion::try_from("new_policy").unwrap(),
            ..first
        };
        self.retry_capture = Some(retry);
        self
    }

    pub fn when_the_source_is_retried_with_author(
        &mut self,
        author: Option<&'static str>,
    ) -> &mut Self {
        let retry = self.retry_capture.as_mut().expect("first capture");
        retry.provider_source_author_id = author;
        retry.source_author = author.map(|id| ActorId::try_from(id).unwrap());
        self.author_retry_result = Some(self.store.capture_source_version(&self.project, retry));
        self
    }

    pub fn then_the_retry_reports_an_author_conflict(&mut self) -> &mut Self {
        assert!(
            matches!(
                self.author_retry_result,
                Some(Err(StoreError::SourceConflict))
            ),
            "a different entity author must conflict: {:?}",
            self.author_retry_result
        );
        self
    }

    pub fn then_the_retry_is_a_no_op(&mut self) -> &mut Self {
        assert!(matches!(self.author_retry_result, Some(Ok(false))));
        self
    }

    pub fn then_the_source_provenance_is_unchanged(&mut self) -> &mut Self {
        let first = self.first_capture.as_ref().expect("first capture");
        let current = self
            .store
            .source_version(&self.project, &first.id)
            .expect("source metadata")
            .expect("captured source");
        assert_eq!(&current, first);
        assert_eq!(
            self.store.source_observation_head(&self.project).unwrap(),
            1
        );
        self
    }

    pub fn when_the_source_is_recaptured_under_another_policy(&mut self) -> &mut Self {
        self.retry_was_new = Some(
            self.store
                .capture_source_version(
                    &self.project,
                    self.retry_capture
                        .as_ref()
                        .expect("source was captured first"),
                )
                .unwrap(),
        );
        self
    }

    pub fn then_first_capture_metadata_still_applies(&mut self) -> &mut Self {
        assert_eq!(self.retry_was_new, Some(false));
        let captured = self
            .store
            .source_version(
                &self.project,
                &SourceVersionId::try_from("version").unwrap(),
            )
            .unwrap()
            .expect("first version");
        assert_eq!(captured.observed_at_millis, 2);
        assert_eq!(captured.upstream_updated_at_millis, Some(1));
        assert_eq!(captured.compilation_mode, CompilationMode::Eager);
        assert_eq!(captured.coverage_requirement, CoverageRequirement::Required);
        assert_eq!(captured.policy_version.as_str(), "initial");
        assert_eq!(
            self.store.source_observation_head(&self.project).unwrap(),
            1
        );
        self
    }

    pub fn then_has_four_source_versions(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .source_observation_head(&self.project)
                .expect("head"),
            4
        );
        self
    }

    pub fn then_source_versions_keep_their_order(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
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

    pub fn then_each_edit_supersedes_the_prior_version(&mut self) -> &mut Self {
        self.then_version_supersedes("controlled:DEV-C3:issue:v2", "controlled:DEV-C3:issue:v1")
            .then_version_supersedes("controlled:DEV-C3:issue:v3", "controlled:DEV-C3:issue:v2")
    }

    pub fn then_edit_provenance_remains_cold(&mut self) -> &mut Self {
        self.then_version_has_cold_metadata("controlled:DEV-C3:issue:v2")
    }

    pub fn then_original_issue_body_remains_available(&mut self) -> &mut Self {
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

    pub fn then_issue_is_open_without_duplicate_project_changes(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
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

    pub fn then_provider_fact_has_a_policy_decision(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
        let issue = merl_ingest::fixture_issue_id(fixture).expect("Issue identity");
        let evaluation = self
            .store
            .object_policy_evaluation(&self.project, &issue)
            .expect("policy provenance")
            .expect("policy decision");
        let record = self
            .store
            .policy_evaluation(&self.project, &evaluation)
            .expect("policy decision")
            .expect("recorded decision");
        assert_eq!(
            record
                .committed_revision
                .map(merl_core::ProjectRevision::get),
            Some(1)
        );
        assert_eq!(record.inputs.len(), 1);
        assert_eq!(record.inputs[0].input.kind(), "provider_observation");
        self
    }

    pub fn then_github_identity_and_edits_survive_import(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
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

    pub fn then_provider_facts_remain_available(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
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

    pub fn when_provider_snapshot_bytes_are_erased(&mut self) -> &mut Self {
        let fixture = self.fixture.as_ref().expect("Issue fixture");
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
