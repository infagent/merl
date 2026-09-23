use merl_compiler::{
    CompilerLimits, FakeCompiler, ProcessCompiler, RunMode, RunRequest, execute_compilation,
    prepare_context_expansion, prepare_eager_compilation, prepare_replay_compilation,
    rebuild_recorded_context, record_compilation_result,
};
use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, EventId, ObjectId, ObjectKind, ProjectId, SourceBindingId, SourceId,
    SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{SourceBinding, SourceCapture, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub struct Expansion {
    directory: PathBuf,
    store: Store,
    project: ProjectId,
    inspected: Option<Value>,
    contexts: Vec<Value>,
    lineage: Option<Value>,
    failures: Vec<(String, String)>,
    retry_pairs: Vec<(Value, Value)>,
    guarded_child: Option<String>,
    erasure_rejected: bool,
}
impl Drop for Expansion {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Expansion {
    pub fn given_a_compiler_missing_an_earlier_decision() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "merl-expansion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("directory");
        let mut store = Store::open(&directory.join("project.sqlite")).expect("store");
        let project = ProjectId::try_from("Expansion").unwrap();
        store.create_project(&project).expect("project");
        let mut s = Self {
            directory,
            store,
            project,
            inspected: None,
            contexts: Vec::new(),
            lineage: None,
            failures: Vec::new(),
            retry_pairs: Vec::new(),
            guarded_child: None,
            erasure_rejected: false,
        };
        s.decision("earlier", "Keep gain fixed");
        s.decision("other", "Use double precision");
        s.capture_in_scope(
            "note-v1",
            "Revisit the earlier decision.",
            "note-thread",
            CoverageRequirement::Required,
        );
        s.decision("future", "Sweep gain instead");
        s
    }
    fn decision(&mut self, id: &str, text: &str) {
        let payload = merl_core::PayloadId::try_from(id).unwrap();
        self.store
            .put_payload(&self.project, &payload, text.as_bytes())
            .unwrap();
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from(id).unwrap(),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").unwrap(),
                occurred_at_millis: 5,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from(id).unwrap(),
                    object: ObjectId::try_from(id).unwrap(),
                    kind: ObjectKind::try_from("decision").unwrap(),
                    payload: Some(payload),
                    issue_scope: Some("other-thread".into()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .unwrap();
    }
    pub fn when_it_requests_the_decision(&mut self) -> &mut Self {
        let prepared = prepare_eager_compilation(
            &mut self.store,
            &self.project,
            &SourceVersionId::try_from("note-v1").unwrap(),
            &FakeCompiler,
            RunRequest {
                id: "root",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 20,
            },
        )
        .unwrap()
        .unwrap();
        let response = json!({"schema":"merl.compiler-response/v1", "assertions":[], "context_required":[{"reference":"earlier"}]}).to_string().into_bytes();
        assert!(matches!(
            record_compilation_result(&mut self.store, &self.project, &prepared, Ok(response), 21),
            Err(merl_compiler::CompileError::ContextRequired)
        ));
        self
    }
    pub fn when_the_work_is_inspected_after_restart(&mut self) -> &mut Self {
        self.store = Store::open(&self.directory.join("project.sqlite")).unwrap();
        let args = [
            "compilation",
            "show",
            "--run",
            "root",
            "--project",
            "Expansion",
            "--database",
            self.directory.join("project.sqlite").to_str().unwrap(),
            "--json",
        ]
        .map(str::to_owned);
        let result = match merl_cli::run_with_clock(&args, &|| Ok(30)) {
            merl_cli::CliResponse::Success(s) => s,
            other => panic!("inspect durable expansion work: {other:?}"),
        };
        self.inspected = Some(serde_json::from_str(&result).unwrap());
        self
    }
    pub fn then_the_request_is_pending_and_coverage_is_open(&mut self) -> &mut Self {
        let result = self.inspected.as_ref().unwrap();
        assert_eq!(result["expansion"]["status"], "pending");
        assert_eq!(result["expansion"]["round"], 1);
        assert_eq!(result["expansion"]["references"], json!(["earlier"]));
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .unwrap()
                .required_gaps,
            1
        );
        self
    }
    fn cli(&self, command: &[&str]) -> Value {
        let mut args = command.iter().map(ToString::to_string).collect::<Vec<_>>();
        args.extend([
            "--project".into(),
            "Expansion".into(),
            "--database".into(),
            self.directory
                .join("project.sqlite")
                .to_str()
                .unwrap()
                .into(),
            "--json".into(),
        ]);
        match merl_cli::run_with_clock(&args, &|| Ok(50)) {
            merl_cli::CliResponse::Success(s) => serde_json::from_str(&s).unwrap(),
            other => panic!("CLI: {other:?}"),
        }
    }
    fn request(&mut self, run: &str, reference: &str, budget: CompilerLimits) {
        let prepared = prepare_eager_compilation(
            &mut self.store,
            &self.project,
            &SourceVersionId::try_from("note-v1").unwrap(),
            &FakeCompiler,
            RunRequest {
                id: run,
                limits: budget,
                mode: RunMode::Live,
                now_millis: 20,
            },
        )
        .unwrap()
        .unwrap();
        self.record_request(&prepared, reference);
    }
    fn record_request(&mut self, prepared: &merl_compiler::PreparedCompilation, reference: &str) {
        let bytes = json!({"schema":"merl.compiler-response/v1","assertions":[{"source":"note-v1","span_start":0,"span_end":7,"subject":"D1","predicate":"decision","value":"none","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":null}],"context_required":[{"reference":reference}]}).to_string().into_bytes();
        assert!(matches!(
            record_compilation_result(&mut self.store, &self.project, prepared, Ok(bytes), 40),
            Err(merl_compiler::CompileError::ContextRequired)
        ));
    }
    pub fn when_two_rounds_resume_after_restart(&mut self) -> &mut Self {
        let first =
            prepare_context_expansion(&mut self.store, &self.project, "root", &FakeCompiler, 30)
                .unwrap()
                .unwrap();
        let id = first.id().to_owned();
        let saved = self
            .store
            .load_compilation_context(&self.project, &id)
            .unwrap()
            .rendered;
        self.store = Store::open(&self.directory.join("project.sqlite")).unwrap();
        let recovered =
            prepare_context_expansion(&mut self.store, &self.project, "root", &FakeCompiler, 31)
                .unwrap()
                .unwrap();
        assert_eq!(id, recovered.id());
        assert_eq!(
            saved,
            self.store
                .load_compilation_context(&self.project, recovered.id())
                .unwrap()
                .rendered
        );
        self.contexts.push(serde_json::from_slice(&saved).unwrap());
        self.record_request(&recovered, "other");
        let second =
            prepare_context_expansion(&mut self.store, &self.project, &id, &FakeCompiler, 41)
                .unwrap()
                .unwrap();
        let saved = self
            .store
            .load_compilation_context(&self.project, second.id())
            .unwrap()
            .rendered;
        self.contexts.push(serde_json::from_slice(&saved).unwrap());
        let rebuilt =
            rebuild_recorded_context(&self.store, &self.project, second.id(), limits()).unwrap();
        assert_eq!(rebuilt.rendered, saved);
        let bytes = json!({"schema":"merl.compiler-response/v1","assertions":[{"source":"note-v1","span_start":0,"span_end":7,"subject":"D1","predicate":"decision","value":"none","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":null}]}).to_string().into_bytes();
        record_compilation_result(&mut self.store, &self.project, &second, Ok(bytes), 42).unwrap();
        assert!(
            prepare_context_expansion(&mut self.store, &self.project, &id, &FakeCompiler, 43)
                .unwrap()
                .is_none()
        );
        self.lineage = Some(self.cli(&["source", "assertions", "--run", second.id()]));
        let replay = prepare_replay_compilation(
            &mut self.store,
            &self.project,
            second.id(),
            rebuilt,
            &FakeCompiler,
            RunRequest {
                id: "replay-expanded",
                limits: limits(),
                mode: RunMode::Replay,
                now_millis: 44,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            rebuild_recorded_context(&self.store, &self.project, replay.id(), limits())
                .unwrap()
                .rendered,
            saved
        );
        let result = execute_compilation(&replay, &FakeCompiler);
        record_compilation_result(&mut self.store, &self.project, &replay, result, 45).unwrap();
        self
    }
    pub fn then_only_requested_historical_context_is_added(&mut self) -> &mut Self {
        assert_eq!(
            self.contexts[0]["objects"],
            json!([{"id":"earlier","revision":1,"body":"Keep gain fixed"}])
        );
        assert_eq!(self.contexts[1]["objects"].as_array().unwrap().len(), 2);
        for context in &self.contexts {
            assert_eq!(context["interpretation_basis_revision"], 2);
            assert_eq!(context["source_observation_cutoff"], 1);
            assert_eq!(context["sources"].as_array().unwrap().len(), 1);
            assert!(!context.to_string().contains("Sweep gain"));
        }
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .unwrap()
                .required_gaps,
            0
        );
        self
    }
    pub fn then_final_assertions_expand_through_both_rounds(&mut self) -> &mut Self {
        let result = self.lineage.as_ref().unwrap();
        let rounds = result["assertions"][0]["context_rounds"]
            .as_array()
            .unwrap();
        assert_eq!(rounds.len(), 3);
        assert_eq!(rounds[0]["run"], "root");
        assert_eq!(rounds[0]["expansion"]["references"], json!(["earlier"]));
        assert_eq!(rounds[1]["expansion"]["references"], json!(["other"]));
        assert_eq!(rounds[2]["outcome"], "succeeded");
        self
    }
    fn failure_cases(&mut self) -> Vec<(&'static str, &'static str, CompilerLimits, &'static str)> {
        let foreign = ProjectId::try_from("Foreign").unwrap();
        self.store.create_project(&foreign).unwrap();
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("foreign").unwrap(),
                project: foreign,
                actor: ActorId::try_from("owner").unwrap(),
                occurred_at_millis: 5,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("foreign").unwrap(),
                    object: ObjectId::try_from("foreign-only").unwrap(),
                    kind: ObjectKind::try_from("decision").unwrap(),
                    payload: None,
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                }],
            })
            .unwrap();
        let initial_bytes = merl_compiler::build_context(
            &self.store,
            &self.project,
            &SourceVersionId::try_from("note-v1").unwrap(),
            limits(),
        )
        .unwrap()
        .rendered
        .len();
        vec![
            (
                "encoded-limit",
                "earlier",
                CompilerLimits {
                    input_bytes: initial_bytes + 1,
                    ..limits()
                },
                "input_budget",
            ),
            ("foreign", "foreign-only", limits(), "expansion_reference"),
            ("missing", "missing", limits(), "expansion_reference"),
            ("future", "future", limits(), "expansion_reference"),
            ("loop", "note-v1", limits(), "expansion_loop"),
            (
                "round-limit",
                "earlier",
                CompilerLimits {
                    expansion_rounds: 0,
                    ..limits()
                },
                "expansion_round_budget",
            ),
            (
                "text-limit",
                "earlier",
                CompilerLimits {
                    payload_bytes: 30,
                    ..limits()
                },
                "input_budget",
            ),
        ]
    }
    pub fn when_invalid_or_exhausted_expansions_are_attempted(&mut self) -> &mut Self {
        let cases = self.failure_cases();
        for (run, reference, budget, code) in cases {
            self.request(run, reference, budget);
            assert!(
                prepare_context_expansion(&mut self.store, &self.project, run, &FakeCompiler, 30)
                    .is_err()
            );
            self.failures.push((run.into(), code.into()));
        }
        self.request(
            "spent-round",
            "earlier",
            CompilerLimits {
                expansion_rounds: 1,
                ..limits()
            },
        );
        let child = prepare_context_expansion(
            &mut self.store,
            &self.project,
            "spent-round",
            &FakeCompiler,
            30,
        )
        .unwrap()
        .unwrap();
        self.record_request(&child, "other");
        assert!(matches!(
            prepare_context_expansion(
                &mut self.store,
                &self.project,
                child.id(),
                &FakeCompiler,
                41
            ),
            Err(merl_compiler::CompileError::ExpansionRoundBudget)
        ));
        self.failures
            .push((child.id().into(), "expansion_round_budget".into()));
        self.request(
            "object-limit",
            "earlier",
            CompilerLimits {
                objects: 1,
                ..limits()
            },
        );
        let child = prepare_context_expansion(
            &mut self.store,
            &self.project,
            "object-limit",
            &FakeCompiler,
            30,
        )
        .unwrap()
        .unwrap();
        self.record_request(&child, "other");
        assert!(matches!(
            prepare_context_expansion(
                &mut self.store,
                &self.project,
                child.id(),
                &FakeCompiler,
                41
            ),
            Err(merl_compiler::CompileError::InputBudget)
        ));
        self.failures
            .push((child.id().into(), "input_budget".into()));
        self.store
            .erase_payload(
                &self.project,
                &merl_core::PayloadId::try_from("earlier").unwrap(),
            )
            .unwrap();
        self.request("erased-reference", "earlier", limits());
        assert!(matches!(
            prepare_context_expansion(
                &mut self.store,
                &self.project,
                "erased-reference",
                &FakeCompiler,
                41
            ),
            Err(merl_compiler::CompileError::MissingEvidence)
        ));
        self.failures
            .push(("erased-reference".into(), "missing_evidence".into()));
        self.store = Store::open(&self.directory.join("project.sqlite")).unwrap();
        self.inspected = Some(self.cli(&["compilation", "list"]));
        self
    }
    pub fn then_each_failure_is_durable_without_partial_assertions(&mut self) -> &mut Self {
        let work = self.inspected.as_ref().unwrap()["work"].as_array().unwrap();
        assert_eq!(work.len(), self.failures.len() + 2);
        for (run, code) in &self.failures {
            let item = work.iter().find(|w| w["parent_run"] == *run).unwrap();
            assert_eq!(item["failure_code"], *code);
            assert!(matches!(
                item["status"].as_str(),
                Some("blocked" | "exhausted")
            ));
            assert!(
                self.store
                    .observed_assertions(&self.project, run)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                self.store
                    .compilation_run_status(&self.project, item["child_run"].as_str().unwrap())
                    .unwrap()
                    .is_none()
            );
        }
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .unwrap()
                .required_gaps,
            1
        );
        assert_eq!(self.store.project_revision(&self.project).unwrap().get(), 3);
        self
    }
    pub fn when_a_later_note_requests_another_threads_source(&mut self) -> &mut Self {
        self.capture_in_scope(
            "elsewhere",
            "A prior observation.",
            "elsewhere-thread",
            CoverageRequirement::Optional,
        );
        self.capture_in_scope(
            "new-trigger",
            "Revisit that observation.",
            "new-thread",
            CoverageRequirement::Required,
        );
        let prepared = prepare_eager_compilation(
            &mut self.store,
            &self.project,
            &SourceVersionId::try_from("new-trigger").unwrap(),
            &FakeCompiler,
            RunRequest {
                id: "source-root",
                limits: limits(),
                mode: RunMode::Live,
                now_millis: 20,
            },
        )
        .unwrap()
        .unwrap();
        let bytes=json!({"schema":"merl.compiler-response/v1","assertions":[],"context_required":[{"reference":"elsewhere"}]}).to_string().into_bytes();
        assert!(matches!(
            record_compilation_result(&mut self.store, &self.project, &prepared, Ok(bytes), 21),
            Err(merl_compiler::CompileError::ContextRequired)
        ));
        let child = prepare_context_expansion(
            &mut self.store,
            &self.project,
            "source-root",
            &FakeCompiler,
            22,
        )
        .unwrap()
        .unwrap();
        self.contexts.push(
            serde_json::from_slice(
                &self
                    .store
                    .load_compilation_context(&self.project, child.id())
                    .unwrap()
                    .rendered,
            )
            .unwrap(),
        );
        let result = execute_compilation(&child, &FakeCompiler);
        record_compilation_result(&mut self.store, &self.project, &child, result, 23).unwrap();
        self
    }
    pub fn then_only_the_named_source_is_added(&mut self) -> &mut Self {
        let context = &self.contexts[0];
        assert_eq!(
            context["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["elsewhere", "new-trigger"]
        );
        assert_eq!(context["source_observation_cutoff"], 3);
        assert_eq!(
            self.store
                .semantic_coverage_in_scope(&self.project, "new-thread")
                .unwrap()
                .required_gaps,
            0
        );
        self
    }
    pub fn when_the_cli_resumes_and_retries_process_work(&mut self) -> &mut Self {
        for (run, response) in [
            (
                "process-more",
                r#"printf '%s' '{"schema":"merl.compiler-response/v1","assertions":[],"context_required":[{"reference":"other"}]}'"#,
            ),
            (
                "process-root",
                r#"printf '%s' '{"schema":"merl.compiler-response/v1","assertions":[]}'"#,
            ),
        ] {
            let adapter = ProcessCompiler::new(
                "/bin/sh",
                vec!["-c".into(), response.into()],
                "v1",
                "test-model",
                [0; 32],
            )
            .unwrap();
            let prepared = prepare_eager_compilation(
                &mut self.store,
                &self.project,
                &SourceVersionId::try_from("note-v1").unwrap(),
                &adapter,
                RunRequest {
                    id: run,
                    limits: limits(),
                    mode: RunMode::Live,
                    now_millis: 20,
                },
            )
            .unwrap()
            .unwrap();
            self.record_request(&prepared, "earlier");
            let before = self.cli(&["compilation", "show", "--run", run]);
            let command = [
                "compilation",
                "expand",
                "--run",
                run,
                "--program",
                "/bin/sh",
                "--compiler-arg",
                "-c",
                "--compiler-arg",
                response,
                "--compiler-version",
                "v1",
                "--model",
                "test-model",
                "--prompt-digest",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ];
            self.inspected = Some(self.cli(&command));
            self.lineage = Some(self.cli(&command));
            self.retry_pairs.push((
                self.inspected.clone().unwrap(),
                self.lineage.clone().unwrap(),
            ));
            assert!(matches!(
                prepare_context_expansion(&mut self.store, &self.project, run, &FakeCompiler, 51),
                Err(merl_compiler::CompileError::UnauthorizedCompilation)
            ));
            let child = self.inspected.as_ref().unwrap()["expansion"]["child_run"]
                .as_str()
                .unwrap();
            assert_eq!(before["expansion"]["child_run"], child);
            self.contexts
                .push(self.cli(&["compilation", "show", "--run", child]));
        }
        self
    }
    pub fn then_the_original_budget_and_completed_successor_are_reused(&mut self) -> &mut Self {
        assert!(self.retry_pairs.iter().all(|(first, retry)| first == retry));
        assert_eq!(
            self.inspected.as_ref().unwrap()["expansion"]["status"],
            "completed"
        );
        assert!(
            self.contexts
                .iter()
                .all(|context| context["limits"] == json!(limits().as_array()))
        );
        assert_eq!(self.store.project_revision(&self.project).unwrap().get(), 3);
        assert_eq!(
            self.store
                .semantic_coverage(&self.project)
                .unwrap()
                .required_gaps,
            0
        );
        self
    }
    pub fn when_erasure_intervenes_before_the_expanded_intent_commits(&mut self) -> &mut Self {
        let status = self
            .store
            .compilation_run_status(&self.project, "root")
            .unwrap()
            .unwrap();
        let work = self
            .store
            .expansion_work(&self.project, "root")
            .unwrap()
            .unwrap();
        let saved = self
            .store
            .load_compilation_context(&self.project, "root")
            .unwrap();
        let mut rendered: Value = serde_json::from_slice(&saved.rendered).unwrap();
        rendered["objects"] = json!([{"id":"earlier","revision":1,"body":"Keep gain fixed"}]);
        let rendered = serde_json::to_vec(&rendered).unwrap();
        let objects = vec![(
            ObjectId::try_from("earlier").unwrap(),
            merl_core::ObjectRevision::try_from(1).unwrap(),
        )];
        self.store
            .erase_payload(
                &self.project,
                &merl_core::PayloadId::try_from("earlier").unwrap(),
            )
            .unwrap();
        let budget = limits();
        let record = merl_store::CompilationIntent {
            id: &work.child_run,
            source: &status.source,
            context: &rendered,
            source_window: &saved.source_window,
            objects: &objects,
            interpretation_basis_revision: status.interpretation_basis_revision,
            source_observation_cutoff: status.source_observation_cutoff,
            renderer_version: "json_v1",
            selector_version: "context_expansion_v1",
            compiler_id: &status.compiler_id,
            compiler_version: &status.compiler_version,
            model_id: &status.model_id,
            prompt_digest: status.prompt_digest,
            adapter_config_digest: status.adapter_config_digest,
            mode: "live",
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
        };
        self.erasure_rejected = self
            .store
            .prepare_compilation(&self.project, &record)
            .is_err();
        self.guarded_child = Some(work.child_run);
        self
    }
    pub fn then_erased_content_has_no_new_retained_copy(&mut self) -> &mut Self {
        assert!(self.erasure_rejected);
        let child = self.guarded_child.as_ref().unwrap();
        assert!(
            self.store
                .compilation_run_status(&self.project, child)
                .unwrap()
                .is_none()
        );
        let payload = merl_core::PayloadId::try_from(format!("ctx_{child}").as_str()).unwrap();
        assert!(matches!(
            self.store.read_payload(&self.project, &payload),
            Err(merl_store::StoreError::PayloadMissing)
        ));
        self
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
                    compilation_mode: CompilationMode::Eager,
                    coverage_requirement: requirement,
                    policy_version: CapturePolicyVersion::try_from("test-v1").expect("policy"),
                },
            )
            .expect("capture");
    }
}
fn limits() -> CompilerLimits {
    CompilerLimits {
        input_bytes: 4096,
        output_bytes: 4096,
        output_tokens: 512,
        assertions: 8,
        context_requests: 2,
        expansion_rounds: 2,
        payload_bytes: 2048,
        source_window: 4,
        objects: 8,
    }
}
