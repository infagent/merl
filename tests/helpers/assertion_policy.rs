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
    pub fn when_the_accepted_relation_is_expanded_and_its_source_purge_previewed(mut self) -> Self {
        let store = self.store();
        let relation = store
            .issue_state(&id("P1"), &id("D1"), "issue-1")
            .unwrap()
            .relations[0]
            .relation
            .id
            .clone();
        self.inspected = Some(self.cli(&["show", relation.as_str(), "--source"]));
        let preview = store
            .preview_source_purge(&id("P1"), &id("source-compound"))
            .unwrap();
        self.inbox = Some(
            json!({"relations":preview.relations.iter().map(merl_core::RelationId::as_str).collect::<Vec<_>>(),"expected":relation.as_str()}),
        );
        self
    }
    pub fn then_expansion_and_purge_name_the_exact_relation_origin(self) {
        let expanded = self.inspected.as_ref().unwrap();
        assert_eq!(expanded["schema"], "merl.relation/v1", "{expanded}");
        assert_eq!(
            expanded["policy_origin"]["input"]["review"]["relation"]["run"],
            "compound"
        );
        assert_eq!(
            expanded["policy_origin"]["input"]["review"]["relation"]["subject_basis"]["assertion"]
                ["source_version"],
            "source-compound"
        );
        let preview = self.inbox.as_ref().unwrap();
        assert_eq!(preview["relations"], json!([preview["expected"]]));
    }
    pub fn given_a_decision_and_a_grounded_relation() -> Self {
        let mut s = Self::new();
        let mut store = s.store();
        merl_policy::apply_current(
            &mut store,
            &id("P1"),
            &id("agent"),
            id("seed-eval"),
            id("seed-batch"),
            NOW,
            &[merl_policy::Proposal::Command {
                id: id("seed"),
                event: merl_core::DomainEvent::PutObject {
                    id: id("seed-event"),
                    object: id("D0"),
                    kind: id("decision"),
                    payload: None,
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            }],
        )
        .unwrap();
        drop(store);
        let mut response = Self::response(&[Self::assertion("D1")]);
        response["relations"] = json!([{"subject":"D1","predicate":"supports","object":"D0"}]);
        s.compile("compound", Some("alice"), RunMode::Live, Some(response));
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn given_a_decision_and_invalid_relations() -> Self {
        let mut s = Self::new();
        let mut response = Self::response(&[Self::assertion("D1")]);
        response["relations"] = json!([
            {"subject":"D1","predicate":"supports","object":"missing"},
            {"subject":"invalid endpoint prose","predicate":"supports","object":"D1"}
        ]);
        s.compile("compound", Some("alice"), RunMode::Live, Some(response));
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        s
    }
    pub fn then_only_the_relations_are_rejected(self) {
        let output = &self.outputs[0];
        let outcomes: Vec<_> = output["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["outcome"].as_str().unwrap())
            .collect();
        assert_eq!(outcomes, ["accepted", "rejected", "rejected"]);
        assert_eq!(output["revision"], self.basis + 1);
        assert_eq!(
            self.inspected.as_ref().unwrap()["relations"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    pub fn when_the_relation_is_applied_reviewed_and_rebuilt(mut self) -> Self {
        self = self.when_the_run_is_inspected_and_applied();
        let candidate = self.outputs[0]["inputs"][1]["input"]
            .as_str()
            .unwrap_or("missing-relation")
            .to_owned();
        self.outputs
            .push(self.cli(&["candidate", "show", &candidate]));
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "accept-relation",
        ]));
        self.outputs.push(self.cli(&["project", "rebuild"]));
        self.outputs
            .push(self.cli(&["project", "view", "--focus", "D1"]));
        self.retry = Some(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "accept-relation",
        ]));
        self.detail = Some(self.cli(&["candidate", "show", &candidate]));
        self
    }
    pub fn then_the_relation_retains_its_compiler_basis_and_focused_view(self) {
        assert_eq!(self.outputs[0]["inputs"][0]["outcome"], "accepted");
        assert_eq!(self.outputs[0]["inputs"][1]["outcome"], "candidate");
        assert_eq!(
            self.outputs[2]["outcome"], "accepted",
            "{}",
            self.outputs[2]
        );
        assert_eq!(self.retry.as_ref().unwrap(), &self.outputs[2]);
        let detail = self.detail.as_ref().unwrap();
        assert_eq!(detail["status"], "accepted", "{detail}");
        assert_eq!(detail["relation"]["run"], "compound");
        assert_eq!(detail["relation"]["index"], 0);
        assert_eq!(
            detail["relation"]["subject_basis"]["assertion"]["span"],
            json!({"start":0,"end":13})
        );
        assert_eq!(detail["relation"]["object_basis"]["object"], "D0");
        let objects = self.outputs[4]["objects"]
            .as_array()
            .expect("focused objects");
        assert!(
            objects.iter().any(|o| o["id"] == "D0"),
            "{}",
            self.outputs[4]
        );
    }
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

impl AssertionScenario {
    pub fn given_a_candidate_for_review() -> Self {
        let mut s = Self::new();
        s.compile(
            "ungranted",
            Some("bob"),
            RunMode::Live,
            Some(Self::response(&[Self::assertion("D1")])),
        );
        s.basis = s.store().project_revision(&id("P1")).unwrap().get();
        let result = s.cli(&[
            "source",
            "apply",
            "--run",
            "ungranted",
            "--actor",
            "worker",
            "--id",
            "candidate-origin",
        ]);
        assert_eq!(result["inputs"][0]["outcome"], "candidate", "{result}");
        s.outputs.push(result);
        s
    }
    fn candidate_id(&self) -> String {
        self.outputs[0]["inputs"][0]["input"]
            .as_str()
            .unwrap()
            .into()
    }
    pub fn when_the_candidate_is_inspected_and_accepted(mut self) -> Self {
        let candidate = self.candidate_id();
        self.inspected = Some(self.cli(&["candidate", "list"]));
        self.detail = Some(self.cli(&["candidate", "show", &candidate]));
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
        ]));
        self.retry = Some(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
        ]));
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-2",
        ]));
        self.outputs
            .push(self.cli(&["show", "D1", "--source", "--history"]));
        self.inbox = Some(self.cli(&["inbox", "poll", "--agent", "reader"]));
        self
    }
    pub fn then_review_keeps_provenance_and_accepts_once(self) {
        let listing = self.inspected.as_ref().unwrap();
        assert_eq!(listing["schema"], "merl.candidates/v1", "{listing}");
        assert_eq!(listing["candidates"].as_array().unwrap().len(), 1);
        assert_eq!(
            self.inbox.as_ref().unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let detail = self.detail.as_ref().unwrap();
        assert_eq!(detail["assertion"]["subject"], "D1");
        assert_eq!(detail["dependency_state"], "current");
        assert_eq!(
            self.outputs[1]["outcome"], "accepted",
            "{}",
            self.outputs[1]
        );
        assert_eq!(self.outputs[1], *self.retry.as_ref().unwrap());
        assert_eq!(self.outputs[2]["outcome"], "conflict");
        assert_eq!(
            self.outputs[3]["policy_origin"]["input"]["review"]["candidate"],
            self.outputs[0]["inputs"][0]["input"]
        );
        assert_eq!(
            self.outputs[3]["policy_origin"]["input"]["review"]["assertion"]["subject"],
            "D1"
        );
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 1
        );
    }
    pub fn when_the_candidate_is_rejected_and_its_reason_is_erased(mut self) -> Self {
        let candidate = self.candidate_id();
        self.outputs.push(self.cli(&[
            "candidate",
            "reject",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
            "--reason",
            "The experiment does not measure this effect",
        ]));
        self.detail = Some(self.cli(&["candidate", "show", &candidate]));
        if let Some(reason) = self.detail.as_ref().unwrap()["reviews"][0]["reason_payload"].as_str()
        {
            self.store().erase_payload(&id("P1"), &id(reason)).unwrap();
        }
        self.inspected = Some(self.cli(&["candidate", "show", &candidate]));
        self.outputs.push(self.cli(&[
            "source",
            "apply",
            "--run",
            "ungranted",
            "--actor",
            "worker",
            "--id",
            "after-reject",
        ]));
        self
    }
    pub fn then_rejection_survives_without_retaining_reason_text(self) {
        assert_eq!(
            self.outputs[1]["outcome"], "accepted",
            "{}",
            self.outputs[1]
        );
        assert_eq!(self.detail.as_ref().unwrap()["status"], "rejected");
        let inspected = self.inspected.as_ref().unwrap();
        assert_eq!(inspected["assertion"]["subject"], "D1");
        assert_eq!(inspected["reviews"][0]["reason_available"], false);
        assert!(!inspected.to_string().contains("experiment does not"));
        assert_eq!(self.outputs[2]["inputs"][0]["outcome"], "rejected");
        assert!(self.store().object(&id("P1"), &id("D1")).unwrap().is_none());
    }
    pub fn when_the_candidate_is_corrected(mut self) -> Self {
        let candidate = self.candidate_id();
        self.outputs.push(self.cli(&[
            "candidate",
            "correct",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
            "--subject",
            "Q1",
            "--kind",
            "question",
            "--value",
            "none",
            "--reason",
            "This is an open question",
        ]));
        self.detail = Some(self.cli(&["candidate", "show", &candidate]));
        self.inspected = Some(self.cli(&["show", "Q1", "--source", "--history"]));
        self
    }
    pub fn then_the_accepted_object_expands_to_both_interpretations(self) {
        assert_eq!(
            self.outputs[1]["outcome"], "accepted",
            "{}",
            self.outputs[1]
        );
        let detail = self.detail.as_ref().unwrap();
        assert_eq!(detail["status"], "corrected");
        assert_eq!(detail["assertion"]["subject"], "D1");
        assert_eq!(detail["reviews"][0]["proposal"]["subject"], "Q1");
        let object = self.inspected.as_ref().unwrap();
        assert_eq!(object["kind"], "question");
        assert_eq!(
            object["policy_origin"]["input"]["review"]["assertion"]["subject"],
            "D1"
        );
        assert!(self.store().object(&id("P1"), &id("D1")).unwrap().is_none());
    }
    pub fn when_review_is_attempted_without_permission_then_after_an_edit(mut self) -> Self {
        let candidate = self.candidate_id();
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "alice",
            "--id",
            "unauthorized",
        ]));
        Self::capture(
            &mut self.store(),
            "source-ungranted",
            "edited",
            Some("source-ungranted"),
            Some("bob"),
        );
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "stale",
        ]));
        self.retry = Some(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "stale",
        ]));
        self
    }
    pub fn then_unauthorized_review_is_rejected_and_stale_review_conflicts(self) {
        assert_eq!(
            self.outputs[1]["outcome"], "rejected",
            "{}",
            self.outputs[1]
        );
        assert_eq!(
            self.outputs[2]["outcome"], "conflict",
            "{}",
            self.outputs[2]
        );
        assert_eq!(self.outputs[2], *self.retry.as_ref().unwrap());
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis
        );
        assert!(self.store().object(&id("P1"), &id("D1")).unwrap().is_none());
    }
}

pub struct ReviewCases(Vec<(&'static str, AssertionScenario)>);
impl ReviewCases {
    pub fn given_prepared_review_cases() -> Self {
        Self(
            [
                "grant",
                "edit",
                "purge",
                "value",
                "target",
                "resolved",
                "unrelated",
            ]
            .into_iter()
            .map(|case| (case, AssertionScenario::given_a_candidate_for_review()))
            .collect(),
        )
    }
    pub fn when_dependencies_change_before_commit(mut self) -> Self {
        for (case, s) in &mut self.0 {
            s.store()
                .put_payload(
                    &id("P1"),
                    &id("replacement-value"),
                    b"Reviewed interpretation",
                )
                .unwrap();
            s.store()
                .put_payload(&id("P1"), &id("review-note"), b"Correct the interpretation")
                .unwrap();
            let review = merl_store::CandidateReview {
                id: id("prepared-review"),
                candidate: id(&s.candidate_id()),
                actor: id("agent"),
                action: if *case == "value" {
                    merl_store::ReviewAction::Correct {
                        object: id("D1"),
                        kind: id("decision"),
                        payload: Some(id("replacement-value")),
                    }
                } else {
                    merl_store::ReviewAction::Accept
                },
                reason: if *case == "value" {
                    Some(id("review-note"))
                } else {
                    None
                },
            };
            let prepared =
                merl_policy::prepare_candidate_review(&mut s.store(), &id("P1"), &review, NOW + 10)
                    .unwrap();
            match *case {
                "grant" => {
                    s.cli(&[
                        "project",
                        "authority",
                        "revoke",
                        "--actor",
                        "admin",
                        "--subject",
                        "agent",
                        "--permission",
                        "command_actor",
                        "--id",
                        "revoke",
                        "--reason",
                        "Responsibility changed",
                    ]);
                }
                "edit" => AssertionScenario::capture(
                    &mut s.store(),
                    "source-ungranted",
                    "edited",
                    Some("source-ungranted"),
                    Some("bob"),
                ),
                "purge" => {
                    let payload = s
                        .store()
                        .source_version(&id("P1"), &id("source-ungranted"))
                        .unwrap()
                        .unwrap()
                        .payload
                        .unwrap();
                    s.store().erase_payload(&id("P1"), &payload).unwrap();
                }
                "value" => {
                    s.store()
                        .erase_payload(&id("P1"), &id("replacement-value"))
                        .unwrap();
                }
                "target" => s.update_object("D1", "decision"),
                "resolved" => {
                    s.cli(&[
                        "candidate",
                        "accept",
                        &s.candidate_id(),
                        "--actor",
                        "agent",
                        "--id",
                        "other-review",
                    ]);
                }
                "unrelated" => s.update_object("unrelated", "task"),
                _ => unreachable!(),
            }
            s.commit_result = Some(prepared.commit(&mut s.store()));
            let record = s
                .store()
                .policy_evaluation(&id("P1"), &prepared.evaluation.id)
                .unwrap()
                .unwrap();
            s.outputs.push(json!({"outcome":record.inputs[0].disposition.as_str(),"conflict":record.conflict.map(|c|c.reason_code)}));
        }
        self
    }
    pub fn then_each_changed_dependency_records_a_conflict(self) {
        for (case, s) in self.0 {
            if case == "unrelated" {
                assert!(s.commit_result.as_ref().unwrap().is_ok());
                assert_eq!(s.outputs[1]["outcome"], "accepted");
            } else {
                assert!(
                    matches!(
                        s.commit_result,
                        Some(Err(merl_store::StoreError::PolicyConflict))
                    ),
                    "{case}: {:?}",
                    s.commit_result
                );
                assert_eq!(
                    s.outputs[1]["outcome"], "conflict",
                    "{case}: {}",
                    s.outputs[1]
                );
                assert!(!s.outputs[1]["conflict"].is_null());
                if !matches!(case, "target" | "resolved") {
                    assert!(s.store().object(&id("P1"), &id("D1")).unwrap().is_none());
                }
            }
        }
    }
}
impl AssertionScenario {
    fn update_object(&self, object: &str, kind: &str) {
        merl_policy::apply_current(
            &mut self.store(),
            &id("P1"),
            &id("agent"),
            id("update-eval"),
            id("update-batch"),
            NOW + 12,
            &[merl_policy::Proposal::Command {
                id: id("update-command"),
                event: merl_core::DomainEvent::PutObject {
                    id: id("update-event"),
                    object: id(object),
                    kind: id(kind),
                    payload: None,
                    issue_scope: Some("issue-1".into()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            }],
        )
        .unwrap();
    }
    pub fn when_the_candidate_is_corrected_then_the_object_changes(self) -> Self {
        let mut s = self.when_the_candidate_is_corrected();
        s.update_object("Q1", "question");
        s.inspected = Some(s.cli(&["show", "Q1", "--source", "--history"]));
        s
    }
    pub fn then_history_retains_the_correction_and_original_evidence(self) {
        let object = self.inspected.unwrap();
        assert_eq!(object["history"].as_array().unwrap().len(), 2);
        let evidence = &object["evidence_history"][0];
        assert_eq!(evidence["assertion"]["subject"], "D1", "{object}");
        assert_eq!(evidence["review"]["proposal"]["subject"], "Q1");
        assert_eq!(evidence["assertion"]["evidence"]["status"], "available");
    }
}

impl AssertionScenario {
    pub fn when_review_is_previewed_then_retried_after_revocation_and_rebuild(mut self) -> Self {
        let candidate = self.candidate_id();
        self.outputs.push(self.cli(&[
            "candidate",
            "correct",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "preview",
            "--subject",
            "Q1",
            "--kind",
            "question",
            "--value",
            "none",
            "--reason",
            "Open question",
            "--dry-run",
        ]));
        self.detail = Some(self.cli(&["candidate", "show", &candidate]));
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
        ]));
        self.cli(&[
            "project",
            "authority",
            "revoke",
            "--actor",
            "admin",
            "--subject",
            "agent",
            "--permission",
            "command_actor",
            "--id",
            "revoke",
            "--reason",
            "Responsibility changed",
        ]);
        self.cli(&["project", "rebuild"]);
        self.outputs.push(self.cli(&[
            "candidate",
            "accept",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
        ]));
        self.outputs.push(self.cli(&[
            "candidate",
            "reject",
            &candidate,
            "--actor",
            "agent",
            "--id",
            "review-1",
            "--reason",
            "Different intent",
        ]));
        self.inspected = Some(self.cli(&["project", "view"]));
        self.retry = Some(self.cli(&["help", "candidate", "correct"]));
        let args = [
            "candidate",
            "show",
            &candidate,
            "--project",
            "P1",
            "--database",
            self.database.0.to_str().unwrap(),
        ]
        .map(str::to_owned);
        self.outputs
            .push(match merl_cli::run_with_clock(&args, &|| Ok(NOW)) {
                merl_cli::CliResponse::Success(text) => json!(text),
                other => panic!("human inspection failed: {other:?}"),
            });
        self
    }
    pub fn then_preview_is_read_only_and_retry_preserves_the_original_outcome(self) {
        assert_eq!(
            self.outputs[1]["schema"],
            "merl.candidate-review-preview/v1"
        );
        assert_eq!(self.outputs[1]["outcome"], "accepted");
        assert_eq!(self.detail.as_ref().unwrap()["reviews"], json!([]));
        assert_eq!(self.outputs[2], self.outputs[3]);
        assert_eq!(self.outputs[4]["code"], "POLICY_INPUT_CONFLICT");
        assert_eq!(
            self.store().project_revision(&id("P1")).unwrap().get(),
            self.basis + 2
        );
        assert!(
            self.store()
                .candidate_review(&id("P1"), &id("preview"))
                .unwrap()
                .is_none()
        );
        assert!(self.store().object(&id("P1"), &id("Q1")).unwrap().is_none());
        assert_eq!(self.retry.as_ref().unwrap()["schema"], "merl.help/v1");
        let human = self.outputs[5].as_str().unwrap();
        assert!(
            human.contains("D1") && human.contains("source-ungranted") && human.contains("agent")
        );
        assert!(human.contains("accepted"));
    }
}

pub struct ReviewRecoveryCases {
    cases: Vec<(
        &'static str,
        &'static str,
        AssertionScenario,
        merl_core::PayloadId,
    )>,
}
impl ReviewRecoveryCases {
    fn arguments<'a>(operation: &'a str, candidate: &'a str) -> Vec<&'a str> {
        let mut args = vec![
            "candidate",
            operation,
            candidate,
            "--actor",
            "agent",
            "--id",
            "interrupted-review",
            "--reason",
            "The author left this question open",
        ];
        if operation == "correct" {
            args.extend(["--subject", "Q1", "--kind", "question", "--value", "none"]);
        }
        args
    }
    pub fn given_review_reasons_without_requests() -> Self {
        let mut cases = Vec::new();
        for (operation, state) in [
            ("reject", "matching"),
            ("correct", "matching"),
            ("reject", "different"),
            ("reject", "erased"),
        ] {
            // Obtain the reason reference through public inspection in a separate
            // database, so this crash fixture does not duplicate the ID algorithm.
            let reference = AssertionScenario::given_a_candidate_for_review();
            let candidate = reference.candidate_id();
            let completed = reference.cli(&Self::arguments(operation, &candidate));
            assert_eq!(completed["outcome"], "accepted");
            let detail = reference.cli(&["candidate", "show", &candidate]);
            let payload = id(detail["reviews"][0]["reason_payload"].as_str().unwrap());
            let scenario = AssertionScenario::given_a_candidate_for_review();
            let bytes: &[u8] = if state == "different" {
                b"Different retained reason"
            } else {
                b"The author left this question open"
            };
            {
                let mut store = scenario.store();
                store.put_payload(&id("P1"), &payload, bytes).unwrap();
                if state == "erased" {
                    store.erase_payload(&id("P1"), &payload).unwrap();
                }
                assert!(
                    store
                        .candidate_review(&id("P1"), &id("interrupted-review"))
                        .unwrap()
                        .is_none()
                );
            }
            cases.push((operation, state, scenario, payload));
        }
        Self { cases }
    }
    pub fn when_the_same_commands_are_retried_after_restart(mut self) -> Self {
        for (operation, _, scenario, _) in &mut self.cases {
            let candidate = scenario.candidate_id();
            let args = Self::arguments(operation, &candidate);
            scenario.outputs.push(scenario.cli(&args));
            scenario.outputs.push(scenario.cli(&args));
        }
        self
    }
    pub fn then_matching_reasons_recover_and_conflicting_or_erased_reasons_stay_unchanged(self) {
        for (operation, state, scenario, payload) in self.cases {
            let result = &scenario.outputs[1];
            assert_eq!(result, &scenario.outputs[2]);
            let store = scenario.store();
            if state == "matching" {
                assert_eq!(result["outcome"], "accepted", "{operation}: {result}");
                assert_eq!(
                    store.project_revision(&id("P1")).unwrap().get(),
                    scenario.basis + 1
                );
                assert!(
                    store
                        .candidate_review(&id("P1"), &id("interrupted-review"))
                        .unwrap()
                        .is_some()
                );
                assert_eq!(
                    store.read_payload(&id("P1"), &payload).unwrap(),
                    merl_store::PayloadRead::Available(
                        b"The author left this question open".to_vec()
                    )
                );
            } else {
                assert_eq!(result["code"], "POLICY_INPUT_CONFLICT", "{state}: {result}");
                assert_eq!(
                    store.project_revision(&id("P1")).unwrap().get(),
                    scenario.basis
                );
                assert!(
                    store
                        .candidate_review(&id("P1"), &id("interrupted-review"))
                        .unwrap()
                        .is_none()
                );
                let expected = if state == "erased" {
                    merl_store::PayloadRead::Unavailable
                } else {
                    merl_store::PayloadRead::Available(b"Different retained reason".to_vec())
                };
                assert_eq!(store.read_payload(&id("P1"), &payload).unwrap(), expected);
            }
        }
    }
}

pub struct RelationReviewCases {
    cases: Vec<(AssertionScenario, String)>,
    outcomes: Vec<(String, String, usize)>,
}
impl RelationReviewCases {
    pub fn given_relation_only_candidates() -> Self {
        let cases = ["unchanged", "unauthorized", "endpoint", "evidence", "revocation", "rejection"]
            .into_iter().map(|change| {
                let mut s=AssertionScenario::given_a_decision_and_a_grounded_relation().when_the_run_is_inspected_and_applied();
                let response=json!({"schema":"merl.compiler-response/v1","assertions":[],"relations":[{"subject":"D0","predicate":"answers","object":"D1"}]});
                s.compile("edges-only",Some("alice"),RunMode::Live,Some(response));
                s.outputs.push(s.cli(&["source","apply","--run","edges-only","--actor","worker","--id","apply-edges"]));
                (s,change.to_owned())
            }).collect();
        Self {
            cases,
            outcomes: Vec::new(),
        }
    }
    pub fn when_reviews_run_across_dependency_changes(mut self) -> Self {
        for (s, change) in &self.cases {
            let candidate = s.outputs.last().unwrap()["inputs"][0]["input"]
                .as_str()
                .expect("relation-only candidate");
            let review = merl_store::CandidateReview {
                id: id("guarded-review"),
                candidate: id(candidate),
                actor: id(if change == "unauthorized" {
                    "outsider"
                } else {
                    "agent"
                }),
                action: merl_store::ReviewAction::Accept,
                reason: None,
            };
            let mut store = s.store();
            let prepared =
                merl_policy::prepare_candidate_review(&mut store, &id("P1"), &review, NOW + 10)
                    .unwrap();
            change_relation_review_dependency(s, &mut store, candidate, change);
            match prepared.commit(&mut store) {
                Ok(_) | Err(merl_store::StoreError::PolicyConflict) => {}
                Err(e) => panic!("{change}: {e}"),
            }
            let result = store
                .policy_evaluation(&id("P1"), &prepared.evaluation.id)
                .unwrap()
                .unwrap();
            let edges = store
                .issue_state(&id("P1"), &id("D1"), "issue-1")
                .unwrap()
                .relations
                .len();
            self.outcomes.push((
                change.clone(),
                result.inputs[0].disposition.as_str().into(),
                edges,
            ));
            if change == "unchanged" {
                let detail = s.cli(&["candidate", "show", candidate]);
                assert_eq!(
                    detail["reviews"][0]["relation"]["run"], "edges-only",
                    "{detail}"
                );
                let reapplied = s.cli(&[
                    "source",
                    "apply",
                    "--run",
                    "edges-only",
                    "--actor",
                    "worker",
                    "--id",
                    "apply-again",
                ]);
                assert_eq!(reapplied["inputs"][0]["outcome"], "duplicate");
            }
        }
        self
    }
    pub fn then_only_current_authorized_reviews_accept_edges(self) {
        for (change, outcome, edges) in self.outcomes {
            assert_eq!(
                outcome,
                match change.as_str() {
                    "unchanged" => "accepted",
                    "unauthorized" => "rejected",
                    _ => "conflict",
                },
                "{change}"
            );
            assert_eq!(edges, usize::from(change == "unchanged"), "{change}");
        }
    }
}

fn change_relation_review_dependency(
    s: &AssertionScenario,
    store: &mut Store,
    candidate: &str,
    change: &str,
) {
    match change {
        "endpoint" => {
            merl_policy::apply_current(
                store,
                &id("P1"),
                &id("agent"),
                id("change-eval"),
                id("change-batch"),
                NOW + 11,
                &[merl_policy::Proposal::Command {
                    id: id("change"),
                    event: merl_core::DomainEvent::PutObject {
                        id: id("change-event"),
                        object: id("D0"),
                        kind: id("decision"),
                        payload: None,
                        issue_scope: None,
                        lifecycle: merl_core::ObjectLifecycle::Superseded,
                    },
                }],
            )
            .unwrap();
        }
        "evidence" => {
            store
                .erase_payload(&id("P1"), &id("ctx_edges-only"))
                .unwrap();
        }
        "revocation" => {
            let result = s.cli(&[
                "project",
                "authority",
                "revoke",
                "--id",
                "revoke-reviewer",
                "--actor",
                "admin",
                "--subject",
                "agent",
                "--permission",
                "command_actor",
                "--reason",
                "End responsibility",
            ]);
            assert_eq!(result["outcome"], "accepted");
        }
        "rejection" => {
            let result = s.cli(&[
                "candidate",
                "reject",
                candidate,
                "--actor",
                "agent",
                "--id",
                "reject-edge",
                "--reason",
                "The evidence does not answer this question",
            ]);
            assert_eq!(result["outcome"], "accepted");
        }
        _ => {}
    }
}
