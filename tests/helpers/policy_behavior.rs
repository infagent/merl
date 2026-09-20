use merl_compiler::{
    CompileError, CompilerAdapter, CompilerLimits, RunMode, RunRequest, execute_compilation,
    prepare_compilation, record_compilation_result,
};
use merl_core::{
    CompilationMode, CoverageRequirement, DomainEvent, DomainEventBatch, PayloadId,
    PolicyDisposition, ProjectId, ProjectRevision, ProviderIssueState, ProviderObservation,
};
use merl_policy::{PolicyRules, PreparedPolicy, Proposal, evaluate};
use merl_store::{SourceBinding, SourceCapture, Store, StoreError};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const NOW: i64 = 1_700_000_000_000;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TempStoreFile(PathBuf);

impl TempStoreFile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "merl-policy-test-{}-{}.sqlite",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("unique test database");
        Self(path)
    }
}

impl Drop for TempStoreFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn id<T: for<'a> TryFrom<&'a str>>(value: &str) -> T
where
    for<'a> <T as TryFrom<&'a str>>::Error: std::fmt::Debug,
{
    T::try_from(value).expect("valid structural ID")
}

fn event(id_value: &str, object: &str, kind: &str, payload: Option<PayloadId>) -> DomainEvent {
    DomainEvent::PutObject {
        id: id(id_value),
        object: id(object),
        kind: id(kind),
        payload,
    }
}

fn binding() -> SourceBinding {
    SourceBinding {
        id: id("github-binding"),
        provider: id("github"),
        provider_namespace_id: "repo-1".into(),
        namespace_digest: Sha256::digest(b"repo-1").into(),
    }
}

fn limits() -> CompilerLimits {
    CompilerLimits {
        input_bytes: 4096,
        output_bytes: 4096,
        output_tokens: 512,
        assertions: 4,
        context_requests: 1,
        expansion_rounds: 1,
        payload_bytes: 2048,
        source_window: 2,
        objects: 4,
    }
}

struct StatementCompiler {
    version: &'static str,
    body_len: usize,
    attributed_to: Option<&'static str>,
}

impl CompilerAdapter for StatementCompiler {
    fn id(&self) -> &'static str {
        "policy-test"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn model(&self) -> &'static str {
        "deterministic"
    }
    fn prompt_digest(&self) -> [u8; 32] {
        Sha256::digest(b"policy-test").into()
    }
    fn compile(&self, _context: &[u8], _limits: CompilerLimits) -> Result<Vec<u8>, CompileError> {
        Ok(format!(
            r#"{{"schema":"merl.compiler-response/v1","assertions":[{{"source":"{}","span_start":0,"span_end":{},"subject":"D1","predicate":"decision","value":"none","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":{}}}]}}"#,
            self.version,
            self.body_len,
            self.attributed_to.map_or("null".to_owned(), |value| format!("\"{value}\""))
        ).into_bytes())
    }
}

pub struct PolicyScenario {
    store: Store,
    project: ProjectId,
    rules: PolicyRules,
    prepared: Option<PreparedPolicy>,
    result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
    stale_result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
    retry_disposition: Option<PolicyDisposition>,
    persisted_file: Option<TempStoreFile>,
    invalid_result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
}

impl PolicyScenario {
    fn new() -> Self {
        let mut store = Store::open_in_memory().expect("store");
        let project = id("policy-project");
        store.create_project(&project).expect("project");
        Self {
            store,
            project,
            rules: PolicyRules {
                version: id("policy-v1"),
                decision_authors: vec![id("alice")],
                command_actors: vec![id("alice")],
                administrators: vec![id("admin")],
            },
            prepared: None,
            result: None,
            stale_result: None,
            retry_disposition: None,
            persisted_file: None,
            invalid_result: None,
        }
    }

    fn file_backed() -> Self {
        let file = TempStoreFile::new();
        let mut scenario = Self::new();
        let mut store = Store::open(&file.0).expect("file-backed authority");
        store.create_project(&scenario.project).expect("project");
        scenario.store = store;
        scenario.persisted_file = Some(file);
        scenario
    }

    fn capture(&mut self, version: &'static str, body: &'static str, author: &str) {
        self.store
            .capture_source_version(
                &self.project,
                &SourceCapture {
                    binding: binding(),
                    source: id(version),
                    provider_entity_id: version,
                    context_scope_id: version,
                    version: id(version),
                    provider_version_id: version,
                    kind: id("issue_comment"),
                    supersedes: None,
                    ambiguous_order_with_previous: false,
                    created_at_millis: NOW,
                    occurred_at_millis: NOW,
                    upstream_updated_at_millis: Some(NOW),
                    observed_at_millis: NOW,
                    actor: Some(id(author)),
                    provider_actor_id: Some(author),
                    source_author: Some(id(author)),
                    provider_source_author_id: Some(author),
                    body: Some(body.as_bytes()),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::Eager,
                    coverage_requirement: CoverageRequirement::Required,
                    policy_version: id("capture-v1"),
                },
            )
            .expect("source capture");
    }

    fn compile(
        &mut self,
        version: &'static str,
        body_len: usize,
        attributed_to: Option<&'static str>,
        run_id: &'static str,
    ) {
        let compiler = StatementCompiler {
            version,
            body_len,
            attributed_to,
        };
        let prepared = prepare_compilation(
            &mut self.store,
            &self.project,
            &id(version),
            &compiler,
            RunRequest {
                id: run_id,
                limits: limits(),
                mode: RunMode::Live,
                now_millis: NOW + 1,
            },
        )
        .expect("prepare compilation")
        .expect("new work");
        let response = execute_compilation(&prepared, &compiler);
        record_compilation_result(&mut self.store, &self.project, &prepared, response, NOW + 2)
            .expect("record compilation");
    }

    pub fn given_an_authorized_human_and_an_agent_relay() -> Self {
        let mut scenario = Self::new();
        let direct = "Use fixed gain";
        let relay = "Alice said use fixed gain";
        scenario.capture("direct-v1", direct, "alice");
        scenario.capture("relay-v1", relay, "agent");
        scenario.compile("direct-v1", direct.len(), None, "direct-run");
        scenario.compile("relay-v1", relay.len(), Some("alice"), "relay-run");
        scenario
    }

    pub fn when_both_propose_the_same_decision(&mut self) -> &mut Self {
        let proposals = [
            Proposal::ObservedAssertion {
                id: id("direct-assertion"),
                run: id("direct-run"),
                index: 0,
                event: event("direct-event", "D1", "decision", None),
            },
            Proposal::ObservedAssertion {
                id: id("relay-assertion"),
                run: id("relay-run"),
                index: 0,
                event: event("relay-event", "D1", "decision", None),
            },
        ];
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("authority"),
            id("policy-eval"),
            id("policy-batch"),
            NOW + 3,
            &self.rules,
            &proposals,
        )
        .expect("evaluate");
        self.result = Some(prepared.commit(&mut self.store));
        self.prepared = Some(prepared);
        self
    }

    pub fn then_only_the_direct_human_assertion_is_accepted(&mut self) -> &mut Self {
        assert_eq!(
            self.result
                .as_ref()
                .expect("result")
                .as_ref()
                .expect("commit")
                .map(ProjectRevision::get),
            Some(1)
        );
        let evaluation = self.prepared.as_ref().expect("evaluation");
        assert_eq!(
            evaluation.evaluation.inputs[0].disposition,
            PolicyDisposition::Accepted
        );
        assert_eq!(
            evaluation.evaluation.inputs[1].disposition,
            PolicyDisposition::Candidate
        );
        assert!(
            self.store
                .object(&self.project, &id("D1"))
                .expect("object")
                .is_some()
        );
        self
    }

    pub fn then_each_disposition_has_a_provenance_path(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .object_policy_evaluation(&self.project, &id("D1"))
                .expect("provenance"),
            Some(id("policy-eval"))
        );
        let recorded = self
            .store
            .policy_evaluation(&self.project, &id("policy-eval"))
            .expect("evaluation")
            .expect("exists");
        assert_eq!(recorded.inputs.len(), 2);
        assert_eq!(recorded.inputs[0].disposition, PolicyDisposition::Accepted);
        assert_eq!(recorded.inputs[1].disposition, PolicyDisposition::Candidate);
        assert_eq!(recorded.version.as_str(), "policy-v1");
        assert_eq!(
            recorded.configuration_digest,
            self.rules.configuration_digest()
        );
        assert_eq!(recorded.basis_project_revision.get(), 0);
        assert_eq!(recorded.reads.len(), 1);
        assert_eq!(recorded.writes.len(), 1);
        assert_eq!(recorded.events.len(), 1);
        assert!(matches!(
            recorded.inputs[0].input,
            merl_core::PolicyInput::ObservedAssertion { .. }
        ));
        self
    }

    pub fn when_the_original_assertion_is_replayed_under_a_new_handle(&mut self) -> &mut Self {
        let retry = evaluate(
            &self.store,
            &self.project,
            &id("authority"),
            id("replayed-eval"),
            id("replayed-batch"),
            NOW + 4,
            &self.rules,
            &[Proposal::ObservedAssertion {
                id: id("new-assertion-handle"),
                run: id("direct-run"),
                index: 0,
                event: event("replayed-event", "D1", "decision", None),
            }],
        )
        .expect("reevaluate assertion");
        self.retry_disposition = Some(retry.evaluation.inputs[0].disposition);
        self.stale_result = Some(retry.commit(&mut self.store));
        self
    }

    pub fn then_no_second_decision_is_created(&mut self) {
        assert_eq!(self.retry_disposition, Some(PolicyDisposition::Duplicate));
        assert_eq!(
            self.stale_result
                .as_ref()
                .expect("retry")
                .as_ref()
                .expect("commit"),
            &None
        );
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
    }

    pub fn given_a_prepared_decision_with_a_read_dependency() -> Self {
        let mut scenario = Self::new();
        scenario.seed("initial", "D1", "decision");
        scenario.prepared =
            Some(scenario.command("update-1", "eval-1", "batch-1", "event-1", "D1"));
        scenario
    }

    fn seed(&mut self, batch: &str, object: &str, kind: &str) {
        self.store
            .commit(&DomainEventBatch {
                id: id(batch),
                project: self.project.clone(),
                actor: id("alice"),
                occurred_at_millis: NOW,
                events: vec![event(&format!("event-{batch}"), object, kind, None)],
            })
            .expect("seed accepted state");
    }

    fn command(
        &self,
        input: &str,
        evaluation: &str,
        batch: &str,
        event_id: &str,
        object: &str,
    ) -> PreparedPolicy {
        evaluate(
            &self.store,
            &self.project,
            &id("alice"),
            id(evaluation),
            id(batch),
            NOW + 1,
            &self.rules,
            &[Proposal::Command {
                id: id(input),
                event: event(event_id, object, "decision", None),
            }],
        )
        .expect("evaluate command")
    }

    pub fn when_an_unrelated_object_changes(&mut self) -> &mut Self {
        self.seed("unrelated", "X1", "fact");
        let prepared = self.prepared.take().expect("prepared decision");
        self.result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_the_decision_can_commit(&mut self) -> &mut Self {
        assert_eq!(
            self.result
                .as_ref()
                .expect("result")
                .as_ref()
                .expect("commit")
                .map(ProjectRevision::get),
            Some(3)
        );
        assert_eq!(
            self.store
                .object(&self.project, &id("D1"))
                .expect("object")
                .expect("decision")
                .revision
                .get(),
            2
        );
        self
    }

    pub fn when_the_command_is_retried_with_the_same_meaning(&mut self) -> &mut Self {
        let retry = self.command(
            "update-1",
            "retry-command-eval",
            "retry-command-batch",
            "retry-command-event",
            "D1",
        );
        self.retry_disposition = Some(retry.evaluation.inputs[0].disposition);
        self.stale_result = Some(retry.commit(&mut self.store));
        self
    }

    pub fn then_the_retry_creates_no_new_revision(&mut self) -> &mut Self {
        assert_eq!(self.retry_disposition, Some(PolicyDisposition::Duplicate));
        assert_eq!(
            self.stale_result
                .as_ref()
                .expect("retry")
                .as_ref()
                .expect("recorded"),
            &None
        );
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            3
        );
        self
    }

    pub fn when_the_read_dependency_changes_before_another_commit(&mut self) -> &mut Self {
        let prepared = self.command("update-2", "eval-2", "batch-2", "event-2", "D1");
        self.seed("competing", "D1", "decision");
        self.stale_result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_the_stale_decision_is_rejected_without_partial_state(&mut self) {
        assert!(matches!(
            self.stale_result,
            Some(Err(StoreError::PolicyConflict))
        ));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            4
        );
        assert_eq!(
            self.store
                .accepted_event_count(&self.project)
                .expect("events"),
            4
        );
        assert!(
            self.store
                .policy_evaluation(&self.project, &id("eval-2"))
                .expect("evaluation lookup")
                .is_none()
        );
    }

    pub fn given_a_subscriber_and_a_trusted_issue_observation() -> Self {
        let mut scenario = Self::file_backed();
        scenario.capture("issue-v1", "Issue body", "alice");
        scenario
            .store
            .subscribe_all(&scenario.project, &id("engineer"))
            .expect("subscribe");
        let payload = id("snapshot-1");
        scenario
            .store
            .put_payload(&scenario.project, &payload, b"Issue closed")
            .expect("snapshot");
        let observation = ProviderObservation {
            id: id("provider-input"),
            binding: id("github-binding"),
            issue: id("I1"),
            state: ProviderIssueState::Closed,
            upstream_updated_at_millis: Some(NOW),
            closed_at_millis: Some(NOW),
            label_provider_ids: Some(vec!["label-1".into()]),
            assignee_provider_ids: Some(vec!["actor-1".into()]),
            snapshot_payload: payload.clone(),
            observed_at_millis: NOW,
        };
        scenario.prepared = Some(
            evaluate(
                &scenario.store,
                &scenario.project,
                &id("provider-worker"),
                id("provider-eval"),
                id("provider-batch"),
                NOW,
                &scenario.rules,
                &[Proposal::ProviderObservation {
                    observation,
                    event: event("provider-event", "I1", "provider_issue", Some(payload)),
                }],
            )
            .expect("deterministic provider evaluation"),
        );
        scenario
    }

    pub fn when_the_provider_observation_is_accepted_twice(&mut self) -> &mut Self {
        let prepared = self.prepared.as_ref().expect("prepared observation");
        self.result = Some(prepared.commit(&mut self.store));
        let retry = evaluate(
            &self.store,
            &self.project,
            &id("provider-worker"),
            id("retry-eval"),
            id("retry-batch"),
            NOW + 1,
            &self.rules,
            &[Proposal::ProviderObservation {
                observation: prepared.provider.as_ref().expect("provider input").clone(),
                event: prepared
                    .evaluation
                    .batch
                    .as_ref()
                    .expect("accepted batch")
                    .events[0]
                    .clone(),
            }],
        )
        .expect("reevaluate same provider input");
        self.retry_disposition = Some(retry.evaluation.inputs[0].disposition);
        self.stale_result = Some(retry.commit(&mut self.store));
        self
    }

    pub fn when_a_batch_references_an_unavailable_payload(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("alice"),
            id("invalid-eval"),
            id("invalid-batch"),
            NOW,
            &self.rules,
            &[Proposal::Command {
                id: id("invalid-command"),
                event: event(
                    "invalid-event",
                    "D9",
                    "decision",
                    Some(id("missing-payload")),
                ),
            }],
        )
        .expect("evaluate invalid proposal");
        self.invalid_result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_neither_state_nor_inbox_exposes_that_batch(&mut self) -> &mut Self {
        assert!(matches!(
            self.invalid_result,
            Some(Err(StoreError::InvalidBatch))
        ));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            0
        );
        assert!(
            self.store
                .object(&self.project, &id("D9"))
                .expect("projection")
                .is_none()
        );
        assert!(
            self.store
                .inbox_after(&self.project, &id("engineer"), 0.into())
                .expect("inbox")
                .is_empty()
        );
        assert!(
            self.store
                .policy_evaluation(&self.project, &id("invalid-eval"))
                .expect("evaluation lookup")
                .is_none()
        );
        self
    }

    pub fn then_one_revision_and_one_inbox_entry_exist(&mut self) -> &mut Self {
        assert_eq!(
            self.result
                .as_ref()
                .expect("first")
                .as_ref()
                .expect("commit")
                .map(ProjectRevision::get),
            Some(1)
        );
        assert_eq!(
            self.stale_result
                .as_ref()
                .expect("retry")
                .as_ref()
                .expect("retry commit"),
            &None
        );
        assert_eq!(self.retry_disposition, Some(PolicyDisposition::Duplicate));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        let inbox = self
            .store
            .inbox_after(&self.project, &id("engineer"), 0.into())
            .expect("inbox");
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].revision.get(), 1);
        let reopened = Store::open(&self.persisted_file.as_ref().expect("database file").0)
            .expect("reopen authority store");
        assert_eq!(
            reopened
                .project_revision(&self.project)
                .expect("durable revision")
                .get(),
            1
        );
        assert_eq!(
            reopened
                .inbox_after(&self.project, &id("engineer"), 0.into())
                .expect("durable inbox"),
            inbox
        );
        self
    }

    pub fn then_the_provider_fact_has_a_policy_provenance_path(&mut self) {
        let head = self
            .store
            .provider_issue_head(&self.project, &id("I1"))
            .expect("provider mirror")
            .expect("issue fact");
        assert_eq!(head.input.state, ProviderIssueState::Closed);
        assert_eq!(
            self.store
                .object_policy_evaluation(&self.project, &id("I1"))
                .expect("provenance"),
            Some(id("provider-eval"))
        );
    }

    pub fn given_an_agent_without_decision_authority() -> Self {
        Self::new()
    }

    pub fn when_the_agent_requests_a_decision(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("agent"),
            id("denied-eval"),
            id("denied-batch"),
            NOW,
            &self.rules,
            &[Proposal::Command {
                id: id("same-command"),
                event: event("denied-event", "D1", "decision", None),
            }],
        )
        .expect("evaluate request");
        self.result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_the_request_is_rejected_without_a_project_change(&mut self) -> &mut Self {
        assert_eq!(
            self.result
                .as_ref()
                .expect("outcome")
                .as_ref()
                .expect("recorded"),
            &None
        );
        let record = self
            .store
            .policy_evaluation(&self.project, &id("denied-eval"))
            .expect("policy record")
            .expect("evaluation");
        assert_eq!(record.inputs[0].disposition, PolicyDisposition::Rejected);
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            0
        );
        self
    }

    pub fn when_the_agent_reuses_the_command_id_for_another_decision(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("agent"),
            id("changed-eval"),
            id("changed-batch"),
            NOW + 1,
            &self.rules,
            &[Proposal::Command {
                id: id("same-command"),
                event: event("changed-event", "D2", "decision", None),
            }],
        )
        .expect("evaluate changed request");
        self.stale_result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_the_second_request_conflicts_without_a_project_change(&mut self) -> &mut Self {
        assert!(matches!(
            self.stale_result,
            Some(Err(StoreError::PolicyInputConflict))
        ));
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            0
        );
        assert!(
            self.store
                .policy_evaluation(&self.project, &id("changed-eval"))
                .expect("policy lookup")
                .is_none()
        );
        self
    }

    pub fn when_an_administrator_performs_a_maintenance_action(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("admin"),
            id("admin-eval"),
            id("admin-batch"),
            NOW + 2,
            &self.rules,
            &[Proposal::AdministrativeAction {
                id: id("maintenance-1"),
                event: event("admin-event", "D3", "decision", None),
            }],
        )
        .expect("evaluate maintenance action");
        self.stale_result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_only_the_administrators_action_changes_accepted_state(&mut self) {
        assert_eq!(
            self.stale_result
                .as_ref()
                .expect("outcome")
                .as_ref()
                .expect("commit")
                .map(ProjectRevision::get),
            Some(1)
        );
        assert!(
            self.store
                .object(&self.project, &id("D1"))
                .expect("first decision")
                .is_none()
        );
        assert!(
            self.store
                .object(&self.project, &id("D2"))
                .expect("second decision")
                .is_none()
        );
        assert!(
            self.store
                .object(&self.project, &id("D3"))
                .expect("maintenance result")
                .is_some()
        );
        let record = self
            .store
            .policy_evaluation(&self.project, &id("admin-eval"))
            .expect("policy record")
            .expect("accepted evaluation");
        assert_eq!(record.inputs[0].input.kind(), "administrative_action");
    }
}
