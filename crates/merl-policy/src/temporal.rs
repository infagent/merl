//! Guard temporal acceptance and preserve planning meaning during support refreshes.

use crate::PolicyError;
use merl_core::{
    ObjectId, PolicyDisposition, ProjectId,
    temporal::{Span, TemporalValue},
};
use merl_store::{Candidate, Execution, PayloadRead, Store, StructuralAssertion};

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
    if assertion
        .temporal
        .iter()
        .map(|t| (&t.original.role, &t.value))
        .collect::<Vec<_>>()
        != original_temporal
            .iter()
            .map(|t| (&t.original.role, &t.value))
            .collect::<Vec<_>>()
    {
        return Ok(true);
    }
    match (
        original
            .as_ref()
            .and_then(|a| a.deferral.as_ref().map(|d| (a, d))),
        &assertion.deferral,
    ) {
        (None, None) => Ok(false),
        (Some((original, old)), Some(new)) if old.accepted == new.accepted => {
            // Offsets locate evidence; equal offsets across edits do not establish
            // equal meaning. Compare the semantic origin, even after support refreshes.
            let old_reason = reason_bytes(store, project, original, &old.reason)?;
            let new_reason = reason_bytes(store, project, assertion, &new.reason)?;
            Ok(old_reason.is_none() || new_reason.is_none() || old_reason != new_reason)
        }
        _ => Ok(true),
    }
}

/// Resolve only the reason span; erased or missing evidence cannot prove equality.
fn reason_bytes(
    store: &Store,
    project: &ProjectId,
    assertion: &StructuralAssertion,
    span: &Span,
) -> Result<Option<Vec<u8>>, PolicyError> {
    let Some(payload) = store
        .source_version(project, &assertion.source)?
        .and_then(|s| s.payload)
    else {
        return Ok(None);
    };
    let PayloadRead::Available(bytes) = store.read_payload(project, &payload)? else {
        return Ok(None);
    };
    Ok(bytes.get(span.start..span.end).map(<[u8]>::to_vec))
}
