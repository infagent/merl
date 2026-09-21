use merl_compiler::{
    CompileError, CompilerAdapter, CompilerLimits, RunMode, RunRequest, execute_compilation,
    prepare_compilation, record_compilation_result,
};
use merl_core::{
    CompilationMode, CoverageRequirement, DomainEvent, DomainEventBatch, PayloadId,
    PolicyDisposition, ProjectId, ProjectRevision, ProviderIssueState, ProviderObservation,
    SourceVersionId,
};
use merl_policy::{PolicyRules, PreparedPolicy, Proposal, evaluate};
use merl_store::{
    EvidenceImpact, PurgePreview, SourceBinding, SourceCapture, Store, StoreError, SupportStatus,
};
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
        issue_scope: None,
        lifecycle: merl_core::ObjectLifecycle::Active,
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
    value: &'static str,
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
            r#"{{"schema":"merl.compiler-response/v1","assertions":[{{"source":"{}","span_start":0,"span_end":{},"subject":"D1","predicate":"decision","value":"{}","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":{}}}]}}"#,
            self.version,
            self.body_len,
            self.value,
            self.attributed_to.map_or("null".to_owned(), |value| format!("\"{value}\""))
        ).into_bytes())
    }
}

pub struct PolicyScenario {
    store: Store,
    project: ProjectId,
    rules: PolicyRules,
    prepared: Option<PreparedPolicy>,
    prepared_retry: Option<PreparedPolicy>,
    result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
    stale_result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
    retry_disposition: Option<PolicyDisposition>,
    persisted_file: Option<TempStoreFile>,
    invalid_result: Option<Result<Option<merl_core::ProjectRevision>, StoreError>>,
    provider_collision_rejected: bool,
    pending_impacts: Vec<EvidenceImpact>,
    purge_preview: Option<PurgePreview>,
    stale_purge_rejected: bool,
    expected_provider: Option<merl_store::AcceptedProviderObservation>,
    expected_issue: Option<merl_store::IssueState>,
    pending_purge: Option<merl_store::PurgeAudit>,
}

impl PolicyScenario {
    pub fn given_a_decision_supported_by_an_issue_comment() -> Self {
        let mut scenario = Self::new();
        let body = "Use fixed gain";
        scenario.capture("direct-v1", body, "alice");
        scenario.compile("direct-v1", body.len(), None, "direct-run");
        let accepted = evaluate(
            &scenario.store,
            &scenario.project,
            &id("authority"),
            id("support-eval"),
            id("support-batch"),
            NOW + 3,
            &scenario.rules,
            &[Proposal::ObservedAssertion {
                id: id("support-assertion"),
                run: id("direct-run"),
                index: 0,
                event: event("support-event", "D1", "decision", None),
            }],
        )
        .expect("evaluate source assertion");
        accepted
            .commit(&mut scenario.store)
            .expect("accept decision");
        scenario
    }

    pub fn given_a_decision_compiled_with_an_earlier_comment() -> Self {
        let mut scenario = Self::file_backed();
        scenario.capture_in_scope("context-v1", "Keep the baseline.", "alice", "issue-204");
        let body = "Use fixed gain";
        scenario.capture_in_scope("direct-v1", body, "alice", "issue-204");
        scenario.compile("direct-v1", body.len(), None, "context-dependent-run");
        let accepted = evaluate(
            &scenario.store,
            &scenario.project,
            &id("authority"),
            id("context-eval"),
            id("context-batch"),
            NOW + 3,
            &scenario.rules,
            &[Proposal::ObservedAssertion {
                id: id("context-assertion"),
                run: id("context-dependent-run"),
                index: 0,
                event: event("context-event", "D1", "decision", None),
            }],
        )
        .expect("evaluate context-dependent assertion");
        accepted
            .commit(&mut scenario.store)
            .expect("accept decision");
        scenario
    }

    pub fn given_a_later_compiler_context_containing_an_accepted_decision() -> Self {
        let mut scenario = Self::file_backed();
        let body = "Use fixed gain SECRET_PURGE_CHAIN_7461";
        scenario.capture("direct-v1", body, "alice");
        scenario.compile_with_value("direct-v1", body.len(), "decision-body", "direct-run");
        let payload = id("decision-body");
        scenario
            .store
            .put_payload(&scenario.project, &payload, body.as_bytes())
            .expect("decision payload");
        let accepted = evaluate(
            &scenario.store,
            &scenario.project,
            &id("authority"),
            id("support-eval"),
            id("support-batch"),
            NOW + 3,
            &scenario.rules,
            &[Proposal::ObservedAssertion {
                id: id("support-assertion"),
                run: id("direct-run"),
                index: 0,
                event: event("support-event", "D1", "decision", Some(payload)),
            }],
        )
        .expect("evaluate source assertion");
        accepted
            .commit(&mut scenario.store)
            .expect("accept decision");
        let later = "Check D1";
        scenario.capture("later-v1", later, "alice");
        scenario.compile("later-v1", later.len(), None, "later-run");
        scenario
    }

    pub fn then_the_preview_includes_the_later_context(&mut self) -> &mut Self {
        let preview = self.purge_preview.as_ref().expect("preview");
        assert!(preview.runs.iter().any(|run| run.as_str() == "later-run"));
        self
    }

    pub fn then_the_later_context_and_protected_bytes_are_gone(&mut self) {
        assert!(
            self.store
                .load_compilation_context(&self.project, "later-run")
                .is_err()
        );
        let file = self.persisted_file.as_ref().expect("file-backed authority");
        let bytes = std::fs::read(&file.0).expect("read active database");
        assert!(
            !bytes
                .windows(b"SECRET_PURGE_CHAIN_7461".len())
                .any(|part| part == b"SECRET_PURGE_CHAIN_7461")
        );
    }

    pub fn when_the_earlier_comment_is_edited(&mut self) -> &mut Self {
        self.capture_edit(
            "context-v1",
            "context-v2",
            "Keep the baseline!",
            "issue-204",
        );
        self
    }

    pub fn when_the_authority_restarts(&mut self) -> &mut Self {
        let file = self.persisted_file.as_ref().expect("file-backed authority");
        self.store = Store::open(&file.0).expect("reopen authority");
        self
    }

    pub fn when_pending_evidence_work_is_listed(&mut self) -> &mut Self {
        self.pending_impacts = self
            .store
            .pending_evidence_impacts(&self.project)
            .expect("pending evidence work");
        self
    }

    pub fn then_the_affected_derivation_can_be_resumed(&mut self) -> &mut Self {
        assert_eq!(self.pending_impacts.len(), 1);
        let impact = &self.pending_impacts[0];
        assert_eq!(impact.affected_run.as_str(), "context-dependent-run");
        assert_eq!(impact.trigger.as_str(), "direct-v1");
        assert_eq!(impact.changed_source.as_str(), "context-v1");
        assert_eq!(
            impact.replacement.as_ref().map(SourceVersionId::as_str),
            Some("context-v2")
        );
        assert_eq!(impact.next_action, "recompile");
        self
    }

    pub fn then_no_evidence_work_remains(&mut self) {
        assert!(self.pending_impacts.is_empty());
    }

    pub fn when_the_comment_is_edited(&mut self) -> &mut Self {
        self.capture_edit("direct-v1", "direct-v2", "Use fixed gain.", "direct-v1");
        self
    }

    fn capture_edit(&mut self, source: &str, version: &str, body: &str, scope: &str) {
        self.store
            .capture_source_version(
                &self.project,
                &SourceCapture {
                    binding: binding(),
                    source: id(source),
                    provider_entity_id: source,
                    context_scope_id: scope,
                    version: id(version),
                    provider_version_id: version,
                    kind: id("issue_comment"),
                    supersedes: Some(id(source)),
                    ambiguous_order_with_previous: false,
                    created_at_millis: NOW,
                    occurred_at_millis: NOW + 4,
                    upstream_updated_at_millis: Some(NOW + 4),
                    observed_at_millis: NOW + 4,
                    actor: Some(id("alice")),
                    provider_actor_id: Some("alice"),
                    source_author: Some(id("alice")),
                    provider_source_author_id: Some("alice"),
                    body: Some(body.as_bytes()),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::Eager,
                    coverage_requirement: CoverageRequirement::Required,
                    policy_version: id("capture-v1"),
                },
            )
            .expect("capture edit");
    }

    pub fn then_the_decision_is_active_with_revalidation_pending(&mut self) -> &mut Self {
        assert!(
            self.store
                .object(&self.project, &id("D1"))
                .expect("decision")
                .is_some()
        );
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .expect("support"),
            SupportStatus::RevalidationPending,
        );
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .expect("queue"),
            1
        );
        let impacts = self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .expect("impact trail");
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].next_action, "recompile");
        assert!(impacts[0].revalidated_by.is_none());
        self
    }

    pub fn when_the_edit_confirms_the_same_decision(&mut self) -> &mut Self {
        let body = "Use fixed gain.";
        self.compile("direct-v2", body.len(), None, "edited-run");
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("authority"),
            id("revalidation-eval"),
            id("revalidation-batch"),
            NOW + 6,
            &self.rules,
            &[Proposal::ObservedAssertion {
                id: id("edited-assertion"),
                run: id("edited-run"),
                index: 0,
                event: event("revalidation-event", "D1", "decision", None),
            }],
        )
        .expect("evaluate edit");
        prepared
            .commit(&mut self.store)
            .expect("revalidate decision");
        self
    }

    pub fn when_the_affected_derivation_is_recompiled_and_confirmed(&mut self) -> &mut Self {
        let body = "Use fixed gain";
        let impact = self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .expect("impact trail")
            .into_iter()
            .next()
            .expect("impact");
        let compiler = StatementCompiler {
            version: "direct-v1",
            body_len: body.len(),
            attributed_to: None,
            value: "none",
        };
        let prepared_run = prepare_compilation(
            &mut self.store,
            &self.project,
            &id("direct-v1"),
            &compiler,
            RunRequest {
                id: &impact.id,
                limits: limits(),
                mode: RunMode::Hindsight,
                now_millis: NOW + 5,
            },
        )
        .expect("prepare affected derivation")
        .expect("new run");
        let response = execute_compilation(&prepared_run, &compiler);
        record_compilation_result(
            &mut self.store,
            &self.project,
            &prepared_run,
            response,
            NOW + 6,
        )
        .expect("record revised interpretation");
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("authority"),
            id("context-revalidation-eval"),
            id("context-revalidation-batch"),
            NOW + 6,
            &self.rules,
            &[Proposal::ObservedAssertion {
                id: id("context-revalidation-assertion"),
                run: id(&impact.id),
                index: 0,
                event: event("context-revalidation-event", "D1", "decision", None),
            }],
        )
        .expect("evaluate revised context");
        prepared.commit(&mut self.store).expect("confirm decision");
        self
    }

    pub fn then_the_context_impact_is_resolved_and_support_is_current(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .expect("support"),
            SupportStatus::Current,
        );
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .expect("queue"),
            0
        );
        let impacts = self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .expect("impact trail");
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].affected_run.as_str(), "context-dependent-run");
        assert_eq!(impacts[0].trigger.as_str(), "direct-v1");
        let revised = self
            .store
            .load_compilation_context(&self.project, &impacts[0].id)
            .expect("revised compiler input");
        assert!(
            revised
                .source_window
                .iter()
                .any(|item| item.as_str() == "context-v2")
        );
        assert!(
            revised
                .source_window
                .iter()
                .any(|item| item.as_str() == "direct-v1")
        );
        assert_eq!(
            impacts[0]
                .revalidated_by
                .as_ref()
                .map(merl_core::EventId::as_str),
            Some("context-revalidation-event")
        );
        self
    }

    pub fn when_a_provider_observation_targets_that_decision(&mut self) -> &mut Self {
        let payload = id("provider-collision-snapshot");
        self.store
            .put_payload(&self.project, &payload, b"provider snapshot")
            .expect("snapshot");
        let observation = ProviderObservation {
            id: id("provider-collision-input"),
            binding: binding().id,
            issue: id("D1"),
            state: ProviderIssueState::Open,
            upstream_updated_at_millis: None,
            closed_at_millis: None,
            label_provider_ids: Some(Vec::new()),
            assignee_provider_ids: Some(Vec::new()),
            snapshot_payload: payload.clone(),
            observed_at_millis: NOW + 7,
        };
        self.provider_collision_rejected = match evaluate(
            &self.store,
            &self.project,
            &id("provider_observation"),
            id("provider-collision-eval"),
            id("provider-collision-batch"),
            NOW + 7,
            &self.rules,
            &[Proposal::ProviderObservation {
                observation,
                event: event(
                    "provider-collision-event",
                    "D1",
                    "provider_issue",
                    Some(payload),
                ),
            }],
        ) {
            Ok(prepared) => prepared.commit(&mut self.store).is_err(),
            Err(_) => true,
        };
        self
    }

    pub fn then_the_provider_observation_is_rejected_and_the_decision_remains(&mut self) {
        assert!(self.provider_collision_rejected);
        let object = self
            .store
            .object(&self.project, &id("D1"))
            .expect("decision")
            .expect("present");
        assert_eq!(object.kind.as_str(), "decision");
        assert!(
            self.store
                .provider_issue_head(&self.project, &id("D1"))
                .expect("provider")
                .is_none()
        );
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
    }

    pub fn when_the_object_projection_is_rebuilt(&mut self) -> &mut Self {
        self.store
            .rebuild_projection(&self.project)
            .expect("rebuild projection");
        self
    }

    pub fn then_the_decision_has_current_support_again(&mut self) {
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .expect("support"),
            SupportStatus::Current,
        );
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .expect("queue"),
            0
        );
        let impacts = self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .expect("impact trail");
        assert_eq!(
            impacts[0]
                .revalidated_by
                .as_ref()
                .map(merl_core::EventId::as_str),
            Some("revalidation-event")
        );
    }

    pub fn when_the_source_bytes_are_erased(&mut self) -> &mut Self {
        self.store
            .erase_payload(&self.project, &id("src_direct-v1"))
            .expect("erase source bytes");
        self
    }

    pub fn when_the_source_purge_is_previewed(&mut self) -> &mut Self {
        self.purge_preview = Some(
            self.store
                .preview_source_purge(&self.project, &id("direct-v1"))
                .expect("purge preview"),
        );
        self
    }

    pub fn when_the_earlier_comment_is_purged(&mut self) -> &mut Self {
        let source = id("context-v1");
        let preview = self
            .store
            .preview_source_purge(&self.project, &source)
            .expect("context purge preview");
        self.store
            .purge_source(
                &self.project,
                &source,
                &id("admin"),
                b"Sensitive text",
                NOW + 8,
                preview.confirm_digest,
            )
            .expect("purge context source");
        self
    }

    pub fn then_both_required_sources_report_unavailable_evidence(&mut self) {
        let coverage = self
            .store
            .semantic_coverage(&self.project)
            .expect("coverage");
        assert_eq!(coverage.required_purged, 2);
        assert_eq!(coverage.required_gaps, 2);
    }

    pub fn then_the_preview_names_the_affected_run_and_decision(&mut self) -> &mut Self {
        let preview = self.purge_preview.as_ref().expect("preview");
        assert!(
            preview
                .payloads
                .iter()
                .any(|payload| payload.id.as_str() == "src_direct-v1")
        );
        assert!(preview.runs.iter().any(|run| run.as_str() == "direct-run"));
        assert!(
            preview
                .assertions
                .iter()
                .any(|assertion| assertion.run.as_str() == "direct-run" && assertion.index == 0)
        );
        assert!(preview.objects.iter().any(|object| object.as_str() == "D1"));
        self
    }

    pub fn when_the_source_is_compiled_again(&mut self) -> &mut Self {
        self.compile("direct-v1", "Use fixed gain".len(), None, "later-run");
        self
    }

    pub fn when_the_old_preview_is_confirmed(&mut self) -> &mut Self {
        let digest = self.purge_preview.as_ref().expect("preview").confirm_digest;
        self.stale_purge_rejected = matches!(
            self.store.purge_source(
                &self.project,
                &id("direct-v1"),
                &id("admin"),
                b"Sensitive text",
                NOW + 8,
                digest,
            ),
            Err(StoreError::InvalidPurge)
        );
        self
    }

    pub fn then_the_purge_is_rejected_and_source_bytes_remain(&mut self) {
        assert!(self.stale_purge_rejected);
        assert!(matches!(
            self.store
                .read_payload(&self.project, &id("src_direct-v1"))
                .expect("source"),
            merl_store::PayloadRead::Available(_)
        ));
        assert!(
            self.store
                .purge_audit(&self.project, &id("direct-v1"))
                .expect("audit")
                .is_none()
        );
    }

    pub fn when_the_preview_is_confirmed(&mut self) -> &mut Self {
        let preview = self.purge_preview.as_ref().expect("preview");
        self.store
            .purge_source(
                &self.project,
                &id("direct-v1"),
                &id("admin"),
                b"Credential posted in comment",
                NOW + 8,
                preview.confirm_digest,
            )
            .expect("audited purge");
        self
    }

    pub fn when_the_purge_is_confirmed_while_another_reader_is_active(&mut self) -> &mut Self {
        let file = self.persisted_file.as_ref().expect("file-backed authority");
        let reader = rusqlite::Connection::open(&file.0).expect("second reader");
        reader
            .pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL for concurrent reader");
        reader
            .execute_batch("BEGIN")
            .expect("open read transaction");
        let _: i64 = reader
            .query_row("SELECT COUNT(*) FROM payloads", [], |row| row.get(0))
            .expect("hold WAL snapshot");
        let preview = self.purge_preview.as_ref().expect("preview");
        self.pending_purge = Some(
            self.store
                .purge_source(
                    &self.project,
                    &id("direct-v1"),
                    &id("admin"),
                    b"Credential posted in comment",
                    NOW + 8,
                    preview.confirm_digest,
                )
                .expect("committed purge receipt"),
        );
        reader
            .execute_batch("ROLLBACK")
            .expect("release read transaction");
        self
    }

    pub fn then_the_purge_is_accepted_with_scrubbing_pending(&mut self) -> &mut Self {
        let receipt = self.pending_purge.as_ref().expect("purge receipt");
        assert!(!receipt.completed);
        assert!(
            self.store
                .purge_audit(&self.project, &id("direct-v1"))
                .expect("durable receipt")
                .is_some()
        );
        self
    }

    pub fn then_the_pending_scrub_completes(&mut self) {
        assert!(
            self.store
                .purge_audit(&self.project, &id("direct-v1"))
                .expect("durable receipt")
                .expect("purge")
                .completed
        );
    }

    pub fn then_source_and_compiler_bytes_are_unavailable_with_an_audit_record(&mut self) {
        assert!(matches!(
            self.store
                .read_payload(&self.project, &id("src_direct-v1"))
                .expect("source payload"),
            merl_store::PayloadRead::Unavailable
        ));
        assert!(
            self.store
                .load_compilation_context(&self.project, "direct-run")
                .is_err()
        );
        assert!(
            self.store
                .purge_audit(&self.project, &id("direct-v1"))
                .expect("audit lookup")
                .is_some()
        );
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .expect("support"),
            SupportStatus::Unsupported
        );
    }

    pub fn then_the_decision_remains_but_support_is_unsupported(&mut self) {
        assert!(
            self.store
                .object(&self.project, &id("D1"))
                .expect("decision")
                .is_some()
        );
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .expect("support"),
            SupportStatus::Unsupported,
        );
        let coverage = self
            .store
            .semantic_coverage(&self.project)
            .expect("coverage");
        assert_eq!(coverage.required_purged, 1);
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .expect("queue"),
            1
        );
    }

    pub fn given_a_prepared_batch_with_one_competing_input() -> Self {
        let mut scenario = Self::new();
        scenario.prepared = Some(scenario.command(
            "shared-command",
            "first-eval",
            "first-batch",
            "first-event",
            "D1",
        ));
        scenario.prepared_retry = Some(
            evaluate(
                &scenario.store,
                &scenario.project,
                &id("alice"),
                id("mixed-eval"),
                id("mixed-batch"),
                NOW,
                &scenario.rules,
                &[
                    Proposal::Command {
                        id: id("shared-command"),
                        event: event("mixed-event-one", "D1", "decision", None),
                    },
                    Proposal::Command {
                        id: id("new-command"),
                        event: event("mixed-event-two", "D2", "decision", None),
                    },
                ],
            )
            .expect("prepare mixed batch"),
        );
        scenario
    }

    pub fn when_the_competing_input_commits_first(&mut self) -> &mut Self {
        self.when_both_are_committed()
    }

    pub fn then_the_mixed_batch_conflicts_without_applying_its_new_input(&mut self) {
        assert!(matches!(
            self.stale_result,
            Some(Err(StoreError::PolicyConflict))
        ));
        assert!(
            self.store
                .object(&self.project, &id("D2"))
                .expect("object lookup")
                .is_none()
        );
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        let record = self
            .store
            .policy_evaluation(&self.project, &id("mixed-eval"))
            .expect("policy record")
            .expect("conflict outcome");
        assert_eq!(
            record.conflict.expect("overlap").reason_code,
            "accepted_input_overlap"
        );
    }

    pub fn given_two_prepared_evaluations_of_the_same_command() -> Self {
        let mut scenario = Self::new();
        scenario.prepared = Some(scenario.command(
            "shared-command",
            "first-eval",
            "first-batch",
            "first-event",
            "D1",
        ));
        scenario.prepared_retry = Some(scenario.command(
            "shared-command",
            "second-eval",
            "second-batch",
            "second-event",
            "D1",
        ));
        scenario
    }

    pub fn given_two_prepared_evaluations_of_one_assertion() -> Self {
        let mut scenario = Self::new();
        let body = "Use fixed gain";
        scenario.capture("direct-v1", body, "alice");
        scenario.compile("direct-v1", body.len(), None, "direct-run");
        for (slot, handle, evaluation, batch, event_id) in [
            (
                0,
                "first-handle",
                "first-eval",
                "first-batch",
                "first-event",
            ),
            (
                1,
                "second-handle",
                "second-eval",
                "second-batch",
                "second-event",
            ),
        ] {
            let prepared = evaluate(
                &scenario.store,
                &scenario.project,
                &id("authority"),
                id(evaluation),
                id(batch),
                NOW + 3,
                &scenario.rules,
                &[Proposal::ObservedAssertion {
                    id: id(handle),
                    run: id("direct-run"),
                    index: 0,
                    event: event(event_id, "D1", "decision", None),
                }],
            )
            .expect("prepare assertion");
            if slot == 0 {
                scenario.prepared = Some(prepared);
            } else {
                scenario.prepared_retry = Some(prepared);
            }
        }
        scenario
    }

    pub fn when_both_are_committed(&mut self) -> &mut Self {
        self.result = Some(
            self.prepared
                .take()
                .expect("first evaluation")
                .commit(&mut self.store),
        );
        self.stale_result = Some(
            self.prepared_retry
                .take()
                .expect("second evaluation")
                .commit(&mut self.store),
        );
        self
    }

    pub fn then_the_second_is_recorded_as_a_duplicate(&mut self) {
        assert_eq!(
            self.result
                .as_ref()
                .expect("first outcome")
                .as_ref()
                .expect("first commit")
                .map(ProjectRevision::get),
            Some(1)
        );
        assert!(matches!(self.stale_result, Some(Ok(None))));
        let record = self
            .store
            .policy_evaluation(&self.project, &id("second-eval"))
            .expect("policy lookup")
            .expect("second evaluation");
        assert_eq!(record.inputs[0].disposition, PolicyDisposition::Duplicate);
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            1
        );
        assert_eq!(
            self.store
                .accepted_event_count(&self.project)
                .expect("events"),
            1
        );
    }

    pub fn given_two_authorized_command_proposals() -> Self {
        Self::file_backed()
    }

    pub fn when_they_are_accepted_together(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("alice"),
            id("compound-eval"),
            id("compound-batch"),
            NOW,
            &self.rules,
            &[
                Proposal::Command {
                    id: id("command-one"),
                    event: event("event-one", "D1", "decision", None),
                },
                Proposal::Command {
                    id: id("command-two"),
                    event: event("event-two", "D2", "decision", None),
                },
            ],
        )
        .expect("evaluate compound decision");
        self.result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_each_object_resolves_to_its_own_input(&mut self) {
        assert_eq!(
            self.result
                .as_ref()
                .expect("result")
                .as_ref()
                .expect("commit")
                .map(ProjectRevision::get),
            Some(1)
        );
        self.store = Store::open(&self.persisted_file.as_ref().expect("store file").0)
            .expect("reopen authority");
        for (object, event, command) in [
            ("D1", "event-one", "command-one"),
            ("D2", "event-two", "command-two"),
        ] {
            let origin = self
                .store
                .object_policy_origin(&self.project, &id(object))
                .expect("origin lookup")
                .expect("policy origin");
            assert_eq!(origin.evaluation, id("compound-eval"));
            assert_eq!(origin.event, id(event));
            assert_eq!(origin.input, merl_core::PolicyInput::Command(id(command)));
        }
    }

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
            prepared_retry: None,
            result: None,
            stale_result: None,
            retry_disposition: None,
            persisted_file: None,
            invalid_result: None,
            provider_collision_rejected: false,
            pending_impacts: Vec::new(),
            purge_preview: None,
            stale_purge_rejected: false,
            expected_provider: None,
            expected_issue: None,
            pending_purge: None,
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
        self.capture_in_scope(version, body, author, version);
    }

    fn capture_in_scope(
        &mut self,
        version: &'static str,
        body: &'static str,
        author: &str,
        scope: &str,
    ) {
        self.store
            .capture_source_version(
                &self.project,
                &SourceCapture {
                    binding: binding(),
                    source: id(version),
                    provider_entity_id: version,
                    context_scope_id: scope,
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
        self.compile_with_attribution_and_value(version, body_len, attributed_to, "none", run_id);
    }

    fn compile_with_value(
        &mut self,
        version: &'static str,
        body_len: usize,
        value: &'static str,
        run_id: &'static str,
    ) {
        self.compile_with_attribution_and_value(version, body_len, None, value, run_id);
    }

    fn compile_with_attribution_and_value(
        &mut self,
        version: &'static str,
        body_len: usize,
        attributed_to: Option<&'static str>,
        value: &'static str,
        run_id: &'static str,
    ) {
        let compiler = StatementCompiler {
            version,
            body_len,
            attributed_to,
            value,
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
        let mut scenario = Self::file_backed();
        scenario.seed("initial", "D1", "decision");
        scenario.prepared =
            Some(scenario.command("update-1", "eval-1", "batch-1", "event-1", "D1"));
        scenario
    }

    fn seed(&mut self, batch: &str, object: &str, kind: &str) {
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
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
        self.store = Store::open(&self.persisted_file.as_ref().expect("store file").0)
            .expect("reopen authority");
        let conflict = self
            .store
            .policy_evaluation(&self.project, &id("eval-2"))
            .expect("evaluation lookup")
            .expect("durable conflict outcome");
        assert_eq!(conflict.basis_project_revision.get(), 3);
        assert!(conflict.committed_revision.is_none());
        let detail = conflict.conflict.expect("changed dependency");
        assert_eq!(detail.reason_code, "object_read_changed");
        assert_eq!(detail.target_id.as_deref(), Some("D1"));
        assert_eq!(detail.expected_revision, Some(2));
        assert_eq!(detail.actual_revision, Some(3));
        assert_eq!(conflict.inputs[0].disposition, PolicyDisposition::Conflict);
        assert_eq!(conflict.reads.len(), 1);
        assert_eq!(conflict.writes.len(), 1);
        assert!(conflict.events.is_empty());
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

    pub fn when_the_provider_reconfirms_the_issue(&mut self) -> &mut Self {
        self.store
            .note_provider_seen(&self.project, &id("I1"), NOW + 5)
            .expect("provider sighting");
        self.expected_provider = self
            .store
            .provider_issue_head(&self.project, &id("I1"))
            .expect("current provider head");
        self.expected_issue = Some(
            self.store
                .issue_state(&self.project, &id("I1"), "issue-v1")
                .expect("current Issue state"),
        );
        self
    }

    pub fn when_disposable_issue_projections_are_cleared(&mut self) -> &mut Self {
        let file = self.persisted_file.as_ref().expect("file-backed authority");
        let connection = rusqlite::Connection::open(&file.0).expect("open disposable projection");
        connection
            .execute(
                "DELETE FROM provider_issue_heads WHERE project_id=?1",
                [self.project.as_str()],
            )
            .expect("clear provider head");
        self
    }

    pub fn then_the_provider_issue_state_and_sighting_are_restored(&mut self) {
        assert_eq!(
            self.store
                .provider_issue_head(&self.project, &id("I1"))
                .expect("restored provider head"),
            self.expected_provider,
        );
        assert_eq!(
            self.store
                .issue_state(&self.project, &id("I1"), "issue-v1")
                .expect("restored Issue state"),
            self.expected_issue.clone().expect("Issue before rebuild"),
        );
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

    pub fn when_the_agent_retries_the_same_rejected_command(&mut self) -> &mut Self {
        let prepared = evaluate(
            &self.store,
            &self.project,
            &id("agent"),
            id("denied-retry-eval"),
            id("denied-retry-batch"),
            NOW + 1,
            &self.rules,
            &[Proposal::Command {
                id: id("same-command"),
                event: event("denied-event", "D1", "decision", None),
            }],
        )
        .expect("reevaluate rejected command");
        self.stale_result = Some(prepared.commit(&mut self.store));
        self
    }

    pub fn then_the_retry_records_another_rejection_without_a_project_change(
        &mut self,
    ) -> &mut Self {
        assert!(matches!(self.stale_result, Some(Ok(None))));
        let record = self
            .store
            .policy_evaluation(&self.project, &id("denied-retry-eval"))
            .expect("policy record")
            .expect("retry evaluation");
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
