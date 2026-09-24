//! Immutable relation proposals retain endpoint evidence separately from accepted edges.

use super::{
    PolicyConflictDetail, Store, StoreError, compilation_evidence_current, valid_record_id,
};
use merl_core::{
    CompilationRunId, ObjectRevision, PolicyDisposition, PolicyEvaluation, PolicyInput,
    PolicyInputId, ProjectId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Evidence that lets a compiler name an object endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelationBasis {
    /// Source-grounded assertion at this index in the same run.
    Assertion(u32),
    /// Accepted object revision selected into the recorded context.
    Object(ObjectRevision),
}

/// One compiler relation, including a bounded rejection for invalid endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuralRelation {
    /// Validated subject identifier; absent if its wire value was invalid.
    pub subject: Option<String>,
    /// Validated relation predicate.
    pub predicate: Option<String>,
    /// Validated object identifier; absent if its wire value was invalid.
    pub object: Option<String>,
    /// Immutable grounding for the subject.
    pub subject_basis: Option<RelationBasis>,
    /// Immutable grounding for the object.
    pub object_basis: Option<RelationBasis>,
    /// Structural reason why this relation cannot enter policy as a candidate.
    pub rejection: Option<String>,
}

fn basis(
    assertion: Option<u32>,
    revision: Option<i64>,
) -> Result<Option<RelationBasis>, StoreError> {
    match (assertion, revision) {
        (Some(i), None) => Ok(Some(RelationBasis::Assertion(i))),
        (None, Some(r)) => Ok(Some(RelationBasis::Object(
            ObjectRevision::try_from(u64::try_from(r).map_err(|_| StoreError::CorruptHistory)?)
                .map_err(|_| StoreError::CorruptHistory)?,
        ))),
        (None, None) => Ok(None),
        _ => Err(StoreError::CorruptHistory),
    }
}
impl Store {
    /// Reads structural relations even after response payload erasure.
    ///
    /// # Errors
    /// Returns an error for damaged metadata or failed storage reads.
    pub fn observed_relations(
        &self,
        project: &ProjectId,
        run: &CompilationRunId,
    ) -> Result<Vec<StructuralRelation>, StoreError> {
        let mut stmt=self.connection.prepare("SELECT subject_id,predicate,object_id,subject_assertion,subject_revision,object_assertion,object_revision,rejection FROM observed_relations WHERE project_id=?1 AND run_id=?2 ORDER BY relation_index")?;
        let rows = stmt.query_map(params![project.as_str(), run.as_str()], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        })?;
        rows.map(|row| {
            let (subject, predicate, object, sa, sr, oa, or, rejection) = row?;
            Ok(StructuralRelation {
                subject,
                predicate,
                object,
                subject_basis: basis(sa, sr)?,
                object_basis: basis(oa, or)?,
                rejection,
            })
        })
        .collect()
    }
}

pub(super) fn insert(
    tx: &Transaction<'_>,
    project: &ProjectId,
    run: &str,
    relations: &[StructuralRelation],
) -> Result<(), StoreError> {
    for (index, r) in relations.iter().enumerate() {
        if [&r.subject, &r.predicate, &r.object, &r.rejection]
            .into_iter()
            .flatten()
            .any(|s| !valid_record_id(s))
        {
            return Err(StoreError::InvalidCompilation);
        }
        let columns = |b: &Option<RelationBasis>| -> Result<_, StoreError> {
            Ok(match b {
                Some(RelationBasis::Assertion(i)) => (Some(i64::from(*i)), None),
                Some(RelationBasis::Object(r)) => (
                    None,
                    Some(i64::try_from(r.get()).map_err(|_| StoreError::InvalidCompilation)?),
                ),
                None => (None, None),
            })
        };
        let (sa, sr) = columns(&r.subject_basis)?;
        let (oa, or) = columns(&r.object_basis)?;
        // Grounding must come from this run, so a caller cannot substitute a later object revision.
        for (endpoint, b) in [(&r.subject, &r.subject_basis), (&r.object, &r.object_basis)] {
            let valid=match b {
                Some(RelationBasis::Assertion(i))=>tx.query_row("SELECT EXISTS(SELECT 1 FROM observed_assertions WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3 AND subject_id=?4)",params![project.as_str(),run,i,endpoint],|r|r.get::<_,bool>(0))?,
                Some(RelationBasis::Object(rev))=>tx.query_row("SELECT EXISTS(SELECT 1 FROM compilation_context_objects WHERE project_id=?1 AND run_id=?2 AND object_id=?3 AND object_revision=?4)",params![project.as_str(),run,endpoint,i64::try_from(rev.get()).map_err(|_|StoreError::InvalidCompilation)?],|r|r.get::<_,bool>(0))?,
                None=>r.rejection.is_some(),
            };
            if !valid {
                return Err(StoreError::InvalidCompilation);
            }
        }
        tx.execute(
            "INSERT INTO observed_relations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                project.as_str(),
                run,
                i64::try_from(index).map_err(|_| StoreError::InvalidCompilation)?,
                r.subject,
                r.predicate,
                r.object,
                sa,
                sr,
                oa,
                or,
                r.rejection
            ],
        )?;
    }
    Ok(())
}

fn review_origin(
    c: &Connection,
    p: &ProjectId,
    id: &PolicyInputId,
) -> Result<Option<(CompilationRunId, u32)>, StoreError> {
    let row = c
        .query_row(
            "SELECT run_id,relation_index FROM relation_reviews WHERE project_id=?1 AND id=?2",
            params![p.as_str(), id.as_str()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)),
        )
        .optional()?;
    row.map(|(run, i)| {
        Ok((
            CompilationRunId::try_from(run.as_str()).map_err(|_| StoreError::CorruptHistory)?,
            i,
        ))
    })
    .transpose()
}

pub(super) fn resolution(
    c: &Connection,
    p: &ProjectId,
    run: &CompilationRunId,
    index: u32,
) -> Result<Option<String>, StoreError> {
    Ok(c.query_row("SELECT r.action FROM relation_reviews r JOIN accepted_policy_inputs a ON a.project_id=r.project_id AND a.input_id=r.id AND a.input_kind='command' WHERE r.project_id=?1 AND r.run_id=?2 AND r.relation_index=?3 LIMIT 1",params![p.as_str(),run.as_str(),index],|r|r.get(0)).optional()?)
}

pub(super) fn validate_reviews(
    c: &Connection,
    e: &PolicyEvaluation,
) -> Result<Option<PolicyConflictDetail>, StoreError> {
    for input in &e.inputs {
        let origin = match &input.input {
            PolicyInput::ObservedRelation { run, index, .. } => Some((run.clone(), *index)),
            PolicyInput::Command(id) => review_origin(c, &e.project, id)?,
            _ => None,
        };
        if let Some((run, index)) = origin {
            let exists=c.query_row("SELECT EXISTS(SELECT 1 FROM observed_relations WHERE project_id=?1 AND run_id=?2 AND relation_index=?3)",params![e.project.as_str(),run.as_str(),index],|r|r.get::<_,bool>(0))?;
            if !exists {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            if input.disposition != PolicyDisposition::Accepted {
                continue;
            }
            let rejection = if resolution(c, &e.project, &run, index)?.is_some() {
                Some("accepted_input_overlap")
            } else {
                let rejecting = if let PolicyInput::Command(id) = &input.input {
                    c.query_row("SELECT action='reject' FROM relation_reviews WHERE project_id=?1 AND id=?2",params![e.project.as_str(),id.as_str()],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false)
                } else {
                    false
                };
                if !rejecting && !compilation_evidence_current(c, &e.project, &run)? {
                    Some("relation_evidence_changed")
                } else {
                    None
                }
            };
            if let Some(reason) = rejection {
                return Ok(Some(PolicyConflictDetail {
                    reason_code: reason.into(),
                    target_id: Some(input.input.id().as_str().into()),
                    expected_revision: None,
                    actual_revision: None,
                }));
            }
        }
    }
    Ok(None)
}

pub(super) fn record_lineage(tx: &Transaction<'_>, e: &PolicyEvaluation) -> Result<(), StoreError> {
    for input in &e.inputs {
        let origin = match &input.input {
            PolicyInput::ObservedRelation { run, index, .. } => Some((run.clone(), *index)),
            PolicyInput::Command(id) => review_origin(tx, &e.project, id)?,
            _ => None,
        };
        if let Some((run, index)) = origin {
            tx.execute(
                "INSERT INTO policy_relation_inputs VALUES (?1,?2,?3,?4,?5)",
                params![
                    e.project.as_str(),
                    e.id.as_str(),
                    input.input.id().as_str(),
                    run.as_str(),
                    index
                ],
            )?;
        }
    }
    Ok(())
}
