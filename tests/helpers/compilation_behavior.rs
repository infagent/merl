use merl_compiler::{
    CompilerLimits, FakeCompiler, ProcessCompiler, RunMode, RunRequest, SequentialReplay,
    build_context, execute_compilation, prepare_authorized_compilation, prepare_compilation,
    prepare_replay_compilation, rebuild_recorded_context, record_compilation_result,
};
use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, EventId, ObjectId, ObjectKind, ProjectId, SourceBindingId, SourceId,
    SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{
    CompilationAuthorizationConfig, CompilationIntent, SemanticCoverage, SourceBinding,
    SourceCapture, Store,
};
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

pub struct CompilationAuthorizationScenario {
    store: Store,
    project: ProjectId,
    source: SourceVersionId,
    other_source: SourceVersionId,
    exact_prepared: bool,
    rejected_attempts: usize,
}

impl CompilationAuthorizationScenario {
    pub fn given_an_on_demand_source_with_one_accepted_request() -> Self {
        let project = ProjectId::try_from("CompileAuthorization").expect("project ID");
        let mut store = Store::open_in_memory().expect("store");
        store.create_project(&project).expect("project");
        let mut scenario = Self {
            store,
            project,
            source: SourceVersionId::try_from("optional-v1").expect("source"),
            other_source: SourceVersionId::try_from("other-v1").expect("other source"),
            exact_prepared: false,
            rejected_attempts: 0,
        };
        scenario.capture_on_demand("optional-v1");
        scenario.capture_on_demand("other-v1");

        let prompt_digest = [7; 32];
        let intent = scenario
            .store
            .prepare_compilation_authorization(
                &scenario.project,
                &scenario.source,
                "authorized-run",
                b"Needed for the assigned task",
                CompilationAuthorizationConfig {
                    compiler_id: "process",
                    compiler_version: "v1",
                    model_id: "model-a",
                    prompt_digest,
                },
            )
            .expect("authorization intent");
        scenario
            .store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("accept-compile-request").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("pm").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("accept-compile-request-event").expect("event"),
                    object: intent.object,
                    kind: ObjectKind::try_from("source_compilation_request").expect("kind"),
                    payload: Some(intent.reason),
                    issue_scope: Some("issue-1".into()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .expect("accepted authorization");
        scenario
    }

    pub fn when_low_level_preparation_is_attempted(&mut self) -> &mut Self {
        let exact = process_compiler("v1", "model-a", [7; 32]);
        let attempts = [
            (
                &self.source,
                "missing-run",
                process_compiler("v1", "model-a", [7; 32]),
            ),
            (
                &self.other_source,
                "authorized-run",
                process_compiler("v1", "model-a", [7; 32]),
            ),
            (
                &self.source,
                "authorized-run",
                process_compiler("v2", "model-a", [7; 32]),
            ),
            (
                &self.source,
                "authorized-run",
                process_compiler("v1", "model-b", [7; 32]),
            ),
            (
                &self.source,
                "authorized-run",
                process_compiler("v1", "model-a", [8; 32]),
            ),
        ];
        for (source, run, adapter) in attempts {
            let result = prepare_authorized_compilation(
                &mut self.store,
                &self.project,
                source,
                &adapter,
                RunRequest {
                    id: run,
                    limits: limits(),
                    mode: RunMode::Live,
                    now_millis: 30,
                },
            );
            if matches!(
                result,
                Err(merl_compiler::CompileError::UnauthorizedCompilation)
            ) {
                self.rejected_attempts += 1;
            }
        }
        self.exact_prepared = prepare_authorized_compilation(
            &mut self.store,
            &self.project,
            &self.source,
            &exact,
            RunRequest {
                id: "authorized-run",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 31,
            },
        )
        .expect("authorized preparation")
        .is_some();
        self
    }

    pub fn then_only_the_exact_authorized_run_is_prepared(&mut self) -> &mut Self {
        assert_eq!(self.rejected_attempts, 5);
        assert!(self.exact_prepared);
        assert_eq!(
            self.store
                .compilation_run_ids(&self.project)
                .expect("compiler runs"),
            vec!["authorized-run"]
        );
        self
    }

    fn capture_on_demand(&mut self, version: &'static str) {
        let binding = SourceBinding {
            id: SourceBindingId::try_from("authorization-binding").expect("binding"),
            provider: SourceProvider::try_from("controlled").expect("provider"),
            provider_namespace_id: "authorization".into(),
            namespace_digest: Sha256::digest(b"authorization").into(),
        };
        self.store
            .capture_source_version(
                &self.project,
                &SourceCapture {
                    binding,
                    source: SourceId::try_from(version).expect("source"),
                    provider_entity_id: version,
                    context_scope_id: "issue-1",
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
                    source_author: None,
                    provider_source_author_id: None,
                    body: Some(b"Optional note"),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::OnDemand,
                    coverage_requirement: CoverageRequirement::Optional,
                    policy_version: CapturePolicyVersion::try_from("test-v1").expect("policy"),
                },
            )
            .expect("capture");
    }
}

fn process_compiler(version: &str, model: &str, prompt_digest: [u8; 32]) -> ProcessCompiler {
    ProcessCompiler {
        program: "unused".into(),
        args: Vec::new(),
        version: version.into(),
        model: model.into(),
        prompt_digest,
    }
}

pub struct HistoricalIssue {
    store: Store,
    project: ProjectId,
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
    future_basis_rejected: bool,
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
            future_basis_rejected: false,
        }
    }

    pub fn when_the_first_decision_is_accepted_before_the_next_comment(&mut self) -> &mut Self {
        let second =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O2:v1").expect("second source");
        let first =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O1:v1").expect("first source");
        let mut replay = SequentialReplay::new(self.project.clone());
        replay
            .next_context(&mut self.store, &first, limits())
            .expect("first context");
        let before = replay
            .next_context(&mut self.store, &second, limits())
            .expect("second context");
        self.before = Some(serde_json::from_slice(&before.rendered).expect("before JSON"));
        let payload = merl_core::PayloadId::try_from("decision-body").expect("payload");
        self.store
            .put_payload(&self.project, &payload, b"Gain fixed for run A")
            .expect("decision payload");
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("accepted-after-O2").expect("batch"),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("decision-event").expect("event"),
                    object: ObjectId::try_from("gain-decision").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: Some(payload),
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .expect("accept decision");
        let third =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O3:v1").expect("third source");
        let after = replay
            .next_context(&mut self.store, &third, limits())
            .expect("third context");
        self.after = Some(serde_json::from_slice(&after.rendered).expect("after JSON"));
        self
    }

    pub fn when_a_future_decision_is_accepted_before_the_first_comment_is_bound(
        &mut self,
    ) -> &mut Self {
        let payload = merl_core::PayloadId::try_from("future-payload").expect("payload");
        self.store
            .put_payload(&self.project, &payload, b"Future decision")
            .expect("payload");
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("future-batch").expect("batch"),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("future-event").expect("event"),
                    object: ObjectId::try_from("future-decision").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: Some(payload),
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .expect("accept future decision");
        let first =
            merl_ingest::fixture_version_id("controlled:DEV-C1:O1:v1").expect("first source");
        self.future_basis_rejected = SequentialReplay::new(self.project.clone())
            .next_context(&mut self.store, &first, limits())
            .is_err();
        self
    }

    pub fn then_the_first_comment_rejects_the_future_basis(&mut self) -> &mut Self {
        assert!(self.future_basis_rejected);
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

enum ReplayCheck {
    NotRun,
    Matching,
    MissingEvidence,
}

pub struct CompilationScenario {
    store: Store,
    project: ProjectId,
    note: SourceVersionId,
    context: Option<serde_json::Value>,
    coverage: Option<SemanticCoverage>,
    compile_failed: bool,
    recovery_succeeded: bool,
    hindsight_context: Option<serde_json::Value>,
    replay_check: ReplayCheck,
    legacy_context: Option<Vec<u8>>,
    legacy_replay: Option<Result<Vec<u8>, merl_compiler::CompileError>>,
    rerun_context: Option<Vec<u8>>,
    rerun_selector: Option<String>,
    rerun_kept_revision: bool,
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
            recovery_succeeded: false,
            hindsight_context: None,
            replay_check: ReplayCheck::NotRun,
            legacy_context: None,
            legacy_replay: None,
            rerun_context: None,
            rerun_selector: None,
            rerun_kept_revision: false,
        }
    }

    pub fn given_a_note_followed_by_a_later_decision() -> Self {
        Self::note_followed_by_a_later_decision(CoverageRequirement::Optional)
    }

    fn note_followed_by_a_later_decision(requirement: CoverageRequirement) -> Self {
        let mut scenario = Self::new();
        scenario.capture(
            "note-v1",
            "Please revisit the earlier decision.",
            requirement,
        );
        let payload = merl_core::PayloadId::try_from("later-payload").expect("payload");
        scenario
            .store
            .put_payload(&scenario.project, &payload, b"Future decision: vary gain")
            .expect("payload");
        scenario
            .store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("later-batch").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 20,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("later-event").expect("event"),
                    object: ObjectId::try_from("future-decision").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: Some(payload),
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .expect("accept later decision");
        scenario
    }

    pub fn given_a_required_note_followed_by_a_later_decision() -> Self {
        Self::note_followed_by_a_later_decision(CoverageRequirement::Required)
    }

    pub fn given_more_accepted_objects_than_the_context_budget() -> Self {
        let mut scenario = Self::new();
        let events = (0..9)
            .map(|index| DomainEvent::PutObject {
                id: EventId::try_from(format!("event-{index}").as_str()).expect("event"),
                object: ObjectId::try_from(format!("object-{index}").as_str()).expect("object"),
                kind: ObjectKind::try_from("decision").expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            })
            .collect();
        scenario
            .store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("many-objects").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 5,
                events,
            })
            .expect("accepted objects");
        scenario.capture(
            "note-v1",
            "Which objects matter?",
            CoverageRequirement::Required,
        );
        scenario
    }

    pub fn given_required_and_optional_issue_notes() -> Self {
        let mut scenario = Self::new();
        scenario.capture_in_scope(
            "required-issue-note",
            "A decision is needed.",
            "issue-204",
            CoverageRequirement::Required,
        );
        scenario.capture_in_scope(
            "optional-issue-note",
            "Background reading.",
            "issue-204",
            CoverageRequirement::Optional,
        );
        scenario.capture_in_scope(
            "other-issue-note",
            "Other issue needs work.",
            "issue-999",
            CoverageRequirement::Required,
        );
        scenario.note = SourceVersionId::try_from("required-issue-note").expect("source");
        scenario
    }

    pub fn when_issue_coverage_is_inspected(&mut self) -> &mut Self {
        self.coverage = Some(
            self.store
                .semantic_coverage_in_scope(&self.project, "issue-204")
                .expect("Issue coverage"),
        );
        self
    }

    pub fn then_only_the_required_issue_note_is_a_gap(&mut self) -> &mut Self {
        let coverage = self.coverage.expect("coverage");
        assert_eq!(coverage.observation_head, 2);
        assert_eq!(coverage.processed_through, 0);
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.required_pending, 0);
        assert_eq!(coverage.optional_cold, 1);
        self
    }

    pub fn when_the_required_issue_note_is_compiled(&mut self) -> &mut Self {
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "required-issue-run",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 30,
            },
        )
        .expect("compile required note");
        self.when_issue_coverage_is_inspected()
    }

    pub fn then_issue_coverage_is_complete_with_an_optional_attachment(&mut self) -> &mut Self {
        let coverage = self.coverage.expect("coverage");
        assert_eq!(coverage.observation_head, 2);
        assert_eq!(coverage.processed_through, 2);
        assert_eq!(coverage.required_gaps, 0);
        assert_eq!(coverage.optional_cold, 1);
        self
    }

    pub fn given_many_unrelated_objects_and_a_named_decision() -> Self {
        let mut scenario = Self::new();
        let mut events: Vec<_> = (0..9)
            .map(|index| DomainEvent::PutObject {
                id: EventId::try_from(format!("noise-event-{index}").as_str()).expect("event"),
                object: ObjectId::try_from(format!("A{index}").as_str()).expect("object"),
                kind: ObjectKind::try_from("fact").expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            })
            .collect();
        events.push(DomainEvent::PutObject {
            id: EventId::try_from("decision-event").expect("event"),
            object: ObjectId::try_from("D18").expect("object"),
            kind: ObjectKind::try_from("decision").expect("kind"),
            payload: None,
            issue_scope: None,
            lifecycle: merl_core::ObjectLifecycle::Active,
        });
        scenario
            .store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("project-state").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 5,
                events,
            })
            .expect("accepted state");
        scenario.capture_in_scope(
            "issue-comment-v1",
            "Does D18 still apply?",
            "issue-204",
            CoverageRequirement::Required,
        );
        scenario.note = SourceVersionId::try_from("issue-comment-v1").expect("source");
        scenario
    }

    pub fn when_an_older_selector_run_is_recorded(&mut self) -> &mut Self {
        let current = build_context(&self.store, &self.project, &self.note, limits())
            .expect("current context");
        let old_selection = self
            .store
            .objects_at_revision(
                &self.project,
                current.interpretation_basis_revision,
                limits().objects,
            )
            .expect("old prefix selection");
        let old_objects = old_selection
            .items
            .iter()
            .map(|(id, revision, _)| {
                format!(
                    r#"{{"id":"{}","revision":{},"body":null}}"#,
                    id.as_str(),
                    revision.get()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let rendered = String::from_utf8(current.rendered).expect("context JSON");
        let object_start = rendered.find("\"objects\":").expect("objects key");
        let truncated_start = rendered
            .find(",\"objects_truncated\":")
            .expect("truncation key");
        let old_rendered = format!(
            "{}\"objects\":[{}]{}",
            &rendered[..object_start],
            old_objects,
            &rendered[truncated_start..]
        )
        .into_bytes();
        let references = old_selection
            .items
            .into_iter()
            .map(|(id, revision, _)| (id, revision))
            .collect::<Vec<_>>();
        let budget = limits();
        self.store
            .prepare_compilation(
                &self.project,
                &CompilationIntent {
                    id: "legacy-selector-run",
                    source: &self.note,
                    context: &old_rendered,
                    source_window: &current.source_window,
                    objects: &references,
                    interpretation_basis_revision: current.interpretation_basis_revision,
                    source_observation_cutoff: current.source_observation_cutoff,
                    renderer_version: "json_v1",
                    selector_version: "object_id_prefix_v1",
                    compiler_id: "fake",
                    compiler_version: "v1",
                    model_id: "deterministic",
                    prompt_digest: Sha256::digest(b"legacy").into(),
                    mode: "replay",
                    max_input_bytes: budget.input_bytes,
                    max_output_bytes: budget.output_bytes,
                    max_output_tokens: budget.output_tokens,
                    max_assertions: budget.assertions,
                    max_context_requests: budget.context_requests,
                    max_expansion_rounds: budget.expansion_rounds,
                    max_payload_bytes: budget.payload_bytes,
                    max_source_window: budget.source_window,
                    max_objects: budget.objects,
                    started_at_millis: 30,
                },
            )
            .expect("record legacy selector run");
        self.legacy_context = Some(old_rendered);
        self
    }

    pub fn when_that_run_is_rebuilt(&mut self) -> &mut Self {
        self.legacy_replay = Some(
            rebuild_recorded_context(&self.store, &self.project, "legacy-selector-run", limits())
                .map(|context| context.rendered),
        );
        self
    }

    pub fn then_the_original_selection_and_bytes_match(&mut self) -> &mut Self {
        assert_eq!(
            self.legacy_replay
                .take()
                .expect("replay result")
                .expect("legacy replay"),
            self.legacy_context
                .as_ref()
                .expect("recorded input")
                .clone()
        );
        self
    }

    pub fn when_that_input_is_replayed_with_a_new_compiler(&mut self) -> &mut Self {
        let before = self
            .store
            .project_revision(&self.project)
            .expect("accepted revision");
        let rebuilt =
            rebuild_recorded_context(&self.store, &self.project, "legacy-selector-run", limits())
                .expect("verified legacy input");
        let prepared = prepare_replay_compilation(
            &mut self.store,
            &self.project,
            "legacy-selector-run",
            rebuilt,
            &FakeCompiler,
            RunRequest {
                id: "legacy-rerun",
                limits: limits(),
                mode: RunMode::Replay,
                now_millis: 31,
            },
        )
        .expect("prepare replay")
        .expect("new replay run");
        let response = execute_compilation(&prepared, &FakeCompiler);
        record_compilation_result(&mut self.store, &self.project, &prepared, response, 32)
            .expect("record replay result");
        self.rerun_context = Some(
            self.store
                .load_compilation_context(&self.project, "legacy-rerun")
                .expect("new replay input")
                .rendered,
        );
        self.rerun_selector = Some(
            self.store
                .compilation_run_status(&self.project, "legacy-rerun")
                .expect("new replay status")
                .expect("run")
                .selector_version,
        );
        self.rerun_kept_revision = self
            .store
            .project_revision(&self.project)
            .expect("accepted revision")
            == before;
        self
    }

    pub fn then_the_new_run_uses_the_verified_bytes_and_original_selector(&mut self) {
        assert_eq!(self.rerun_context.as_ref(), self.legacy_context.as_ref());
        assert_eq!(self.rerun_selector.as_deref(), Some("object_id_prefix_v1"));
        let original = self
            .store
            .load_compilation_context(&self.project, "legacy-selector-run")
            .expect("original manifest");
        let replay = self
            .store
            .load_compilation_context(&self.project, "legacy-rerun")
            .expect("replay manifest");
        assert_eq!(replay.source_window, original.source_window);
        assert_eq!(replay.objects, original.objects);
        let status = self
            .store
            .compilation_run_status(&self.project, "legacy-rerun")
            .expect("replay status")
            .expect("new run");
        assert_eq!(status.renderer_version, "json_v1");
        assert_eq!(status.mode, "replay");
        assert!(self.rerun_kept_revision);
    }

    pub fn given_many_other_issue_objects_and_one_local_decision() -> Self {
        let mut scenario = Self::new();
        let mut events: Vec<_> = (0..9)
            .map(|index| DomainEvent::PutObject {
                id: EventId::try_from(format!("other-event-{index}").as_str()).expect("event"),
                object: ObjectId::try_from(format!("A{index}").as_str()).expect("object"),
                kind: ObjectKind::try_from("fact").expect("kind"),
                payload: None,
                issue_scope: Some("issue-999".into()),
                lifecycle: merl_core::ObjectLifecycle::Active,
            })
            .collect();
        events.push(DomainEvent::PutObject {
            id: EventId::try_from("local-decision-event").expect("event"),
            object: ObjectId::try_from("D18").expect("object"),
            kind: ObjectKind::try_from("decision").expect("kind"),
            payload: None,
            issue_scope: Some("issue-204".into()),
            lifecycle: merl_core::ObjectLifecycle::Active,
        });
        scenario
            .store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("scoped-state").expect("batch"),
                project: scenario.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 5,
                events,
            })
            .expect("accepted state");
        scenario.capture_in_scope(
            "local-comment",
            "Does the prior decision still apply?",
            "issue-204",
            CoverageRequirement::Required,
        );
        scenario.note = SourceVersionId::try_from("local-comment").expect("source");
        scenario
    }

    pub fn then_its_issue_decision_is_selected(&mut self) -> &mut Self {
        let context = self.context.as_ref().expect("context");
        let objects = context["objects"].as_array().expect("objects");
        assert!(objects.iter().any(|object| object["id"] == "D18"));
        self
    }

    pub fn when_the_issue_comment_is_compiled(&mut self) -> &mut Self {
        let context = build_context(&self.store, &self.project, &self.note, limits())
            .expect("bounded issue context");
        self.context = Some(serde_json::from_slice(&context.rendered).expect("context JSON"));
        self
    }

    pub fn then_the_named_decision_is_selected_before_unrelated_objects(&mut self) -> &mut Self {
        let context = self.context.as_ref().expect("context");
        let objects = context["objects"].as_array().expect("objects");
        assert_eq!(objects.len(), limits().objects);
        assert!(objects.iter().any(|object| object["id"] == "D18"));
        assert_eq!(context["objects_truncated"], true);
        self
    }

    pub fn given_an_edited_issue_with_a_different_editor() -> Self {
        let mut scenario = Self::new();
        let fixture = merl_corpus::github::fixture_from_graphql_pages(
            "DEV-FAKE",
            "2026-01-02T00:00:00Z",
            include_bytes!("../fixtures/github_two_page_edit.json"),
        )
        .expect("GitHub fixture");
        merl_ingest::import_fixture(&mut scenario.store, &scenario.project, &fixture)
            .expect("import edited Issue");
        scenario.note = merl_ingest::fixture_version_id("edit-1").expect("edited version");
        scenario
    }

    pub fn when_the_edited_version_is_compiled(&mut self) -> &mut Self {
        let mut budget = limits();
        budget.source_window = 1;
        let context =
            build_context(&self.store, &self.project, &self.note, budget).expect("edited context");
        self.context = Some(serde_json::from_slice(&context.rendered).expect("context JSON"));
        let response = format!(
            r#"{{"schema":"merl.compiler-response/v1","assertions":[{{"source":"{}","span_start":0,"span_end":3,"subject":"D1","predicate":"gain","value":"variable","act":"report","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":"user-2"}}]}}"#,
            self.note
        );
        let adapter = ProcessCompiler {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!("read -r request; printf '%s' '{response}'"),
            ],
            version: "v1".into(),
            model: "test-model".into(),
            prompt_digest: Sha256::digest(b"edited-issue-prompt").into(),
        };
        run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &adapter,
            RunRequest {
                id: "edited-run",
                limits: budget,
                mode: RunMode::Live,
                now_millis: 1_767_300_000_000,
            },
        )
        .expect("compile edited version");
        self
    }

    pub fn then_the_assertion_uses_source_authorship_without_verifying_a_relay(
        &mut self,
    ) -> &mut Self {
        let captured = self
            .store
            .source_version(&self.project, &self.note)
            .expect("source lookup")
            .expect("edited source");
        assert_eq!(
            captured.provider_source_author_id.as_deref(),
            Some("user-1")
        );
        assert_eq!(captured.provider_actor_id.as_deref(), Some("user-2"));
        let source = &self.context.as_ref().expect("context")["sources"][0];
        let author = source["source_author_id"].as_str().expect("source author");
        let editor = source["version_actor_id"].as_str().expect("version editor");
        assert_ne!(author, editor);
        assert!(
            source["occurred_at_millis"].as_i64().expect("version time")
                > source["created_at_millis"].as_i64().expect("creation time")
        );
        let assertion = self
            .store
            .observed_assertions(&self.project, "edited-run")
            .expect("assertions")
            .into_iter()
            .next()
            .expect("one assertion");
        assert_eq!(assertion.asserted_by.as_deref(), Some(author));
        assert_eq!(assertion.attributed_to.as_deref(), Some("user-2"));
        assert!(!assertion.attribution_verified);
        self
    }

    pub fn when_the_note_is_compiled_with_hindsight(&mut self) -> &mut Self {
        let prepared = prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "hindsight-run",
                limits: limits(),
                mode: RunMode::Hindsight,
                now_millis: 30,
            },
        )
        .expect("prepare hindsight")
        .expect("new run");
        self.hindsight_context = Some(
            serde_json::from_slice(
                &self
                    .store
                    .load_compilation_context(&self.project, prepared.id())
                    .expect("saved context")
                    .rendered,
            )
            .expect("context JSON"),
        );
        record_compilation_result(
            &mut self.store,
            &self.project,
            &prepared,
            execute_compilation(&prepared, &FakeCompiler),
            31,
        )
        .expect("record hindsight");
        self.coverage = Some(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage"),
        );
        self
    }

    pub fn then_current_state_is_visible_but_required_coverage_remains_open(
        &mut self,
    ) -> &mut Self {
        let context = self.hindsight_context.as_ref().expect("hindsight context");
        assert_eq!(context["interpretation_basis_revision"], 1);
        assert_eq!(context["objects"].as_array().expect("objects").len(), 1);
        assert_eq!(self.coverage.expect("coverage").required_gaps, 1);
        self
    }

    pub fn then_only_the_budgeted_objects_are_selected(&mut self) -> &mut Self {
        let context = self.context.as_ref().expect("context");
        assert_eq!(
            context["objects"].as_array().expect("objects").len(),
            limits().objects
        );
        assert_eq!(context["objects_truncated"], true);
        self
    }

    pub fn then_all_nine_limits_are_retained(&mut self) -> &mut Self {
        let status = self
            .store
            .compilation_run_status(&self.project, "pending-run")
            .expect("run lookup")
            .expect("run");
        assert_eq!(status.limits, [4096, 4096, 512, 8, 2, 1, 2048, 4, 8]);
        self
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
        let response = r#"{"schema":"merl.compiler-response/v1","assertions":[{"source":"note-v1","span_start":2,"span_end":9,"subject":"T1","predicate":"blocked","value":"true","act":"report","epistemic_basis":"observed","polarity":"positive","confidence_millis":900,"attributed_to":null}]}"#;
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
                now_millis: 30,
            },
        )
        .expect("prepare run")
        .expect("new run");
        drop(prepared);
        self
    }

    pub fn when_the_compiler_requests_more_context(&mut self) -> &mut Self {
        let response = r#"{"schema":"merl.compiler-response/v1","assertions":[],"context_required":[{"reference":"D18"}]}"#;
        let adapter = ProcessCompiler {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!("read -r request; printf '%s' '{response}'"),
            ],
            version: "v1".into(),
            model: "test-model".into(),
            prompt_digest: Sha256::digest(b"context-request-prompt").into(),
        };
        let _ = run_compiler(
            &mut self.store,
            &self.project,
            &self.note,
            &adapter,
            RunRequest {
                id: "context-needed-run",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 30,
            },
        );
        self
    }

    pub fn then_the_run_needs_expansion_and_coverage_remains_open(&mut self) -> &mut Self {
        let status = self
            .store
            .compilation_run_status(&self.project, "context-needed-run")
            .expect("run lookup")
            .expect("run");
        assert!(status.completed);
        assert!(!status.succeeded);
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage")
                .required_gaps,
            1
        );
        self
    }

    pub fn when_the_run_is_recovered_after_source_bytes_disappear(&mut self) -> &mut Self {
        let request = RunRequest {
            id: "recover-run",
            limits: limits(),
            mode: RunMode::Live,
            now_millis: 30,
        };
        let prepared = prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            request,
        )
        .expect("prepare")
        .expect("new run");
        drop(prepared);
        let source = self
            .store
            .source_version(&self.project, &self.note)
            .expect("source lookup")
            .expect("source");
        self.store
            .erase_payload(
                &self.project,
                source.payload.as_ref().expect("source payload"),
            )
            .expect("erase original source bytes");
        self.recovery_succeeded = prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            request,
        )
        .ok()
        .flatten()
        .is_some_and(|prepared| {
            let raw = execute_compilation(&prepared, &FakeCompiler);
            record_compilation_result(&mut self.store, &self.project, &prepared, raw, 31).is_ok()
        });
        self.recovery_succeeded &= prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            request,
        )
        .is_ok_and(|prepared| prepared.is_none());
        self
    }

    pub fn then_the_original_context_is_still_ready_for_execution(&mut self) -> &mut Self {
        assert!(self.recovery_succeeded);
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
        assert_eq!(assertions[0].asserted_by, None);
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
                    source_author: None,
                    provider_source_author_id: None,
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

    pub fn when_the_recorded_input_is_rebuilt_and_the_compiler_is_rerun(&mut self) -> &mut Self {
        let rebuilt = merl_compiler::rebuild_recorded_context(
            &self.store,
            &self.project,
            "run-one",
            limits(),
        )
        .expect("recorded input");
        let original = self
            .store
            .load_compilation_context(&self.project, "run-one")
            .expect("original input");
        self.replay_check = if rebuilt.rendered == original.rendered {
            ReplayCheck::Matching
        } else {
            ReplayCheck::NotRun
        };
        let prepared = prepare_replay_compilation(
            &mut self.store,
            &self.project,
            "run-one",
            rebuilt,
            &FakeCompiler,
            RunRequest {
                id: "replay-run",
                limits: limits(),
                mode: RunMode::Replay,
                now_millis: 40,
            },
        )
        .expect("prepare replay")
        .expect("new replay run");
        let response = execute_compilation(&prepared, &FakeCompiler);
        record_compilation_result(&mut self.store, &self.project, &prepared, response, 40)
            .expect("record replay run");
        self
    }

    pub fn then_replay_matches_the_recorded_input_and_leaves_the_decision_alone(
        &mut self,
    ) -> &mut Self {
        assert!(matches!(self.replay_check, ReplayCheck::Matching));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        assert_eq!(
            self.store
                .compilation_run_count(&self.project, &self.note)
                .expect("runs"),
            2
        );
        self
    }

    pub fn when_the_source_bytes_are_erased(&mut self) -> &mut Self {
        self.store
            .erase_payload(
                &self.project,
                &merl_core::PayloadId::try_from("src_note-v1").expect("payload"),
            )
            .expect("erase source");
        self
    }

    pub fn when_the_recorded_input_is_rebuilt_again(&mut self) -> &mut Self {
        self.replay_check = if matches!(
            merl_compiler::rebuild_recorded_context(
                &self.store,
                &self.project,
                "run-one",
                limits(),
            ),
            Err(merl_compiler::CompileError::MissingEvidence)
        ) {
            ReplayCheck::MissingEvidence
        } else {
            ReplayCheck::NotRun
        };
        self
    }

    pub fn then_replay_reports_missing_evidence(&mut self) {
        assert!(matches!(self.replay_check, ReplayCheck::MissingEvidence));
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

    pub fn when_a_retry_is_prepared(&mut self) -> &mut Self {
        prepare_compilation(
            &mut self.store,
            &self.project,
            &self.note,
            &FakeCompiler,
            RunRequest {
                id: "retry-run",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 40,
            },
        )
        .expect("prepare retry")
        .expect("pending work");
        self.coverage = Some(
            self.store
                .semantic_coverage(&self.project)
                .expect("coverage"),
        );
        self
    }

    pub fn then_coverage_reports_pending_instead_of_failed(&mut self) -> &mut Self {
        let coverage = self.coverage.expect("coverage");
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.required_pending, 1);
        assert_eq!(coverage.required_failed, 0);
        assert_eq!(coverage.optional_cold, 1);
        self
    }

    pub fn then_the_retry_has_a_later_durable_attempt_order(&mut self) {
        let failed = self
            .store
            .compilation_run_status(&self.project, "budget-run")
            .expect("failed run")
            .expect("run");
        let pending = self
            .store
            .compilation_run_status(&self.project, "retry-run")
            .expect("pending run")
            .expect("run");
        assert!(pending.attempt_order > failed.attempt_order);
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
