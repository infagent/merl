//! Unresolved calendar values cannot gain authority through an acceptance shortcut.

use crate::PolicyError;
use merl_core::{ObjectId, PolicyDisposition, ProjectId, temporal::TemporalValue};
use merl_store::{Candidate, Execution, Store, StructuralAssertion};

pub(super) fn unresolved(assertion: &StructuralAssertion) -> bool {
    assertion
        .temporal
        .iter()
        .any(|value| matches!(value.value, TemporalValue::Unresolved { .. }))
}

pub(super) fn review_failure(
    store: &Store,
    project: &ProjectId,
    candidate: &Candidate,
) -> Result<Option<(PolicyDisposition, &'static str)>, PolicyError> {
    let assertion = store
        .observed_assertions(project, candidate.run.as_str())?
        .into_iter()
        .nth(candidate.index as usize)
        .ok_or(PolicyError::InvalidProposal)?;
    if unresolved(&assertion) {
        return Ok(Some((PolicyDisposition::Rejected, "temporal_unresolved")));
    }
    let object =
        ObjectId::try_from(assertion.subject.as_str()).map_err(|_| PolicyError::InvalidProposal)?;
    if assertion.deferral.is_some()
        && store
            .task_state(project, &object)?
            .is_some_and(|t| t.execution != Execution::NotStarted)
    {
        return Ok(Some((
            PolicyDisposition::Rejected,
            "task_deferral_requires_not_started",
        )));
    }
    Ok(None)
}

pub(super) fn changed(
    store: &Store,
    project: &ProjectId,
    object: &ObjectId,
    assertion: &StructuralAssertion,
) -> Result<bool, PolicyError> {
    let original = store.object_assertion(project, object)?;
    let original_temporal = original.as_ref().map_or(&[][..], |a| a.temporal.as_slice());
    Ok(assertion
        .temporal
        .iter()
        .map(|t| (&t.original.role, &t.value))
        .collect::<Vec<_>>()
        != original_temporal
            .iter()
            .map(|t| (&t.original.role, &t.value))
            .collect::<Vec<_>>()
        || assertion.deferral != original.and_then(|a| a.deferral))
}
