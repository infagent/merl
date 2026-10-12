//! Freshness advances independently of accepted provider facts. Sightings name
//! the fact they reconfirm, so a concurrent fact change cannot inherit stale proof.

use merl_core::{ObjectId, PolicyInputId, ProjectId};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{Store, StoreError};

impl Store {
    /// Records a reconfirmation and its upstream activity time without accepting a revision.
    ///
    /// The caller must compare all provider facts except the activity timestamp.
    /// `observation` pins that comparison to the immutable accepted input. Both the
    /// local seen time and upstream watermark must remain monotonic.
    ///
    /// # Errors
    /// Returns a conflict if the accepted observation changed, a stale-observation
    /// error for regressed freshness, or an error for invalid times or storage failure.
    pub fn note_provider_sighting(
        &mut self,
        project: &ProjectId,
        issue: &ObjectId,
        observation: &PolicyInputId,
        seen_at_millis: i64,
        upstream_updated_at_millis: Option<i64>,
    ) -> Result<(), StoreError> {
        if upstream_updated_at_millis.is_some_and(|updated| updated > seen_at_millis) {
            return Err(StoreError::InvalidBatch);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head: Option<(String, i64, Option<i64>)> = transaction
            .query_row(
                "SELECT observation_id, last_seen_at_millis, upstream_updated_at_millis
             FROM provider_issue_freshness WHERE project_id=?1 AND issue_id=?2",
                params![project.as_str(), issue.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (accepted, seen, updated) = head.ok_or(StoreError::CorruptHistory)?;
        if accepted != observation.as_str() {
            return Err(StoreError::PolicyConflict);
        }
        if seen_at_millis < seen
            || (updated.is_some() && upstream_updated_at_millis < updated)
            || (seen_at_millis == seen && upstream_updated_at_millis != updated)
        {
            return Err(StoreError::StaleProviderObservation);
        }
        transaction.execute(
            "INSERT OR IGNORE INTO provider_issue_sightings
             (project_id,issue_id,observation_id,seen_at_millis,upstream_updated_at_millis)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                project.as_str(),
                issue.as_str(),
                observation.as_str(),
                seen_at_millis,
                upstream_updated_at_millis
            ],
        )?;
        transaction.execute(
            "UPDATE provider_issue_heads SET last_seen_at_millis=?3
             WHERE project_id=?1 AND issue_id=?2",
            params![project.as_str(), issue.as_str(), seen_at_millis],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sightings_keep_the_accepted_watermark_after_upgrade() {
        let store = Store::open_in_memory().unwrap();
        // Recreate only the prior sighting schema; the migration must not rewrite
        // the accepted input or require timestamps absent from old sightings.
        store.connection.execute_batch(
            "DROP VIEW provider_issue_freshness;
             DROP INDEX provider_sighting_watermarks;
             ALTER TABLE provider_issue_sightings DROP COLUMN upstream_updated_at_millis;
             INSERT INTO projects(id,current_revision) VALUES ('P1',1);
             INSERT INTO source_bindings(project_id,id,provider,provider_namespace_id,namespace_digest)
               VALUES ('P1','binding','github','repo',zeroblob(32));
             INSERT INTO payloads(project_id,id,digest,bytes) VALUES ('P1','snapshot',zeroblob(32),X'7B7D');
             INSERT INTO domain_event_batches(project_id,id,revision,actor_id,occurred_at_millis)
               VALUES ('P1','batch',1,'provider',10);
             INSERT INTO provider_observations
               (project_id,id,binding_id,issue_id,issue_state,upstream_updated_at_millis,
                label_ids_known,assignee_ids_known,snapshot_payload_id,observed_at_millis,accepted_batch_id)
               VALUES ('P1','observation','binding','I1','open',8,0,0,'snapshot',10,'batch');
             INSERT INTO provider_issue_heads VALUES ('P1','I1','observation',10,12);
             INSERT INTO provider_issue_sightings VALUES ('P1','I1','observation',12);
             PRAGMA user_version=30;",
        ).unwrap();
        let mut migrated = Store::from_connection(store.connection).unwrap();
        let project = ProjectId::try_from("P1").unwrap();
        let issue = ObjectId::try_from("I1").unwrap();
        let before = migrated
            .provider_issue_head(&project, &issue)
            .unwrap()
            .unwrap();
        assert_eq!(before.input.upstream_updated_at_millis, Some(8));
        assert_eq!(before.latest_upstream_updated_at_millis, Some(8));
        assert_eq!(before.last_seen_at_millis, 12);
        migrated
            .note_provider_sighting(&project, &issue, &before.input.id, 15, Some(14))
            .unwrap();
        let after = migrated
            .provider_issue_head(&project, &issue)
            .unwrap()
            .unwrap();
        assert_eq!(after.input, before.input);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.latest_upstream_updated_at_millis, Some(14));
        assert_eq!(after.last_seen_at_millis, 15);
    }
}
