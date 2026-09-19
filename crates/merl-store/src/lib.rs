//! SQLite authority for accepted events and independently erasable payloads.

use std::{error::Error, fmt, path::Path};

use merl_core::{
    DomainEvent, DomainEventBatch, ObjectId, ObjectKind, ObjectRevision, PayloadId, ProjectId,
    ProjectRevision,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 1;

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
        if version != 0 && version != SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
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

    /// Commits a nonempty accepted batch and its projection in one transaction.
    ///
    /// # Errors
    /// Returns an error for missing projects, invalid references, duplicate IDs,
    /// or a failed SQLite transaction. No partial revision becomes visible.
    pub fn commit(&mut self, batch: &DomainEventBatch) -> Result<ProjectRevision, StoreError> {
        if batch.events.is_empty() {
            return Err(StoreError::InvalidBatch);
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
        assert!(matches!(error, StoreError::UnsupportedSchema(2)));
    }
}
