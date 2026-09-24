//! Structured semantic actions use durable grants and immutable, recoverable receipts.

use crate::{PolicyError, PreparedPolicy, Proposal, assertions::identity, evaluate_current};
use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    EventId, ObjectId, ObjectKind, ObjectLifecycle, PayloadId, PolicyDisposition, PolicyEvaluation,
    PolicyEvaluationId, PolicyInputId, PolicyRead, PolicyWrite, ProjectId, ReasonCode,
    SourceBindingId, SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{
    CommandOperation, Commitment, Execution, PayloadRead, RecordedPolicyEvaluation, Scheduling,
    SemanticCommandRecord, SourceBinding, SourceCapture, Store, StoreError, TaskState,
};
use sha2::{Digest, Sha256};

/// Maximum UTF-8 bytes in one command statement, reason, or supplemental note.
///
/// Commands author short statements. The 8 KiB bound keeps receipt transactions
/// and one object's expansion below the default 48 KiB compiler payload budget.
pub const MAX_COMMAND_TEXT_BYTES: usize = 8 * 1024;

/// One typed semantic action, before prose moves into protected storage.
#[derive(Clone, Debug)]
pub struct SemanticCommand {
    /// Stable request identity; reuse only for identical content.
    pub id: PolicyInputId,
    /// Local audit actor evaluated under current command authority.
    pub actor: ActorId,
    /// Requested action.
    pub operation: CommandOperation,
    /// Target semantic object.
    pub object: ObjectId,
    /// Target semantic kind.
    pub kind: ObjectKind,
    /// Scope for a new object; updates inherit their existing scope.
    pub issue_scope: Option<String>,
    /// New semantic statement or resolution text.
    pub summary: Option<String>,
    /// Explanation for deferral.
    pub reason: Option<String>,
    /// Reconsideration date in YYYY-MM-DD form.
    pub review_at: Option<String>,
    /// Supplemental source text; it grants no semantic authority.
    pub note: Option<String>,
}

/// Returns the evaluation identity owned by a semantic command receipt.
///
/// # Errors
/// Returns an error if the derived structural identity is invalid.
pub fn semantic_command_evaluation(
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<PolicyEvaluationId, PolicyError> {
    identity("command_eval", &[project.as_str(), id.as_str()])
}

/// Previews a command without retaining content, sources, receipts, or policy records.
///
/// # Errors
/// Returns malformed-content, identity-conflict, or storage errors.
pub fn preview_semantic_command(
    store: &Store,
    project: &ProjectId,
    command: &SemanticCommand,
    now: i64,
) -> Result<PolicyEvaluation, PolicyError> {
    let (record, _) = build_record(store, project, command, now)?;
    Ok(prepare_record(store, project, &record)?.evaluation)
}

/// Retains an immutable request and prepares acceptance under current grants.
///
/// The receipt and note commit together before policy evaluation. A restarted
/// caller can repeat the same request without changing its target dependency or
/// restoring erased content. The final policy transaction checks that dependency.
///
/// # Errors
/// Returns malformed input, identity conflict, or storage errors.
pub fn prepare_semantic_command(
    store: &mut Store,
    project: &ProjectId,
    command: &SemanticCommand,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    let (record, payloads) = build_record(store, project, command, now)?;
    let note = command
        .note
        .as_ref()
        .map(|body| note_capture(project, &record, body))
        .transpose()?;
    let record = store.record_semantic_command(project, &record, &payloads, note.as_ref())?;
    prepare_record(store, project, &record)
}

/// Applies a structured command and returns its durable outcome on exact retries.
///
/// # Errors
/// Changed content under the same ID conflicts. Invalid requests and unavailable
/// storage return errors; policy rejection and dependency conflict return receipts.
pub fn submit_semantic_command(
    store: &mut Store,
    project: &ProjectId,
    command: &SemanticCommand,
    now: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    let evaluation = semantic_command_evaluation(project, &command.id)?;
    if let Some(prior) = store.semantic_command(project, &command.id)? {
        if prior.request_digest != request_digest(command) {
            return Err(StoreError::PolicyInputConflict.into());
        }
        if let Some(result) = store.policy_evaluation(project, &evaluation)? {
            return Ok(result);
        }
    }
    let prepared = prepare_semantic_command(store, project, command, now)?;
    match prepared.commit(store) {
        Ok(_) | Err(StoreError::PolicyConflict) => {}
        Err(e) => return Err(e.into()),
    }
    store
        .policy_evaluation(project, &evaluation)?
        .ok_or(StoreError::CorruptHistory.into())
}

fn request_digest(command: &SemanticCommand) -> [u8; 32] {
    let mut hash = Sha256::new();
    for part in [
        Some(command.id.as_str()),
        Some(command.actor.as_str()),
        Some(command.operation.as_str()),
        Some(command.object.as_str()),
        Some(command.kind.as_str()),
        command.issue_scope.as_deref(),
        command.summary.as_deref(),
        command.reason.as_deref(),
        command.review_at.as_deref(),
        command.note.as_deref(),
    ] {
        hash.update([u8::from(part.is_some())]);
        let bytes = part.unwrap_or("").as_bytes();
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    hash.finalize().into()
}

type Content = Vec<(PayloadId, Vec<u8>)>;
fn build_record(
    store: &Store,
    project: &ProjectId,
    command: &SemanticCommand,
    now: i64,
) -> Result<(SemanticCommandRecord, Content), PolicyError> {
    let digest = request_digest(command);
    if let Some(prior) = store.semantic_command(project, &command.id)? {
        if prior.request_digest != digest {
            return Err(StoreError::PolicyInputConflict.into());
        }
        return Ok((prior, Vec::new()));
    }
    for text in [&command.summary, &command.reason, &command.note]
        .into_iter()
        .flatten()
    {
        if text.trim().is_empty() || text.len() > MAX_COMMAND_TEXT_BYTES {
            return Err(PolicyError::InvalidCommand(
                "command text must contain 1 to 8192 UTF-8 bytes",
            ));
        }
    }
    if command
        .issue_scope
        .as_ref()
        .is_some_and(|s| merl_core::ObjectId::try_from(s.as_str()).is_err())
    {
        return Err(PolicyError::InvalidCommand("invalid Issue scope"));
    }
    if let Some(date) = &command.review_at {
        let format = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
            .map_err(|_| PolicyError::InvalidProposal)?;
        if date.len() != 10 || time::Date::parse(date, &format).is_err() {
            return Err(PolicyError::InvalidCommand(
                "review date must be a valid YYYY-MM-DD date",
            ));
        }
    }
    let current = store.object(project, &command.object)?;
    let mut payloads = Vec::new();
    let mut retain =
        |name: &str, value: &Option<String>| -> Result<Option<PayloadId>, PolicyError> {
            value
                .as_ref()
                .map(|text| {
                    let id = identity::<PayloadId>(name, &[project.as_str(), command.id.as_str()])?;
                    payloads.push((id.clone(), text.as_bytes().to_vec()));
                    Ok(id)
                })
                .transpose()
        };
    let payload = retain("command_text", &command.summary)?
        .or_else(|| current.as_ref().and_then(|o| o.payload.clone()));
    let reason = retain("command_reason", &command.reason)?;
    let mut record = SemanticCommandRecord {
        id: command.id.clone(),
        actor: command.actor.clone(),
        operation: command.operation,
        object: command.object.clone(),
        kind: command.kind.clone(),
        issue_scope: command
            .issue_scope
            .clone()
            .or_else(|| current.as_ref().and_then(|o| o.issue_scope.clone())),
        payload,
        reason,
        source: command
            .note
            .as_ref()
            .map(|_| {
                identity::<SourceVersionId>(
                    "command_note",
                    &[project.as_str(), command.id.as_str()],
                )
            })
            .transpose()?,
        expected_revision: current.as_ref().map(|o| o.revision),
        request_digest: digest,
        occurred_at_millis: now,
        task: None,
        rejection: None,
    };
    let failure = validate_action(store, project, command, current.as_ref(), &mut record)?;
    record.rejection = failure
        .map(ReasonCode::try_from)
        .transpose()
        .map_err(|_| PolicyError::InvalidProposal)?;
    Ok((record, payloads))
}

fn validate_action(
    store: &Store,
    project: &ProjectId,
    command: &SemanticCommand,
    current: Option<&merl_store::ProjectedObject>,
    record: &mut SemanticCommandRecord,
) -> Result<Option<&'static str>, PolicyError> {
    let kind = command.kind.as_str();
    if !matches!(
        kind,
        "decision" | "question" | "finding" | "hypothesis" | "claim" | "task"
    ) {
        return Ok(Some("unsupported_command_kind"));
    }
    if current.is_some_and(|o| {
        o.kind != command.kind
            || command
                .issue_scope
                .as_ref()
                .is_some_and(|s| o.issue_scope.as_ref() != Some(s))
    }) {
        return Ok(Some("command_target_mismatch"));
    }
    if command.operation == CommandOperation::Create {
        if current.is_some() {
            return Ok(Some("command_target_exists"));
        }
        if command.summary.is_none() || command.reason.is_some() || command.review_at.is_some() {
            return Ok(Some("invalid_command_arguments"));
        }
        if kind == "task" {
            record.task = Some(TaskState {
                reason_span: None,
                review_when: None,
                start_after: None,
                temporal: Vec::new(),
                commitment: Commitment::Pending,
                scheduling: Scheduling::Unscheduled,
                execution: Execution::NotStarted,
                reason: None,
                review_at: None,
            });
        }
        return Ok(None);
    }
    let Some(current) = current else {
        return Ok(Some("command_target_missing"));
    };
    if current.lifecycle != ObjectLifecycle::Active {
        return Ok(Some("command_target_inactive"));
    }
    if command.operation == CommandOperation::Resolve {
        return Ok((!matches!(kind, "question" | "finding")
            || command.summary.is_none()
            || command.reason.is_some()
            || command.review_at.is_some())
        .then_some("invalid_command_arguments"));
    }
    if kind != "task" || command.summary.is_some() {
        return Ok(Some("invalid_command_arguments"));
    }
    let Some(mut task) = store.task_state(project, &command.object)? else {
        return Ok(Some("task_planning_unknown"));
    };
    if command.operation != CommandOperation::Defer
        && (command.reason.is_some() || command.review_at.is_some())
    {
        return Ok(Some("invalid_command_arguments"));
    }
    let failure = match command.operation {
        CommandOperation::Accept if task.commitment == Commitment::Pending => {
            task.commitment = Commitment::Accepted;
            None
        }
        CommandOperation::Defer if task.execution == Execution::InProgress => {
            Some("task_in_progress")
        }
        CommandOperation::Defer
            if task.execution == Execution::NotStarted
                && command.reason.is_some()
                && command.review_at.is_some() =>
        {
            task.scheduling = Scheduling::Deferred;
            task.reason.clone_from(&record.reason);
            clear_review_evidence(&mut task);
            task.review_at.clone_from(&command.review_at);
            None
        }
        CommandOperation::Start
            if task.commitment == Commitment::Accepted
                && task.execution == Execution::NotStarted
                && task.scheduling != Scheduling::Deferred =>
        {
            task.execution = Execution::InProgress;
            None
        }
        CommandOperation::Complete if task.execution == Execution::InProgress => {
            task.execution = Execution::Completed;
            None
        }
        _ => Some("invalid_task_transition"),
    };
    record.reason.clone_from(&task.reason);
    record.task = Some(task);
    Ok(failure)
}

fn prepare_record(
    store: &Store,
    project: &ProjectId,
    record: &SemanticCommandRecord,
) -> Result<PreparedPolicy, PolicyError> {
    let event = DomainEvent::PutObject {
        id: identity::<EventId>("command_event", &[project.as_str(), record.id.as_str()])?,
        object: record.object.clone(),
        kind: record.kind.clone(),
        payload: record.payload.clone(),
        issue_scope: record.issue_scope.clone(),
        lifecycle: ObjectLifecycle::Active,
    };
    let mut prepared = evaluate_current(
        store,
        project,
        &record.actor,
        semantic_command_evaluation(project, &record.id)?,
        identity::<BatchId>("command_batch", &[project.as_str(), record.id.as_str()])?,
        record.occurred_at_millis,
        &[Proposal::Command {
            id: record.id.clone(),
            event,
        }],
    )?;
    if prepared.evaluation.inputs[0].disposition == PolicyDisposition::Accepted {
        let mut failure = record.rejection.as_ref().map(ReasonCode::as_str);
        for payload in [&record.payload, &record.reason].into_iter().flatten() {
            // Previews can reference content they have not retained yet.
            if matches!(
                store.read_payload(project, payload),
                Ok(PayloadRead::Unavailable)
            ) {
                failure = Some("command_content_unavailable");
            }
        }
        if let Some(failure) = failure {
            prepared.evaluation.inputs[0].disposition = PolicyDisposition::Rejected;
            prepared.evaluation.inputs[0].reason =
                ReasonCode::try_from(failure).map_err(|_| PolicyError::InvalidProposal)?;
            prepared.evaluation.batch = None;
            prepared.evaluation.writes.clear();
            prepared.evaluation.event_origins.clear();
        } else {
            prepared.evaluation.reads.push(PolicyRead::Object {
                id: record.object.clone(),
                revision: record.expected_revision,
            });
            prepared.evaluation.writes = vec![PolicyWrite::Object {
                object: record.object.clone(),
                expected_revision: record.expected_revision,
            }];
        }
    }
    Ok(prepared)
}

fn note_capture<'a>(
    project: &ProjectId,
    record: &'a SemanticCommandRecord,
    text: &'a str,
) -> Result<SourceCapture<'a>, PolicyError> {
    let source = record.source.as_ref().ok_or(PolicyError::InvalidProposal)?;
    Ok(SourceCapture {
        binding: SourceBinding {
            id: identity::<SourceBindingId>("command_binding", &[project.as_str()])?,
            provider: SourceProvider::try_from("merl").map_err(|_| PolicyError::InvalidProposal)?,
            provider_namespace_id: project.to_string(),
            namespace_digest: Sha256::digest(project.as_str().as_bytes()).into(),
        },
        source: SourceId::try_from(source.as_str()).map_err(|_| PolicyError::InvalidProposal)?,
        provider_entity_id: record.id.as_str(),
        context_scope_id: record
            .issue_scope
            .as_deref()
            .unwrap_or(record.object.as_str()),
        version: source.clone(),
        provider_version_id: "initial",
        kind: SourceKind::try_from("supplemental_note")
            .map_err(|_| PolicyError::InvalidProposal)?,
        supersedes: None,
        ambiguous_order_with_previous: false,
        author_time: None,
        created_at_millis: record.occurred_at_millis,
        occurred_at_millis: record.occurred_at_millis,
        upstream_updated_at_millis: None,
        observed_at_millis: record.occurred_at_millis,
        actor: Some(record.actor.clone()),
        provider_actor_id: None,
        source_author: Some(record.actor.clone()),
        provider_source_author_id: None,
        body: Some(text.as_bytes()),
        edit_diff: None,
        edit_deleted_at_millis: None,
        missing_body_reason: None,
        compilation_mode: CompilationMode::CaptureOnly,
        coverage_requirement: CoverageRequirement::Optional,
        policy_version: CapturePolicyVersion::try_from("command_note_v1")
            .map_err(|_| PolicyError::InvalidProposal)?,
    })
}

// A note's conversation key bounds compiler context; it does not make the note
// an Issue. Supplemental assertions inherit the command's semantic scope.
pub(super) fn source_issue_scope(
    store: &Store,
    project: &ProjectId,
    source: &merl_store::StoredSourceVersion,
) -> Result<Option<String>, PolicyError> {
    Ok(match store.source_command(project, &source.id)? {
        Some(command) => command.issue_scope,
        None => Some(source.context_scope_id.clone()),
    })
}

fn clear_review_evidence(task: &mut TaskState) {
    task.reason_span = None;
    task.review_when = None;
    task.temporal
        .retain(|t| t.original.role != merl_core::temporal::TemporalRole::ReviewAt);
}
