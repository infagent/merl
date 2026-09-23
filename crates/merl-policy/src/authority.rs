//! Administrative grant changes use the same policy, revision, and inbox path as other work.

use std::fmt::Write as _;

use merl_core::{
    ActorId, AuthorityPermission, BatchId, DomainEvent, EventId, ObjectKind, ObjectLifecycle,
    PayloadId, PolicyEvaluationId, PolicyInputId, ProjectId,
};
use merl_store::{RecordedPolicyEvaluation, Store, StoreError};
use sha2::{Digest, Sha256};

use crate::{PolicyError, Proposal, apply_current, recorded_outcome};

/// One retryable request to grant or revoke semantic authority.
#[derive(Clone, Debug)]
pub struct AuthorityChange {
    /// Client identity reused only for identical request content.
    pub id: PolicyInputId,
    /// Acting administrator; a trusted client claim in local mode.
    pub actor: ActorId,
    /// Actor whose semantic permission changes.
    pub subject: ActorId,
    /// Decision-author or command-actor authority.
    pub permission: AuthorityPermission,
    /// `true` grants the permission; `false` revokes it.
    pub grant: bool,
    /// Audit explanation stored in an erasable payload.
    pub reason: String,
}

/// Evaluates and records an administrative grant change, or returns its original result.
///
/// Retrying an accepted grant after revocation does not restore authority. A rejected
/// request needs a new ID to receive a fresh evaluation after policy changes.
///
/// # Errors
/// Rejects empty reasons, changed retry content, stale dependencies, and storage failures.
pub fn change_authority(
    store: &mut Store,
    project: &ProjectId,
    change: &AuthorityChange,
    now_millis: i64,
) -> Result<RecordedPolicyEvaluation, PolicyError> {
    if change.reason.trim().is_empty() {
        return Err(PolicyError::InvalidProposal);
    }
    let object = store.prepare_authority_grant(project, &change.subject, change.permission)?;
    let identity = digest_parts(&[project.as_str(), change.id.as_str()]);
    let evaluation = PolicyEvaluationId::try_from(format!("grant_eval_{identity}").as_str())
        .map_err(|_| PolicyError::InvalidProposal)?;
    let payload = PayloadId::try_from(
        format!(
            "grant_reason_{}",
            digest_parts(&[&identity, &change.reason])
        )
        .as_str(),
    )
    .map_err(|_| PolicyError::InvalidProposal)?;
    let proposal = Proposal::AdministrativeAction {
        id: PolicyInputId::try_from(format!("grant_input_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
        event: DomainEvent::PutObject {
            id: EventId::try_from(format!("grant_event_{identity}").as_str())
                .map_err(|_| PolicyError::InvalidProposal)?,
            object,
            kind: ObjectKind::try_from("authority_grant")
                .map_err(|_| PolicyError::InvalidProposal)?,
            payload: Some(payload.clone()),
            issue_scope: None,
            lifecycle: if change.grant {
                ObjectLifecycle::Active
            } else {
                ObjectLifecycle::Invalidated
            },
        },
    };
    if let Some(record) = recorded_outcome(
        store,
        project,
        &change.actor,
        &evaluation,
        std::slice::from_ref(&proposal),
    )? {
        return Ok(record);
    }
    match store.read_payload(project, &payload) {
        Err(StoreError::PayloadMissing) => {
            store.put_payload(project, &payload, change.reason.as_bytes())?;
        }
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    apply_current(
        store,
        project,
        &change.actor,
        evaluation.clone(),
        BatchId::try_from(format!("grant_batch_{identity}").as_str())
            .map_err(|_| PolicyError::InvalidProposal)?,
        now_millis,
        &[proposal],
    )
}

fn digest_parts(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize()
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
