//! Relation review grants authority to an edge without borrowing either endpoint's grant.

use crate::assertions::identity;
use crate::{PolicyError, PreparedPolicy, Proposal, evaluate_current};
use merl_core::{
    BatchId, CompilationRunId, DomainEvent, EventId, ObjectId, ObjectKind, ObjectLifecycle,
    PolicyDisposition, PolicyInputId, ProjectId, Relation, RelationId, RelationKind,
};
use merl_store::{
    Candidate, CandidateReview, RelationBasis, ReviewAction, Store, StructuralRelation,
};

fn recorded(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    index: u32,
) -> Result<StructuralRelation, PolicyError> {
    store
        .observed_relations(project, run)?
        .into_iter()
        .nth(index as usize)
        .ok_or(PolicyError::InvalidProposal)
}
fn event(
    project: &ProjectId,
    run: &CompilationRunId,
    index: u32,
    request: &PolicyInputId,
    r: &StructuralRelation,
) -> Result<DomainEvent, PolicyError> {
    let position = index.to_string();
    Ok(DomainEvent::PutRelation {
        id: identity::<EventId>(
            "relation_event",
            &[project.as_str(), request.as_str(), run.as_str(), &position],
        )?,
        relation: Relation {
            id: identity::<RelationId>("relation", &[project.as_str(), run.as_str(), &position])?,
            project: project.clone(),
            subject: ObjectId::try_from(r.subject.as_deref().unwrap_or("invalid"))
                .map_err(|_| PolicyError::InvalidProposal)?,
            kind: RelationKind::try_from(r.predicate.as_deref().unwrap_or("invalid"))
                .map_err(|_| PolicyError::InvalidProposal)?,
            object: ObjectId::try_from(r.object.as_deref().unwrap_or("invalid"))
                .map_err(|_| PolicyError::InvalidProposal)?,
        },
    })
}
pub(super) fn proposals(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    request: &PolicyInputId,
) -> Result<Vec<Proposal>, PolicyError> {
    store
        .observed_relations(project, run)?
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let index = u32::try_from(i).map_err(|_| PolicyError::InvalidProposal)?;
            Ok(Proposal::ObservedRelation {
                id: identity(
                    "relation_input",
                    &[project.as_str(), run.as_str(), &index.to_string()],
                )?,
                run: run.clone(),
                index,
                event: event(project, run, index, request, r)?,
            })
        })
        .collect()
}
pub(super) fn disposition(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    index: u32,
    proposed: &DomainEvent,
) -> Result<(PolicyDisposition, &'static str), PolicyError> {
    let r = recorded(store, project, run, index)?;
    let expected = event(project, run, index, &identity("check", &[])?, &r)?;
    if let (
        DomainEvent::PutRelation { relation: a, .. },
        DomainEvent::PutRelation { relation: b, .. },
    ) = (proposed, &expected)
    {
        if a != b {
            return Err(PolicyError::InvalidProposal);
        }
    } else {
        return Err(PolicyError::InvalidProposal);
    }
    if let Some(reason) = r.rejection {
        return Ok((
            PolicyDisposition::Rejected,
            match reason.as_str() {
                "invalid_relation_identifier" => "invalid_relation_identifier",
                _ => "ungrounded_relation_endpoint",
            },
        ));
    }
    if !matches!(
        r.predicate.as_deref(),
        Some(
            "supports"
                | "disputes"
                | "answers"
                | "addresses"
                | "updates"
                | "depends_on"
                | "blocks"
                | "supersedes"
        )
    ) {
        return Ok((
            PolicyDisposition::Rejected,
            "unsupported_relation_predicate",
        ));
    }
    if let Some(c) = store.candidate(
        project,
        &identity(
            "relation_input",
            &[project.as_str(), run.as_str(), &index.to_string()],
        )?,
    )? && let Some(action) = store.candidate_resolution(project, &c)?
    {
        return Ok(if action == "accept" {
            (PolicyDisposition::Duplicate, "relation_already_accepted")
        } else {
            (PolicyDisposition::Rejected, "candidate_resolved")
        });
    }
    if !store.compilation_evidence_current(project, run)? {
        return Ok((PolicyDisposition::Rejected, "relation_evidence_changed"));
    }
    Ok((PolicyDisposition::Candidate, "relation_requires_review"))
}

pub(super) fn dependencies_current(
    store: &Store,
    project: &ProjectId,
    c: &Candidate,
) -> Result<bool, PolicyError> {
    if !store.compilation_evidence_current(project, &c.run)? {
        return Ok(false);
    }
    let r = recorded(store, project, &c.run, c.index)?;
    for (endpoint, basis) in [(&r.subject, &r.subject_basis), (&r.object, &r.object_basis)] {
        let endpoint = ObjectId::try_from(endpoint.as_deref().ok_or(PolicyError::InvalidProposal)?)
            .map_err(|_| PolicyError::InvalidProposal)?;
        let Some(current) = store.object(project, &endpoint)? else {
            return Ok(false);
        };
        match basis {
            Some(RelationBasis::Object(revision)) if current.revision == *revision => {}
            Some(RelationBasis::Assertion(index)) => {
                if !store.accepted_assertion(project, &c.run, *index)? {
                    return Ok(false);
                }
                let origin = store.object_policy_origin(project, &endpoint)?;
                let Some(origin) = origin else {
                    return Ok(false);
                };
                let direct = matches!(&origin.input,merl_core::PolicyInput::ObservedAssertion{run,index:i,..} if run==&c.run && i==index);
                let reviewed = if let merl_core::PolicyInput::Command(id) = &origin.input {
                    if let Some(review) = store.candidate_review(project, id)? {
                        review.action == ReviewAction::Accept
                            && store
                                .candidate(project, &review.candidate)?
                                .is_some_and(|value| {
                                    !value.relation && value.run == c.run && value.index == *index
                                })
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !direct && !reviewed {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

pub(super) fn build_review(
    store: &Store,
    project: &ProjectId,
    review: &CandidateReview,
    c: &Candidate,
    now: i64,
) -> Result<PreparedPolicy, PolicyError> {
    if matches!(review.action, ReviewAction::Correct { .. }) {
        return Err(PolicyError::InvalidCommand(
            "relation candidates support accept or reject",
        ));
    }
    let r = recorded(store, project, &c.run, c.index)?;
    let semantic = review.action == ReviewAction::Accept;
    let current = dependencies_current(store, project, c)?;
    // A stale or missing endpoint still needs a durable conflict receipt. Use a control
    // proposal for that evaluation; remove its event before returning the conflict.
    let proposed = if semantic && current {
        event(project, &c.run, c.index, &review.id, &r)?
    } else {
        DomainEvent::PutObject {
            id: identity(
                "relation_review_event",
                &[project.as_str(), review.id.as_str()],
            )?,
            object: identity("relation_review", &[project.as_str(), c.id.as_str()])?,
            kind: ObjectKind::try_from("candidate_review")
                .map_err(|_| PolicyError::InvalidProposal)?,
            payload: review.reason.clone(),
            issue_scope: None,
            lifecycle: ObjectLifecycle::Active,
        }
    };
    let mut prepared = evaluate_current(
        store,
        project,
        &review.actor,
        crate::candidate_review_evaluation(project, &review.id)?,
        identity::<BatchId>("review_batch", &[project.as_str(), review.id.as_str()])?,
        now,
        &[Proposal::Command {
            id: review.id.clone(),
            event: proposed,
        }],
    )?;
    if prepared.evaluation.inputs[0].disposition == PolicyDisposition::Accepted {
        let failure = if store.candidate_resolution(project, c)?.is_some() {
            Some("candidate_already_resolved")
        } else if semantic && !current {
            Some("relation_dependencies_changed")
        } else {
            None
        };
        if let Some(reason) = failure {
            prepared.evaluation.inputs[0].disposition = PolicyDisposition::Conflict;
            prepared.evaluation.inputs[0].reason = merl_core::ReasonCode::try_from(reason)
                .map_err(|_| PolicyError::InvalidProposal)?;
            prepared.evaluation.batch = None;
            prepared.evaluation.writes.clear();
            prepared.evaluation.event_origins.clear();
            prepared.evaluation.reads.clear();
        }
    }
    Ok(prepared)
}
