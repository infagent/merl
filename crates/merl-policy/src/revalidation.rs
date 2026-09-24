//! Explicit promotion of reviewed hindsight interpretations under command authority.

use crate::assertions::identity;
use crate::{PolicyError, PreparedPolicy, Proposal, evaluate_current};
use merl_core::{
    BatchId, DomainEvent, ObjectId, ObjectLifecycle, PayloadId, PolicyDisposition,
    PolicyEvaluationId, PolicyInputId, PolicyRead, ProjectId, ReasonCode, Relation,
};
use merl_store::{
    RecordedPolicyEvaluation, RevalidationAction, RevalidationReview, Store, StoreError,
};

/// Returns the durable evaluation identity for a resolution request.
///
/// # Errors
/// Returns an error if the generated identity cannot be represented.
pub fn revalidation_evaluation(
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<PolicyEvaluationId, PolicyError> {
    identity("revalidation_eval", &[project.as_str(), id.as_str()])
}

/// Records and evaluates a review without committing its accepted events.
///
/// The original object, current grants, and revised evidence remain guarded until
/// commit. A run may finish under old grants without gaining authority to resolve work.
/// # Errors
/// Rejects changed request identities, malformed actions, or missing provenance.
#[expect(
    clippy::too_many_lines,
    reason = "the action mapping and its dependency guards form one review boundary"
)]
pub fn prepare_revalidation_review(
    store: &mut Store,
    project: &ProjectId,
    review: &RevalidationReview,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    use RevalidationAction::{Confirm, Invalidate, Supersede, Unavailable, Weaken};
    if (review.action == Unavailable) != review.run.is_none()
        || matches!(review.action, Confirm | Supersede) != review.assertion_index.is_some()
    {
        return Err(PolicyError::InvalidProposal);
    }
    if let Some(prior) = store.revalidation_review(project, &review.id)?
        && prior != *review
    {
        return Err(StoreError::PolicyInputConflict.into());
    }
    let impact = store
        .evidence_impact(project, &review.impact)?
        .ok_or(PolicyError::InvalidProposal)?;
    let object = store
        .object(project, &impact.object)?
        .ok_or(PolicyError::InvalidProposal)?;
    let mut failure = impact
        .revalidated_by
        .as_ref()
        .map(|_| "evidence_impact_resolved");
    if object.lifecycle != ObjectLifecycle::Active {
        failure = Some("revalidation_target_changed");
    }
    let mut replacement = None;
    if let Some(run) = &review.run {
        let mut root = run.as_str().to_owned();
        while let Some(parent) = store.expansion_parent(project, &root)? {
            root = parent;
        }
        let status = store
            .compilation_run_status(project, run.as_str())?
            .ok_or(PolicyError::InvalidProposal)?;
        if store
            .revalidation_impact(project, &root)?
            .is_none_or(|i| i.id != impact.id)
            || status.mode != "hindsight"
            || !status.succeeded
            || status.needs_context
        {
            return Err(PolicyError::RevalidationRunIneligible);
        }
        if !store.compilation_evidence_current(project, run)? {
            failure = Some("assertion_evidence_changed");
        }
        if store
            .object_at_revision(
                project,
                &impact.object,
                status.interpretation_basis_revision,
            )?
            .map(|o| o.0)
            != Some(object.revision)
        {
            failure = Some("revalidation_target_changed");
        }
        if let Some(index) = review.assertion_index {
            let assertion = store
                .observed_assertions(project, run.as_str())?
                .into_iter()
                .nth(index as usize)
                .ok_or(PolicyError::InvalidProposal)?;
            let subject = ObjectId::try_from(assertion.subject.as_str())
                .map_err(|_| PolicyError::InvalidProposal)?;
            let payload = if assertion.value == "none" {
                None
            } else {
                Some(
                    PayloadId::try_from(assertion.value.as_str())
                        .map_err(|_| PolicyError::InvalidProposal)?,
                )
            };
            if assertion.predicate != object.kind.as_str() || assertion.polarity != "positive" {
                return Err(PolicyError::InvalidProposal);
            }
            match review.action {
                Confirm if subject != impact.object || payload != object.payload => {
                    return Err(PolicyError::InvalidProposal);
                }
                Supersede
                    if subject == impact.object || store.object(project, &subject)?.is_some() =>
                {
                    return Err(PolicyError::InvalidProposal);
                }
                _ => {}
            }
            replacement = Some((subject, payload));
        }
    } else if !store.revalidation_evidence_unavailable(project, &impact.id)? {
        failure = Some("evidence_still_available");
    }
    let mut events = Vec::new();
    let parts = [project.as_str(), review.id.as_str()];
    let old_payload = if review.action == Unavailable
        && matches!(
            object
                .payload
                .as_ref()
                .map(|p| store.read_payload(project, p))
                .transpose()?,
            Some(merl_store::PayloadRead::Unavailable)
        ) {
        None
    } else {
        object.payload.clone()
    };
    events.push(DomainEvent::PutObject {
        id: identity("revalidation_event", &parts)?,
        object: impact.object.clone(),
        kind: object.kind.clone(),
        payload: old_payload,
        issue_scope: object.issue_scope.clone(),
        lifecycle: match review.action {
            Supersede => ObjectLifecycle::Superseded,
            Invalidate => ObjectLifecycle::Invalidated,
            Confirm | Weaken | Unavailable => ObjectLifecycle::Active,
        },
    });
    if review.action == Supersede {
        let (subject, payload) = replacement.ok_or(PolicyError::InvalidProposal)?;
        events.push(DomainEvent::PutObject {
            id: identity("revalidation_replacement", &parts)?,
            object: subject.clone(),
            kind: object.kind.clone(),
            payload,
            issue_scope: object.issue_scope.clone(),
            lifecycle: ObjectLifecycle::Active,
        });
        events.push(DomainEvent::PutRelation {
            id: identity("revalidation_relation_event", &parts)?,
            relation: Relation {
                id: identity("revalidation_relation", &parts)?,
                project: project.clone(),
                subject,
                kind: merl_core::RelationKind::try_from("supersedes")
                    .map_err(|_| PolicyError::InvalidProposal)?,
                object: impact.object.clone(),
            },
        });
    }
    // Evaluate one command, then attach its other atomic effects to that same input.
    let mut prepared = evaluate_current(
        store,
        project,
        &review.actor,
        revalidation_evaluation(project, &review.id)?,
        identity::<BatchId>("revalidation_batch", &parts)?,
        now,
        &[Proposal::Command {
            id: review.id.clone(),
            event: events[0].clone(),
        }],
    )?;
    if prepared.evaluation.inputs[0].disposition == PolicyDisposition::Accepted {
        if let Some(reason) = failure {
            prepared.evaluation.inputs[0].disposition = PolicyDisposition::Conflict;
            prepared.evaluation.inputs[0].reason =
                ReasonCode::try_from(reason).map_err(|_| PolicyError::InvalidProposal)?;
            prepared.evaluation.batch = None;
            prepared.evaluation.writes.clear();
            prepared.evaluation.event_origins.clear();
        } else {
            for event in events.into_iter().skip(1) {
                match &event {
                    DomainEvent::PutObject { id, object, .. } => {
                        prepared.evaluation.reads.push(PolicyRead::Object {
                            id: object.clone(),
                            revision: None,
                        });
                        prepared
                            .evaluation
                            .writes
                            .push(merl_core::PolicyWrite::Object {
                                object: object.clone(),
                                expected_revision: None,
                            });
                        prepared
                            .evaluation
                            .event_origins
                            .push(merl_core::PolicyEventOrigin {
                                event: id.clone(),
                                input_index: 0,
                            });
                    }
                    DomainEvent::PutRelation { id, relation } => {
                        prepared
                            .evaluation
                            .writes
                            .push(merl_core::PolicyWrite::Relation {
                                relation: relation.id.clone(),
                                expected_revision: None,
                            });
                        prepared
                            .evaluation
                            .event_origins
                            .push(merl_core::PolicyEventOrigin {
                                event: id.clone(),
                                input_index: 0,
                            });
                    }
                }
                prepared
                    .evaluation
                    .batch
                    .as_mut()
                    .ok_or(PolicyError::InvalidProposal)?
                    .events
                    .push(event);
            }
        }
    }
    store.record_revalidation_review(project, review)?;
    Ok(prepared)
}

/// Resolves changed support through current policy and returns exact retry outcomes.
///
/// # Errors
/// Rejects changed request content and missing or ineligible compiler provenance.
pub fn resolve_revalidation(
    store: &mut Store,
    project: &ProjectId,
    review: &RevalidationReview,
    now: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    if let Some(prior) = store.revalidation_review(project, &review.id)? {
        if prior != *review {
            return Err(StoreError::PolicyInputConflict.into());
        }
        if let Some(record) =
            store.policy_evaluation(project, &revalidation_evaluation(project, &review.id)?)?
        {
            return Ok(record);
        }
    }
    let prepared = prepare_revalidation_review(store, project, review, now)?;
    match prepared.commit(store) {
        Ok(_) | Err(StoreError::PolicyConflict) => {}
        Err(error) => return Err(error.into()),
    }
    store
        .policy_evaluation(project, &prepared.evaluation.id)?
        .ok_or(StoreError::CorruptHistory.into())
}
