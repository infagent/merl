//! Applies recorded assertions without accepting caller-supplied authors or events.

use crate::{
    PolicyError, PolicyRules, PreparedPolicy, Proposal, evaluate_current, recorded_outcome,
};
use merl_core::{
    ActorId, BatchId, CompilationRunId, DomainEvent, EventId, ObjectId, ObjectKind,
    ObjectLifecycle, PayloadId, PolicyDisposition, PolicyEvaluationId, PolicyInputId, ProjectId,
};
use merl_store::{PayloadRead, RecordedPolicyEvaluation, Store, StoreError};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

pub(super) fn identity<T: for<'a> TryFrom<&'a str>>(
    prefix: &str,
    parts: &[&str],
) -> Result<T, PolicyError> {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    let mut digest = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(digest, "{byte:02x}").map_err(|_| PolicyError::InvalidProposal)?;
    }
    T::try_from(format!("{prefix}_{digest}").as_str()).map_err(|_| PolicyError::InvalidProposal)
}

fn proposals(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    request: &PolicyInputId,
) -> Result<Vec<Proposal>, PolicyError> {
    let status = store
        .compilation_run_status(project, run.as_str())?
        .ok_or(PolicyError::InvalidProposal)?;
    if status.mode != "live" || !status.completed || !status.succeeded || status.needs_context {
        return Err(PolicyError::RunIneligible);
    }
    store
        .observed_assertions(project, run.as_str())?
        .into_iter()
        .enumerate()
        .map(|(index, assertion)| {
            let source = store
                .source_version(project, &assertion.source)?
                .ok_or(PolicyError::InvalidProposal)?;
            let index = u32::try_from(index).map_err(|_| PolicyError::InvalidProposal)?;
            let position = index.to_string();
            Ok(Proposal::ObservedAssertion {
                id: identity::<PolicyInputId>(
                    "assertion",
                    &[project.as_str(), run.as_str(), &position],
                )?,
                run: run.clone(),
                index,
                event: DomainEvent::PutObject {
                    id: identity::<EventId>(
                        "assertion_event",
                        &[project.as_str(), request.as_str(), run.as_str(), &position],
                    )?,
                    object: ObjectId::try_from(assertion.subject.as_str())
                        .map_err(|_| PolicyError::InvalidProposal)?,
                    kind: ObjectKind::try_from(assertion.predicate.as_str())
                        .map_err(|_| PolicyError::InvalidProposal)?,
                    payload: if assertion.value == "none" {
                        None
                    } else {
                        Some(
                            PayloadId::try_from(assertion.value.as_str())
                                .map_err(|_| PolicyError::InvalidProposal)?,
                        )
                    },
                    issue_scope: crate::commands::source_issue_scope(store, project, &source)?,
                    lifecycle: ObjectLifecycle::Active,
                },
            })
        })
        .collect()
}

/// Prepares one run's assertions against current state and durable grants.
///
/// The request ID identifies this evaluation attempt. Input identities depend on
/// the project, run, and assertion index, so a new attempt cannot accept an input twice.
/// An empty successful run returns `None` and creates no policy evaluation.
///
/// # Errors
/// Rejects non-live or incomplete runs, missing provenance, and invalid stored identities.
pub fn prepare_assertions(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    actor: &ActorId,
    request: &PolicyInputId,
    now_millis: i64,
) -> Result<Option<PreparedPolicy>, PolicyError> {
    let proposals = proposals(store, project, run, request)?;
    if proposals.is_empty() {
        return Ok(None);
    }
    evaluate_current(
        store,
        project,
        actor,
        identity::<PolicyEvaluationId>("assertion_eval", &[project.as_str(), request.as_str()])?,
        identity::<BatchId>("assertion_batch", &[project.as_str(), request.as_str()])?,
        now_millis,
        &proposals,
    )
    .map(Some)
}

/// Commits one run's dispositions and accepted events in one policy transaction.
///
/// Identical retries return the recorded outcome, including conflicts. A new request
/// ID reevaluates candidates under current grants without repeating extraction.
///
/// # Errors
/// Rejects changed request content, ineligible runs, and storage failures.
pub fn apply_assertions(
    store: &mut Store,
    project: &ProjectId,
    run: &CompilationRunId,
    actor: &ActorId,
    request: &PolicyInputId,
    now_millis: i64,
) -> Result<Option<RecordedPolicyEvaluation>, PolicyError> {
    let inputs = proposals(store, project, run, request)?;
    let evaluation =
        identity::<PolicyEvaluationId>("assertion_eval", &[project.as_str(), request.as_str()])?;
    if let Some(record) = recorded_outcome(store, project, actor, &evaluation, &inputs)? {
        return Ok(Some(record));
    }
    let Some(prepared) = prepare_assertions(store, project, run, actor, request, now_millis)?
    else {
        return Ok(None);
    };
    match prepared.commit(store) {
        Ok(_) | Err(StoreError::PolicyConflict) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(Some(
        store
            .policy_evaluation(project, &evaluation)?
            .ok_or(StoreError::CorruptHistory)?,
    ))
}

pub(super) fn disposition(
    store: &Store,
    project: &ProjectId,
    rules: &PolicyRules,
    run: &CompilationRunId,
    index: u32,
    event: &DomainEvent,
) -> Result<(PolicyDisposition, &'static str), PolicyError> {
    let status = store
        .compilation_run_status(project, run.as_str())?
        .ok_or(PolicyError::InvalidProposal)?;
    if !status.succeeded || status.needs_context {
        return Err(PolicyError::InvalidProposal);
    }
    let assertion = store
        .observed_assertions(project, run.as_str())?
        .into_iter()
        .nth(index as usize)
        .ok_or(PolicyError::InvalidProposal)?;
    let source = store
        .source_version(project, &assertion.source)?
        .ok_or(PolicyError::InvalidProposal)?;
    let DomainEvent::PutObject {
        object,
        kind,
        payload,
        issue_scope,
        lifecycle,
        ..
    } = event
    else {
        return Err(PolicyError::InvalidProposal);
    };
    let expected_scope = crate::commands::source_issue_scope(store, project, &source)?;
    if assertion.subject != object.as_str()
        || assertion.predicate != kind.as_str()
        || assertion.value != payload.as_ref().map_or("none", PayloadId::as_str)
        || source.source_author.as_ref().map(ActorId::as_str) != assertion.asserted_by.as_deref()
        || issue_scope
            .as_deref()
            .is_some_and(|scope| Some(scope) != expected_scope.as_deref())
        || *lifecycle != ObjectLifecycle::Active
    {
        return Err(PolicyError::InvalidProposal);
    }
    if let Some(candidate) = store.candidate(
        project,
        &identity::<PolicyInputId>(
            "assertion",
            &[project.as_str(), run.as_str(), &index.to_string()],
        )?,
    )? && let Some(resolution) = store.candidate_resolution(project, &candidate)?
    {
        return Ok(if resolution == "accept" {
            (PolicyDisposition::Duplicate, "assertion_already_accepted")
        } else {
            (PolicyDisposition::Rejected, "candidate_resolved")
        });
    }
    if store.accepted_assertion(project, run, index)? {
        return Ok((PolicyDisposition::Duplicate, "assertion_already_accepted"));
    }
    if !store.compilation_evidence_current(project, run)? {
        return Ok((PolicyDisposition::Rejected, "assertion_evidence_changed"));
    }
    if !supported_kind(kind) {
        return Ok((
            PolicyDisposition::Rejected,
            "unsupported_assertion_predicate",
        ));
    }
    if let Some(disposition) = supplemental_disposition(store, project, &assertion)? {
        return Ok(disposition);
    }
    if let Some(current) = store.object(project, object)?
        && (current.kind != *kind || current.issue_scope != *issue_scope)
    {
        return Ok((PolicyDisposition::Rejected, "assertion_target_mismatch"));
    }
    if let Some(payload) = payload {
        match store.read_payload(project, payload) {
            Ok(PayloadRead::Available(_)) => {}
            Ok(PayloadRead::Unavailable) | Err(StoreError::PayloadMissing) => {
                return Ok((PolicyDisposition::Rejected, "assertion_value_unavailable"));
            }
            Err(error) => return Err(error.into()),
        }
    }
    // Attribution from a compiler cannot authenticate a quoted decision, even when
    // the person quoting it also holds a grant. Evaluate the original source instead.
    if assertion.attributed_to.is_some()
        || source
            .source_author
            .as_ref()
            .is_none_or(|author| !rules.decision_authors.contains(author))
    {
        return Ok((PolicyDisposition::Candidate, "direct_authority_unverified"));
    }
    if kind.as_str() == "decision"
        && assertion.act == "request"
        && assertion.epistemic_basis == "reported"
        && assertion.polarity == "positive"
    {
        Ok((PolicyDisposition::Accepted, "authorized_explicit_decision"))
    } else {
        Ok((PolicyDisposition::Candidate, "outside_decision_authority"))
    }
}

pub(super) fn supported_kind(kind: &ObjectKind) -> bool {
    matches!(
        kind.as_str(),
        "decision"
            | "requirement"
            | "question"
            | "fact"
            | "blocker"
            | "finding"
            | "hypothesis"
            | "claim"
            | "task"
            | "experiment"
            | "next_action"
    )
}

fn supplemental_disposition(
    store: &Store,
    project: &ProjectId,
    assertion: &merl_store::StructuralAssertion,
) -> Result<Option<(PolicyDisposition, &'static str)>, PolicyError> {
    if let Some(command) = store.source_command(project, &assertion.source)? {
        let accepted = store
            .accepted_policy_input(
                project,
                &merl_core::PolicyInput::Command(command.id.clone()),
            )?
            .is_some();
        // A different subject may repeat the originating act or express a separate one.
        // Preserve that ambiguity for review instead of discarding project intent.
        if accepted
            && assertion.predicate == command.kind.as_str()
            && assertion.act == "request"
            && assertion.polarity == "positive"
        {
            return Ok(Some(if assertion.subject == command.object.as_str() {
                (PolicyDisposition::Duplicate, "covered_by_command")
            } else {
                (
                    PolicyDisposition::Candidate,
                    "possible_supplemental_duplicate",
                )
            }));
        }
        // Notes may report new evidence, but a compiler cannot replace the object
        // whose accepted transition the note explains. Corrections require review.
        if accepted && assertion.subject == command.object.as_str() {
            return Ok(Some((
                PolicyDisposition::Candidate,
                "supplemental_correction",
            )));
        }
    }
    Ok(None)
}
