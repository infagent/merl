//! Immutable review requests and candidate discovery over recorded policy outcomes.

use super::{PolicyConflictDetail, Store, StoreError, compilation_evidence_current};
use merl_core::{
    ActorId, CompilationRunId, ObjectId, ObjectKind, PayloadId, PolicyDisposition,
    PolicyEvaluation, PolicyEvaluationId, PolicyInput, PolicyInputId, ProjectId, ProjectRevision,
    ReasonCode,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

/// The first recorded candidate disposition for one immutable assertion input.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Stable input identity used by review commands.
    pub id: PolicyInputId,
    /// Evaluation that first held this interpretation for review.
    pub evaluation: PolicyEvaluationId,
    /// Compiler origin, retained even after review.
    pub run: CompilationRunId,
    /// Position in the compiler's immutable response.
    pub index: u32,
    /// Accepted state against which the candidate was proposed.
    pub basis: ProjectRevision,
    /// Policy's original explanation.
    pub reason: ReasonCode,
}

/// The reviewer's explicit semantic action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReviewAction {
    /// Adopt the recorded interpretation without changing its content.
    Accept,
    /// Close the candidate without accepting its proposed semantic effect.
    Reject,
    /// Adopt a replacement interpretation while preserving the compiler's assertion.
    Correct {
        /// Replacement subject selected by the reviewer.
        object: ObjectId,
        /// Supported semantic predicate.
        kind: ObjectKind,
        /// Protected replacement content, or no content.
        payload: Option<PayloadId>,
    },
}
impl ReviewAction {
    /// Returns the stable command verb.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::Correct { .. } => "correct",
        }
    }
}

/// A review request whose identity fixes its actor, target, action, and reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateReview {
    /// Client retry identity, distinct from the candidate's identity.
    pub id: PolicyInputId,
    /// Original candidate input.
    pub candidate: PolicyInputId,
    /// Actor requesting the action; current command grants decide authority.
    pub actor: ActorId,
    /// Explicit review instruction.
    pub action: ReviewAction,
    /// Erasable audit note, required for rejection and correction.
    pub reason: Option<PayloadId>,
}

fn identifier<T: for<'a> TryFrom<&'a str>>(value: &str) -> Result<T, StoreError> {
    T::try_from(value).map_err(|_| StoreError::CorruptHistory)
}

impl Store {
    /// Retains a review reason, reusing identical bytes after an interrupted request.
    ///
    /// A process can stop after writing the reason but before recording its review.
    /// Compare and insert under one writer transaction so retries can reuse that
    /// payload without overwriting another value or restoring erased bytes.
    ///
    /// # Errors
    /// Returns an identity conflict if the existing digest or bytes differ, or the
    /// payload was erased. Missing projects and storage failures also return errors.
    pub fn put_review_reason(
        &mut self,
        project: &ProjectId,
        id: &PayloadId,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let digest = Sha256::digest(bytes);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let matches: Option<bool> = transaction.query_row(
            "SELECT digest=?3 AND COALESCE(bytes=?4,0) FROM payloads WHERE project_id=?1 AND id=?2",
            params![project.as_str(), id.as_str(), digest.as_slice(), bytes],
            |row| row.get(0),
        ).optional()?;
        match matches {
            Some(false) => return Err(StoreError::PolicyInputConflict),
            Some(true) => {}
            None => {
                transaction.execute(
                    "INSERT INTO payloads (project_id,id,digest,bytes) VALUES (?1,?2,?3,?4)",
                    params![project.as_str(), id.as_str(), digest.as_slice(), bytes],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Finds an assertion candidate without expanding source prose.
    ///
    /// # Errors
    /// Returns storage errors or invalid recorded identities.
    pub fn candidate(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<Candidate>, StoreError> {
        let row = self.connection.query_row(
            "SELECT i.evaluation_id,a.run_id,a.assertion_index,e.basis_project_revision,i.reason_code
             FROM policy_evaluation_inputs i JOIN policy_evaluations e
             ON e.project_id=i.project_id AND e.id=i.evaluation_id
             JOIN policy_assertion_inputs a ON a.project_id=i.project_id AND a.evaluation_id=i.evaluation_id AND a.input_id=i.input_id
             WHERE i.project_id=?1 AND i.input_id=?2 AND i.input_kind='observed_assertion' AND i.disposition='candidate'
             ORDER BY e.rowid LIMIT 1", params![project.as_str(), id.as_str()],
            |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,u32>(2)?, r.get::<_,i64>(3)?, r.get::<_,String>(4)?))).optional()?;
        row.map(|(evaluation, run, index, basis, reason)| {
            Ok(Candidate {
                id: id.clone(),
                evaluation: identifier(&evaluation)?,
                run: identifier(&run)?,
                index,
                basis: ProjectRevision::from(
                    u64::try_from(basis).map_err(|_| StoreError::CorruptHistory)?,
                ),
                reason: identifier(&reason)?,
            })
        })
        .transpose()
    }

    /// Lists at most 100 candidate identities after a stable lexical cursor.
    ///
    /// Resolved candidates remain discoverable for audit; callers inspect their status.
    /// # Errors
    /// Rejects invalid limits and unreadable policy history.
    pub fn candidate_page(
        &self,
        project: &ProjectId,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Candidate>, bool), StoreError> {
        if !(1..=100).contains(&limit) {
            return Err(StoreError::InvalidPolicyEvaluation);
        }
        self.project_revision(project)?;
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT input_id FROM policy_evaluation_inputs WHERE project_id=?1 AND input_kind='observed_assertion'
             AND disposition='candidate' AND input_id>?2 ORDER BY input_id LIMIT ?3")?;
        let ids = statement
            .query_map(
                params![
                    project.as_str(),
                    after.unwrap_or(""),
                    i64::try_from(limit + 1).map_err(|_| StoreError::InvalidPolicyEvaluation)?
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = ids.len() > limit;
        let entries = ids
            .into_iter()
            .take(limit)
            .map(|id| {
                self.candidate(project, &identifier(&id)?)?
                    .ok_or(StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((entries, more))
    }

    /// Records an immutable review intent before evaluation, without resolving it.
    ///
    /// # Errors
    /// Rejects unknown candidates, missing reasons, and changed retry content.
    pub fn record_candidate_review(
        &mut self,
        project: &ProjectId,
        review: &CandidateReview,
    ) -> Result<(), StoreError> {
        if let Some(prior) = self.candidate_review(project, &review.id)? {
            return if prior == *review {
                Ok(())
            } else {
                Err(StoreError::PolicyInputConflict)
            };
        }
        let candidate = self
            .candidate(project, &review.candidate)?
            .ok_or(StoreError::InvalidPolicyEvaluation)?;
        if review.action != ReviewAction::Accept && review.reason.is_none() {
            return Err(StoreError::InvalidPolicyEvaluation);
        }
        let (object, kind, payload) = match &review.action {
            ReviewAction::Correct {
                object,
                kind,
                payload,
            } => (
                Some(object.as_str()),
                Some(kind.as_str()),
                payload.as_ref().map(PayloadId::as_str),
            ),
            _ => (None, None, None),
        };
        self.connection.execute(
            "INSERT INTO candidate_reviews (project_id,id,candidate_id,original_evaluation_id,run_id,assertion_index,actor_id,action,object_id,object_kind,payload_id,reason_payload_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![project.as_str(),review.id.as_str(),review.candidate.as_str(),candidate.evaluation.as_str(),candidate.run.as_str(),i64::from(candidate.index),review.actor.as_str(),review.action.as_str(),object,kind,payload,review.reason.as_ref().map(PayloadId::as_str)])?;
        Ok(())
    }

    /// Reads the original request, including requests that policy rejected.
    ///
    /// # Errors
    /// Returns storage or structural-history errors.
    pub fn candidate_review(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<CandidateReview>, StoreError> {
        let row = self.connection.query_row(
            "SELECT candidate_id,actor_id,action,object_id,object_kind,payload_id,reason_payload_id FROM candidate_reviews WHERE project_id=?1 AND id=?2",
            params![project.as_str(),id.as_str()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?))).optional()?;
        row.map(
            |(candidate, actor, action, object, kind, payload, reason)| {
                Ok(CandidateReview {
                    id: id.clone(),
                    candidate: identifier(&candidate)?,
                    actor: identifier(&actor)?,
                    action: match action.as_str() {
                        "accept" => ReviewAction::Accept,
                        "reject" => ReviewAction::Reject,
                        "correct" => ReviewAction::Correct {
                            object: identifier(&object.ok_or(StoreError::CorruptHistory)?)?,
                            kind: identifier(&kind.ok_or(StoreError::CorruptHistory)?)?,
                            payload: payload.as_deref().map(identifier).transpose()?,
                        },
                        _ => return Err(StoreError::CorruptHistory),
                    },
                    reason: reason.as_deref().map(identifier).transpose()?,
                })
            },
        )
        .transpose()
    }

    /// Lists a candidate's review requests in receipt order, bounded to 100 per page.
    ///
    /// # Errors
    /// Returns storage errors or corrupt requests.
    pub fn candidate_reviews(
        &self,
        project: &ProjectId,
        candidate: &PolicyInputId,
        offset: usize,
    ) -> Result<(Vec<CandidateReview>, bool), StoreError> {
        let mut statement = self.connection.prepare("SELECT id FROM candidate_reviews WHERE project_id=?1 AND candidate_id=?2 ORDER BY rowid LIMIT 101 OFFSET ?3")?;
        let ids = statement
            .query_map(
                params![
                    project.as_str(),
                    candidate.as_str(),
                    i64::try_from(offset).map_err(|_| StoreError::InvalidPolicyEvaluation)?
                ],
                |r| r.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let more = ids.len() > 100;
        Ok((
            ids.into_iter()
                .take(100)
                .map(|id| {
                    self.candidate_review(project, &identifier(&id)?)?
                        .ok_or(StoreError::CorruptHistory)
                })
                .collect::<Result<Vec<_>, _>>()?,
            more,
        ))
    }

    /// Returns the accepted review action, independently of later failed attempts.
    ///
    /// # Errors
    /// Returns storage errors.
    pub fn candidate_resolution(
        &self,
        project: &ProjectId,
        candidate: &Candidate,
    ) -> Result<Option<String>, StoreError> {
        resolution(&self.connection, project, &candidate.run, candidate.index)
    }
}

pub(super) fn resolution(
    connection: &Connection,
    project: &ProjectId,
    run: &CompilationRunId,
    index: u32,
) -> Result<Option<String>, StoreError> {
    Ok(connection.query_row("SELECT r.action FROM candidate_reviews r JOIN accepted_policy_inputs a ON a.project_id=r.project_id AND a.input_id=r.id AND a.input_kind='command'
        WHERE r.project_id=?1 AND r.run_id=?2 AND r.assertion_index=?3 LIMIT 1",params![project.as_str(),run.as_str(),i64::from(index)],|r|r.get(0)).optional()?)
}

pub(super) fn review_lineage(
    connection: &Connection,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<Option<(CompilationRunId, u32)>, StoreError> {
    let row = connection.query_row("SELECT run_id,assertion_index FROM candidate_reviews WHERE project_id=?1 AND id=?2 AND action!='reject'",params![project.as_str(),id.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?))).optional()?;
    row.map(|(run, index)| Ok((identifier(&run)?, index)))
        .transpose()
}

pub(super) fn validate_reviews(
    connection: &Connection,
    evaluation: &PolicyEvaluation,
) -> Result<Option<PolicyConflictDetail>, StoreError> {
    for input in &evaluation.inputs {
        if input.disposition != PolicyDisposition::Accepted {
            continue;
        }
        let lineage = match &input.input {
            PolicyInput::ObservedAssertion { run, index, .. } => Some((run.clone(), *index)),
            PolicyInput::Command(id) => {
                let row=connection.query_row("SELECT run_id,assertion_index,action FROM candidate_reviews WHERE project_id=?1 AND id=?2",params![evaluation.project.as_str(),id.as_str()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u32>(1)?,r.get::<_,String>(2)?))).optional()?;
                if let Some((run, index, action)) = row {
                    let run = identifier(&run)?;
                    if action != "reject"
                        && (!compilation_evidence_current(connection, &evaluation.project, &run)?
                            || !review_value_current(connection, &evaluation.project, id)?)
                    {
                        return Ok(Some(conflict("assertion_evidence_changed", run.as_str())));
                    }
                    Some((run, index))
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some((run, index)) = lineage {
            if matches!(input.input, PolicyInput::ObservedAssertion { .. })
                && !compilation_evidence_current(connection, &evaluation.project, &run)?
            {
                return Ok(Some(conflict("assertion_evidence_changed", run.as_str())));
            }
            if resolution(connection, &evaluation.project, &run, index)?.is_some() {
                return Ok(Some(conflict(
                    "accepted_input_overlap",
                    input.input.id().as_str(),
                )));
            }
            if matches!(input.input,PolicyInput::Command(_)) && connection.query_row("SELECT EXISTS(SELECT 1 FROM accepted_assertions WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3)",params![evaluation.project.as_str(),run.as_str(),i64::from(index)],|r|r.get::<_,bool>(0))? {
                return Ok(Some(conflict("accepted_input_overlap",input.input.id().as_str())));
            }
        }
    }
    Ok(None)
}
fn conflict(reason: &str, target: &str) -> PolicyConflictDetail {
    PolicyConflictDetail {
        reason_code: reason.into(),
        target_id: Some(target.into()),
        expected_revision: None,
        actual_revision: None,
    }
}

pub(super) fn record_review_lineage(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    committed: bool,
) -> Result<(), StoreError> {
    for input in &evaluation.inputs {
        if let PolicyInput::Command(id) = &input.input
            && let Some((run, index)) = review_lineage(transaction, &evaluation.project, id)?
        {
            transaction.execute("INSERT INTO policy_assertion_inputs (project_id,evaluation_id,input_id,run_id,assertion_index) VALUES (?1,?2,?3,?4,?5)",params![evaluation.project.as_str(),evaluation.id.as_str(),id.as_str(),run.as_str(),i64::from(index)])?;
            if committed && input.disposition == PolicyDisposition::Accepted {
                transaction.execute("INSERT INTO accepted_assertions (project_id,run_id,assertion_index,evaluation_id) SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM candidate_reviews WHERE project_id=?1 AND id=?5 AND action='accept')",params![evaluation.project.as_str(),run.as_str(),i64::from(index),evaluation.id.as_str(),id.as_str()])?;
            }
        }
    }
    Ok(())
}

// A reviewer may name a payload outside the compiler's context. Its erasure must
// invalidate prepared acceptance too, even while the source bytes remain intact.
fn review_value_current(
    connection: &Connection,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<bool, StoreError> {
    Ok(connection.query_row(
        "SELECT CASE WHEN r.action='correct' AND r.payload_id IS NULL THEN 1
            WHEN r.action='accept' AND a.value_id='none' THEN 1
            ELSE EXISTS(SELECT 1 FROM payloads p WHERE p.project_id=r.project_id
                AND p.id=CASE WHEN r.action='correct' THEN r.payload_id ELSE a.value_id END AND p.bytes IS NOT NULL) END
         FROM candidate_reviews r JOIN observed_assertions a ON a.project_id=r.project_id AND a.run_id=r.run_id AND a.assertion_index=r.assertion_index
         WHERE r.project_id=?1 AND r.id=?2", params![project.as_str(),id.as_str()],|row|row.get(0))?)
}
