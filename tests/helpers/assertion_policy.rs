use merl_compiler::{
    CompileError, CompilerAdapter, CompilerLimits, RunMode, RunRequest, prepare_eager_compilation,
    prepare_evaluation_compilation, prepare_hindsight_compilation, prepare_replay_compilation,
    record_compilation_result,
};
use merl_core::{CompilationMode, CoverageRequirement, ProjectId};
use merl_store::{SourceBinding, SourceCapture, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const NOW: i64 = 1_700_000_000_000;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn id<T: for<'a> TryFrom<&'a str>>(s: &str) -> T
where
    for<'a> <T as TryFrom<&'a str>>::Error: std::fmt::Debug,
{
    T::try_from(s).expect("structural identity")
}

struct Database(PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "merl-assertions-{}-{}.sqlite",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("unique database");
        Self(path)
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Compiler;
impl CompilerAdapter for Compiler {
    fn id(&self) -> &'static str {
        "assertion-test"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn model(&self) -> &'static str {
        "fixture"
    }
    fn prompt_digest(&self) -> [u8; 32] {
        [1; 32]
    }
    fn compile(&self, _: &[u8], _: CompilerLimits) -> Result<Vec<u8>, CompileError> {
        unreachable!("responses are controlled by the scenario")
    }
}

pub struct AssertionScenario {
    database: Database,
    runs: Vec<(String, String)>,
    outputs: Vec<Value>,
    inspected: Option<Value>,
    detail: Option<Value>,
    retry: Option<Value>,
    inbox: Option<Value>,
    commit_result: Option<Result<Option<merl_core::ProjectRevision>, merl_store::StoreError>>,
    basis: u64,
}
impl AssertionScenario {
    fn new() -> Self {
        let scenario = Self {
            database: Database::new(),
            runs: Vec::new(),
            outputs: Vec::new(),
            inspected: None,
            detail: None,
            retry: None,
            inbox: None,
            commit_result: None,
            basis: 0,
        };
        let mut store = scenario.store();
        store.create_project(&id("P1")).unwrap();
        store
            .grant_administrator_unchecked_bootstrap(&id("P1"), &id("admin"))
            .unwrap();
        drop(store);
        scenario.grant("alice", "decision_author", "grant-decision");
        scenario.grant("agent", "command_actor", "grant-command");
        scenario
            .store()
            .subscribe_all(&id("P1"), &id("reader"))
            .unwrap();
        scenario
    }
    fn store(&self) -> Store {
        Store::open(&self.database.0).unwrap()
    }
    fn cli(&self, args: &[&str]) -> Value {
        let mut args: Vec<String> = args.iter().map(|s| (*s).into()).collect();
        args.extend([
            "--project".into(),
            "P1".into(),
            "--database".into(),
            self.database.0.to_str().unwrap().into(),
            "--json".into(),
        ]);
        match merl_cli::run_with_clock(&args, &|| Ok(NOW + 10)) {
            merl_cli::CliResponse::Success(s) => serde_json::from_str(&s).unwrap(),
            merl_cli::CliResponse::JsonError(e) => serde_json::from_str(&e.as_json()).unwrap(),
            merl_cli::CliResponse::HumanError(e) => panic!("unexpected human error: {e}"),
        }
    }
    fn grant(&self, subject: &str, permission: &str, request: &str) {
        let result = self.cli(&[
            "project",
            "authority",
            "grant",
            "--id",
            request,
            "--actor",
            "admin",
            "--subject",
            subject,
            "--permission",
            permission,
            "--reason",
            "Project responsibility",
        ]);
        assert_eq!(result["outcome"], "accepted");
    }
    fn assertion(subject: &str) -> Value {
        json!({"source":"S1", "span_start":0,"span_end":13,"subject":subject,"predicate":"decision","value":"none","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":null})
    }
    fn capture(
        store: &mut Store,
        source: &str,
        version: &str,
        prior: Option<&str>,
        author: Option<&str>,
    ) {
        store
            .capture_source_version(
                &id("P1"),
                &SourceCapture {
                    binding: SourceBinding {
                        id: id("binding"),
                        provider: id("github"),
                        provider_namespace_id: "repo".into(),
                        namespace_digest: Sha256::digest(b"repo").into(),
                    },
                    source: id(source),
                    provider_entity_id: source,
                    context_scope_id: "issue-1",
                    version: id(version),
                    provider_version_id: version,
                    kind: id("issue_comment"),
                    supersedes: prior.map(id),
                    ambiguous_order_with_previous: false,
                    created_at_millis: NOW,
                    occurred_at_millis: NOW,
                    upstream_updated_at_millis: Some(if prior.is_some() { NOW + 3 } else { NOW }),
                    observed_at_millis: if prior.is_some() { NOW + 3 } else { NOW },
                    actor: author.map(id),
                    provider_actor_id: author,
                    source_author: author.map(id),
                    provider_source_author_id: author,
                    body: Some(if prior.is_some() {
                        b"Correction: use variable gain."
                    } else {
                        b"Use fixed gain. Please add a task. The other item is unclear."
                    }),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::Eager,
                    coverage_requirement: CoverageRequirement::Required,
                    policy_version: id("capture-v1"),
                },
            )
            .unwrap();
    }
    fn compile(&mut self, run: &str, author: Option<&str>, mode: RunMode, response: Option<Value>) {
        let mut store = self.store();
        let source = format!("source-{run}");
        Self::capture(&mut store, &source, &source, None, author);
        let limits = CompilerLimits {
            input_bytes: 32768,
            output_bytes: 16384,
            output_tokens: 2048,
            assertions: 32,
            context_requests: 2,
            expansion_rounds: 1,
            payload_bytes: 4096,
            source_window: 1,
            objects: 4,
        };
        let request = RunRequest {
            id: run,
            limits,
            mode,
            now_millis: NOW + 1,
        };
        let prepared = match mode {
            RunMode::Live => {
                prepare_eager_compilation(&mut store, &id("P1"), &id(&source), &Compiler, request)
            }
            RunMode::Eval => prepare_evaluation_compilation(
                &mut store,
                &id("P1"),
                &id(&source),
                &Compiler,
                request,
            ),
            RunMode::Hindsight => prepare_hindsight_compilation(
                &mut store,
                &id("P1"),
                &id(&source),
                &Compiler,
                request,
            ),
            RunMode::Replay => {
                prepare_eager_compilation(
                    &mut store,
                    &id("P1"),
                    &id(&source),
                    &Compiler,
                    RunRequest {
                        id: "replay-original",
                        mode: RunMode::Live,
                        limits,
                        now_millis: NOW + 1,
                    },
                )
                .unwrap();
                let rebuilt = merl_compiler::rebuild_recorded_context(
                    &store,
                    &id("P1"),
                    "replay-original",
                    limits,
                )
                .unwrap();
                prepare_replay_compilation(
                    &mut store,
                    &id("P1"),
                    "replay-original",
                    rebuilt,
                    &Compiler,
                    request,
                )
            }
        }
        .unwrap()
        .unwrap();
        if let Some(mut response) = response {
            for a in response["assertions"].as_array_mut().unwrap() {
                a["source"] = json!(source);
            }
            if let Some(items) = response.get_mut("unresolved").and_then(Value::as_array_mut) {
                for a in items {
                    a["source"] = json!(source);
                }
            }
            let recorded = record_compilation_result(
                &mut store,
                &id("P1"),
                &prepared,
                Ok(serde_json::to_vec(&response).unwrap()),
                NOW + 2,
            );
            if run != "context" && run != "failed" {
                recorded.expect(run);
            }
        }
    }
    fn response(assertions: &[Value]) -> Value {
        json!({"schema":"merl.compiler-response/v1","assertions":assertions})
    }
    pub fn given_a_compiled_compound_comment() -> Self {
        let mut s = Self::new();
        let mut task = Self::assertion("T1");
        task["predicate"] = json!("task");
        task["span_start"] = json!(16);
        task["span_end"] = json!(33);
        let mut forbidden = Self::assertion("A1");
        forbidden["predicate"] = json!("authority_grant");
        s.store()
            .put_payload(&id("P1"), &id("decision-text"), b"Use fixed gain")
            .unwrap();
        let mut decision = Self::assertion("D1");
        decision["value"] = json!("decision-text");
        let mut response = Self::response(&[decision, Self::assertion("D2"), task, forbidden]);
        response["unresolved"] = json!([{"source":"S1","span_start":34,"span_end":59}]);
        s.compile("compound", Some("alice"), RunMode::Live, Some(response));
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn when_the_run_is_inspected_and_applied(mut self) -> Self {
        self.inspected = Some(self.cli(&["source", "assertions", "--run", "compound"]));
        self.outputs.push(self.cli(&[
            "source", "apply", "--run", "compound", "--actor", "worker", "--id", "apply-1",
        ]));
        self.detail = Some(self.cli(&["show", "D1", "--source", "--history"]));
        self.inbox = Some(self.cli(&["inbox", "poll", "--agent", "reader"]));
        self
    }
    pub fn then_each_assertion_has_its_own_outcome_and_exact_provenance(self) -> Self {
        let result = &self.outputs[0];
        assert_eq!(
            result["schema"], "merl.assertion-application/v1",
            "{result}"
        );
        let outcomes: Vec<_> = result["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["outcome"].as_str().unwrap())
            .collect();
        assert_eq!(outcomes, ["accepted", "accepted", "candidate", "rejected"]);
        assert_eq!(result["revision"], self.basis + 1);
        let inspect = self.inspected.as_ref().unwrap();
        assert_eq!(inspect["schema"], "merl.assertions/v1");
        assert_eq!(inspect["assertions"].as_array().unwrap().len(), 4);
        assert_eq!(inspect["unresolved"][0]["span_start"], 34);
        let store = self.store();
        let project: ProjectId = id("P1");
        assert!(store.object(&project, &id("T1")).unwrap().is_none());
        assert!(store.object(&project, &id("A1")).unwrap().is_none());
        let detail = self.detail.as_ref().unwrap();
        assert_eq!(detail["payload_ref"], "decision-text");
        assert_eq!(detail["policy_origin"]["evaluation"], result["evaluation"]);
        let assertion = &detail["policy_origin"]["input"]["assertion"];
        assert_eq!(assertion["run"], "compound");
        assert_eq!(assertion["index"], 0);
        assert_eq!(assertion["source_version"], "source-compound");
        assert_eq!(assertion["span"], json!({"start":0, "end":13}));
        let inbox = self.inbox.as_ref().unwrap();
        assert_eq!(inbox["entries"].as_array().unwrap().len(), 1, "{inbox}");
        self
    }
    pub fn when_the_application_is_retried_after_restart(mut self) -> Self {
        self.retry = Some(self.cli(&[
            "source", "apply", "--run", "compound", "--actor", "worker", "--id", "apply-1",
        ]));
        self
    }
    pub fn then_the_original_outcome_returns_without_another_batch(self) {
        assert_eq!(self.retry.as_ref().unwrap(), &self.outputs[0]);
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 1
        );
    }
    pub fn given_the_permission_matrix() -> Self {
        let mut s = Self::new();
        for (n, field, value, author, expected) in [
            ("direct", "act", json!("request"), Some("alice"), "accepted"),
            (
                "task",
                "predicate",
                json!("task"),
                Some("alice"),
                "candidate",
            ),
            ("claim", "act", json!("claim"), Some("alice"), "candidate"),
            ("report", "act", json!("report"), Some("alice"), "candidate"),
            (
                "proposal",
                "act",
                json!("propose"),
                Some("alice"),
                "candidate",
            ),
            (
                "negative",
                "polarity",
                json!("negative"),
                Some("alice"),
                "candidate",
            ),
            (
                "inferred",
                "epistemic_basis",
                json!("inferred"),
                Some("alice"),
                "candidate",
            ),
            (
                "quoted",
                "attributed_to",
                json!("admin"),
                Some("alice"),
                "candidate",
            ),
            (
                "relay",
                "attributed_to",
                json!("alice"),
                Some("agent"),
                "candidate",
            ),
            (
                "command",
                "act",
                json!("request"),
                Some("agent"),
                "candidate",
            ),
            ("unknown", "act", json!("request"), None, "candidate"),
            (
                "unmapped",
                "predicate",
                json!("launch_missiles"),
                Some("alice"),
                "rejected",
            ),
            (
                "control",
                "predicate",
                json!("source_compilation_request"),
                Some("alice"),
                "rejected",
            ),
        ] {
            let mut a = Self::assertion(&format!("object-{n}"));
            a[field] = value;
            s.compile(n, author, RunMode::Live, Some(Self::response(&[a])));
            s.runs.push((n.into(), expected.into()));
        }
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn when_each_run_is_applied(mut self) -> Self {
        for (run, _) in &self.runs {
            self.outputs.push(self.cli(&[
                "source", "apply", "--run", run, "--actor", "worker", "--id", run,
            ]));
        }
        self
    }
    pub fn then_only_direct_explicit_decisions_are_accepted(self) {
        for ((run, expected), output) in self.runs.iter().zip(&self.outputs) {
            assert_eq!(
                output["inputs"][0]["outcome"].as_str(),
                Some(expected.as_str()),
                "{run}: {output}"
            );
        }
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 1
        );
    }
    pub fn given_a_decision_without_an_author_grant() -> Self {
        let mut s = Self::new();
        s.compile(
            "ungranted",
            Some("bob"),
            RunMode::Live,
            Some(Self::response(&[Self::assertion("D-bob")])),
        );
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    fn apply(&self, run: &str, request: &str) -> Value {
        self.cli(&[
            "source", "apply", "--run", run, "--actor", "worker", "--id", request,
        ])
    }
    fn revoke(&self, subject: &str, request: &str) {
        let result = self.cli(&[
            "project",
            "authority",
            "revoke",
            "--id",
            request,
            "--actor",
            "admin",
            "--subject",
            subject,
            "--permission",
            "decision_author",
            "--reason",
            "Responsibility changed",
        ]);
        assert_eq!(result["outcome"], "accepted");
    }
    pub fn when_it_is_evaluated_before_and_after_grant_changes(mut self) -> Self {
        self.outputs.push(self.apply("ungranted", "before-grant"));
        self.grant("bob", "decision_author", "grant-bob");
        self.outputs.push(self.apply("ungranted", "before-grant"));
        self.outputs.push(self.apply("ungranted", "after-grant"));
        self.outputs.push(self.apply("ungranted", "fresh-retry"));
        self.revoke("bob", "revoke-bob");
        self.outputs.push(self.apply("ungranted", "after-grant"));
        self.compile(
            "revoked",
            Some("bob"),
            RunMode::Live,
            Some(Self::response(&[Self::assertion("D-revoked")])),
        );
        self.outputs.push(self.apply("revoked", "after-revoke"));
        self.outputs.push(self.apply("revoked", "after-grant"));
        self
    }
    pub fn then_retries_are_stable_and_new_attempts_respect_authority(self) {
        assert_eq!(self.outputs[0]["inputs"][0]["outcome"], "candidate");
        assert_eq!(self.outputs[0], self.outputs[1]);
        assert_eq!(self.outputs[2]["inputs"][0]["outcome"], "accepted");
        assert_eq!(self.outputs[3]["inputs"][0]["outcome"], "duplicate");
        assert!(self.outputs[3]["revision"].is_null());
        assert_eq!(self.outputs[2], self.outputs[4]);
        assert_eq!(self.outputs[5]["inputs"][0]["outcome"], "candidate");
        assert_eq!(self.outputs[6]["code"], "POLICY_INPUT_CONFLICT");
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 3
        );
    }
    pub fn when_authority_is_revoked_before_the_prepared_application_commits(mut self) -> Self {
        let mut store = self.store();
        let prepared = merl_policy::prepare_assertions(
            &store,
            &id("P1"),
            &id("compound"),
            &id("worker"),
            &id("stale"),
            NOW + 10,
        )
        .unwrap()
        .unwrap();
        self.revoke("alice", "revoke-alice");
        self.commit_result = Some(prepared.commit(&mut store));
        self.outputs.push(self.apply("compound", "stale"));
        self
    }
    pub fn then_the_attempt_conflicts_without_partial_semantic_state(self) {
        assert!(matches!(
            self.commit_result,
            Some(Err(merl_store::StoreError::PolicyConflict))
        ));
        let output = &self.outputs[0];
        assert!(output["revision"].is_null(), "{output}");
        assert_eq!(output["inputs"][0]["outcome"], "conflict", "{output}");
        assert!(!output["conflict"].is_null());
        assert!(self.store().object(&id("P1"), &id("D1")).unwrap().is_none());
        assert!(self.store().object(&id("P1"), &id("D2")).unwrap().is_none());
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 1
        );
    }
    pub fn when_the_compiler_context_is_erased_before_application(mut self) -> Self {
        let mut store = self.store();
        let preview = store
            .preview_source_purge(&id("P1"), &id("source-compound"))
            .unwrap();
        store
            .purge_source(
                &id("P1"),
                &id("source-compound"),
                &id("admin"),
                b"Remove retained evidence",
                NOW + 5,
                preview.confirm_digest,
            )
            .unwrap();
        self.inspected = Some(self.cli(&["source", "assertions", "--run", "compound"]));
        self.outputs.push(self.apply("compound", "erased"));
        self
    }
    pub fn then_inspection_reports_unavailable_and_application_accepts_nothing(self) {
        assert_eq!(
            self.inspected.as_ref().unwrap()["response_available"],
            false
        );
        assert!(self.inspected.as_ref().unwrap()["unresolved"].is_null());
        assert!(self.outputs[0]["revision"].is_null());
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
    }
    pub fn given_competing_assertions() -> Self {
        let mut s = Self::new();
        s.compile(
            "competing",
            Some("alice"),
            RunMode::Live,
            Some(Self::response(&[
                Self::assertion("D1"),
                Self::assertion("D1"),
            ])),
        );
        s.runs.push(("competing".into(), "conflict".into()));
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn then_competing_assertions_create_no_accepted_event(self) {
        let output = &self.outputs[0];
        assert_eq!(output["inputs"][0]["outcome"], "conflict", "{output}");
        assert_eq!(output["inputs"][1]["outcome"], "conflict");
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
    }
    pub fn when_evidence_changes_after_preparation(mut self) -> Self {
        let mut store = self.store();
        let prepared = merl_policy::prepare_assertions(
            &store,
            &id("P1"),
            &id("compound"),
            &id("worker"),
            &id("stale-evidence"),
            NOW + 10,
        )
        .unwrap()
        .unwrap();
        Self::capture(
            &mut store,
            "source-compound",
            "edited",
            Some("source-compound"),
            Some("alice"),
        );
        self.commit_result = Some(prepared.commit(&mut store));
        self.outputs.push(self.apply("compound", "stale-evidence"));
        self
    }
    pub fn then_changed_evidence_prevents_the_whole_batch(self) {
        assert!(matches!(
            self.commit_result,
            Some(Err(merl_store::StoreError::PolicyConflict))
        ));
        assert_eq!(
            self.outputs[0]["conflict"]["reason"],
            "assertion_evidence_changed"
        );
        assert!(self.store().object(&id("P1"), &id("D1")).unwrap().is_none());
        assert!(self.store().object(&id("P1"), &id("D2")).unwrap().is_none());
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
    }
    pub fn when_the_application_is_previewed_and_help_is_requested(mut self) -> Self {
        self.outputs.push(self.cli(&[
            "source",
            "apply",
            "--run",
            "compound",
            "--actor",
            "worker",
            "--id",
            "preview",
            "--dry-run",
        ]));
        self.outputs.push(self.cli(&["help", "source", "apply"]));
        self.outputs
            .push(self.cli(&["help", "source", "assertions"]));
        let args = [
            "source",
            "assertions",
            "--run",
            "compound",
            "--project",
            "P1",
            "--database",
            self.database.0.to_str().unwrap(),
        ]
        .map(str::to_owned);
        let merl_cli::CliResponse::Success(human) =
            merl_cli::run_with_clock(&args, &|| Ok(NOW + 10))
        else {
            panic!("human inspection succeeds");
        };
        self.inspected = Some(json!(human));
        self
    }
    pub fn then_the_preview_explains_outcomes_without_committing(self) {
        assert_eq!(self.outputs[0]["schema"], "merl.assertion-preview/v1");
        assert_eq!(self.outputs[0]["inputs"][0]["outcome"], "accepted");
        assert_eq!(self.outputs[0]["inputs"][2]["outcome"], "candidate");
        assert_eq!(self.outputs[1]["schema"], "merl.help/v1");
        assert_eq!(self.outputs[2]["schema"], "merl.help/v1");
        assert!(
            self.outputs[1]["usage"]
                .as_str()
                .unwrap()
                .contains("--dry-run")
        );
        assert!(self.outputs[2]["usage"].as_str().unwrap().contains("--run"));
        assert!(
            self.inspected
                .as_ref()
                .unwrap()
                .as_str()
                .unwrap()
                .contains("D1 decision")
        );
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
    }
    pub fn given_ineligible_runs() -> Self {
        let mut s = Self::new();
        for (run, mode) in [
            ("replay", RunMode::Replay),
            ("eval", RunMode::Eval),
            ("hindsight", RunMode::Hindsight),
            ("pending", RunMode::Live),
            ("failed", RunMode::Live),
            ("context", RunMode::Live),
        ] {
            let mut response = Self::response(&[Self::assertion(&format!("D-{run}"))]);
            if run == "context" {
                response["context_required"] = json!([{"reference":"D0"}]);
            }
            if run == "failed" {
                response["unknown_field"] = json!(true);
            }
            s.compile(
                run,
                Some("alice"),
                mode,
                (run != "pending").then_some(response),
            );
            s.runs.push((run.into(), "ASSERTION_RUN_INELIGIBLE".into()));
        }
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn then_each_run_is_explained_without_accepted_changes(self) {
        for ((run, expected), output) in self.runs.iter().zip(&self.outputs) {
            assert_eq!(
                output["code"].as_str(),
                Some(expected.as_str()),
                "{run}: {output}"
            );
        }
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
    }
}
