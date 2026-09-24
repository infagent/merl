//! Closing stale relation support requires the same command authority as accepting an edge.

use crate::assertions::identity;
use crate::{PolicyError, PreparedPolicy, Proposal, evaluate_current};
use merl_core::{DomainEvent, ObjectKind, ObjectLifecycle, PolicyDisposition, ProjectId};
use merl_store::{RecordedPolicyEvaluation, RelationWithdrawal, Store, StoreError};

/// Prepares withdrawal of one stale derivation and records its immutable request.
///
/// The semantic edge remains in history. Committing the receipt closes all pending
/// impacts for that support event; it cannot make the old derivation current again.
/// # Errors
/// Rejects missing impacts, changed retry content, and invalid stored identities.
pub fn prepare_relation_withdrawal(
    store: &mut Store,
    project: &ProjectId,
    review: &RelationWithdrawal,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    let impact = store
        .relation_evidence_impact(project, &review.impact)?
        .ok_or(PolicyError::InvalidCommand(
            "relation impact does not exist",
        ))?;
    let mut prepared = evaluate_current(
        store,
        project,
        &review.actor,
        identity(
            "relation_withdrawal_eval",
            &[project.as_str(), review.id.as_str()],
        )?,
        identity(
            "relation_withdrawal_batch",
            &[project.as_str(), review.id.as_str()],
        )?,
        now,
        &[Proposal::Command {
            id: review.id.clone(),
            event: DomainEvent::PutObject {
                id: identity(
                    "relation_withdrawal_event",
                    &[project.as_str(), review.id.as_str()],
                )?,
                object: identity(
                    "relation_support_review",
                    &[project.as_str(), impact.support_event.as_str()],
                )?,
                kind: ObjectKind::try_from("relation_revalidation")
                    .map_err(|_| PolicyError::InvalidProposal)?,
                payload: None,
                issue_scope: None,
                lifecycle: ObjectLifecycle::Active,
            },
        }],
    )?;
    if impact.resolved && prepared.evaluation.inputs[0].disposition == PolicyDisposition::Accepted {
        prepared.evaluation.inputs[0].disposition = PolicyDisposition::Conflict;
        prepared.evaluation.inputs[0].reason =
            merl_core::ReasonCode::try_from("relation_support_already_resolved")
                .map_err(|_| PolicyError::InvalidProposal)?;
        prepared.evaluation.batch = None;
        prepared.evaluation.writes.clear();
        prepared.evaluation.event_origins.clear();
        prepared.evaluation.reads.clear();
    }
    store.record_relation_withdrawal(project, review)?;
    Ok(prepared)
}

/// Withdraws stale relation support, returning the original receipt on an exact retry.
///
/// # Errors
/// Rejects changed retry content, missing impacts, and storage failures. Policy
/// conflicts are recorded and returned with their original disposition.
pub fn withdraw_relation(
    store: &mut Store,
    project: &ProjectId,
    review: &RelationWithdrawal,
    now: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    if let Some(prior) = store.relation_withdrawal(project, &review.id)? {
        if prior != *review {
            return Err(StoreError::PolicyInputConflict.into());
        }
        if let Some(record) = store.policy_evaluation(
            project,
            &identity(
                "relation_withdrawal_eval",
                &[project.as_str(), review.id.as_str()],
            )?,
        )? {
            return Ok(record);
        }
    }
    let prepared = prepare_relation_withdrawal(store, project, review, now)?;
    match prepared.commit(store) {
        Ok(_) | Err(StoreError::PolicyConflict) => {}
        Err(error) => return Err(error.into()),
    }
    store
        .policy_evaluation(project, &prepared.evaluation.id)?
        .ok_or(StoreError::CorruptHistory.into())
}
