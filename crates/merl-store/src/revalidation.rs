//! Durable review receipts and transaction guards for changed evidence.

use super::{PolicyConflictDetail, Store, StoreError, compilation_evidence_current};
use merl_core::{
    ActorId, CompilationRunId, PolicyDisposition, PolicyEvaluation, PolicyInput, PolicyInputId,
    ProjectId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// A reviewer's instruction for one affected support relation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevalidationAction {
    /// Keep the object's represented meaning and attach fresh support.
    Confirm,
    /// Withdraw this support while preserving the object's lifecycle.
    Weaken,
    /// Supersede the old object with a reviewed assertion's distinct subject.
    Supersede,
    /// Invalidate the old object without adopting a replacement.
    Invalidate,
    /// Close work whose required evidence was erased or observed deleted.
    Unavailable,
}
impl RevalidationAction {
    /// Stable command and receipt spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Weaken => "weaken",
            Self::Supersede => "supersede",
            Self::Invalidate => "invalidate",
            Self::Unavailable => "unavailable",
        }
    }
}
impl std::fmt::Display for RevalidationAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}
impl TryFrom<&str> for RevalidationAction {
    type Error = StoreError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "confirm" => Ok(Self::Confirm),
            "weaken" => Ok(Self::Weaken),
            "supersede" => Ok(Self::Supersede),
            "invalidate" => Ok(Self::Invalidate),
            "unavailable" => Ok(Self::Unavailable),
            _ => Err(StoreError::InvalidPolicyEvaluation),
        }
    }
}

/// Immutable command identity for explicit hindsight promotion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevalidationReview {
    /// Client identity retained across retries.
    pub id: PolicyInputId,
    /// Evidence impact to resolve.
    pub impact: String,
    /// Actor whose current command grant authorizes the resolution.
    pub actor: ActorId,
    /// Reviewed support outcome.
    pub action: RevalidationAction,
    /// Completed revalidation run, or its expansion successor.
    pub run: Option<CompilationRunId>,
    /// Assertion adopted by confirmation or supersession.
    pub assertion_index: Option<u32>,
}

impl Store {
    /// Reserves an immutable attempt for an impact before compiler dispatch.
    ///
    /// A new run ID permits retry after a terminal failure or another source edit.
    /// Existing IDs retain their original impact and compiler configuration.
    /// # Errors
    /// Rejects unknown impacts, unrelated existing runs, and changed retry identities.
    pub fn record_revalidation_attempt(
        &mut self,
        project: &ProjectId,
        impact: &str,
        run: &CompilationRunId,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let prior: Option<String> = transaction
            .query_row(
                "SELECT impact_id FROM revalidation_attempts WHERE project_id=?1 AND run_id=?2",
                params![project.as_str(), run.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            return if prior == impact {
                Ok(())
            } else {
                Err(StoreError::PolicyInputConflict)
            };
        }
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM compilation_runs WHERE project_id=?1 AND id=?2)",
            params![project.as_str(), run.as_str()],
            |r| r.get(0),
        )?;
        if exists && run.as_str() != impact {
            return Err(StoreError::PolicyInputConflict);
        }
        transaction.execute(
            "INSERT INTO revalidation_attempts(project_id,run_id,impact_id) VALUES(?1,?2,?3)",
            params![project.as_str(), run.as_str(), impact],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Finds the impact behind a revalidation attempt, including legacy impact IDs.
    ///
    /// # Errors
    /// Returns storage or corrupt-lineage errors.
    pub fn revalidation_impact(
        &self,
        project: &ProjectId,
        run: &str,
    ) -> Result<Option<super::EvidenceImpact>, StoreError> {
        let impact: Option<String> = self
            .connection
            .query_row(
                "SELECT impact_id FROM revalidation_attempts WHERE project_id=?1 AND run_id=?2",
                params![project.as_str(), run],
                |r| r.get(0),
            )
            .optional()?;
        self.evidence_impact(project, impact.as_deref().unwrap_or(run))
    }

    /// Selects the latest captured version of one stable source identity.
    ///
    /// # Errors
    /// Returns an error if the source or its history is missing or corrupt.
    pub fn latest_source_version(
        &self,
        project: &ProjectId,
        version: &merl_core::SourceVersionId,
    ) -> Result<merl_core::SourceVersionId, StoreError> {
        let latest: String=self.connection.query_row("SELECT latest.id FROM source_versions latest JOIN source_versions prior ON prior.project_id=latest.project_id AND prior.source_id=latest.source_id WHERE prior.project_id=?1 AND prior.id=?2 ORDER BY latest.sequence DESC LIMIT 1",params![project.as_str(),version.as_str()],|r|r.get(0))?;
        merl_core::SourceVersionId::try_from(latest.as_str())
            .map_err(|_| StoreError::CorruptHistory)
    }

    /// Replaces changed versions within the affected run's recorded source set.
    ///
    /// Several edits may accumulate before work runs. This window includes the
    /// latest version of each recorded source, without adding ambient history.
    /// # Errors
    /// Returns unavailable-lineage or storage errors.
    pub fn revalidation_source_window(
        &self,
        project: &ProjectId,
        impact: &super::EvidenceImpact,
    ) -> Result<Vec<merl_core::SourceVersionId>, StoreError> {
        let selected = self.compilation_context_sources(project, impact.affected_run.as_str())?;
        let mut ordered = Vec::new();
        for version in selected {
            let latest = self.latest_source_version(project, &version)?;
            let source = self
                .source_version(project, &latest)?
                .ok_or(StoreError::InvalidCompilation)?;
            ordered.push((source.sequence, latest));
        }
        ordered.sort_by_key(|s| s.0);
        ordered.dedup();
        Ok(ordered.into_iter().map(|s| s.1).collect())
    }

    /// Finds the latest reserved attempt so pending work can resume after restart.
    ///
    /// # Errors
    /// Returns storage or corrupt-identity errors.
    pub fn latest_revalidation_attempt(
        &self,
        project: &ProjectId,
        impact: &str,
    ) -> Result<Option<CompilationRunId>, StoreError> {
        let run: Option<String>=self.connection.query_row("SELECT run_id FROM revalidation_attempts WHERE project_id=?1 AND impact_id=?2 ORDER BY rowid DESC LIMIT 1",params![project.as_str(),impact],|r|r.get(0)).optional()?;
        run.map(|r| CompilationRunId::try_from(r.as_str()).map_err(|_| StoreError::CorruptHistory))
            .transpose()
    }

    /// Reads a review receipt even after its evidence is erased.
    ///
    /// # Errors
    /// Returns storage or corrupt-history errors.
    pub fn revalidation_review(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<RevalidationReview>, StoreError> {
        read_review(&self.connection, project, id)
    }

    /// Records a review before policy, preserving its original retry meaning.
    ///
    /// # Errors
    /// Changed retry content conflicts; malformed references and storage failures fail.
    pub fn record_revalidation_review(
        &mut self,
        project: &ProjectId,
        review: &RevalidationReview,
    ) -> Result<(), StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(prior) = read_review(&transaction, project, &review.id)? {
            return if prior == *review {
                Ok(())
            } else {
                Err(StoreError::PolicyInputConflict)
            };
        }
        transaction.execute("INSERT INTO revalidation_reviews(project_id,id,impact_id,actor_id,action,run_id,assertion_index) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![project.as_str(),review.id.as_str(),review.impact,review.actor.as_str(),review.action.as_str(),review.run.as_ref().map(CompilationRunId::as_str),review.assertion_index])?;
        transaction.commit()?;
        Ok(())
    }

    /// Reports whether erased or bodyless source evidence prevents this rerun.
    ///
    /// # Errors
    /// Returns errors for an unknown impact or unreadable source lineage.
    pub fn revalidation_evidence_unavailable(
        &self,
        project: &ProjectId,
        impact: &str,
    ) -> Result<bool, StoreError> {
        unavailable(&self.connection, project, impact)
    }
}

fn read_review(
    connection: &Connection,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<Option<RevalidationReview>, StoreError> {
    let row=connection.query_row("SELECT impact_id,actor_id,action,run_id,assertion_index FROM revalidation_reviews WHERE project_id=?1 AND id=?2",params![project.as_str(),id.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<u32>>(4)?))).optional()?;
    row.map(|(impact, actor, action, run, index)| {
        Ok(RevalidationReview {
            id: id.clone(),
            impact,
            actor: ActorId::try_from(actor.as_str()).map_err(|_| StoreError::CorruptHistory)?,
            action: RevalidationAction::try_from(action.as_str())?,
            run: run
                .map(|s| {
                    CompilationRunId::try_from(s.as_str()).map_err(|_| StoreError::CorruptHistory)
                })
                .transpose()?,
            assertion_index: index,
        })
    })
    .transpose()
}

fn unavailable(
    connection: &Connection,
    project: &ProjectId,
    impact: &str,
) -> Result<bool, StoreError> {
    // Inspect the exact replacement window, not the obsolete input's erased bytes.
    // A retained replacement can still support a new interpretation after old bytes vanish.
    Ok(connection.query_row("SELECT i.replacement_version_id IS NULL OR EXISTS (
        SELECT 1 FROM compilation_context_sources cs
        JOIN source_versions s ON s.project_id=cs.project_id AND s.id=(SELECT latest.id FROM source_versions latest JOIN source_versions old ON old.project_id=latest.project_id AND old.source_id=latest.source_id WHERE old.project_id=cs.project_id AND old.id=cs.source_version_id ORDER BY latest.sequence DESC LIMIT 1)
        LEFT JOIN payloads p ON p.project_id=s.project_id AND p.id=s.payload_id
        WHERE cs.project_id=i.project_id AND cs.run_id=i.affected_run_id AND (s.payload_id IS NULL OR p.bytes IS NULL))
        FROM evidence_impacts i WHERE i.project_id=?1 AND i.id=?2",params![project.as_str(),impact],|r|r.get(0))?)
}

pub(super) fn validate_reviews(
    connection: &Connection,
    evaluation: &PolicyEvaluation,
) -> Result<Option<PolicyConflictDetail>, StoreError> {
    for input in &evaluation.inputs {
        if input.disposition != PolicyDisposition::Accepted {
            continue;
        }
        let PolicyInput::Command(id) = &input.input else {
            continue;
        };
        let Some(review) = read_review(connection, &evaluation.project, id)? else {
            continue;
        };
        if let Some(batch) = &evaluation.batch {
            for event in &batch.events {
                if let merl_core::DomainEvent::PutObject {
                    payload: Some(payload),
                    ..
                } = event
                {
                    let retained: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM payloads WHERE project_id=?1 AND id=?2 AND bytes IS NOT NULL)",params![evaluation.project.as_str(),payload.as_str()],|r|r.get(0))?;
                    if !retained {
                        return Ok(Some(PolicyConflictDetail {
                            reason_code: "command_content_unavailable".into(),
                            target_id: Some(payload.to_string()),
                            expected_revision: None,
                            actual_revision: None,
                        }));
                    }
                }
            }
        }
        let resolved: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM evidence_revalidations WHERE project_id=?1 AND impact_id=?2)",params![evaluation.project.as_str(),review.impact],|r|r.get(0))?;
        let reason = if resolved {
            Some("accepted_input_overlap")
        } else if let Some(run) = &review.run {
            (!compilation_evidence_current(connection, &evaluation.project, run)?)
                .then_some("assertion_evidence_changed")
        } else {
            (!unavailable(connection, &evaluation.project, &review.impact)?)
                .then_some("assertion_evidence_changed")
        };
        if let Some(reason) = reason {
            return Ok(Some(PolicyConflictDetail {
                reason_code: reason.into(),
                target_id: Some(review.impact),
                expected_revision: None,
                actual_revision: None,
            }));
        }
    }
    Ok(None)
}

pub(super) fn assertion_lineage(
    connection: &Connection,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<Option<(CompilationRunId, u32)>, StoreError> {
    Ok(read_review(connection, project, id)?.and_then(|r| r.run.zip(r.assertion_index)))
}

pub(super) fn record_resolutions(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
) -> Result<(), StoreError> {
    for (index, input) in evaluation.inputs.iter().enumerate() {
        let PolicyInput::Command(id) = &input.input else {
            continue;
        };
        let Some(review) = read_review(transaction, &evaluation.project, id)? else {
            continue;
        };
        if input.disposition != PolicyDisposition::Accepted {
            continue;
        }
        let origin = evaluation
            .event_origins
            .iter()
            .find(|o| o.input_index as usize == index)
            .ok_or(StoreError::InvalidPolicyEvaluation)?;
        transaction.execute("INSERT INTO evidence_revalidations(project_id,impact_id,event_id,outcome)
             SELECT i.project_id,i.id,?3,?4 FROM evidence_impacts i
             JOIN evidence_impacts reviewed ON reviewed.project_id=i.project_id AND reviewed.support_event_id=i.support_event_id
             WHERE reviewed.project_id=?1 AND reviewed.id=?2
               AND NOT EXISTS(SELECT 1 FROM evidence_revalidations r WHERE r.project_id=i.project_id AND r.impact_id=i.id)",params![evaluation.project.as_str(),review.impact,origin.event.as_str(),if review.action==RevalidationAction::Confirm {"current"} else {"unsupported"}])?;
        if let Some((run, index)) = review.run.zip(review.assertion_index) {
            transaction.execute("INSERT INTO policy_assertion_inputs(project_id,evaluation_id,input_id,run_id,assertion_index) VALUES(?1,?2,?3,?4,?5)",params![evaluation.project.as_str(),evaluation.id.as_str(),id.as_str(),run.as_str(),index])?;
        }
    }
    Ok(())
}
