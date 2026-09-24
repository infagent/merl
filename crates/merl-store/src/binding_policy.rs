//! Accepted control objects select binding policy without rewriting captured evidence.

use merl_core::{CapturePolicyVersion, ObjectId, PayloadId, ProjectId, SourceBindingId};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::{BindingCapturePolicy, ProjectedObject, Store, StoreError};

/// Current binding defaults and the accepted object that selected them.
#[derive(Clone, Debug)]
pub struct BindingPolicyState {
    /// Defaults for subsequent observations of this binding.
    pub policy: BindingCapturePolicy,
    /// Absent for the initial capture policy; present after an accepted change.
    pub change: Option<ProjectedObject>,
}

impl Store {
    /// Reads effective policy and its accepted provenance from one snapshot.
    ///
    /// # Errors
    /// Returns an error for an unknown binding, corrupt history, or unreadable storage.
    pub fn binding_policy(
        &self,
        project: &ProjectId,
        binding: &SourceBindingId,
    ) -> Result<BindingPolicyState, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        let policy = super::read_capture_policy(&transaction, project, binding)?
            .ok_or(StoreError::BindingMissing)?;
        let change = self.object(project, &binding_policy_object(binding)?)?;
        transaction.commit()?;
        Ok(BindingPolicyState { policy, change })
    }

    /// Retains a typed policy proposal without accepting it.
    ///
    /// Each reason reference identifies one immutable proposal, including the expected
    /// version. Rejected proposals cannot influence the binding's effective policy.
    ///
    /// # Errors
    /// Returns an error for an unknown binding, changed identity content, or storage failure.
    pub fn prepare_binding_policy(
        &self,
        project: &ProjectId,
        binding: &SourceBindingId,
        policy: &BindingCapturePolicy,
        expected: &CapturePolicyVersion,
        reason: &PayloadId,
    ) -> Result<ObjectId, StoreError> {
        self.capture_policy(project, binding)?
            .ok_or(StoreError::BindingMissing)?;
        let object = binding_policy_object(binding)?;
        self.connection.execute(
            "INSERT OR IGNORE INTO binding_policy_changes VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                project.as_str(),
                reason.as_str(),
                binding.as_str(),
                object.as_str(),
                policy.mode.as_str(),
                policy.coverage.as_str(),
                policy.version.as_str(),
                expected.as_str()
            ],
        )?;
        let matches: bool = self.connection.query_row(
            "SELECT binding_id=?3 AND object_id=?4 AND compilation_mode=?5 AND coverage_requirement=?6 AND policy_version=?7 AND expected_version=?8 FROM binding_policy_changes WHERE project_id=?1 AND reason_payload_id=?2",
            params![project.as_str(),reason.as_str(),binding.as_str(),object.as_str(),policy.mode.as_str(),policy.coverage.as_str(),policy.version.as_str(),expected.as_str()], |row| row.get(0),
        )?;
        if !matches {
            return Err(StoreError::PolicyInputConflict);
        }
        Ok(object)
    }
}

/// Returns the stable control object for one binding's accepted policy.
///
/// # Errors
/// Returns an error if the derived identity violates the structural ID contract.
pub fn binding_policy_object(binding: &SourceBindingId) -> Result<ObjectId, StoreError> {
    ObjectId::try_from(
        format!(
            "binding_policy_{}",
            Sha256::digest(binding.as_str().as_bytes()).iter().fold(
                String::new(),
                |mut text, byte| {
                    let _ = write!(text, "{byte:02x}");
                    text
                }
            )
        )
        .as_str(),
    )
    .map_err(|_| StoreError::CorruptHistory)
}

pub(super) fn accepted_policy(
    connection: &Connection,
    project: &ProjectId,
    binding: &SourceBindingId,
) -> Result<Option<(String, String, String)>, StoreError> {
    Ok(connection.query_row(
        "SELECT change.compilation_mode,change.coverage_requirement,change.policy_version
         FROM binding_policy_changes change JOIN objects object
         ON object.project_id=change.project_id AND object.id=change.object_id AND object.payload_id=change.reason_payload_id
         WHERE change.project_id=?1 AND change.binding_id=?2 AND object.kind='binding_compilation_policy' AND object.lifecycle='active'",
        params![project.as_str(),binding.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?)
}

pub(super) fn validate_target(
    connection: &Connection,
    project: &ProjectId,
    object: &ObjectId,
    reason: &PayloadId,
) -> Result<(), StoreError> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM binding_policy_changes WHERE project_id=?1 AND object_id=?2 AND reason_payload_id=?3)",
        params![project.as_str(),object.as_str(),reason.as_str()], |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::InvalidBatch)
    }
}
