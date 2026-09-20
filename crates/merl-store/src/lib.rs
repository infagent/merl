//! SQLite authority for accepted events and independently erasable payloads.

use std::{error::Error, fmt, path::Path};

use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, ObjectId, ObjectKind, ObjectRevision, PayloadId, ProjectId, ProjectRevision,
    ProviderIssueState, ProviderObservation, SourceBindingId, SourceId, SourceKind, SourceProvider,
    SourceVersionId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 2;

/// Failures at the local persistence boundary.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite could not complete the requested operation.
    Storage(String),
    /// The file has a schema newer than this executable understands.
    UnsupportedSchema(i64),
    /// The requested project does not exist.
    ProjectMissing,
    /// A batch has no events or refers to an unavailable payload.
    InvalidBatch,
    /// The requested payload reference does not exist in this project.
    PayloadMissing,
    /// A required structural record is inconsistent with accepted history.
    CorruptHistory,
    /// A retried source identity disagrees with the version already captured.
    SourceConflict,
    /// A source version violates causal ordering or body-retention rules.
    InvalidSource,
    /// An older provider snapshot cannot replace a newer accepted mirror.
    StaleProviderObservation,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(message) => write!(formatter, "storage error: {message}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported SQLite schema version {version}")
            }
            Self::ProjectMissing => formatter.write_str("project does not exist"),
            Self::InvalidBatch => formatter.write_str("batch has no events or invalid references"),
            Self::PayloadMissing => formatter.write_str("payload does not exist"),
            Self::CorruptHistory => formatter.write_str("accepted history is inconsistent"),
            Self::SourceConflict => {
                formatter.write_str("source identity conflicts with captured history")
            }
            Self::InvalidSource => {
                formatter.write_str("source version has invalid lineage or metadata")
            }
            Self::StaleProviderObservation => {
                formatter.write_str("provider observation is older than the accepted mirror")
            }
        }
    }
}

impl Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

/// The result of resolving a protected payload reference.
#[derive(Debug, Eq, PartialEq)]
pub enum PayloadRead {
    /// Protected bytes remain available to authorized readers.
    Available(Vec<u8>),
    /// The reference remains, but protected bytes were erased.
    Unavailable,
}

/// The current projection of one accepted object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedObject {
    /// Stable object identity.
    pub id: ObjectId,
    /// Bounded classification.
    pub kind: ObjectKind,
    /// Protected content reference, if any.
    pub payload: Option<PayloadId>,
    /// Revision of this object, independent of the project revision.
    pub revision: ObjectRevision,
    /// Project revision in which the object last changed.
    pub project_revision: ProjectRevision,
}

/// Stable project attachment for a provider namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBinding {
    /// Project-specific identity of the attachment.
    pub id: SourceBindingId,
    /// Bounded provider name.
    pub provider: SourceProvider,
    /// Upstream namespace identity, retained for provider re-query.
    pub provider_namespace_id: String,
    /// Digest of the provider's immutable repository or namespace identity.
    pub namespace_digest: [u8; 32],
}

/// Reason exact source bytes could not be captured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingSourceBody {
    /// A provider diff does not prove the old complete body.
    PriorVersionUnavailable,
    /// The provider deleted the content before capture.
    DeletedByProvider,
}

impl MissingSourceBody {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PriorVersionUnavailable => "prior_version_unavailable",
            Self::DeletedByProvider => "deleted_by_provider",
        }
    }

    fn from_str(value: &str) -> Result<Self, StoreError> {
        match value {
            "prior_version_unavailable" => Ok(Self::PriorVersionUnavailable),
            "deleted_by_provider" => Ok(Self::DeletedByProvider),
            _ => Err(StoreError::CorruptHistory),
        }
    }
}

/// One immutable source version presented to the local authority.
#[derive(Debug)]
pub struct SourceCapture<'a> {
    /// Binding that selected the source for this project.
    pub binding: SourceBinding,
    /// Stable external entity identity, scoped to the project.
    pub source: SourceId,
    /// Upstream entity ID, shared by its edits.
    pub provider_entity_id: &'a str,
    /// Stable identity of this external version.
    pub version: SourceVersionId,
    /// Upstream edit or version identity.
    pub provider_version_id: &'a str,
    /// Bounded source type.
    pub kind: SourceKind,
    /// Previous version of the same entity, if any.
    pub supersedes: Option<SourceVersionId>,
    /// A provider timestamp tie did not establish cross-stream order.
    pub ambiguous_order_with_previous: bool,
    /// Provider creation time, normalized to UTC milliseconds.
    pub created_at_millis: i64,
    /// Time this version became visible upstream.
    pub occurred_at_millis: i64,
    /// Time Merl captured this provider snapshot.
    pub observed_at_millis: i64,
    /// Stable provider actor identity when one is available.
    pub actor: Option<ActorId>,
    /// Stable upstream actor ID, when the provider exposes one.
    pub provider_actor_id: Option<&'a str>,
    /// Exact bytes, kept in the project's erasable payload scope.
    pub body: Option<&'a [u8]>,
    /// Provider edit diff, also protected because it can contain source text.
    pub edit_diff: Option<&'a [u8]>,
    /// When the provider removed this edit's retained content.
    pub edit_deleted_at_millis: Option<i64>,
    /// Why exact bytes are unavailable.
    pub missing_body_reason: Option<MissingSourceBody>,
    /// Compilation timing selected without reading the body.
    pub compilation_mode: CompilationMode,
    /// Whether this source must be processed for completeness.
    pub coverage_requirement: CoverageRequirement,
    /// Binding-policy version effective at capture.
    pub policy_version: CapturePolicyVersion,
}

/// Queryable structural metadata for one captured source version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSourceVersion {
    /// Stable version identity.
    pub id: SourceVersionId,
    /// Entity shared by all its versions.
    pub source: SourceId,
    /// Upstream entity identity, preserved separately from mutable display names.
    pub provider_entity_id: String,
    /// Upstream version identity.
    pub provider_version_id: String,
    /// Binding that selected this capture.
    pub binding: SourceBindingId,
    /// Bounded source classification.
    pub kind: SourceKind,
    /// Prior version, if this capture was an edit.
    pub supersedes: Option<SourceVersionId>,
    /// A provider timestamp tie does not establish exact cross-stream order.
    pub ambiguous_order_with_previous: bool,
    /// Project observation sequence, independent of accepted revision.
    pub sequence: u64,
    /// Provider time at which this version became visible.
    pub occurred_at_millis: i64,
    /// Provider creation time of the external entity.
    pub created_at_millis: i64,
    /// Time Merl captured the provider snapshot.
    pub observed_at_millis: i64,
    /// Stable upstream actor ID, if available.
    pub provider_actor_id: Option<String>,
    /// Effective compilation mode selected at capture.
    pub compilation_mode: CompilationMode,
    /// Effective coverage requirement selected at capture.
    pub coverage_requirement: CoverageRequirement,
    /// Version of the capture policy used for this source.
    pub policy_version: CapturePolicyVersion,
    /// Digest retained even if the source body is erased.
    pub body_digest: Option<[u8; 32]>,
    /// Why exact source bytes were unavailable at capture.
    pub missing_body_reason: Option<MissingSourceBody>,
    /// Exact bytes are resolved separately and may later be erased.
    pub payload: Option<PayloadId>,
    /// Provider diff bytes, kept in an independently erasable payload.
    pub edit_diff_payload: Option<PayloadId>,
    /// When the provider removed this edit's retained content.
    pub edit_deleted_at_millis: Option<i64>,
}

/// The accepted provider fact currently mirrored for one Issue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedProviderObservation {
    /// Typed input and source binding that justified the mirror.
    pub input: ProviderObservation,
    /// Project revision at which the fact was accepted.
    pub revision: ProjectRevision,
}

/// One SQLite connection used as a local serialized project authority.
pub struct Store {
    connection: Connection,
}

impl fmt::Debug for Store {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Opens or creates a local store and applies supported migrations.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot open the path or the schema is newer
    /// than this executable understands.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    /// Opens a local store that disappears when this value is dropped.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot initialize the in-memory database.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut connection: Connection) -> Result<Self, StoreError> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if !(0..=SCHEMA_VERSION).contains(&version) {
            return Err(StoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
            transaction.execute_batch(include_str!("../migrations/0002_sources.sql"))?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        } else if version == 1 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(include_str!("../migrations/0002_sources.sql"))?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        Ok(Self { connection })
    }

    /// Creates the accepted-history boundary for a project at revision zero.
    ///
    /// # Errors
    /// Returns an error if the project already exists or storage fails.
    pub fn create_project(&mut self, project: &ProjectId) -> Result<(), StoreError> {
        self.connection
            .execute("INSERT INTO projects (id) VALUES (?1)", [project.as_str()])?;
        Ok(())
    }

    /// Returns the revision accepted by this local authority.
    ///
    /// # Errors
    /// Returns an error if the project is missing or storage fails.
    pub fn project_revision(&self, project: &ProjectId) -> Result<ProjectRevision, StoreError> {
        let revision: Option<i64> = self
            .connection
            .query_row(
                "SELECT current_revision FROM projects WHERE id = ?1",
                [project.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let revision = revision.ok_or(StoreError::ProjectMissing)?;
        Ok(ProjectRevision::from(
            u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
        ))
    }

    /// Stores protected bytes in this project's independently erasable scope.
    ///
    /// # Errors
    /// Returns an error if the project or payload identity is invalid in storage.
    pub fn put_payload(
        &mut self,
        project: &ProjectId,
        id: &PayloadId,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let digest = Sha256::digest(bytes);
        self.connection.execute(
            "INSERT INTO payloads (project_id, id, digest, bytes) VALUES (?1, ?2, ?3, ?4)",
            params![project.as_str(), id.as_str(), digest.as_slice(), bytes],
        )?;
        Ok(())
    }

    /// Resolves protected bytes without erasing the structural reference.
    ///
    /// # Errors
    /// Returns an error if the reference does not exist or storage fails.
    pub fn read_payload(
        &self,
        project: &ProjectId,
        id: &PayloadId,
    ) -> Result<PayloadRead, StoreError> {
        let bytes: Option<Option<Vec<u8>>> = self
            .connection
            .query_row(
                "SELECT bytes FROM payloads WHERE project_id = ?1 AND id = ?2",
                params![project.as_str(), id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match bytes {
            Some(Some(bytes)) => Ok(PayloadRead::Available(bytes)),
            Some(None) => Ok(PayloadRead::Unavailable),
            None => Err(StoreError::PayloadMissing),
        }
    }

    /// Marks one project's protected bytes unavailable while retaining its digest and ID.
    ///
    /// This is the payload boundary needed for later audited purge. SQLite may
    /// retain prior pages or WAL bytes until that separate workflow runs.
    ///
    /// # Errors
    /// Returns an error if the reference does not exist or storage fails.
    pub fn erase_payload(&mut self, project: &ProjectId, id: &PayloadId) -> Result<(), StoreError> {
        let changed = self.connection.execute(
            "UPDATE payloads SET bytes = NULL, erased = 1 WHERE project_id = ?1 AND id = ?2",
            params![project.as_str(), id.as_str()],
        )?;
        if changed == 0 {
            return Err(StoreError::PayloadMissing);
        }
        Ok(())
    }

    /// Captures one immutable external version without accepting its meaning.
    ///
    /// A retry is a no-op when upstream identity, lineage, and content agree.
    /// The first capture fixes observation time and effective compilation policy;
    /// a later poll cannot rewrite them. New versions advance observation sequence
    /// independently of accepted project revision.
    ///
    /// # Errors
    /// Returns an error for a missing project, invalid lineage, conflicting
    /// retry, or failed storage transaction.
    pub fn capture_source_version(
        &mut self,
        project: &ProjectId,
        capture: &SourceCapture<'_>,
    ) -> Result<bool, StoreError> {
        if capture.created_at_millis > capture.occurred_at_millis
            || capture.occurred_at_millis > capture.observed_at_millis
            || capture.edit_deleted_at_millis.is_some_and(|deleted| {
                deleted < capture.occurred_at_millis || deleted > capture.observed_at_millis
            })
            || capture.body.is_some() == capture.missing_body_reason.is_some()
            || !valid_provider_id(&capture.binding.provider_namespace_id)
            || !valid_provider_id(capture.provider_entity_id)
            || !valid_provider_id(capture.provider_version_id)
            || capture
                .provider_actor_id
                .is_some_and(|id| !valid_provider_id(id))
            || Sha256::digest(capture.binding.provider_namespace_id.as_bytes()).as_slice()
                != capture.binding.namespace_digest
        {
            return Err(StoreError::InvalidSource);
        }
        let body_digest = capture.body.map(Sha256::digest);
        let edit_diff_digest = capture.edit_diff.map(Sha256::digest);
        let capture_digest =
            source_capture_digest(capture, body_digest.as_ref(), edit_diff_digest.as_ref());
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head: Option<i64> = transaction
            .query_row(
                "SELECT source_observation_head FROM projects WHERE id = ?1",
                [project.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let head = head.ok_or(StoreError::ProjectMissing)?;
        ensure_source_binding(&transaction, project, &capture.binding)?;
        let existing: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT capture_digest FROM source_versions WHERE project_id = ?1 AND id = ?2",
                params![project.as_str(), capture.version.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            return if existing == capture_digest {
                Ok(false)
            } else {
                Err(StoreError::SourceConflict)
            };
        }
        let previous: Option<String> = transaction
            .query_row(
                "SELECT id FROM source_versions WHERE project_id = ?1 AND source_id = ?2 ORDER BY sequence DESC LIMIT 1",
                params![project.as_str(), capture.source.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if previous.as_deref() != capture.supersedes.as_ref().map(SourceVersionId::as_str) {
            return Err(StoreError::InvalidSource);
        }
        let sequence = head.checked_add(1).ok_or(StoreError::CorruptHistory)?;
        let payload = capture
            .body
            .map(|_| PayloadId::try_from(format!("src_{}", capture.version).as_str()))
            .transpose()
            .map_err(|_| StoreError::InvalidSource)?;
        let edit_diff_payload = capture
            .edit_diff
            .map(|_| PayloadId::try_from(format!("diff_{}", capture.version).as_str()))
            .transpose()
            .map_err(|_| StoreError::InvalidSource)?;
        if let (Some(body), Some(id), Some(digest)) = (capture.body, &payload, &body_digest) {
            insert_protected_payload(&transaction, project, id, body, digest)?;
        }
        if let (Some(diff), Some(id), Some(digest)) =
            (capture.edit_diff, &edit_diff_payload, &edit_diff_digest)
        {
            insert_protected_payload(&transaction, project, id, diff, digest)?;
        }
        insert_source_version(
            &transaction,
            project,
            capture,
            sequence,
            &capture_digest,
            SourcePayloadRefs {
                body_digest: body_digest.as_ref(),
                body: payload.as_ref(),
                edit_diff_digest: edit_diff_digest.as_ref(),
                edit_diff: edit_diff_payload.as_ref(),
            },
        )?;
        transaction.execute(
            "UPDATE projects SET source_observation_head = ?2 WHERE id = ?1",
            params![project.as_str(), sequence],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Returns the latest observed source sequence for a project.
    ///
    /// # Errors
    /// Returns an error if the project is missing or storage fails.
    pub fn source_observation_head(&self, project: &ProjectId) -> Result<u64, StoreError> {
        let head: Option<i64> = self
            .connection
            .query_row(
                "SELECT source_observation_head FROM projects WHERE id = ?1",
                [project.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        u64::try_from(head.ok_or(StoreError::ProjectMissing)?)
            .map_err(|_| StoreError::CorruptHistory)
    }

    /// Resolves captured lineage without loading protected source text.
    ///
    /// # Errors
    /// Returns an error if stored structural values are invalid or storage fails.
    pub fn source_version(
        &self,
        project: &ProjectId,
        id: &SourceVersionId,
    ) -> Result<Option<StoredSourceVersion>, StoreError> {
        let row: Option<RawSourceVersion> = self
            .connection
            .query_row(
                "SELECT source_id, provider_entity_id, provider_version_id, binding_id, kind,
                    supersedes_id, ambiguous_order_with_previous, sequence,
                    occurred_at_millis, created_at_millis, observed_at_millis,
                    provider_actor_id, compilation_mode, coverage_requirement,
                    capture_policy_version, body_digest, missing_body_reason, payload_id,
                    edit_diff_payload_id, edit_deleted_at_millis
             FROM source_versions WHERE project_id = ?1 AND id = ?2",
                params![project.as_str(), id.as_str()],
                RawSourceVersion::from_row,
            )
            .optional()?;
        row.map(|row| {
            Ok(StoredSourceVersion {
                id: id.clone(),
                source: SourceId::try_from(row.source_id.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                provider_entity_id: row.provider_entity_id,
                provider_version_id: row.provider_version_id,
                binding: SourceBindingId::try_from(row.binding_id.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                kind: SourceKind::try_from(row.kind.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                supersedes: row
                    .supersedes_id
                    .map(|value| SourceVersionId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                ambiguous_order_with_previous: row.ambiguous_order_with_previous != 0,
                sequence: u64::try_from(row.sequence).map_err(|_| StoreError::CorruptHistory)?,
                occurred_at_millis: row.occurred_at_millis,
                created_at_millis: row.created_at_millis,
                observed_at_millis: row.observed_at_millis,
                provider_actor_id: row.provider_actor_id,
                compilation_mode: CompilationMode::try_from(row.compilation_mode.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                coverage_requirement: CoverageRequirement::try_from(
                    row.coverage_requirement.as_str(),
                )
                .map_err(|_| StoreError::CorruptHistory)?,
                policy_version: CapturePolicyVersion::try_from(row.capture_policy_version.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                body_digest: row
                    .body_digest
                    .map(|digest| digest.try_into().map_err(|_| StoreError::CorruptHistory))
                    .transpose()?,
                missing_body_reason: row
                    .missing_body_reason
                    .map(|reason| MissingSourceBody::from_str(&reason))
                    .transpose()?,
                payload: row
                    .payload_id
                    .map(|value| PayloadId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                edit_diff_payload: row
                    .edit_diff_payload_id
                    .map(|value| PayloadId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                edit_deleted_at_millis: row.edit_deleted_at_millis,
            })
        })
        .transpose()
    }

    /// Resolves one source capture by the project's observation sequence.
    ///
    /// # Errors
    /// Returns an error if stored identity is invalid or storage fails.
    pub fn source_version_at(
        &self,
        project: &ProjectId,
        sequence: u64,
    ) -> Result<Option<StoredSourceVersion>, StoreError> {
        let sequence = i64::try_from(sequence).map_err(|_| StoreError::InvalidSource)?;
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM source_versions WHERE project_id = ?1 AND sequence = ?2",
                params![project.as_str(), sequence],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|value| {
            let id = SourceVersionId::try_from(value.as_str())
                .map_err(|_| StoreError::CorruptHistory)?;
            self.source_version(project, &id)?
                .ok_or(StoreError::CorruptHistory)
        })
        .transpose()
    }

    /// Commits a nonempty accepted batch and its projection in one transaction.
    ///
    /// # Errors
    /// Returns an error for missing projects, invalid references, duplicate IDs,
    /// or a failed SQLite transaction. No partial revision becomes visible.
    pub fn commit(&mut self, batch: &DomainEventBatch) -> Result<ProjectRevision, StoreError> {
        self.commit_inner(batch, None)
    }

    /// Accepts a typed provider fact with its event batch in one transaction.
    ///
    /// A snapshot observed before the current provider mirror cannot replace
    /// it, even if it arrived through a valid source binding.
    ///
    /// # Errors
    /// Returns an error for a stale observation, a mismatched batch, invalid
    /// references, or a failed SQLite transaction.
    pub fn commit_provider_observation(
        &mut self,
        batch: &DomainEventBatch,
        observation: &ProviderObservation,
    ) -> Result<ProjectRevision, StoreError> {
        self.commit_inner(batch, Some(observation))
    }

    fn commit_inner(
        &mut self,
        batch: &DomainEventBatch,
        observation: Option<&ProviderObservation>,
    ) -> Result<ProjectRevision, StoreError> {
        if batch.events.is_empty() {
            return Err(StoreError::InvalidBatch);
        }
        if let Some(observation) = observation {
            validate_provider_batch(batch, observation)?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT current_revision FROM projects WHERE id = ?1",
                [batch.project.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let revision = current
            .ok_or(StoreError::ProjectMissing)?
            .checked_add(1)
            .ok_or(StoreError::CorruptHistory)?;
        if let Some(observation) = observation {
            ensure_fresh_provider_observation(&transaction, &batch.project, observation)?;
        }
        transaction.execute(
            "INSERT INTO domain_event_batches (project_id, id, revision, actor_id, occurred_at_millis) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![batch.project.as_str(), batch.id.as_str(), revision, batch.actor.as_str(), batch.occurred_at_millis],
        )?;
        for (index, event) in batch.events.iter().enumerate() {
            match event {
                DomainEvent::PutObject {
                    id,
                    object,
                    kind,
                    payload,
                } => {
                    if let Some(payload) = payload {
                        let available: Option<i64> = transaction
                            .query_row(
                                "SELECT erased FROM payloads WHERE project_id = ?1 AND id = ?2",
                                params![batch.project.as_str(), payload.as_str()],
                                |row| row.get(0),
                            )
                            .optional()?;
                        if available != Some(0) {
                            return Err(StoreError::InvalidBatch);
                        }
                    }
                    transaction.execute(
                        "INSERT INTO domain_events (project_id, batch_id, id, event_index, event_kind, object_id, object_kind, payload_id) VALUES (?1, ?2, ?3, ?4, 'put_object', ?5, ?6, ?7)",
                        params![batch.project.as_str(), batch.id.as_str(), id.as_str(), i64::try_from(index).map_err(|_| StoreError::InvalidBatch)?, object.as_str(), kind.as_str(), payload.as_ref().map(PayloadId::as_str)],
                    )?;
                    apply_put(
                        &transaction,
                        &batch.project,
                        object,
                        kind,
                        payload.as_ref(),
                        revision,
                    )?;
                }
            }
        }
        if let Some(observation) = observation {
            insert_provider_observation(&transaction, batch, observation)?;
        }
        transaction.execute(
            "UPDATE projects SET current_revision = ?2 WHERE id = ?1",
            params![batch.project.as_str(), revision],
        )?;
        transaction.commit()?;
        Ok(ProjectRevision::from(
            u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
        ))
    }

    /// Reads one current accepted object without expanding protected bytes.
    ///
    /// # Errors
    /// Returns an error if stored structural values are invalid or SQLite fails.
    pub fn object(
        &self,
        project: &ProjectId,
        id: &ObjectId,
    ) -> Result<Option<ProjectedObject>, StoreError> {
        let row: Option<(String, Option<String>, i64, i64)> = self
            .connection
            .query_row(
                "SELECT kind, payload_id, object_revision, project_revision FROM objects WHERE project_id = ?1 AND id = ?2",
                params![project.as_str(), id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        row.map(|(kind, payload, revision, project_revision)| {
            Ok(ProjectedObject {
                id: id.clone(),
                kind: ObjectKind::try_from(kind.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                payload: payload
                    .map(|value| PayloadId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                revision: ObjectRevision::try_from(
                    u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                )
                .map_err(|_| StoreError::CorruptHistory)?,
                project_revision: ProjectRevision::from(
                    u64::try_from(project_revision).map_err(|_| StoreError::CorruptHistory)?,
                ),
            })
        })
        .transpose()
    }

    /// Returns the latest accepted provider fact for an Issue mirror.
    ///
    /// # Errors
    /// Returns an error if stored structural values are invalid or storage fails.
    #[expect(
        clippy::type_complexity,
        reason = "SQLite row keeps the provider fact columns explicit"
    )]
    pub fn provider_issue_head(
        &self,
        project: &ProjectId,
        issue: &ObjectId,
    ) -> Result<Option<AcceptedProviderObservation>, StoreError> {
        let row: Option<(String, String, String, String, Option<i64>, Option<i64>, i64, i64, i64, i64)> = self
            .connection
            .query_row(
                "SELECT o.id, o.binding_id, o.issue_state, o.snapshot_payload_id,
                    o.upstream_updated_at_millis, o.closed_at_millis, o.observed_at_millis, b.revision,
                    o.label_ids_known, o.assignee_ids_known
             FROM provider_issue_heads h
             JOIN provider_observations o
               ON o.project_id = h.project_id AND o.id = h.observation_id
             JOIN domain_event_batches b
               ON b.project_id = o.project_id AND b.id = o.accepted_batch_id
             WHERE h.project_id = ?1 AND h.issue_id = ?2",
                params![project.as_str(), issue.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                    ))
                },
            )
            .optional()?;
        row.map(
            |(
                id,
                binding,
                state,
                payload,
                upstream_updated_at_millis,
                closed_at_millis,
                observed_at_millis,
                revision,
                label_ids_known,
                assignee_ids_known,
            )| {
                let label_provider_ids = provider_fact_ids(
                    &self.connection,
                    "provider_observation_labels",
                    "provider_label_id",
                    project,
                    &id,
                )?;
                let assignee_provider_ids = provider_fact_ids(
                    &self.connection,
                    "provider_observation_assignees",
                    "provider_actor_id",
                    project,
                    &id,
                )?;
                if (label_ids_known == 0 && !label_provider_ids.is_empty())
                    || (assignee_ids_known == 0 && !assignee_provider_ids.is_empty())
                {
                    return Err(StoreError::CorruptHistory);
                }
                Ok(AcceptedProviderObservation {
                    input: ProviderObservation {
                        id: merl_core::PolicyInputId::try_from(id.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        binding: SourceBindingId::try_from(binding.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        issue: issue.clone(),
                        state: ProviderIssueState::try_from(state.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        upstream_updated_at_millis,
                        closed_at_millis,
                        label_provider_ids: (label_ids_known == 1).then_some(label_provider_ids),
                        assignee_provider_ids: (assignee_ids_known == 1)
                            .then_some(assignee_provider_ids),
                        snapshot_payload: PayloadId::try_from(payload.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        observed_at_millis,
                    },
                    revision: ProjectRevision::from(
                        u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                    ),
                })
            },
        )
        .transpose()
    }

    /// Records a later identical provider snapshot without creating a new revision.
    ///
    /// The last-seen time prevents an older, different snapshot from replacing
    /// a value the provider reconfirmed more recently.
    ///
    /// # Errors
    /// Returns an error if the Issue mirror is missing or storage fails.
    pub fn note_provider_seen(
        &mut self,
        project: &ProjectId,
        issue: &ObjectId,
        observed_at_millis: i64,
    ) -> Result<(), StoreError> {
        let changed = self.connection.execute(
            "UPDATE provider_issue_heads
             SET last_seen_at_millis = MAX(last_seen_at_millis, ?3)
             WHERE project_id = ?1 AND issue_id = ?2",
            params![project.as_str(), issue.as_str(), observed_at_millis],
        )?;
        if changed == 0 {
            return Err(StoreError::CorruptHistory);
        }
        Ok(())
    }

    /// Rebuilds the disposable object projection from the accepted event log.
    ///
    /// # Errors
    /// Returns an error if event history is missing, inconsistent, or unreadable.
    pub fn rebuild_projection(&mut self, project: &ProjectId) -> Result<(), StoreError> {
        let expected_revision = self.project_revision(project)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM objects WHERE project_id = ?1",
            [project.as_str()],
        )?;
        let mut statement = transaction.prepare(
            "SELECT b.revision, e.object_id, e.object_kind, e.payload_id
             FROM domain_events e JOIN domain_event_batches b
             ON e.project_id = b.project_id AND e.batch_id = b.id
             WHERE e.project_id = ?1 ORDER BY b.revision, e.event_index",
        )?;
        let rows = statement.query_map([project.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut latest_revision = 0_i64;
        for row in rows {
            let (revision, object, kind, payload) = row?;
            latest_revision = revision;
            let object =
                ObjectId::try_from(object.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            let kind =
                ObjectKind::try_from(kind.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            let payload = payload
                .map(|value| PayloadId::try_from(value.as_str()))
                .transpose()
                .map_err(|_| StoreError::CorruptHistory)?;
            apply_put(
                &transaction,
                project,
                &object,
                &kind,
                payload.as_ref(),
                revision,
            )?;
        }
        drop(statement);
        if u64::try_from(latest_revision).map_err(|_| StoreError::CorruptHistory)?
            != expected_revision.get()
        {
            return Err(StoreError::CorruptHistory);
        }
        transaction.commit()?;
        Ok(())
    }

    /// Counts accepted events without reading their protected payloads.
    ///
    /// # Errors
    /// Returns an error if SQLite cannot read the event log.
    pub fn accepted_event_count(&self, project: &ProjectId) -> Result<u64, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM domain_events WHERE project_id = ?1",
            [project.as_str()],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
    }
}

fn validate_provider_batch(
    batch: &DomainEventBatch,
    observation: &ProviderObservation,
) -> Result<(), StoreError> {
    let [
        DomainEvent::PutObject {
            object,
            kind,
            payload: Some(payload),
            ..
        },
    ] = batch.events.as_slice()
    else {
        return Err(StoreError::InvalidBatch);
    };
    if object != &observation.issue
        || kind.as_str() != "provider_issue"
        || payload != &observation.snapshot_payload
        || batch.occurred_at_millis != observation.observed_at_millis
        || (observation.state == ProviderIssueState::Open && observation.closed_at_millis.is_some())
        || observation
            .upstream_updated_at_millis
            .is_some_and(|updated| updated > observation.observed_at_millis)
        || observation
            .closed_at_millis
            .is_some_and(|closed| closed > observation.observed_at_millis)
        || observation
            .label_provider_ids
            .iter()
            .flatten()
            .any(|id| !valid_provider_id(id))
        || observation
            .assignee_provider_ids
            .iter()
            .flatten()
            .any(|id| !valid_provider_id(id))
    {
        return Err(StoreError::InvalidBatch);
    }
    for ids in [
        &observation.label_provider_ids,
        &observation.assignee_provider_ids,
    ] {
        let mut distinct = std::collections::HashSet::new();
        if !ids.iter().flatten().all(|id| distinct.insert(id)) {
            return Err(StoreError::InvalidBatch);
        }
    }
    Ok(())
}

fn ensure_fresh_provider_observation(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    observation: &ProviderObservation,
) -> Result<(), StoreError> {
    let prior: Option<(i64, Option<i64>)> = transaction
        .query_row(
            "SELECT h.last_seen_at_millis, o.upstream_updated_at_millis
         FROM provider_issue_heads h JOIN provider_observations o
         ON o.project_id = h.project_id AND o.id = h.observation_id
         WHERE h.project_id = ?1 AND h.issue_id = ?2",
            params![project.as_str(), observation.issue.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if prior.is_some_and(|(seen, updated)| {
        seen >= observation.observed_at_millis
            || (updated.is_some() && observation.upstream_updated_at_millis.is_none())
            || updated
                .zip(observation.upstream_updated_at_millis)
                .is_some_and(|(previous, incoming)| incoming <= previous)
    }) {
        return Err(StoreError::StaleProviderObservation);
    }
    Ok(())
}

fn insert_provider_observation(
    transaction: &Transaction<'_>,
    batch: &DomainEventBatch,
    observation: &ProviderObservation,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO provider_observations (
            project_id, id, binding_id, issue_id, issue_state,
            upstream_updated_at_millis, closed_at_millis,
            label_ids_known, assignee_ids_known,
            snapshot_payload_id, observed_at_millis, accepted_batch_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            batch.project.as_str(),
            observation.id.as_str(),
            observation.binding.as_str(),
            observation.issue.as_str(),
            observation.state.as_str(),
            observation.upstream_updated_at_millis,
            observation.closed_at_millis,
            i64::from(observation.label_provider_ids.is_some()),
            i64::from(observation.assignee_provider_ids.is_some()),
            observation.snapshot_payload.as_str(),
            observation.observed_at_millis,
            batch.id.as_str()
        ],
    )?;
    for label in observation.label_provider_ids.iter().flatten() {
        transaction.execute(
            "INSERT INTO provider_observation_labels (project_id, observation_id, provider_label_id) VALUES (?1, ?2, ?3)",
            params![batch.project.as_str(), observation.id.as_str(), label],
        )?;
    }
    for actor in observation.assignee_provider_ids.iter().flatten() {
        transaction.execute(
            "INSERT INTO provider_observation_assignees (project_id, observation_id, provider_actor_id) VALUES (?1, ?2, ?3)",
            params![batch.project.as_str(), observation.id.as_str(), actor],
        )?;
    }
    transaction.execute(
        "INSERT INTO provider_issue_heads (project_id, issue_id, observation_id, observed_at_millis, last_seen_at_millis)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(project_id, issue_id) DO UPDATE SET
           observation_id = excluded.observation_id,
           observed_at_millis = excluded.observed_at_millis,
           last_seen_at_millis = excluded.last_seen_at_millis",
        params![batch.project.as_str(), observation.issue.as_str(), observation.id.as_str(), observation.observed_at_millis],
    )?;
    Ok(())
}

fn provider_fact_ids(
    connection: &Connection,
    table: &str,
    column: &str,
    project: &ProjectId,
    observation_id: &str,
) -> Result<Vec<String>, StoreError> {
    let query = format!(
        "SELECT {column} FROM {table} WHERE project_id = ?1 AND observation_id = ?2 ORDER BY {column}"
    );
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map(params![project.as_str(), observation_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
}

fn apply_put(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    object: &ObjectId,
    kind: &ObjectKind,
    payload: Option<&PayloadId>,
    revision: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO objects (project_id, id, kind, payload_id, object_revision, project_revision)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)
         ON CONFLICT(project_id, id) DO UPDATE SET
           kind = excluded.kind,
           payload_id = excluded.payload_id,
           object_revision = objects.object_revision + 1,
           project_revision = excluded.project_revision",
        params![
            project.as_str(),
            object.as_str(),
            kind.as_str(),
            payload.map(PayloadId::as_str),
            revision
        ],
    )?;
    Ok(())
}

fn ensure_source_binding(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    binding: &SourceBinding,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT OR IGNORE INTO source_bindings (project_id, id, provider, provider_namespace_id, namespace_digest)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![project.as_str(), binding.id.as_str(), binding.provider.as_str(), binding.provider_namespace_id, binding.namespace_digest.as_slice()],
    )?;
    let stored: (String, String, Vec<u8>) = transaction.query_row(
        "SELECT provider, provider_namespace_id, namespace_digest FROM source_bindings WHERE project_id = ?1 AND id = ?2",
        params![project.as_str(), binding.id.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if stored.0 != binding.provider.as_str()
        || stored.1 != binding.provider_namespace_id
        || stored.2 != binding.namespace_digest
    {
        return Err(StoreError::SourceConflict);
    }
    Ok(())
}

fn insert_protected_payload(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    id: &PayloadId,
    bytes: &[u8],
    digest: &sha2::digest::Output<Sha256>,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO payloads (project_id, id, digest, bytes) VALUES (?1, ?2, ?3, ?4)",
        params![project.as_str(), id.as_str(), digest.as_slice(), bytes],
    )?;
    Ok(())
}

#[derive(Clone, Copy)]
struct SourcePayloadRefs<'a> {
    body_digest: Option<&'a sha2::digest::Output<Sha256>>,
    body: Option<&'a PayloadId>,
    edit_diff_digest: Option<&'a sha2::digest::Output<Sha256>>,
    edit_diff: Option<&'a PayloadId>,
}

fn insert_source_version(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    capture: &SourceCapture<'_>,
    sequence: i64,
    capture_digest: &[u8; 32],
    payloads: SourcePayloadRefs<'_>,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO source_versions (
             project_id, id, source_id, provider_entity_id, provider_version_id, binding_id, kind, supersedes_id, ambiguous_order_with_previous, sequence,
             occurred_at_millis, created_at_millis, observed_at_millis, actor_id, provider_actor_id,
             body_digest, edit_diff_digest, capture_digest, payload_id, edit_diff_payload_id, edit_deleted_at_millis, missing_body_reason,
             compilation_mode, coverage_requirement, capture_policy_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
        params![
            project.as_str(), capture.version.as_str(), capture.source.as_str(),
            capture.provider_entity_id, capture.provider_version_id,
            capture.binding.id.as_str(), capture.kind.as_str(),
            capture.supersedes.as_ref().map(SourceVersionId::as_str), i64::from(capture.ambiguous_order_with_previous), sequence,
            capture.occurred_at_millis, capture.created_at_millis, capture.observed_at_millis,
            capture.actor.as_ref().map(ActorId::as_str), capture.provider_actor_id,
            payloads.body_digest.map(AsRef::<[u8]>::as_ref),
            payloads.edit_diff_digest.map(AsRef::<[u8]>::as_ref),
            capture_digest.as_slice(), payloads.body.map(PayloadId::as_str),
            payloads.edit_diff.map(PayloadId::as_str), capture.edit_deleted_at_millis,
            capture.missing_body_reason.map(MissingSourceBody::as_str),
            capture.compilation_mode.as_str(), capture.coverage_requirement.as_str(),
            capture.policy_version.as_str()
        ],
    )?;
    Ok(())
}

struct RawSourceVersion {
    source_id: String,
    provider_entity_id: String,
    provider_version_id: String,
    binding_id: String,
    kind: String,
    supersedes_id: Option<String>,
    ambiguous_order_with_previous: i64,
    sequence: i64,
    occurred_at_millis: i64,
    created_at_millis: i64,
    observed_at_millis: i64,
    provider_actor_id: Option<String>,
    compilation_mode: String,
    coverage_requirement: String,
    capture_policy_version: String,
    body_digest: Option<Vec<u8>>,
    missing_body_reason: Option<String>,
    payload_id: Option<String>,
    edit_diff_payload_id: Option<String>,
    edit_deleted_at_millis: Option<i64>,
}

impl RawSourceVersion {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            source_id: row.get(0)?,
            provider_entity_id: row.get(1)?,
            provider_version_id: row.get(2)?,
            binding_id: row.get(3)?,
            kind: row.get(4)?,
            supersedes_id: row.get(5)?,
            ambiguous_order_with_previous: row.get(6)?,
            sequence: row.get(7)?,
            occurred_at_millis: row.get(8)?,
            created_at_millis: row.get(9)?,
            observed_at_millis: row.get(10)?,
            provider_actor_id: row.get(11)?,
            compilation_mode: row.get(12)?,
            coverage_requirement: row.get(13)?,
            capture_policy_version: row.get(14)?,
            body_digest: row.get(15)?,
            missing_body_reason: row.get(16)?,
            payload_id: row.get(17)?,
            edit_diff_payload_id: row.get(18)?,
            edit_deleted_at_millis: row.get(19)?,
        })
    }
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn source_capture_digest(
    capture: &SourceCapture<'_>,
    body_digest: Option<&sha2::digest::Output<Sha256>>,
    edit_diff_digest: Option<&sha2::digest::Output<Sha256>>,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    for value in [
        capture.binding.id.as_str(),
        capture.binding.provider_namespace_id.as_str(),
        capture.source.as_str(),
        capture.provider_entity_id,
        capture.version.as_str(),
        capture.provider_version_id,
        capture.kind.as_str(),
        capture
            .supersedes
            .as_ref()
            .map_or("", SourceVersionId::as_str),
        capture.actor.as_ref().map_or("", ActorId::as_str),
        capture.provider_actor_id.unwrap_or(""),
        capture
            .missing_body_reason
            .map_or("", MissingSourceBody::as_str),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    // Poll time and effective policy belong to the first capture. A later
    // poll can see the same immutable version under a newer binding policy.
    for value in [capture.created_at_millis, capture.occurred_at_millis] {
        digest.update(value.to_be_bytes());
    }
    digest.update([u8::from(capture.ambiguous_order_with_previous)]);
    if let Some(body_digest) = body_digest {
        digest.update(body_digest);
    }
    digest.update([u8::from(edit_diff_digest.is_some())]);
    if let Some(edit_diff_digest) = edit_diff_digest {
        digest.update(edit_diff_digest);
    }
    digest.update([u8::from(capture.edit_deleted_at_millis.is_some())]);
    if let Some(deleted_at) = capture.edit_deleted_at_millis {
        digest.update(deleted_at.to_be_bytes());
    }
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::{SCHEMA_VERSION, Store, StoreError};
    use rusqlite::Connection;

    #[test]
    fn a_newer_schema_is_rejected_before_migration() {
        let connection = Connection::open_in_memory().expect("open SQLite");
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("set future version");

        let error = Store::from_connection(connection).expect_err("reject future schema");
        assert!(matches!(error, StoreError::UnsupportedSchema(3)));
    }

    #[test]
    fn an_existing_kernel_store_gains_source_capture_without_losing_its_revision() {
        let connection = Connection::open_in_memory().expect("open SQLite");
        connection
            .execute_batch(include_str!("../migrations/0001_initial.sql"))
            .expect("create kernel schema");
        connection
            .execute(
                "INSERT INTO projects (id, current_revision) VALUES ('P1', 7)",
                [],
            )
            .expect("create existing project");
        connection
            .pragma_update(None, "user_version", 1)
            .expect("mark prior schema");

        let store = Store::from_connection(connection).expect("migrate source schema");
        let project = merl_core::ProjectId::try_from("P1").expect("project ID");
        assert_eq!(store.project_revision(&project).expect("revision").get(), 7);
        assert_eq!(store.source_observation_head(&project).expect("head"), 0);
    }
}
