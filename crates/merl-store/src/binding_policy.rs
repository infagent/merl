//! Accepted control objects select binding policy without rewriting captured evidence.

use merl_core::compilation_policy::{
    ActorClass, IssueSourceKind, PolicySelection, PolicyValues, SelectionRule, Selectors,
};
use merl_core::{CapturePolicyVersion, ObjectId, PayloadId, ProjectId, SourceBindingId};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::{BindingCapturePolicy, ProjectedObject, Store, StoreError};

/// Current defaults and selectors, with the accepted object that selected them.
#[derive(Clone, Debug)]
pub struct BindingPolicyState {
    /// Defaults for subsequent observations of this binding.
    pub policy: BindingCapturePolicy,
    /// Bounded rules and author mappings selected by the same accepted version.
    pub selectors: Selectors,
    /// Absent for the initial capture policy; present after an accepted change.
    pub change: Option<ProjectedObject>,
}

impl Store {
    /// Captures live Issue evidence and its policy explanation in one transaction.
    ///
    /// The authority resolves current rules under the writer lock. Capture flags
    /// cannot override an established binding, and retries keep their first selection.
    ///
    /// # Errors
    /// Returns an error for invalid evidence, missing compiler configuration for eager
    /// work, missing bindings, or storage failure.
    pub fn capture_selected_source(
        &mut self,
        project: &ProjectId,
        mut capture: crate::SourceCapture<'_>,
        provider_class: ActorClass,
        compiler_available: bool,
    ) -> Result<bool, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let defaults = super::read_capture_policy(&transaction, project, &capture.binding.id)?
            .ok_or(StoreError::BindingMissing)?;
        let selectors = read_selectors(&transaction, project, &capture.binding.id)?;
        let kind = match capture.kind.as_str() {
            "issue" => Some(IssueSourceKind::Issue),
            "issue_comment" => Some(IssueSourceKind::IssueComment),
            _ => None,
        };
        let (mut values, mut selection) = selectors.resolve(
            PolicyValues {
                mode: defaults.mode,
                coverage: defaults.coverage,
            },
            kind,
            capture.provider_source_author_id,
            provider_class,
        );
        if capture.kind.as_str() == "observed_deletion" {
            values.mode = merl_core::CompilationMode::CaptureOnly;
            selection.rule = SelectionRule::ObservedDeletion;
        }
        if values.mode == merl_core::CompilationMode::Eager && !compiler_available {
            return Err(StoreError::InvalidCompilation);
        }
        capture.compilation_mode = values.mode;
        capture.coverage_requirement = values.coverage;
        capture.policy_version = defaults.version;
        let inserted = super::capture_in_transaction(&transaction, project, &capture, true)?;
        if inserted {
            let selection =
                serde_json::to_string(&selection).map_err(|_| StoreError::CorruptHistory)?;
            transaction.execute(
                "INSERT INTO source_policy_selections VALUES (?1,?2,?3)",
                params![project.as_str(), capture.version.as_str(), selection],
            )?;
        }
        transaction.commit()?;
        Ok(inserted)
    }

    /// Reads a capture's immutable explanation without opening its protected body.
    ///
    /// Older captures and offline imports have no recorded selector explanation.
    ///
    /// # Errors
    /// Returns an error for corrupt selection metadata or failed storage access.
    pub fn source_policy_selection(
        &self,
        project: &ProjectId,
        version: &merl_core::SourceVersionId,
    ) -> Result<Option<PolicySelection>, StoreError> {
        let value: Option<String> = self.connection.query_row("SELECT selection FROM source_policy_selections WHERE project_id=?1 AND source_version_id=?2", params![project.as_str(),version.as_str()], |row| row.get(0)).optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|_| StoreError::CorruptHistory))
            .transpose()
    }

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
        let selectors = read_selectors(&transaction, project, binding)?;
        transaction.commit()?;
        Ok(BindingPolicyState {
            policy,
            selectors,
            change,
        })
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
        selectors: &Selectors,
    ) -> Result<ObjectId, StoreError> {
        self.capture_policy(project, binding)?
            .ok_or(StoreError::BindingMissing)?;
        let object = binding_policy_object(binding)?;
        let selectors = serde_json::to_string(selectors).map_err(|_| StoreError::CorruptHistory)?;
        self.connection.execute(
            "INSERT OR IGNORE INTO binding_policy_changes VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                project.as_str(),
                reason.as_str(),
                binding.as_str(),
                object.as_str(),
                policy.mode.as_str(),
                policy.coverage.as_str(),
                policy.version.as_str(),
                expected.as_str(),
                selectors
            ],
        )?;
        let matches: bool = self.connection.query_row(
            "SELECT binding_id=?3 AND object_id=?4 AND compilation_mode=?5 AND coverage_requirement=?6 AND policy_version=?7 AND expected_version=?8 AND selectors=?9 FROM binding_policy_changes WHERE project_id=?1 AND reason_payload_id=?2",
            params![project.as_str(),reason.as_str(),binding.as_str(),object.as_str(),policy.mode.as_str(),policy.coverage.as_str(),policy.version.as_str(),expected.as_str(),selectors], |row| row.get(0),
        )?;
        if !matches {
            return Err(StoreError::PolicyInputConflict);
        }
        Ok(object)
    }
}

fn read_selectors(
    connection: &Connection,
    project: &ProjectId,
    binding: &SourceBindingId,
) -> Result<Selectors, StoreError> {
    let value: Option<String> = connection.query_row(
        "SELECT change.selectors FROM binding_policy_changes change JOIN objects object
         ON object.project_id=change.project_id AND object.id=change.object_id AND object.payload_id=change.reason_payload_id
         WHERE change.project_id=?1 AND change.binding_id=?2 AND object.kind='binding_compilation_policy' AND object.lifecycle='active'",
        params![project.as_str(),binding.as_str()], |row| row.get(0),
    ).optional()?;
    value.map_or_else(
        || Ok(Selectors::default()),
        |value| serde_json::from_str(&value).map_err(|_| StoreError::CorruptHistory),
    )
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
