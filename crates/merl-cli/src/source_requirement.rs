//! Derive source requirement requests while preserving pre-framing policy receipts.
//!
//! Accepted requirements are read by the caller before this boundary. Rejected
//! receipts still matter here: later grants must not change an exact retry's outcome.

use crate::{CliError, stable_id};
use merl_core::{
    ActorId, BatchId, DomainEvent, ObjectKind, ObjectLifecycle, PolicyEvaluationId, ProjectId,
};
use merl_policy::{PolicyError, Proposal, apply_current};
use merl_store::{CoverageRequirementIntent, RecordedPolicyEvaluation, Store, StoreError};
use sha2::{Digest, Sha256};

/// Separates this encoding from other request formats; changing it changes retry IDs.
const IDENTITY_DOMAIN: &[u8] = b"merl.source-requirement/v2\0";

/// Applies a framed request, or returns a matching immutable receipt from the old CLI.
pub(super) fn apply(
    store: &mut Store,
    project: &ProjectId,
    actor: &ActorId,
    intent: &CoverageRequirementIntent,
    reason: &str,
    now: i64,
) -> Result<RecordedPolicyEvaluation, CliError> {
    let parts = [
        project.as_str(),
        intent.source.as_str(),
        intent.scope.as_str(),
        actor.as_str(),
        reason,
    ];
    let legacy = Request::new(intent, &parts.join("/"))?;
    // Only read existing legacy receipts. New work must never reserve an ambiguous ID.
    if store
        .policy_evaluation(project, &legacy.evaluation)?
        .is_some()
    {
        match legacy.apply(store, project, actor, now) {
            Ok(record) => return Ok(record),
            // A different tuple may own the same old ID. The policy boundary checks
            // the actor and proposal digest before allowing reuse of its receipt.
            Err(PolicyError::Store(StoreError::PolicyInputConflict)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Request::new(intent, &identity(parts))?.apply(store, project, actor, now)?)
}

/// Frames UTF-8 fields with fixed-width byte lengths before hashing.
fn identity(parts: [&str; 5]) -> String {
    let mut hash = Sha256::new();
    hash.update(IDENTITY_DOMAIN);
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    crate::digest_text(&hash.finalize().into())
}

#[derive(Debug)]
struct Request {
    evaluation: PolicyEvaluationId,
    batch: BatchId,
    proposal: Proposal,
}

impl Request {
    fn new(intent: &CoverageRequirementIntent, identity: &str) -> Result<Self, CliError> {
        Ok(Self {
            evaluation: stable_id("source_requirement_evaluation", identity)?,
            batch: stable_id("source_requirement_batch", identity)?,
            proposal: Proposal::AdministrativeAction {
                id: stable_id("source_requirement_input", identity)?,
                event: DomainEvent::PutObject {
                    id: stable_id("source_requirement_event", identity)?,
                    object: intent.object.clone(),
                    kind: ObjectKind::try_from("source_coverage_requirement")
                        .map_err(|error| crate::invalid_input(&error.to_string()))?,
                    payload: Some(intent.reason.clone()),
                    issue_scope: Some(intent.scope.clone()),
                    lifecycle: ObjectLifecycle::Active,
                },
            },
        })
    }

    fn apply(
        self,
        store: &mut Store,
        project: &ProjectId,
        actor: &ActorId,
        now: i64,
    ) -> Result<RecordedPolicyEvaluation, PolicyError> {
        apply_current(
            store,
            project,
            actor,
            self.evaluation,
            self.batch,
            now,
            &[self.proposal],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::identity;
    use std::collections::BTreeSet;

    #[test]
    fn request_fields_and_their_boundaries_affect_identity() {
        let original = ["P1", "S1", "issue-1/alice", "alice", "Review evidence"];
        let mut identities = BTreeSet::from([identity(original)]);
        // Public promotion deduplication can hide a missing actor or reason field.
        // Test the encoding itself so those fields cannot disappear unnoticed.
        for (index, replacement) in ["P2", "S2", "issue-2/alice", "bob", "Other evidence"]
            .into_iter()
            .enumerate()
        {
            let mut changed = original;
            changed[index] = replacement;
            assert!(
                identities.insert(identity(changed)),
                "field {index} was lost"
            );
        }
        for parts in [
            ["P1", "S1", "issue-1", "alice", "alice/Review evidence"],
            ["P1", "S1", "é/alice", "alice", "理由/one"],
            ["P1", "S1", "é", "alice", "alice/理由/one"],
        ] {
            assert!(
                identities.insert(identity(parts)),
                "field boundaries were lost"
            );
        }
    }
}
