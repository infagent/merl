use merl_compiler::{
    CompilerLimits, FakeCompiler, ProcessCompiler, RunMode, RunRequest, build_context,
    build_context_at_basis, execute_compilation, prepare_compilation, record_compilation_result,
};
use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, EventId, ObjectId, ObjectKind, ProjectId, SourceBindingId, SourceId,
    SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{SemanticCoverage, SourceBinding, SourceCapture, Store};
use sha2::{Digest, Sha256};

fn limits() -> CompilerLimits {
    CompilerLimits {
        input_bytes: 4096,
        output_bytes: 4096,
        output_tokens: 512,
        assertions: 8,
        context_requests: 2,
        expansion_rounds: 1,
        payload_bytes: 2048,
        source_window: 4,
        objects: 8,
    }
}

fn run_compiler(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl merl_compiler::CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<(), merl_compiler::CompileError> {
    let completed_at = request.now_millis;
    if let Some(prepared) = prepare_compilation(store, project, source, adapter, request)? {
        let raw = execute_compilation(&prepared, adapter);
        record_compilation_result(store, project, &prepared, raw, completed_at)?;
    }
    Ok(())
}

pub struct HistoricalIssue {
    store: Store,
    project: ProjectId,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
}

impl HistoricalIssue {
    pub fn given_a_four_comment_issue() -> Self {
        let project = ProjectId::try_from("HistoricalIssue").expect("project ID");
        let mut store = Store::open_in_memory().expect("store");
        store.create_project(&project).expect("project");
        let fixture: merl_corpus::fixture::Fixture =
            serde_json::from_str(include_str!("../../corpus/development/DEV-C1.json"))
                .expect("development fixture");
        merl_ingest::import_fixture_for_causal_replay(&mut store, &project, &fixture)
            .expect("historical source import");
        Self {
            store,
            project,
            before: None,
            after: None,
        }
    }

    pub fn when_the_first_decision_is_accepted_before_the_next_comment(&mut self) -> &mut Self {
        let second =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O2:v1").expect("second source");
        let before = build_context_at_basis(
            &self.store,
            &self.project,
            &second,
            merl_core::ProjectRevision::initial(),
            limits(),
        )
        .expect("second context");
        self.before = Some(serde_json::from_slice(&before.rendered).expect("before JSON"));
        let payload = merl_core::PayloadId::try_from("decision-body").expect("payload");
        self.store
            .put_payload(&self.project, &payload, b"Gain fixed for run A")
            .expect("decision payload");
        self.store
            .commit(&DomainEventBatch {
                id: BatchId::try_from("accepted-after-O2").expect("batch"),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("decision-event").expect("event"),
                    object: ObjectId::try_from("gain-decision").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: Some(payload),
                }],
            })
            .expect("accept decision");
        let third =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O3:v1").expect("third source");
        let after = build_context_at_basis(
            &self.store,
            &self.project,
            &third,
            merl_core::ProjectRevision::from(1),
            limits(),
        )
        .expect("third context");
        self.after = Some(serde_json::from_slice(&after.rendered).expect("after JSON"));
        self
    }

    pub fn then_each_comment_sees_only_its_causal_history(&mut self) -> &mut Self {
        let before = self.before.as_ref().expect("second context");
        let after = self.after.as_ref().expect("third context");
        assert_eq!(before["source_observation_cutoff"], 2);
        assert_eq!(before["interpretation_basis_revision"], 0);
        assert_eq!(before["sources"].as_array().expect("sources").len(), 2);
        assert!(before["objects"].as_array().expect("objects").is_empty());
        assert_eq!(after["source_observation_cutoff"], 3);
        assert_eq!(after["interpretation_basis_revision"], 1);
        assert_eq!(after["sources"].as_array().expect("sources").len(), 3);
        assert_eq!(after["objects"].as_array().expect("objects").len(), 1);
        self
    }
}

pub struct CompilationScenario {
    store: Store,
    project: ProjectId,
    note: SourceVersionId,
    context: Option<serde_json::Value>,
    coverage: Option<SemanticCoverage>,
    compile_failed: bool,
}

impl CompilationScenario {
    fn new() -> Self {
        let project = ProjectId::try_from("CompilerTest").expect("project ID");
        let mut store = Store::open_in_memory().expect("store");
        store.create_project(&project).expect("project");
        Self {
            store,
            project,
            note: SourceVersionId::try_from("note-v1").expect("version"),
            context: None,
            coverage: None,
            compile_failed: false,
        }
    }

    pub fn given_a_note_followed_by_a_later_decision() -> Self {
        let mut scenario = Self::new();
        scenario.capture(
            "note-v1",
            "Please revisit the earlier decision.",
            CoverageRequirement::Optional,
        );
        let payload = merl_core::PayloadId::try_from("later-payload").expect("payload");
        scenario
            .store
            .put_payload(&scenario.project, &payload, b"Future decision: vary gain")
            .expect("payload");
        scenario
            .store
            .commit(&DomainEventBatch {
                id: BatchId::try_from("later-batch").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("later-event").expect("event"),
                    object: ObjectId::try_from("future-decision").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: Some(payload),
                }],
            })
            .expect("accept later decision");
        scenario
    }

    pub fn given_required_and_optional_notes() -> Self {
        let mut scenario = Self::new();
        scenario.capture(
            "note-v1",
            "A blocker might be here.",
            CoverageRequirement::Required,
        );
        scenario.capture(
            "note-v2",
            "Optional background note.",
            CoverageRequirement::Optional,
        );
        scenario
    }

    pub fn given_a_note_for_an_external_compiler() -> Self {
        let mut scenario = Self::new();
        scenario.capture(
            "note-v1",
            "A blocker might be here.",
            CoverageRequirement::Required,
        );
        scenario
    }

    pub fn given_interleaved_issue_comments() -> Self {
        let mut scenario = Self::new();
        scenario.capture_in_scope(
            "note-v1",
            "First Issue A comment",
            "issue-a",
            CoverageRequirement::Required,
        );
        scenario.capture_in_scope(
            "other-v1",
            "Unrelated Issue B comment",
            "issue-b",
            CoverageRequirement::Required,
        );
        scenario.capture_in_scope(
            "latest-v1",
            "Second Issue A comment",
            "issue-a",
            CoverageRequirement::Required,
        );
        scenario.note = SourceVersionId::try_from("latest-v1").expect("latest version");
        scenario
    }

    pub fn when_the_latest_comment_context_is_built(&mut self) -> &mut Self {
        let context =
            build_context(&self.store, &self.project, &self.note, limits()).expect("issue context");
        self.context = Some(serde_json::from_slice(&context.rendered).expect("rendered input"));
        self
    }

    pub fn then_other_issue_comments_are_absent(&mut self) -> &mut Self {
        let sources = self.context.as_ref().expect("context")["sources"]
            .as_array()
            .expect("sources");
        let ids: Vec<_> = sources
            .iter()
            .map(|source| source["id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec!["note-v1", "latest-v1"]);
        self
    }

    pub fn when_the_configured_compiler_runs(&mut self) -> &mut Self {
        let response = r#"{"schema":"merl.compiler-response/v1","assertions":[{"source":"note-v1","span_start":2,"span_end":9,"subject":"T1","predicate":"blocked","value":"true","act":"report","epistemic_basis":"observed","polarity":"positive","confidence_millis":900,"asserted_by":"alice","attributed_to":null,"attribution_verified":false}]}"#;
        let adapter = ProcessCompiler {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!("read -r request; printf '%s' '{response}'"),
            ],
            version: "v1".into(),
            model: "test-model".into(),
            prompt_digest: Sha256::digest(b"test-prompt").into(),
        };
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &adapter,
            RunRequest {
                id: "external-run",
                limits: limits(),
                mode: RunMode::Live,
                interpretation_basis_revision: None,
                now_millis: 30,
            },
        )
        .expect("external compile");
        self
    }

    pub fn when_the_authority_prepares_the_compiler_run(&mut self) -> &mut Self {
        let prepared = prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "pending-run",
                limits: limits(),
                mode: RunMode::Live,
                interpretation_basis_revision: None,
                now_millis: 30,
            },
        )
        .expect("prepare run")
        .expect("new run");
        drop(prepared);
        self
    }

    pub fn then_pending_work_is_visible_without_a_running_worker(&mut self) -> &mut Self {
        let pending = self
            .store
            .pending_compilation_runs(&self.project)
            .expect("pending work");
        assert_eq!(pending, vec![("pending-run".into(), self.note.clone())]);
        let status = self
            .store
            .compilation_run_status(&self.project, "pending-run")
            .expect("run lookup")
            .expect("pending run");
        assert!(!status.completed);
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage")
                .required_gaps,
            1
        );
        self
    }

    pub fn then_its_typed_assertion_and_model_provenance_are_retained(&mut self) -> &mut Self {
        let status = self
            .store
            .compilation_run_status(&self.project, "external-run")
            .expect("run lookup")
            .expect("run");
        assert!(status.succeeded);
        assert_eq!(status.model_id, "test-model");
        assert_eq!(status.compiler_version, "v1");
        assert_eq!(
            self.store
                .observed_assertion_count(&self.project, "external-run")
                .expect("assertion count"),
            1
        );
        let assertions = self
            .store
            .observed_assertions(&self.project, "external-run")
            .expect("assertions");
        assert_eq!(assertions[0].predicate, "blocked");
        assert_eq!(assertions[0].span_start, 2);
        assert_eq!(assertions[0].span_end, 9);
        self
    }

    fn capture(&mut self, version: &str, body: &'static str, requirement: CoverageRequirement) {
        self.capture_in_scope(version, body, "note-thread", requirement);
    }

    fn capture_in_scope(
        &mut self,
        version: &str,
        body: &'static str,
        scope: &str,
        requirement: CoverageRequirement,
    ) {
        let namespace = "test-namespace";
        let binding = SourceBinding {
            id: SourceBindingId::try_from("test-binding").expect("binding"),
            provider: SourceProvider::try_from("controlled").expect("provider"),
            provider_namespace_id: namespace.into(),
            namespace_digest: Sha256::digest(namespace.as_bytes()).into(),
        };
        self.store
            .capture_source_version(
                &self.project,
                &SourceCapture {
                    binding,
                    source: SourceId::try_from(version).expect("source"),
                    provider_entity_id: version,
                    context_scope_id: scope,
                    version: SourceVersionId::try_from(version).expect("version"),
                    provider_version_id: version,
                    kind: SourceKind::try_from("note").expect("kind"),
                    supersedes: None,
                    ambiguous_order_with_previous: false,
                    created_at_millis: 10,
                    occurred_at_millis: 10,
                    upstream_updated_at_millis: None,
                    observed_at_millis: 10,
                    actor: None,
                    provider_actor_id: None,
                    body: Some(body.as_bytes()),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::OnDemand,
                    coverage_requirement: requirement,
                    policy_version: CapturePolicyVersion::try_from("test-v1").expect("policy"),
                },
            )
            .expect("capture");
    }

    pub fn when_the_note_is_compiled_on_demand(&mut self) -> &mut Self {
        let context =
            build_context(&self.store, &self.project, &self.note, limits()).expect("context");
        self.context = Some(serde_json::from_slice(&context.rendered).expect("rendered input"));
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "run-one",
                limits: limits(),
                mode: RunMode::Live,
                interpretation_basis_revision: None,
                now_millis: 30,
            },
        )
        .expect("compile note");
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "run-one",
                limits: limits(),
                mode: RunMode::Live,
                interpretation_basis_revision: None,
                now_millis: 31,
            },
        )
        .expect("reuse run");
        self
    }

    pub fn then_the_compiler_sees_only_the_earlier_world(&mut self) -> &mut Self {
        let context = self.context.as_ref().expect("compiler input");
        assert_eq!(context["interpretation_basis_revision"], 0);
        assert_eq!(context["source_observation_cutoff"], 1);
        assert!(context["objects"].as_array().expect("objects").is_empty());
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        self
    }

    pub fn then_the_run_is_recorded_once_for_all_readers(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .compilation_run_count(&self.project, &self.note)
                .expect("run count"),
            1
        );
        self
    }

    pub fn when_the_required_note_fails_its_output_budget(&mut self) -> &mut Self {
        let mut small = limits();
        small.output_bytes = 1;
        self.compile_failed = run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "budget-run",
                limits: small,
                mode: RunMode::Live,
                interpretation_basis_revision: None,
                now_millis: 30,
            },
        )
        .is_err();
        self.coverage = Some(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage"),
        );
        self
    }

    pub fn when_the_required_note_is_compiled_for_evaluation(&mut self) -> &mut Self {
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "eval-run",
                limits: limits(),
                mode: RunMode::Eval,
                interpretation_basis_revision: None,
                now_millis: 30,
            },
        )
        .expect("evaluation run");
        self.coverage = Some(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage"),
        );
        self
    }

    pub fn then_live_coverage_still_has_one_required_gap(&mut self) -> &mut Self {
        let coverage = self.coverage.expect("coverage");
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.required_failed, 0);
        assert_eq!(coverage.optional_cold, 1);
        self
    }

    pub fn then_coverage_reports_the_required_failure_only(&mut self) -> &mut Self {
        assert!(self.compile_failed);
        let coverage = self.coverage.expect("coverage");
        assert_eq!(coverage.observation_head, 2);
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.required_failed, 1);
        assert_eq!(coverage.optional_cold, 1);
        assert_eq!(
            self.store
                .compilation_run_count(&self.project, &self.note)
                .expect("run count"),
            1
        );
        let status = self
            .store
            .compilation_run_status(&self.project, "budget-run")
            .expect("run lookup")
            .expect("failed run");
        assert_eq!(status.failure_code.as_deref(), Some("output_budget"));
        self
    }
}
