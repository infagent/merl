//! Immutable command receipts and planning facts tied to accepted event origins.

use super::*;
use merl_core::{PolicyInput, PolicyInputId, ReasonCode};

macro_rules! choices {
    ($name:ident, $doc:literal, $( $variant:ident => $value:literal : $description:literal ),+ $(,)?) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $(#[doc = $description] $variant),+ }
        impl $name {
            /// Returns the stable stored and public spelling.
            #[must_use]
            pub const fn as_str(self) -> &'static str { match self { $(Self::$variant => $value),+ } }
        }
        impl TryFrom<&str> for $name {
            type Error = StoreError;
            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value { $($value => Ok(Self::$variant)),+, _ => Err(StoreError::InvalidPolicyEvaluation) }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
    };
}
choices!(CommandOperation, "A first-release semantic action.",
    Create => "create": "Create a semantic object; a task begins as a request.",
    Resolve => "resolve": "Close a question or finding with a retained answer.",
    Accept => "accept": "Take responsibility for a task without scheduling it.",
    Defer => "defer": "Postpone planning until the recorded review date.",
    Start => "start": "Begin execution of accepted, non-deferred work.",
    Complete => "complete": "Record completion of work in progress.",
);
choices!(Commitment, "Responsibility for requested work, independent of its schedule.",
    Pending => "pending": "Nobody has accepted responsibility.",
    Accepted => "accepted": "An authorized actor accepted responsibility.",
    Declined => "declined": "The owner declined responsibility.",
);
choices!(Scheduling, "A task's planning state, independent of execution.",
    Unscheduled => "unscheduled": "No schedule exists.",
    Deferred => "deferred": "Planning waits for a recorded reconsideration trigger.",
    Scheduled => "scheduled": "The owner scheduled the work.",
);
choices!(Execution, "Recorded task execution, independent of commitment.",
    NotStarted => "not_started": "Execution has not begun.",
    InProgress => "in_progress": "Work is underway.",
    Blocked => "blocked": "Work cannot proceed.",
    Completed => "completed": "Work has finished.",
    Cancelled => "cancelled": "Work stopped without completion.",
);

/// Structural planning state; explanatory prose remains independently erasable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskState {
    /// Whether the project has taken responsibility.
    pub commitment: Commitment,
    /// Whether the project has scheduled or deferred the task.
    pub scheduling: Scheduling,
    /// What has happened in execution.
    pub execution: Execution,
    /// Protected deferral explanation, if one exists.
    pub reason: Option<PayloadId>,
    /// Validated UTC calendar date for reconsideration, in YYYY-MM-DD form.
    pub review_at: Option<String>,
}

/// A receipt fixes the proposed result and its target dependency before evaluation.
///
/// The receipt alone grants no authority. Readers expose planning state only when
/// an accepted domain event names this command as its origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticCommandRecord {
    /// Stable request identity within the project.
    pub id: PolicyInputId,
    /// Local audit actor whose current grant policy evaluates.
    pub actor: ActorId,
    /// Requested semantic operation.
    pub operation: CommandOperation,
    /// Affected object.
    pub object: ObjectId,
    /// Semantic kind, never a provider or control kind.
    pub kind: ObjectKind,
    /// Optional Issue scope inherited on updates.
    pub issue_scope: Option<String>,
    /// Proposed object content.
    pub payload: Option<PayloadId>,
    /// Optional explanatory content.
    pub reason: Option<PayloadId>,
    /// Optional captured supplemental prose.
    pub source: Option<SourceVersionId>,
    /// Target revision read while preparing this receipt; none means absent.
    pub expected_revision: Option<ObjectRevision>,
    /// Digest of the complete request, including protected content.
    pub request_digest: [u8; 32],
    /// Authority clock value at first receipt.
    pub occurred_at_millis: i64,
    /// Proposed task facets, when this command affects a task.
    pub task: Option<TaskState>,
    /// Stable validation failure, preserved across retries.
    pub rejection: Option<ReasonCode>,
}

impl Store {
    /// Retains content, an optional source, and the immutable receipt together.
    ///
    /// Exact retries reuse the receipt without restoring erased bytes. The payload
    /// list contains only newly authored content; inherited references stay intact.
    ///
    /// # Errors
    /// Returns an identity conflict for changed content or an invalid source/storage error.
    pub fn record_semantic_command(
        &mut self,
        project: &ProjectId,
        record: &SemanticCommandRecord,
        payloads: &[(PayloadId, Vec<u8>)],
        note: Option<&SourceCapture<'_>>,
    ) -> Result<SemanticCommandRecord, StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(prior) = read_command(&tx, project, &record.id)? {
            if prior.request_digest != record.request_digest {
                return Err(StoreError::PolicyInputConflict);
            }
            return Ok(prior);
        }
        for (id, bytes) in payloads {
            let existing: Option<bool> = tx.query_row(
                "SELECT digest=?3 AND COALESCE(bytes=?4,0) FROM payloads WHERE project_id=?1 AND id=?2",
                params![project.as_str(), id.as_str(), Sha256::digest(bytes).as_slice(), bytes], |r|r.get(0)).optional()?;
            match existing {
                Some(false) => return Err(StoreError::PolicyInputConflict),
                Some(true) => {}
                None => insert_protected_payload(&tx, project, id, bytes, &Sha256::digest(bytes))?,
            }
        }
        if let Some(note) = note {
            if record.source.as_ref() != Some(&note.version) {
                return Err(StoreError::InvalidSource);
            }
            capture_in_transaction(&tx, project, note, true)?;
        } else if record.source.is_some() {
            return Err(StoreError::InvalidSource);
        }
        tx.execute("INSERT INTO semantic_commands (project_id,id,actor,operation,object_id,kind,issue_scope,payload_id,reason_id,source_version_id,expected_revision,request_digest,occurred_at_millis,commitment,scheduling,execution,review_at,rejection) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![project.as_str(),record.id.as_str(),record.actor.as_str(),record.operation.as_str(),record.object.as_str(),record.kind.as_str(),record.issue_scope,record.payload.as_ref().map(PayloadId::as_str),record.reason.as_ref().map(PayloadId::as_str),record.source.as_ref().map(SourceVersionId::as_str),record.expected_revision.map(|r| i64::try_from(r.get())).transpose().map_err(|_|StoreError::CorruptHistory)?,record.request_digest.as_slice(),record.occurred_at_millis,record.task.as_ref().map(|t|t.commitment.as_str()),record.task.as_ref().map(|t|t.scheduling.as_str()),record.task.as_ref().map(|t|t.execution.as_str()),record.task.as_ref().and_then(|t|t.review_at.as_deref()),record.rejection.as_ref().map(ReasonCode::as_str)])?;
        tx.commit()?;
        Ok(record.clone())
    }

    /// Reads a receipt without resolving protected content.
    ///
    /// # Errors
    /// Returns storage or corrupt-record errors.
    pub fn semantic_command(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<SemanticCommandRecord>, StoreError> {
        read_command(&self.connection, project, id)
    }

    /// Resolves a supplemental source to its originating receipt, even after rejection.
    ///
    /// # Errors
    /// Returns storage or corrupt-record errors.
    pub fn source_command(
        &self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<Option<SemanticCommandRecord>, StoreError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM semantic_commands WHERE project_id=?1 AND source_version_id=?2",
                params![project.as_str(), source.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| {
            self.semantic_command(
                project,
                &PolicyInputId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?,
            )
        })
        .transpose()
        .map(Option::flatten)
    }

    /// Returns the accepted batch for a command, without inferring acceptance from its receipt.
    ///
    /// # Errors
    /// Returns storage or corrupt-history errors.
    pub fn semantic_command_batch(
        &self,
        project: &ProjectId,
        id: &PolicyInputId,
    ) -> Result<Option<merl_core::BatchId>, StoreError> {
        let batch: Option<String> = self.connection.query_row("SELECT e.batch_id FROM accepted_policy_inputs a JOIN policy_evaluations e ON e.project_id=a.project_id AND e.id=a.evaluation_id WHERE a.project_id=?1 AND a.input_kind='command' AND a.input_id=?2",params![project.as_str(),id.as_str()],|r|r.get(0)).optional()?;
        batch.as_deref().map(parse).transpose()
    }

    /// Reads the command behind the current semantic state, across support reviews.
    ///
    /// Confirmation, weakening, and unavailability change support without changing
    /// task facets or reopening resolved questions. Other interpretations replace
    /// semantic state and still make an older command snapshot inapplicable.
    /// # Errors
    /// Returns storage or corrupt-history errors.
    pub fn object_semantic_command(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<SemanticCommandRecord>, StoreError> {
        let row=self.connection.query_row("SELECT i.input_kind,i.input_id FROM semantic_object_events e
            JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
            LEFT JOIN policy_evaluation_domain_events o ON o.project_id=e.project_id AND o.event_id=e.id
            LEFT JOIN policy_evaluation_inputs i ON i.project_id=o.project_id AND i.evaluation_id=o.evaluation_id AND i.input_index=o.input_index
            LEFT JOIN revalidation_reviews r ON r.project_id=i.project_id AND r.id=i.input_id AND i.input_kind='command'
            WHERE e.project_id=?1 AND e.object_id=?2 AND (r.action IS NULL OR r.action NOT IN ('confirm','weaken','unavailable'))
            ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",params![project.as_str(),object.as_str()],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?))).optional()?;
        match row {
            Some((Some(kind), Some(id))) if kind == "command" => {
                self.semantic_command(project, &parse(&id)?)
            }
            _ => Ok(None),
        }
    }

    /// Reads task facets from the current semantic command origin.
    ///
    /// A later non-command correction makes those facets unknown rather than
    /// silently retaining a stale planning snapshot. Rebuild uses the same history.
    ///
    /// # Errors
    /// Returns storage or corrupt-history errors.
    pub fn task_state(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<TaskState>, StoreError> {
        Ok(self
            .object_semantic_command(project, object)?
            .and_then(|c| c.task))
    }
}

fn read_command(
    connection: &Connection,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<Option<SemanticCommandRecord>, StoreError> {
    let row = connection.query_row("SELECT actor,operation,object_id,kind,issue_scope,payload_id,reason_id,source_version_id,expected_revision,request_digest,occurred_at_millis,commitment,scheduling,execution,review_at,rejection FROM semantic_commands WHERE project_id=?1 AND id=?2",params![project.as_str(),id.as_str()], |r| {
        Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,Option<String>>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<i64>>(8)?,r.get::<_,Vec<u8>>(9)?,r.get::<_,i64>(10)?,r.get::<_,Option<String>>(11)?,r.get::<_,Option<String>>(12)?,r.get::<_,Option<String>>(13)?,r.get::<_,Option<String>>(14)?,r.get::<_,Option<String>>(15)?))
    }).optional()?;
    row.map(
        |(
            actor,
            operation,
            object,
            kind,
            scope,
            payload,
            reason,
            source,
            revision,
            digest,
            now,
            commitment,
            scheduling,
            execution,
            review_at,
            rejection,
        )| {
            let reason: Option<PayloadId> = reason.as_deref().map(parse).transpose()?;
            Ok(SemanticCommandRecord {
                id: id.clone(),
                actor: parse(&actor)?,
                operation: parse(&operation)?,
                object: parse(&object)?,
                kind: parse(&kind)?,
                issue_scope: scope,
                payload: payload.as_deref().map(parse).transpose()?,
                reason: reason.clone(),
                source: source.as_deref().map(parse).transpose()?,
                expected_revision: revision
                    .map(|v| {
                        u64::try_from(v)
                            .ok()
                            .and_then(|v| ObjectRevision::try_from(v).ok())
                            .ok_or(StoreError::CorruptHistory)
                    })
                    .transpose()?,
                request_digest: digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
                occurred_at_millis: now,
                task: commitment
                    .map(|c| {
                        Ok::<_, StoreError>(TaskState {
                            commitment: parse(&c)?,
                            scheduling: parse(
                                scheduling.as_deref().ok_or(StoreError::CorruptHistory)?,
                            )?,
                            execution: parse(
                                execution.as_deref().ok_or(StoreError::CorruptHistory)?,
                            )?,
                            reason,
                            review_at,
                        })
                    })
                    .transpose()?,
                rejection: rejection.as_deref().map(parse).transpose()?,
            })
        },
    )
    .transpose()
}
fn parse<T: for<'a> TryFrom<&'a str>>(value: &str) -> Result<T, StoreError> {
    T::try_from(value).map_err(|_| StoreError::CorruptHistory)
}

pub(super) fn validate_commands(
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
        let Some(record) = read_command(connection, &evaluation.project, id)? else {
            continue;
        };
        for payload in [&record.payload, &record.reason].into_iter().flatten() {
            let available: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM payloads WHERE project_id=?1 AND id=?2 AND bytes IS NOT NULL)",params![evaluation.project.as_str(),payload.as_str()],|r|r.get(0))?;
            if !available {
                return Ok(Some(PolicyConflictDetail {
                    reason_code: "command_content_unavailable".into(),
                    target_id: Some(payload.to_string()),
                    expected_revision: None,
                    actual_revision: None,
                }));
            }
        }
    }
    Ok(None)
}
