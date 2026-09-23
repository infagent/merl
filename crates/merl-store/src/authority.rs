//! Structural grant targets and a consistent snapshot of effective authority.
//!
//! Targets alone carry no permission. Active accepted objects supply the grants;
//! revocation invalidates those objects and projection rebuild restores their state.

use merl_core::{ActorId, AuthorityPermission, ObjectId, ProjectId, ProjectRevision};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::{Store, StoreError};

/// Effective project grants and the revision needed to guard policy preparation.
#[derive(Clone, Debug)]
pub struct AuthorityGrants {
    /// Source authors allowed to make explicit decisions.
    pub decision_authors: Vec<ActorId>,
    /// Actors allowed to submit semantic commands.
    pub command_actors: Vec<ActorId>,
    /// Administrators established during project bootstrap.
    pub administrators: Vec<ActorId>,
    /// Latest accepted change to the grant collection, including revocations.
    pub revision: ProjectRevision,
}

impl Store {
    /// Reads all authority grants from one SQLite snapshot.
    ///
    /// # Errors
    /// Returns an error for an absent project, corrupt identities, or unreadable storage.
    pub fn authority_grants(&self, project: &ProjectId) -> Result<AuthorityGrants, StoreError> {
        let transaction = self.connection.unchecked_transaction()?;
        self.project_revision(project)?;
        let administrators = self.administrators(project)?;
        let mut result = AuthorityGrants {
            decision_authors: Vec::new(),
            command_actors: Vec::new(),
            administrators,
            revision: ProjectRevision::from(0),
        };
        let mut statement = transaction.prepare(
            "SELECT target.actor_id,target.permission FROM authority_grant_targets target
             JOIN objects object ON object.project_id=target.project_id AND object.id=target.object_id
             WHERE target.project_id=?1 AND object.kind='authority_grant' AND object.lifecycle='active'
             ORDER BY target.actor_id,target.permission",
        )?;
        let rows = statement.query_map([project.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (actor, permission) = row?;
            let actor =
                ActorId::try_from(actor.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            match AuthorityPermission::try_from(permission.as_str())
                .map_err(|_| StoreError::CorruptHistory)?
            {
                AuthorityPermission::DecisionAuthor => result.decision_authors.push(actor),
                AuthorityPermission::CommandActor => result.command_actors.push(actor),
            }
        }
        let revision: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(project_revision),0) FROM objects WHERE project_id=?1 AND kind='authority_grant'",
            [project.as_str()], |row| row.get(0),
        )?;
        result.revision =
            ProjectRevision::from(u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?);
        drop(statement);
        transaction.commit()?;
        Ok(result)
    }

    /// Registers a structural grant target without changing anyone's authority.
    ///
    /// The caller proposes an administrative event for this object. Policy acceptance
    /// activates or invalidates it; rejected proposals leave only an inert target.
    ///
    /// # Errors
    /// Returns an error for an absent project or unavailable storage.
    pub fn prepare_authority_grant(
        &self,
        project: &ProjectId,
        actor: &ActorId,
        permission: AuthorityPermission,
    ) -> Result<ObjectId, StoreError> {
        self.project_revision(project)?;
        let mut hash = Sha256::new();
        for part in [project.as_str(), actor.as_str(), permission.as_str()] {
            hash.update((part.len() as u64).to_be_bytes());
            hash.update(part.as_bytes());
        }
        let object = ObjectId::try_from(
            format!(
                "grant_{}",
                hash.finalize()
                    .iter()
                    .fold(String::new(), |mut text, byte| {
                        let _ = write!(text, "{byte:02x}");
                        text
                    })
            )
            .as_str(),
        )
        .map_err(|_| StoreError::CorruptHistory)?;
        self.connection.execute(
            "INSERT OR IGNORE INTO authority_grant_targets (project_id,object_id,actor_id,permission)
             VALUES (?1,?2,?3,?4)",
            params![project.as_str(),object.as_str(),actor.as_str(),permission.as_str()],
        )?;
        Ok(object)
    }
}
