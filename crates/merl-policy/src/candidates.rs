//! Explicit review uses command authority while retaining immutable compiler provenance.

use crate::assertions::identity;
use crate::{PolicyError, PreparedPolicy, Proposal, evaluate_current};
use merl_core::{
    BatchId, DomainEvent, EventId, ObjectId, ObjectKind, ObjectLifecycle, PayloadId,
    PolicyDisposition, PolicyEvaluationId, PolicyInputId, PolicyRead, ProjectId,
};
use merl_store::{
    Candidate, CandidateReview, PayloadRead, RecordedPolicyEvaluation, ReviewAction, Store,
    StoreError,
};

/// Returns the evaluation identity used for a review's durable retry receipt.
///
/// # Errors
/// Returns an error if a generated structural identity cannot be represented.
pub fn candidate_review_evaluation(
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<PolicyEvaluationId, PolicyError> {
    identity("review_eval", &[project.as_str(), id.as_str()])
}

/// Prepares a review under current grants and records its immutable request.
///
/// Recording the request does not resolve the candidate. The returned policy must
/// commit before any semantic or review-state change becomes accepted.
/// # Errors
/// Rejects unknown candidates, changed retry content, or malformed replacement proposals.
pub fn prepare_candidate_review(
    store: &mut Store,
    project: &ProjectId,
    review: &CandidateReview,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    let prepared = build_review(store, project, review, now)?;
    store.record_candidate_review(project, review)?;
    Ok(prepared)
}

/// Evaluates a review without recording a request, payload, or accepted state.
///
/// # Errors
/// Rejects missing candidates and invalid stored provenance or replacement types.
pub fn preview_candidate_review(
    store: &Store,
    project: &ProjectId,
    review: &CandidateReview,
    now: i64,
) -> Result<merl_core::PolicyEvaluation, PolicyError> {
    Ok(build_review(store, project, review, now)?.evaluation)
}

fn build_review(
    store: &Store,
    project: &ProjectId,
    review: &CandidateReview,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    let candidate = store
        .candidate(project, &review.candidate)?
        .ok_or(PolicyError::CandidateMissing)?;
    if candidate.relation {
        return crate::relations::build_review(store, project, review, &candidate, now);
    }
    let (original_object, event) = review_event(store, project, review, &candidate)?;
    let semantic = review.action != ReviewAction::Reject;
    let target_event = event.clone();
    let proposal = Proposal::Command {
        id: review.id.clone(),
        event,
    };
    let mut prepared = evaluate_current(
        store,
        project,
        &review.actor,
        candidate_review_evaluation(project, &review.id)?,
        identity::<BatchId>("review_batch", &[project.as_str(), review.id.as_str()])?,
        now,
        &[proposal],
    )?;
    if prepared.evaluation.inputs[0].disposition != PolicyDisposition::Accepted {
        return Ok(prepared);
    }
    let mut failure = None;
    if store.candidate_resolution(project, &candidate)?.is_some()
        || store.accepted_assertion(project, &candidate.run, candidate.index)?
    {
        failure = Some((PolicyDisposition::Conflict, "candidate_already_resolved"));
    } else if semantic && !store.compilation_evidence_current(project, &candidate.run)? {
        failure = Some((PolicyDisposition::Conflict, "assertion_evidence_changed"));
    }
    if semantic {
        let DomainEvent::PutObject {
            object,
            kind,
            payload,
            issue_scope,
            ..
        } = &target_event
        else {
            return Err(PolicyError::InvalidProposal);
        };
        if store
            .object(project, object)?
            .is_some_and(|current| current.kind != *kind || current.issue_scope != *issue_scope)
        {
            failure = Some((PolicyDisposition::Conflict, "assertion_target_mismatch"));
        }
        if let Some(payload) = payload {
            match store.read_payload(project, payload) {
                Ok(PayloadRead::Available(_)) => {}
                Ok(PayloadRead::Unavailable) | Err(StoreError::PayloadMissing) => {
                    failure = Some((PolicyDisposition::Rejected, "assertion_value_unavailable"));
                }
                Err(error) => return Err(error.into()),
            }
        }
        let expected = store
            .object_at_revision(project, &original_object, candidate.basis)?
            .map(|o| o.0);
        if store.object(project, &original_object)?.map(|o| o.revision) != expected {
            failure = Some((PolicyDisposition::Conflict, "candidate_target_changed"));
        }
        prepared.evaluation.reads.push(PolicyRead::Object {
            id: original_object,
            revision: expected,
        });
    }
    if let Some((disposition, reason)) = failure {
        prepared.evaluation.inputs[0].disposition = disposition;
        prepared.evaluation.inputs[0].reason =
            merl_core::ReasonCode::try_from(reason).map_err(|_| PolicyError::InvalidProposal)?;
        prepared.evaluation.batch = None;
        prepared.evaluation.writes.clear();
        prepared.evaluation.event_origins.clear();
        prepared.evaluation.reads.clear();
        return Ok(prepared);
    }
    Ok(prepared)
}

fn review_event(
    store: &Store,
    project: &ProjectId,
    review: &CandidateReview,
    candidate: &Candidate,
) -> Result<(ObjectId, DomainEvent), PolicyError> {
    let assertion = store
        .observed_assertions(project, candidate.run.as_str())?
        .into_iter()
        .nth(candidate.index as usize)
        .ok_or(PolicyError::InvalidProposal)?;
    let source = store
        .source_version(project, &assertion.source)?
        .ok_or(PolicyError::InvalidProposal)?;
    let control: ObjectId = identity(
        "candidate_state",
        &[
            project.as_str(),
            candidate.run.as_str(),
            &candidate.index.to_string(),
        ],
    )?;
    let control_event = DomainEvent::PutObject {
        id: identity::<EventId>(
            "review_state_event",
            &[project.as_str(), review.id.as_str()],
        )?,
        object: control.clone(),
        kind: ObjectKind::try_from("candidate_review").map_err(|_| PolicyError::InvalidProposal)?,
        payload: review.reason.clone(),
        issue_scope: None,
        lifecycle: ObjectLifecycle::Active,
    };
    let original_object =
        ObjectId::try_from(assertion.subject.as_str()).map_err(|_| PolicyError::InvalidProposal)?;
    let original_kind = ObjectKind::try_from(assertion.predicate.as_str())
        .map_err(|_| PolicyError::InvalidProposal)?;
    let original_payload = if assertion.value == "none" {
        None
    } else {
        Some(
            PayloadId::try_from(assertion.value.as_str())
                .map_err(|_| PolicyError::InvalidProposal)?,
        )
    };
    let target = match &review.action {
        ReviewAction::Accept => Some((original_object.clone(), original_kind, original_payload)),
        ReviewAction::Reject => None,
        ReviewAction::Correct {
            object,
            kind,
            payload,
        } => Some((object.clone(), kind.clone(), payload.clone())),
    };
    if let Some((_, kind, _)) = &target
        && !crate::assertions::supported_kind(kind)
    {
        return Err(PolicyError::InvalidProposal);
    }
    if review.action != ReviewAction::Accept && review.reason.is_none() {
        return Err(PolicyError::InvalidProposal);
    }
    let event = if let Some((object, kind, payload)) = &target {
        DomainEvent::PutObject {
            id: identity(
                "review_semantic_event",
                &[project.as_str(), review.id.as_str()],
            )?,
            object: object.clone(),
            kind: kind.clone(),
            payload: payload.clone(),
            issue_scope: crate::commands::source_issue_scope(store, project, &source)?,
            lifecycle: ObjectLifecycle::Active,
        }
    } else {
        control_event.clone()
    };
    Ok((original_object, event))
}

/// Resolves a candidate through policy, returning recorded outcomes on exact retries.
///
/// A correction is a new command proposal. It never replaces the original assertion.
/// # Errors
/// Rejects changed request content, unknown candidates, and storage failures.
pub fn review_candidate(
    store: &mut Store,
    project: &ProjectId,
    review: &CandidateReview,
    now: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    if let Some(prior) = store.candidate_review(project, &review.id)? {
        if prior != *review {
            return Err(StoreError::PolicyInputConflict.into());
        }
        if let Some(record) =
            store.policy_evaluation(project, &candidate_review_evaluation(project, &review.id)?)?
        {
            return Ok(record);
        }
    }
    let prepared = prepare_candidate_review(store, project, review, now)?;
    match prepared.commit(store) {
        Ok(_) | Err(StoreError::PolicyConflict) => {}
        Err(e) => return Err(e.into()),
    }
    store
        .policy_evaluation(project, &prepared.evaluation.id)?
        .ok_or(StoreError::CorruptHistory.into())
}

/// Reports whether a candidate's original evidence and target still match.
///
/// # Errors
/// Returns errors for unavailable structural provenance or accepted history.
pub fn candidate_dependencies_current(
    store: &Store,
    project: &ProjectId,
    candidate: &Candidate,
) -> Result<bool, PolicyError> {
    if candidate.relation {
        return crate::relations::dependencies_current(store, project, candidate);
    }
    let assertion = store
        .observed_assertions(project, candidate.run.as_str())?
        .into_iter()
        .nth(candidate.index as usize)
        .ok_or(PolicyError::InvalidProposal)?;
    let object =
        ObjectId::try_from(assertion.subject.as_str()).map_err(|_| PolicyError::InvalidProposal)?;
    Ok(store.compilation_evidence_current(project, &candidate.run)?
        && store
            .object_at_revision(project, &object, candidate.basis)?
            .map(|o| o.0)
            == store.object(project, &object)?.map(|o| o.revision))
}
