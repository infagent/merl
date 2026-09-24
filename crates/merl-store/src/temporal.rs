//! Resolve planning state from the assertion that supplied the current semantic version.

use super::{
    Commitment, Execution, RevalidationAction, ReviewAction, Scheduling, Store, StoreError,
    StructuralAssertion, TaskState,
};
use merl_core::PolicyInputId;
use merl_core::temporal::{TemporalRole, TemporalValue};
use merl_core::{ObjectId, ProjectId, ProjectRevision};
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn validate_task_evidence(
    connection: &Connection,
    project: &ProjectId,
    task: &TaskState,
) -> Result<(), StoreError> {
    // Commands may inherit compiler evidence, but cannot author normalized results.
    for temporal in &task.temporal {
        let time: Option<String> = connection.query_row(
            "SELECT author_time FROM source_versions WHERE project_id=?1 AND id=?2",
            params![project.as_str(), temporal.source.to_string()],
            |r| r.get(0),
        )?;
        let time = time
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| StoreError::CorruptHistory)?;
        temporal
            .original
            .validate()
            .map_err(|_| StoreError::InvalidPolicyEvaluation)?;
        if *temporal
            != temporal
                .original
                .normalize(temporal.source.clone(), time.as_ref())
        {
            return Err(StoreError::InvalidPolicyEvaluation);
        }
    }
    Ok(())
}

pub(super) fn validate(
    connection: &Connection,
    project: &ProjectId,
    assertion: &StructuralAssertion,
) -> Result<(), StoreError> {
    if assertion.temporal.is_empty() && assertion.deferral.is_none() {
        return Ok(());
    }
    let (time, body): (Option<String>, Option<Vec<u8>>) = connection.query_row(
        "SELECT s.author_time,p.bytes FROM source_versions s LEFT JOIN payloads p ON p.project_id=s.project_id AND p.id=s.payload_id WHERE s.project_id=?1 AND s.id=?2",
        params![project.as_str(),assertion.source.as_str()], |r| Ok((r.get(0)?,r.get(1)?)))?;
    let time: Option<merl_core::temporal::AuthorTime> = time
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| StoreError::CorruptHistory)?;
    let body = body.ok_or(StoreError::InvalidCompilation)?;
    let body = std::str::from_utf8(&body).map_err(|_| StoreError::InvalidCompilation)?;
    let valid_span = |start, end| {
        start >= assertion.span_start
            && start < end
            && end <= assertion.span_end
            && body.get(start..end).is_some()
    };
    let mut roles = Vec::new();
    for result in &assertion.temporal {
        let expression = &result.original;
        expression
            .validate()
            .map_err(|_| StoreError::InvalidCompilation)?;
        if roles.contains(&expression.role)
            || !valid_span(expression.span_start, expression.span_end)
            || result.source.to_string() != assertion.source.as_str()
            || *result != expression.normalize(result.source.clone(), time.as_ref())
        {
            return Err(StoreError::InvalidCompilation);
        }
        roles.push(expression.role);
    }
    if let Some(deferral) = &assertion.deferral
        && (assertion.predicate != "task"
            || !valid_span(deferral.reason.start, deferral.reason.end)
            || !roles.contains(&TemporalRole::ReviewAt)
                && !roles.contains(&TemporalRole::StartAfter))
    {
        return Err(StoreError::InvalidCompilation);
    }
    Ok(())
}

impl Store {
    /// Reads the assertion adopted by the object's current semantic event.
    ///
    /// Support-only reviews preserve this origin. A correction or command replaces it,
    /// so an older interpretation cannot supply stale planning fields.
    /// # Errors
    /// Returns storage errors or corrupt accepted provenance.
    pub fn object_assertion(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<StructuralAssertion>, StoreError> {
        Ok(self
            .temporal_origin(project, object, self.project_revision(project)?)?
            .assertion)
    }

    /// Reads temporal values from the semantic origin at a historical project revision.
    ///
    /// Later corrections cannot change the values supplied to an earlier compiler context.
    /// # Errors
    /// Returns storage errors or corrupt accepted provenance.
    pub fn object_temporal_at_revision(
        &self,
        project: &ProjectId,
        object: &ObjectId,
        basis: ProjectRevision,
    ) -> Result<Vec<merl_core::temporal::TemporalResult>, StoreError> {
        Ok(self.temporal_origin(project, object, basis)?.temporal)
    }

    fn temporal_origin(
        &self,
        project: &ProjectId,
        object: &ObjectId,
        basis: ProjectRevision,
    ) -> Result<SemanticEvidence, StoreError> {
        let origin = self.connection.query_row(
            "SELECT i.input_kind,i.input_id,a.run_id,a.assertion_index
             FROM semantic_object_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN policy_evaluation_domain_events o ON o.project_id=e.project_id AND o.event_id=e.id
             LEFT JOIN policy_evaluation_inputs i ON i.project_id=o.project_id AND i.evaluation_id=o.evaluation_id AND i.input_index=o.input_index
             LEFT JOIN policy_assertion_inputs a ON a.project_id=i.project_id AND a.input_id=i.input_id AND a.evaluation_id=i.evaluation_id AND i.input_kind='observed_assertion'
             LEFT JOIN revalidation_reviews r ON r.project_id=i.project_id AND r.id=i.input_id AND i.input_kind='command'
             WHERE e.project_id=?1 AND e.object_id=?2 AND b.revision<=?3 AND (r.action IS NULL OR r.action NOT IN ('confirm','weaken','unavailable'))
             ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
            params![project.as_str(),object.as_str(),i64::try_from(basis.get()).map_err(|_|StoreError::CorruptHistory)?],
            |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<u32>>(3)?)),
        ).optional()?;
        let Some((kind, input, run, index)) = origin else {
            return Ok(SemanticEvidence::default());
        };
        let adopted = if kind.as_deref() == Some("observed_assertion") {
            run.zip(index)
        } else if kind.as_deref() == Some("command") {
            let input =
                PolicyInputId::try_from(input.as_deref().ok_or(StoreError::CorruptHistory)?)
                    .map_err(|_| StoreError::CorruptHistory)?;
            if let Some(command) = self.semantic_command(project, &input)? {
                return Ok(SemanticEvidence {
                    assertion: None,
                    temporal: command.task.map_or_else(Vec::new, |t| t.temporal),
                });
            }
            if let Some(review) = self.candidate_review(project, &input)? {
                if review.action == ReviewAction::Accept {
                    let candidate = self
                        .candidate(project, &review.candidate)?
                        .ok_or(StoreError::CorruptHistory)?;
                    Some((candidate.run.to_string(), candidate.index))
                } else {
                    None
                }
            } else if let Some(review) = self.revalidation_review(project, &input)? {
                if review.action == RevalidationAction::Supersede {
                    review
                        .run
                        .map(|r| r.to_string())
                        .zip(review.assertion_index)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let assertion = adopted
            .map(|(run, index)| {
                self.observed_assertions(project, &run)?
                    .into_iter()
                    .nth(index as usize)
                    .ok_or(StoreError::CorruptHistory)
            })
            .transpose()?
            .filter(|assertion| assertion.subject == object.as_str());
        let temporal = assertion
            .as_ref()
            .map_or_else(Vec::new, |a| a.temporal.clone());
        Ok(SemanticEvidence {
            assertion,
            temporal,
        })
    }

    pub(super) fn assertion_task_state(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<TaskState>, StoreError> {
        let Some(assertion) = self.object_assertion(project, object)? else {
            return Ok(None);
        };
        let Some(deferral) = assertion.deferral else {
            return Ok(None);
        };
        let source = self
            .source_version(project, &assertion.source)?
            .ok_or(StoreError::CorruptHistory)?;
        let mut task = TaskState {
            commitment: if deferral.accepted {
                Commitment::Accepted
            } else {
                Commitment::Pending
            },
            scheduling: Scheduling::Deferred,
            execution: Execution::NotStarted,
            reason: source.payload,
            reason_span: Some(deferral.reason),
            review_at: None,
            review_when: None,
            start_after: None,
            temporal: assertion.temporal.clone(),
        };
        for temporal in assertion.temporal {
            match (temporal.original.role, temporal.value) {
                (TemporalRole::ReviewAt, TemporalValue::Date { date }) => {
                    task.review_at = Some(date);
                }
                (TemporalRole::ReviewAt, TemporalValue::Predicate { predicate }) => {
                    task.review_when = Some(predicate);
                }
                (TemporalRole::StartAfter, TemporalValue::Predicate { predicate }) => {
                    task.start_after = Some(predicate);
                }
                _ => {}
            }
        }
        Ok(Some(task))
    }
}

#[derive(Default)]
struct SemanticEvidence {
    assertion: Option<StructuralAssertion>,
    temporal: Vec<merl_core::temporal::TemporalResult>,
}
