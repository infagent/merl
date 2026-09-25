//! Administrator changes to capture policy, guarded by binding version and grants.

use merl_core::compilation_policy::{PolicyEdit, PolicyValues};
use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    EventId, ObjectKind, ObjectLifecycle, PayloadId, PolicyDisposition, PolicyEvaluationId,
    PolicyInputId, PolicyRead, PolicyWrite, ProjectId, ReasonCode, SourceBindingId,
};
use merl_store::{BindingCapturePolicy, RecordedPolicyEvaluation, Store, StoreError};

use crate::authority::digest_parts;
use crate::{PolicyError, PreparedPolicy, Proposal, evaluate_current, recorded_outcome};

/// One administrative edit to a binding's defaults, selectors, or author mappings.
#[derive(Clone, Debug)]
pub struct BindingPolicyChange {
    /// Retry identity; changing any request field under this ID conflicts.
    pub id: PolicyInputId,
    /// Acting administrator, loaded against durable project grants.
    pub actor: ActorId,
    /// Existing project-source attachment.
    pub binding: SourceBindingId,
    /// Version read by the caller before proposing the replacement.
    pub expected_version: CapturePolicyVersion,
    /// Timing for newly captured prose.
    pub mode: CompilationMode,
    /// Completeness requirement for newly captured prose.
    pub coverage: CoverageRequirement,
    /// Audit explanation retained behind an erasable payload reference.
    pub reason: String,
    /// Exact selector, classification, override, or default edit.
    pub edit: PolicyEdit,
}

/// Applies a binding policy edit or returns its original recorded result.
///
/// This action creates no compiler work and changes no earlier source versions.
/// An identical retry keeps its outcome after later policy changes or reason erasure.
///
/// # Errors
/// Returns an error for empty reasons, missing bindings, changed retry content, or storage failures.
pub fn change_binding_policy(
    store: &mut Store,
    project: &ProjectId,
    change: &BindingPolicyChange,
    now_millis: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    let (evaluation, _, proposal, _) = proposal(project, change)?;
    if let Some(record) = recorded_outcome(
        store,
        project,
        &change.actor,
        &evaluation,
        std::slice::from_ref(&proposal),
    )? {
        return Ok(record);
    }
    let prepared = prepare_binding_policy_change(store, project, change, now_millis)?;
    prepared.commit(store)?;
    store
        .policy_evaluation(project, &evaluation)?
        .ok_or(StoreError::CorruptHistory.into())
}

/// Prepares a policy change while leaving accepted configuration untouched.
///
/// Commit checks the binding object and grant collection again. A change to another
/// binding does not invalidate this request. Call [`change_binding_policy`] for retries.
///
/// # Errors
/// Returns an error for invalid request content, unknown bindings, or storage failures.
pub fn prepare_binding_policy_change(
    store: &mut Store,
    project: &ProjectId,
    change: &BindingPolicyChange,
    now_millis: i64,
) -> Result<PreparedPolicy, PolicyError> {
    let current = store.binding_policy(project, &change.binding)?;
    let expected_revision = current.change.map(|object| object.revision);
    let (evaluation, batch, proposal, mut policy) = proposal(project, change)?;
    let mut selectors = current.selectors;
    selectors
        .apply(
            &change.edit,
            PolicyValues {
                mode: change.mode,
                coverage: change.coverage,
            },
        )
        .map_err(|_| PolicyError::InvalidProposal)?;
    if change.edit != PolicyEdit::Defaults {
        policy.mode = current.policy.mode;
        policy.coverage = current.policy.coverage;
    }
    let DomainEvent::PutObject {
        payload: Some(reason),
        ..
    } = proposal.event()
    else {
        return Err(PolicyError::InvalidProposal);
    };
    store.put_review_reason(project, reason, change.reason.as_bytes())?;
    store.prepare_binding_policy(
        project,
        &change.binding,
        &policy,
        &change.expected_version,
        reason,
        &selectors,
    )?;
    let mut prepared = evaluate_current(
        store,
        project,
        &change.actor,
        evaluation,
        batch,
        now_millis,
        &[proposal],
    )?;
    if prepared.evaluation.inputs[0].disposition == PolicyDisposition::Accepted {
        if current.policy.version == change.expected_version {
            // Guard the snapshot that supplied the compared version, even if another
            // administrator committed between that read and general policy evaluation.
            for read in &mut prepared.evaluation.reads {
                if let PolicyRead::Object { revision, .. } = read {
                    *revision = expected_revision;
                }
            }
            for write in &mut prepared.evaluation.writes {
                if let PolicyWrite::Object {
                    expected_revision: revision,
                    ..
                } = write
                {
                    *revision = expected_revision;
                }
            }
        } else {
            prepared.evaluation.inputs[0].disposition = PolicyDisposition::Conflict;
            prepared.evaluation.inputs[0].reason = ReasonCode::try_from("binding_policy_changed")
                .map_err(|_| PolicyError::InvalidProposal)?;
            prepared.evaluation.batch = None;
            prepared.evaluation.event_origins.clear();
            prepared.evaluation.writes.clear();
        }
    }
    Ok(prepared)
}

fn proposal(
    project: &ProjectId,
    change: &BindingPolicyChange,
) -> Result<(PolicyEvaluationId, BatchId, Proposal, BindingCapturePolicy), PolicyError> {
    if change.reason.trim().is_empty() {
        return Err(PolicyError::InvalidProposal);
    }
    let identity = digest_parts(&[project.as_str(), change.id.as_str()]);
    let edit = serde_json::to_string(&change.edit).map_err(|_| PolicyError::InvalidProposal)?;
    let mut content_parts = vec![
        &identity,
        change.binding.as_str(),
        change.expected_version.as_str(),
        change.mode.as_str(),
        change.coverage.as_str(),
        &change.reason,
    ];
    // Keep pre-selector default requests byte-compatible with their recorded receipts.
    if change.edit != PolicyEdit::Defaults {
        content_parts.push(&edit);
    }
    let content = digest_parts(&content_parts);
    let policy = BindingCapturePolicy {
        mode: change.mode,
        coverage: change.coverage,
        version: CapturePolicyVersion::try_from(format!("binding_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
    };
    let proposal = Proposal::AdministrativeAction {
        id: PolicyInputId::try_from(format!("binding_input_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
        event: DomainEvent::PutObject {
            id: EventId::try_from(format!("binding_event_{identity}").as_str())
                .map_err(|_| PolicyError::InvalidProposal)?,
            object: merl_store::binding_policy_object(&change.binding)?,
            kind: ObjectKind::try_from("binding_compilation_policy")
                .map_err(|_| PolicyError::InvalidProposal)?,
            payload: Some(
                PayloadId::try_from(format!("binding_reason_{content}").as_str())
                    .map_err(|_| PolicyError::InvalidProposal)?,
            ),
            issue_scope: None,
            lifecycle: ObjectLifecycle::Active,
        },
    };
    Ok((
        PolicyEvaluationId::try_from(format!("binding_eval_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
        BatchId::try_from(format!("binding_batch_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
        proposal,
        policy,
    ))
}
