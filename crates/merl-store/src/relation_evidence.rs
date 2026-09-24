//! Relation support and review work outlive the projection without changing edge content.

use super::{PolicyConflictDetail, Store, StoreError, SupportStatus};
use merl_core::{
    ActorId, CompilationRunId, EventId, PolicyDisposition, PolicyEvaluation, PolicyInput,
    PolicyInputId, ProjectId, RelationId, SourceVersionId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

// The public revalidation command promises at most 100 work items per page.
const IMPACT_PAGE_SIZE: u8 = 100;

/// Durable evidence work for one accepted relation derivation.
#[derive(Clone, Debug)]
pub struct RelationEvidenceImpact {
    /// Stable work identity, preserved after resolution.
    pub id: String,
    /// Historical semantic edge affected by the change.
    pub relation: RelationId,
    /// Accepted event whose support became stale.
    pub support_event: EventId,
    /// Compiler run behind that accepted event.
    pub run: CompilationRunId,
    /// Changed source, or the run trigger when retained compiler bytes disappeared.
    pub source: SourceVersionId,
    /// Superseding source version, if the provider exposed one.
    pub replacement: Option<SourceVersionId>,
    /// Whether some recorded compiler evidence has been erased.
    pub unavailable: bool,
    /// Whether an accepted withdrawal or fresh reviewed edge closed this work.
    pub resolved: bool,
}

/// Accepted review that closed a stale relation derivation.
#[derive(Clone, Debug)]
pub struct RelationEvidenceResolution {
    /// Policy receipt for the withdrawal or newly accepted edge.
    pub evaluation: merl_core::PolicyEvaluationId,
    /// Fresh derivation of the same triple; absent for an explicit withdrawal.
    pub replacement: Option<RelationId>,
}

/// Explicit withdrawal of stale relation support, under command authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelationWithdrawal {
    /// Client retry identity.
    pub id: PolicyInputId,
    /// Relation impact selected by the reviewer.
    pub impact: String,
    /// Actor requesting withdrawal.
    pub actor: ActorId,
}
fn identifier<T: for<'a> TryFrom<&'a str>>(s: &str) -> Result<T, StoreError> {
    T::try_from(s).map_err(|_| StoreError::CorruptHistory)
}
impl Store {
    /// Reports evidence health without changing the relation's semantic revision.
    ///
    /// # Errors
    /// Returns storage errors while reading accepted lineage.
    pub fn relation_support_status(
        &self,
        project: &ProjectId,
        id: &RelationId,
    ) -> Result<SupportStatus, StoreError> {
        let row=self.connection.query_row(
            "SELECT a.available, EXISTS(SELECT 1 FROM relation_evidence_impacts i WHERE i.project_id=e.project_id AND i.support_event_id=e.id),
              EXISTS(SELECT 1 FROM relation_evidence_resolutions x WHERE x.project_id=e.project_id AND x.support_event_id=e.id)
             FROM relation_events e JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN relation_support_availability a ON a.project_id=e.project_id AND a.event_id=e.id
             WHERE e.project_id=?1 AND e.relation_id=?2 ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
            params![project.as_str(),id.as_str()],|r|Ok((r.get::<_,Option<bool>>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?))).optional()?;
        Ok(match row {
            Some((Some(false), _, _) | (_, true, true)) => SupportStatus::Unsupported,
            Some((_, true, false)) => SupportStatus::RevalidationPending,
            _ => SupportStatus::Current,
        })
    }
    /// Finds the review that retired the relation's latest accepted derivation.
    ///
    /// # Errors
    /// Returns storage errors or damaged review identities.
    pub fn relation_evidence_resolution(
        &self,
        project: &ProjectId,
        id: &RelationId,
    ) -> Result<Option<RelationEvidenceResolution>, StoreError> {
        let row = self.connection.query_row(
            "SELECT x.evaluation_id,x.replacement_relation_id FROM relation_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN relation_evidence_resolutions x ON x.project_id=e.project_id AND x.support_event_id=e.id
             WHERE e.project_id=?1 AND e.relation_id=?2 ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
            params![project.as_str(),id.as_str()], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?))).optional()?;
        match row {
            Some((Some(evaluation), replacement)) => Ok(Some(RelationEvidenceResolution {
                evaluation: identifier(&evaluation)?,
                replacement: replacement.as_deref().map(identifier).transpose()?,
            })),
            _ => Ok(None),
        }
    }
    /// Lists at most 100 pending relation impacts after a stable lexical cursor.
    ///
    /// # Errors
    /// Returns storage errors or damaged structural identities.
    pub fn relation_impact_page(
        &self,
        project: &ProjectId,
        after: Option<&str>,
    ) -> Result<(Vec<RelationEvidenceImpact>, bool), StoreError> {
        let mut stmt = self.connection.prepare(
            "SELECT i.id
             FROM relation_evidence_impacts i
             WHERE i.project_id=?1
             AND i.id>?2
             AND NOT EXISTS(SELECT 1
             FROM relation_evidence_resolutions r
             WHERE r.project_id=i.project_id
             AND r.support_event_id=i.support_event_id)
             ORDER BY i.id
             LIMIT ?3",
        )?;
        let ids = stmt
            .query_map(
                params![project.as_str(), after.unwrap_or(""), IMPACT_PAGE_SIZE + 1],
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = ids.len() > usize::from(IMPACT_PAGE_SIZE);
        let entries = ids
            .iter()
            .take(usize::from(IMPACT_PAGE_SIZE))
            .map(|id| {
                self.relation_evidence_impact(project, id)?
                    .ok_or(StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((entries, more))
    }
    /// Reads a relation impact, including work that a later review resolved.
    ///
    /// # Errors
    /// Returns errors for damaged lineage or failed storage reads.
    pub fn relation_evidence_impact(
        &self,
        project: &ProjectId,
        id: &str,
    ) -> Result<Option<RelationEvidenceImpact>, StoreError> {
        let row=self.connection.query_row("SELECT s.relation_id,s.event_id,s.run_id,i.source_version_id,i.replacement_version_id,NOT s.available,EXISTS(SELECT 1
             FROM relation_evidence_resolutions r
             WHERE r.project_id=i.project_id
             AND r.support_event_id=i.support_event_id)
             FROM relation_evidence_impacts i
             JOIN relation_support_availability s ON s.project_id=i.project_id
             AND s.event_id=i.support_event_id
             WHERE i.project_id=?1
             AND i.id=?2",params![project.as_str(),id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,bool>(5)?,r.get::<_,bool>(6)?))).optional()?;
        row.map(
            |(relation, event, run, source, replacement, unavailable, resolved)| {
                Ok(RelationEvidenceImpact {
                    id: id.into(),
                    relation: identifier(&relation)?,
                    support_event: identifier(&event)?,
                    run: identifier(&run)?,
                    source: identifier(&source)?,
                    replacement: replacement.as_deref().map(identifier).transpose()?,
                    unavailable,
                    resolved,
                })
            },
        )
        .transpose()
    }
    /// Records a withdrawal request before policy can commit its receipt.
    ///
    /// # Errors
    /// Rejects unknown impacts and changed retry content.
    pub fn record_relation_withdrawal(
        &self,
        project: &ProjectId,
        review: &RelationWithdrawal,
    ) -> Result<(), StoreError> {
        if let Some(prior) = self.relation_withdrawal(project, &review.id)? {
            return if prior == *review {
                Ok(())
            } else {
                Err(StoreError::PolicyInputConflict)
            };
        }
        if self
            .relation_evidence_impact(project, &review.impact)?
            .is_none()
        {
            return Err(StoreError::InvalidPolicyEvaluation);
        }
        self.connection.execute(
            "INSERT INTO relation_withdrawals VALUES (?1,?2,?3,?4)",
            params![
                project.as_str(),
                review.id.as_str(),
                review.impact,
                review.actor.as_str()
            ],
        )?;
        Ok(())
    }
    /// Reads the immutable request behind a relation withdrawal receipt.
    ///
    /// # Errors
    /// Returns storage errors or invalid actor identities.
    pub fn relation_withdrawal(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<RelationWithdrawal>, StoreError> {
        let row = self
            .connection
            .query_row(
                "SELECT impact_id,actor_id
             FROM relation_withdrawals
             WHERE project_id=?1
             AND id=?2",
                params![project.as_str(), id.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(impact, actor)| {
            Ok(RelationWithdrawal {
                id: id.clone(),
                impact,
                actor: identifier(&actor)?,
            })
        })
        .transpose()
    }
}

fn insert_impact(
    tx: &Transaction<'_>,
    project: &ProjectId,
    event: &str,
    source: &str,
    replacement: Option<&str>,
) -> Result<(), StoreError> {
    let mut hash = Sha256::new();
    // An erased payload and a source literally named "unavailable" are distinct changes.
    hash.update([u8::from(replacement.is_some())]);
    for part in [project.as_str(), event, source, replacement.unwrap_or("")] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    let mut id = String::from("relation_impact_");
    for byte in hash.finalize() {
        write!(&mut id, "{byte:02x}").expect("writing a digest to a string");
    }
    tx.execute(
        "INSERT OR IGNORE INTO relation_evidence_impacts VALUES (?1,?2,?3,?4,?5)",
        params![project.as_str(), id, event, source, replacement],
    )?;
    Ok(())
}
pub(super) fn record_impacts(
    tx: &Transaction<'_>,
    project: &ProjectId,
    previous: &str,
    replacement: Option<&str>,
) -> Result<(), StoreError> {
    let mut stmt = tx.prepare(
        "SELECT s.event_id
             FROM relation_evidence_supports s
             WHERE s.project_id=?1
             AND EXISTS(SELECT 1
             FROM compilation_context_sources c
             WHERE c.project_id=s.project_id
             AND c.run_id=s.run_id
             AND c.source_version_id=?2)
             AND NOT EXISTS(SELECT 1
             FROM relation_evidence_resolutions r
             WHERE r.project_id=s.project_id
             AND r.support_event_id=s.event_id)",
    )?;
    let events = stmt
        .query_map(params![project.as_str(), previous], |r| {
            r.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for event in events {
        insert_impact(tx, project, &event, previous, replacement)?;
    }
    Ok(())
}
pub(super) fn record_unavailable(
    tx: &Transaction<'_>,
    project: &ProjectId,
) -> Result<(), StoreError> {
    let mut stmt = tx.prepare(
        "SELECT s.event_id,s.trigger
             FROM relation_support_availability s
             WHERE s.project_id=?1
             AND NOT s.available
             AND NOT EXISTS(SELECT 1
             FROM relation_evidence_resolutions r
             WHERE r.project_id=s.project_id
             AND r.support_event_id=s.event_id)",
    )?;
    let rows = stmt
        .query_map([project.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (event, source) in rows {
        insert_impact(tx, project, &event, &source, None)?;
    }
    Ok(())
}
fn insert_supports(
    tx: &Transaction<'_>,
    evaluation: Option<&PolicyEvaluation>,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT OR IGNORE INTO relation_evidence_supports
             SELECT e.project_id,e.id,e.relation_id,p.run_id,p.relation_index
             FROM relation_events e
             JOIN policy_evaluation_relation_events o ON o.project_id=e.project_id
             AND o.event_id=e.id
             JOIN policy_evaluation_inputs i ON i.project_id=o.project_id
             AND i.evaluation_id=o.evaluation_id
             AND i.input_index=o.input_index
             JOIN policy_relation_inputs p ON p.project_id=i.project_id
             AND p.evaluation_id=i.evaluation_id
             AND p.input_id=i.input_id
             WHERE (?1 IS NULL OR (o.project_id=?1
             AND o.evaluation_id=?2))",
        params![
            evaluation.map(|e| e.project.as_str()),
            evaluation.map(|e| e.id.as_str())
        ],
    )?;
    Ok(())
}
pub(super) fn record_acceptance(
    tx: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
) -> Result<(), StoreError> {
    insert_supports(tx, Some(evaluation))?;
    // A newly reviewed triple replaces stale derivations of that same edge. Current
    // independent derivations remain separate; this does not infer any new relation.
    tx.execute(
        "INSERT OR IGNORE INTO relation_evidence_resolutions
             SELECT old.project_id,old.event_id,?2,new.relation_id
             FROM relation_evidence_supports old
             JOIN relation_events prior ON prior.project_id=old.project_id
             AND prior.id=old.event_id
             JOIN relation_events new ON new.project_id=prior.project_id
             AND new.subject_id=prior.subject_id
             AND new.relation_kind=prior.relation_kind
             AND new.object_id=prior.object_id
             JOIN policy_evaluation_relation_events accepted ON accepted.project_id=new.project_id
             AND accepted.event_id=new.id
             JOIN relation_evidence_supports fresh ON fresh.project_id=new.project_id
             AND fresh.event_id=new.id
             WHERE old.project_id=?1
             AND accepted.evaluation_id=?2
             AND old.event_id!=new.id
             AND EXISTS(SELECT 1
             FROM relation_evidence_impacts i
             WHERE i.project_id=old.project_id
             AND i.support_event_id=old.event_id)
             ORDER BY new.relation_id",
        params![evaluation.project.as_str(), evaluation.id.as_str()],
    )?;
    for input in &evaluation.inputs {
        if input.disposition == PolicyDisposition::Accepted
            && let PolicyInput::Command(id) = &input.input
        {
            tx.execute(
                "INSERT INTO relation_evidence_resolutions
             SELECT i.project_id,i.support_event_id,?3,NULL
             FROM relation_withdrawals w
             JOIN relation_evidence_impacts i ON i.project_id=w.project_id
             AND i.id=w.impact_id
             WHERE w.project_id=?1
             AND w.id=?2",
                params![
                    evaluation.project.as_str(),
                    id.as_str(),
                    evaluation.id.as_str()
                ],
            )?;
        }
    }
    Ok(())
}
pub(super) fn validate_withdrawals(
    c: &Connection,
    e: &PolicyEvaluation,
) -> Result<Option<PolicyConflictDetail>, StoreError> {
    for input in &e.inputs {
        if input.disposition == PolicyDisposition::Accepted
            && let PolicyInput::Command(id) = &input.input
        {
            let resolved = c
                .query_row(
                    "SELECT EXISTS(SELECT 1
             FROM relation_evidence_resolutions r
             WHERE r.project_id=i.project_id
             AND r.support_event_id=i.support_event_id)
             FROM relation_withdrawals w
             JOIN relation_evidence_impacts i ON i.project_id=w.project_id
             AND i.id=w.impact_id
             WHERE w.project_id=?1
             AND w.id=?2",
                    params![e.project.as_str(), id.as_str()],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?;
            if resolved == Some(true) {
                return Ok(Some(PolicyConflictDetail {
                    reason_code: "accepted_input_overlap".into(),
                    target_id: Some(id.as_str().into()),
                    expected_revision: None,
                    actual_revision: None,
                }));
            }
        }
    }
    Ok(None)
}
pub(super) fn migrate(tx: &Transaction<'_>) -> Result<(), StoreError> {
    insert_supports(tx, None)?;
    // A source window may contain both an original and its correction. Only
    // versions observed after that run's newest selected version make it stale.
    let mut stmt = tx.prepare(
        "SELECT DISTINCT s.project_id,cs.source_version_id,later.id,s.event_id
             FROM relation_evidence_supports s
             JOIN compilation_context_sources cs ON cs.project_id=s.project_id
             AND cs.run_id=s.run_id
             JOIN source_versions original ON original.project_id=cs.project_id
             AND original.id=cs.source_version_id
             JOIN source_versions later ON later.project_id=original.project_id
             AND later.source_id=original.source_id
             AND later.sequence>(SELECT MAX(selected.sequence)
             FROM compilation_context_sources window
             JOIN source_versions selected ON selected.project_id=window.project_id
             AND selected.id=window.source_version_id
             WHERE window.project_id=s.project_id
             AND window.run_id=s.run_id
             AND selected.source_id=original.source_id)",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (project, source, replacement, event) in rows {
        insert_impact(
            tx,
            &identifier(&project)?,
            &event,
            &source,
            Some(&replacement),
        )?;
    }
    let mut stmt = tx.prepare(
        "SELECT id
             FROM projects",
    )?;
    let projects = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for project in projects {
        record_unavailable(tx, &identifier(&project)?)?;
    }
    Ok(())
}
