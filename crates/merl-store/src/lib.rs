//! SQLite authority for accepted events and independently erasable payloads.

use std::{error::Error, fmt, path::Path};

use merl_core::{
    ActorId, AgentId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, ObjectId, ObjectKind, ObjectRevision, PayloadId, PolicyDisposition,
    PolicyEvaluation, PolicyEvaluationId, PolicyRead, ProjectId, ProjectRevision,
    ProviderIssueState, ProviderObservation, SourceBindingId, SourceId, SourceKind, SourceProvider,
    SourceVersionId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 6;

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
    /// A compiler record refers to unavailable history or exceeds structural limits.
    InvalidCompilation,
    /// Accepted state no longer matches a policy read or proposed write.
    PolicyConflict,
    /// The same immutable input or evaluation identity was reused differently.
    PolicyInputConflict,
    /// An evaluation contradicts its claimed inputs, writes, or batch.
    InvalidPolicyEvaluation,
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
            Self::InvalidCompilation => formatter.write_str("invalid compilation record"),
            Self::PolicyConflict => {
                formatter.write_str("policy dependencies changed before commit")
            }
            Self::PolicyInputConflict => {
                formatter.write_str("policy input identity conflicts with prior accepted work")
            }
            Self::InvalidPolicyEvaluation => {
                formatter.write_str("policy evaluation has inconsistent inputs or writes")
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

/// An accepted change waiting for an agent to read its project delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxEntry {
    /// Subscriber receiving the reference-only notification.
    pub agent: AgentId,
    /// Batch that changed accepted state.
    pub batch: merl_core::BatchId,
    /// One project cursor shared with all accepted changes.
    pub revision: ProjectRevision,
}

/// Recorded outcome of one policy evaluation, including retries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedPolicyEvaluation {
    /// Stable evaluation identity.
    pub id: PolicyEvaluationId,
    /// Authenticated actor that requested the evaluation.
    pub actor: ActorId,
    /// Version of the rules that produced the result.
    pub version: merl_core::PolicyVersion,
    /// Exact authority-configuration digest used with that version.
    pub configuration_digest: [u8; 32],
    /// Accepted-state revision used during evaluation.
    pub basis_project_revision: ProjectRevision,
    /// Accepted revision, absent when every input remained candidate or rejected.
    pub committed_revision: Option<ProjectRevision>,
    /// Failed commit validation, absent for an accepted or non-mutating evaluation.
    pub conflict: Option<PolicyConflictDetail>,
    /// Input decisions in their original evaluation order.
    pub inputs: Vec<merl_core::PolicyInputDecision>,
    /// Dependencies policy read, including guarded absence.
    pub reads: Vec<PolicyRead>,
    /// Targets policy intended to mutate.
    pub writes: Vec<merl_core::PolicyWrite>,
    /// Accepted event identities linked to this evaluation.
    pub events: Vec<merl_core::EventId>,
}

/// Exact accepted input responsible for an object's latest event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPolicyOrigin {
    /// Accepted event that last changed the object.
    pub event: merl_core::EventId,
    /// Evaluation that accepted the event.
    pub evaluation: PolicyEvaluationId,
    /// Input within that evaluation that produced the event.
    pub input: merl_core::PolicyInput,
}

/// Structural reason a prepared policy result could not commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyConflictDetail {
    /// Stable reason code identifying the failed guard.
    pub reason_code: String,
    /// Object or kind whose revision changed, when applicable.
    pub target_id: Option<String>,
    /// Revision policy relied on, when applicable.
    pub expected_revision: Option<i64>,
    /// Revision observed by the authority at commit time, when applicable.
    pub actual_revision: Option<i64>,
}

enum PolicyOverlap {
    Duplicate,
    Conflict(PolicyConflictDetail),
}

struct PolicyHeader {
    actor: String,
    version: String,
    configuration_digest: Vec<u8>,
    basis: i64,
    revision: Option<i64>,
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
    /// Issue or conversation that supplies nearby causal source context.
    pub context_scope_id: &'a str,
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
    /// Entity update time known at capture, not the time of this body edit.
    pub upstream_updated_at_millis: Option<i64>,
    /// Time Merl captured this provider snapshot.
    pub observed_at_millis: i64,
    /// Stable provider actor identity when one is available.
    pub actor: Option<ActorId>,
    /// Stable upstream actor ID, when the provider exposes one.
    pub provider_actor_id: Option<&'a str>,
    /// Author of the external entity, independent of who edited this version.
    pub source_author: Option<ActorId>,
    /// Stable provider ID of the external entity's author.
    pub provider_source_author_id: Option<&'a str>,
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
    /// Conversation selected for bounded recent-source context.
    pub context_scope_id: String,
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
    /// Accepted revision visible immediately before this source was captured.
    pub interpretation_basis_revision: ProjectRevision,
    /// False for pre-migration captures whose historical accepted basis was not recorded.
    pub interpretation_basis_known: bool,
    /// Provider time at which this version became visible.
    pub occurred_at_millis: i64,
    /// Provider creation time of the external entity.
    pub created_at_millis: i64,
    /// Entity update time retained by the first capture, when known.
    pub upstream_updated_at_millis: Option<i64>,
    /// Time Merl captured the provider snapshot.
    pub observed_at_millis: i64,
    /// Stable upstream actor ID, if available.
    pub provider_actor_id: Option<String>,
    /// Author of the external entity, which may differ from the edit actor.
    pub source_author: Option<ActorId>,
    /// Actor who created this source version or edit.
    pub version_actor: Option<ActorId>,
    /// Stable provider ID of the entity author, when known.
    pub provider_source_author_id: Option<String>,
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

/// Immutable intent and causal input for one compiler attempt.
#[derive(Debug)]
pub struct CompilationIntent<'a> {
    /// Stable identity of this attempt.
    pub id: &'a str,
    /// Source version that triggered the context.
    pub source: &'a SourceVersionId,
    /// Exact rendered input, retained behind an erasable payload reference.
    pub context: &'a [u8],
    /// Recent source versions in their rendered order.
    pub source_window: &'a [SourceVersionId],
    /// Accepted objects visible at the historical interpretation basis.
    pub objects: &'a [(ObjectId, ObjectRevision)],
    /// The historical accepted revision, independent of later policy evaluation.
    pub interpretation_basis_revision: ProjectRevision,
    /// Inclusive source-observation cutoff.
    pub source_observation_cutoff: u64,
    /// Bounded structural protocol identifiers.
    pub renderer_version: &'a str,
    /// Bounded structural selection-rule identifier.
    pub selector_version: &'a str,
    /// Compiler implementation identity.
    pub compiler_id: &'a str,
    /// Compiler implementation version.
    pub compiler_version: &'a str,
    /// Model identity, or `deterministic` for an offline extractor.
    pub model_id: &'a str,
    /// Digest of the externally managed prompt; no prompt prose enters the log.
    pub prompt_digest: [u8; 32],
    /// `live`, `replay`, `eval`, or `hindsight`.
    pub mode: &'a str,
    /// Hard limits used for this attempt.
    pub max_input_bytes: usize,
    /// Hard limit on encoded response bytes.
    pub max_output_bytes: usize,
    /// Provider-facing limit on generated model tokens.
    pub max_output_tokens: usize,
    /// Hard limit on assertion count.
    pub max_assertions: usize,
    /// Hard limit on expansion requests.
    pub max_context_requests: usize,
    /// Hard limit on expansion rounds.
    pub max_expansion_rounds: usize,
    /// Hard limit on referenced payload bytes.
    pub max_payload_bytes: usize,
    /// Hard cap on selected source versions.
    pub max_source_window: usize,
    /// Hard cap on selected accepted objects.
    pub max_objects: usize,
    /// Time at which work became durable and recoverable.
    pub started_at_millis: i64,
}

/// Immutable outcome of a prepared compiler attempt.
#[derive(Debug)]
pub struct CompilationResult<'a> {
    /// Identity of the prepared attempt.
    pub run_id: &'a str,
    /// Stable error code on failure, absent on success.
    pub failure_code: Option<&'a str>,
    /// Validated structured response, protected because values may later include text.
    pub response: Option<&'a [u8]>,
    /// Validated structural assertions emitted by a successful response.
    pub assertions: &'a [StructuralAssertion],
    /// The compiler requested another bounded context round.
    pub needs_context: bool,
    /// Completion time, including failures.
    pub completed_at_millis: i64,
}

/// A compiler assertion without copied source prose or arbitrary inline text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructuralAssertion {
    /// Source version and exact byte span that support the assertion.
    pub source: SourceVersionId,
    /// Inclusive start byte in the protected source body.
    pub span_start: usize,
    /// Exclusive end byte in the protected source body.
    pub span_end: usize,
    /// Bounded structural subject identifier.
    pub subject: String,
    /// Bounded predicate identifier.
    pub predicate: String,
    /// Bounded structural value identifier or payload reference.
    pub value: String,
    /// Speech act, epistemic basis, and polarity remain separate policy axes.
    pub act: String,
    /// `observed`, `inferred`, or `reported`.
    pub epistemic_basis: String,
    /// `positive` or `negative`.
    pub polarity: String,
    /// Confidence in thousandths, avoiding float ambiguity in the event log.
    pub confidence_millis: u16,
    /// Speaker who made this assertion, not a quoted authority.
    pub asserted_by: Option<String>,
    /// Quoted or relayed actor, if one was named.
    pub attributed_to: Option<String>,
    /// Whether the attribution was independently verified.
    pub attribution_verified: bool,
}

/// A view of required coverage that does not confuse cold optional material with gaps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SemanticCoverage {
    /// Latest captured observation.
    pub observation_head: u64,
    /// Number of required versions with no successful compilation.
    pub required_gaps: u64,
    /// Required versions whose latest live compiler attempt failed.
    pub required_failed: u64,
    /// Number of optional versions retained without successful compilation.
    pub optional_cold: u64,
}

/// Inspectable outcome of a retained compiler attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilationRunStatus {
    /// Source interpreted by the run.
    pub source: SourceVersionId,
    /// Live, replay, eval, or hindsight.
    pub mode: String,
    /// Whether a bounded response was persisted.
    pub succeeded: bool,
    /// A bounded response requested more context instead of finishing extraction.
    pub needs_context: bool,
    /// Whether this run has an immutable outcome. Pending work survives restart.
    pub completed: bool,
    /// Durable scheduling time, used to reject impossible completion times.
    pub started_at_millis: i64,
    /// Stable failure code, when no response was accepted.
    pub failure_code: Option<String>,
    /// Historical accepted-state basis used to interpret the source.
    pub interpretation_basis_revision: ProjectRevision,
    /// Highest source observation made available to the compiler.
    pub source_observation_cutoff: u64,
    /// Digest of exact rendered input bytes.
    pub context_digest: [u8; 32],
    /// Versioned compiler implementation identity.
    pub compiler_id: String,
    /// Compiler implementation version.
    pub compiler_version: String,
    /// Model identity used by the adapter.
    pub model_id: String,
    /// Prompt or ruleset digest.
    pub prompt_digest: [u8; 32],
    /// Exact compiler budgets fixed by the first attempt with this run ID.
    pub limits: [usize; 9],
}

/// Exact compiler input and manifest retained for a pending run.
#[derive(Debug)]
pub struct StoredCompilationContext {
    /// Protected rendered bytes used for the original attempt.
    pub rendered: Vec<u8>,
    /// Source versions selected when the intent was committed.
    pub source_window: Vec<SourceVersionId>,
    /// Historical object revisions selected for that input.
    pub objects: Vec<(ObjectId, ObjectRevision)>,
}

/// Bounded historical objects and whether the selector omitted more.
#[derive(Debug)]
pub struct SelectedObjects {
    /// The chosen historical object revisions, sorted by stable object ID.
    pub items: Vec<(ObjectId, ObjectRevision, Option<PayloadId>)>,
    /// True when other objects existed at the same basis revision.
    pub truncated: bool,
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
        if version < SCHEMA_VERSION {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if version < 1 {
                transaction.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
            }
            if version < 2 {
                transaction.execute_batch(include_str!("../migrations/0002_sources.sql"))?;
            }
            if version < 3 {
                transaction.execute_batch(include_str!("../migrations/0003_compilation.sql"))?;
            }
            if version < 4 {
                transaction.execute_batch(include_str!("../migrations/0004_policy.sql"))?;
            }
            if version < 5 {
                transaction
                    .execute_batch(include_str!("../migrations/0005_policy_conflicts.sql"))?;
            }
            transaction
                .execute_batch(include_str!("../migrations/0006_policy_event_origins.sql"))?;
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
        self.capture_source_version_with_basis(project, capture, true)
    }

    /// Captures historical bytes without pretending the current accepted revision
    /// was available when the source was authored.
    ///
    /// The sequential replay runner supplies a basis after it processes each
    /// earlier observation. This method belongs only to isolated historical imports.
    ///
    /// # Errors
    /// Uses the same identity and lineage checks as ordinary source capture.
    pub fn capture_historical_source_version(
        &mut self,
        project: &ProjectId,
        capture: &SourceCapture<'_>,
    ) -> Result<bool, StoreError> {
        self.capture_source_version_with_basis(project, capture, false)
    }

    fn capture_source_version_with_basis(
        &mut self,
        project: &ProjectId,
        capture: &SourceCapture<'_>,
        basis_known: bool,
    ) -> Result<bool, StoreError> {
        validate_source_capture(capture)?;
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
        let basis = CaptureBasis {
            revision: current_revision_in_transaction(&transaction, project)?,
            known: basis_known,
        };
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
            basis,
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

    /// Checks that a provider namespace is attached to this project.
    ///
    /// # Errors
    /// Returns a storage error if the binding table cannot be read.
    pub fn source_binding_exists(
        &self,
        project: &ProjectId,
        binding: &SourceBindingId,
    ) -> Result<bool, StoreError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM source_bindings WHERE project_id=?1 AND id=?2)",
                params![project.as_str(), binding.as_str()],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
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
                    supersedes_id, ambiguous_order_with_previous, sequence, interpretation_basis_revision,
                    interpretation_basis_known,
                    occurred_at_millis, created_at_millis, upstream_updated_at_millis, observed_at_millis,
                    provider_actor_id, compilation_mode, coverage_requirement,
                    capture_policy_version, body_digest, missing_body_reason, payload_id,
                    edit_diff_payload_id, edit_deleted_at_millis, context_scope_id,
                    source_author_id, provider_source_author_id, actor_id
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
                context_scope_id: if row.context_scope_id.is_empty() {
                    row.source_id.clone()
                } else {
                    row.context_scope_id
                },
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
                interpretation_basis_revision: ProjectRevision::from(
                    u64::try_from(row.interpretation_basis_revision)
                        .map_err(|_| StoreError::CorruptHistory)?,
                ),
                interpretation_basis_known: row.interpretation_basis_known != 0,
                occurred_at_millis: row.occurred_at_millis,
                created_at_millis: row.created_at_millis,
                upstream_updated_at_millis: row.upstream_updated_at_millis,
                observed_at_millis: row.observed_at_millis,
                provider_actor_id: row.provider_actor_id,
                source_author: row
                    .source_author_id
                    .map(|value| ActorId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                version_actor: row
                    .version_actor_id
                    .map(|value| ActorId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                provider_source_author_id: row.provider_source_author_id,
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

    /// Selects recent source versions from the same conversation at a causal cutoff.
    ///
    /// # Errors
    /// Rejects invalid limits or corrupt stored identities.
    pub fn recent_source_versions_in_scope(
        &self,
        project: &ProjectId,
        scope: &str,
        cutoff: u64,
        limit: usize,
    ) -> Result<Vec<SourceVersionId>, StoreError> {
        if !valid_provider_id(scope) || limit == 0 {
            return Err(StoreError::InvalidCompilation);
        }
        let mut statement = self.connection.prepare(
            "SELECT id FROM source_versions
             WHERE project_id=?1 AND COALESCE(NULLIF(context_scope_id,''),source_id)=?2
               AND sequence<=?3 ORDER BY sequence DESC LIMIT ?4",
        )?;
        let cutoff = i64::try_from(cutoff).map_err(|_| StoreError::InvalidCompilation)?;
        let limit = i64::try_from(limit).map_err(|_| StoreError::InvalidCompilation)?;
        let rows = statement.query_map(params![project.as_str(), scope, cutoff, limit], |row| {
            row.get::<_, String>(0)
        })?;
        let mut selected = rows
            .map(|row| {
                SourceVersionId::try_from(row?.as_str()).map_err(|_| StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, _>>()?;
        selected.reverse();
        Ok(selected)
    }

    /// Binds the next historical observation to the state reached so far.
    ///
    /// A replay starts at project revision zero and advances observations in
    /// capture order. The basis is recorded once; callers cannot supply one.
    ///
    /// # Errors
    /// Rejects a skipped observation or a future state before the first one.
    pub fn bind_next_replay_position(
        &mut self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<ProjectRevision, StoreError> {
        let item = self
            .source_version(project, source)?
            .ok_or(StoreError::InvalidCompilation)?;
        if item.interpretation_basis_known {
            return Err(StoreError::InvalidCompilation);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT interpretation_basis_revision FROM replay_positions
                 WHERE project_id=?1 AND source_version_id=?2",
                params![project.as_str(), source.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(basis) = existing {
            return Ok(ProjectRevision::from(
                u64::try_from(basis).map_err(|_| StoreError::CorruptHistory)?,
            ));
        }
        let previous: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(source_sequence),0) FROM replay_positions WHERE project_id=?1",
            [project.as_str()],
            |row| row.get(0),
        )?;
        let sequence = i64::try_from(item.sequence).map_err(|_| StoreError::InvalidCompilation)?;
        let basis = current_revision_in_transaction(&transaction, project)?;
        if sequence != previous + 1 || (sequence == 1 && basis != 0) {
            return Err(StoreError::InvalidCompilation);
        }
        transaction.execute(
            "INSERT INTO replay_positions
             (project_id,source_version_id,source_sequence,interpretation_basis_revision)
             VALUES (?1,?2,?3,?4)",
            params![project.as_str(), source.as_str(), sequence, basis],
        )?;
        transaction.commit()?;
        Ok(ProjectRevision::from(
            u64::try_from(basis).map_err(|_| StoreError::CorruptHistory)?,
        ))
    }

    /// Returns the immutable interpretation basis established by sequential replay.
    ///
    /// # Errors
    /// Rejects corrupt basis metadata or a failed SQLite read.
    pub fn replay_basis(
        &self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<Option<ProjectRevision>, StoreError> {
        let basis: Option<i64> = self
            .connection
            .query_row(
                "SELECT interpretation_basis_revision FROM replay_positions
             WHERE project_id=?1 AND source_version_id=?2",
                params![project.as_str(), source.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        basis
            .map(|value| {
                Ok(ProjectRevision::from(
                    u64::try_from(value).map_err(|_| StoreError::CorruptHistory)?,
                ))
            })
            .transpose()
    }

    /// Lists accepted object revisions as they stood at a historical revision.
    ///
    /// # Errors
    /// Fails if the requested revision is in the future or history is damaged.
    pub fn objects_at_revision(
        &self,
        project: &ProjectId,
        revision: ProjectRevision,
        limit: usize,
    ) -> Result<SelectedObjects, StoreError> {
        if revision > self.project_revision(project)? || limit == 0 {
            return Err(StoreError::InvalidCompilation);
        }
        let limit = i64::try_from(limit).map_err(|_| StoreError::InvalidCompilation)?;
        let lookahead = limit.checked_add(1).ok_or(StoreError::InvalidCompilation)?;
        let mut statement = self.connection.prepare(
            "WITH history AS (
               SELECT domain_events.object_id, domain_events.payload_id,
                      COUNT(*) OVER (PARTITION BY domain_events.object_id) AS object_revision,
                      ROW_NUMBER() OVER (PARTITION BY domain_events.object_id
                        ORDER BY domain_event_batches.revision DESC, domain_events.event_index DESC) AS rank
               FROM domain_events JOIN domain_event_batches
                 ON domain_event_batches.project_id = domain_events.project_id
                AND domain_event_batches.id = domain_events.batch_id
               WHERE domain_events.project_id = ?1 AND domain_event_batches.revision <= ?2
             ) SELECT object_id, payload_id, object_revision FROM history
               WHERE rank = 1 ORDER BY object_id LIMIT ?3",
        )?;
        let basis = i64::try_from(revision.get()).map_err(|_| StoreError::InvalidCompilation)?;
        let mut rows = statement.query(params![project.as_str(), basis, lookahead])?;
        let mut objects = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let payload: Option<String> = row.get(1)?;
            let count: i64 = row.get(2)?;
            objects.push((
                ObjectId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?,
                ObjectRevision::try_from(
                    u64::try_from(count).map_err(|_| StoreError::CorruptHistory)?,
                )
                .map_err(|_| StoreError::CorruptHistory)?,
                payload
                    .map(|value| PayloadId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
            ));
        }
        let truncated =
            objects.len() > usize::try_from(limit).map_err(|_| StoreError::InvalidCompilation)?;
        objects.truncate(usize::try_from(limit).map_err(|_| StoreError::InvalidCompilation)?);
        Ok(SelectedObjects {
            items: objects,
            truncated,
        })
    }

    /// Persists work and its exact causal input before the compiler runs.
    ///
    /// # Errors
    /// Rejects noncausal contexts, invalid structural metadata, or storage failures.
    pub fn prepare_compilation(
        &mut self,
        project: &ProjectId,
        record: &CompilationIntent<'_>,
    ) -> Result<(), StoreError> {
        validate_compilation_intent(self, project, record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let context_payload = format!("ctx_{}", record.id);
        let context_digest: [u8; 32] = Sha256::digest(record.context).into();
        insert_protected_payload(
            &transaction,
            project,
            &PayloadId::try_from(context_payload.as_str())
                .map_err(|_| StoreError::InvalidCompilation)?,
            record.context,
            &Sha256::digest(record.context),
        )?;
        let interpretation_basis = i64::try_from(record.interpretation_basis_revision.get())
            .map_err(|_| StoreError::InvalidCompilation)?;
        let cutoff = i64::try_from(record.source_observation_cutoff)
            .map_err(|_| StoreError::InvalidCompilation)?;
        let max_input =
            i64::try_from(record.max_input_bytes).map_err(|_| StoreError::InvalidCompilation)?;
        let max_output =
            i64::try_from(record.max_output_bytes).map_err(|_| StoreError::InvalidCompilation)?;
        let max_tokens =
            i64::try_from(record.max_output_tokens).map_err(|_| StoreError::InvalidCompilation)?;
        let max_assertions =
            i64::try_from(record.max_assertions).map_err(|_| StoreError::InvalidCompilation)?;
        let max_requests = i64::try_from(record.max_context_requests)
            .map_err(|_| StoreError::InvalidCompilation)?;
        let max_rounds = i64::try_from(record.max_expansion_rounds)
            .map_err(|_| StoreError::InvalidCompilation)?;
        let max_payload =
            i64::try_from(record.max_payload_bytes).map_err(|_| StoreError::InvalidCompilation)?;
        let max_source_window =
            i64::try_from(record.max_source_window).map_err(|_| StoreError::InvalidCompilation)?;
        let max_objects =
            i64::try_from(record.max_objects).map_err(|_| StoreError::InvalidCompilation)?;
        transaction.execute(
            "INSERT INTO compilation_runs (
              project_id,id,source_version_id,context_digest,context_payload_id,
              interpretation_basis_revision,source_observation_cutoff,renderer_version,selector_version,
              max_input_bytes,max_output_bytes,max_output_tokens,max_assertions,max_context_requests,max_expansion_rounds,max_payload_bytes,
              max_source_window,max_objects,
              compiler_id,compiler_version,model_id,prompt_digest,mode,started_at_millis
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
            params![project.as_str(), record.id, record.source.as_str(), context_digest.as_slice(),
                context_payload, interpretation_basis, cutoff,
                record.renderer_version, record.selector_version, max_input, max_output,
                max_tokens, max_assertions, max_requests, max_rounds, max_payload,
                max_source_window, max_objects,
                record.compiler_id, record.compiler_version, record.model_id, record.prompt_digest.as_slice(),
                record.mode, record.started_at_millis],
        )?;
        insert_context_references(&transaction, project, record)?;
        transaction.commit()?;
        Ok(())
    }

    /// Finishes a prepared attempt without mutating its run intent or accepted state.
    ///
    /// # Errors
    /// Rejects an unknown, already completed, or structurally invalid result.
    pub fn complete_compilation(
        &mut self,
        project: &ProjectId,
        result: &CompilationResult<'_>,
    ) -> Result<(), StoreError> {
        let status = self
            .compilation_run_status(project, result.run_id)?
            .ok_or(StoreError::InvalidCompilation)?;
        if status.completed
            || result.completed_at_millis < status.started_at_millis
            || (result.failure_code.is_none() != result.response.is_some())
            || result
                .response
                .is_some_and(|bytes| bytes.len() > status.limits[1])
            || result.assertions.len() > status.limits[3]
            || (result.response.is_none() && !result.assertions.is_empty())
            || (result.needs_context && result.response.is_none())
            || result
                .failure_code
                .is_some_and(|code| !valid_record_id(code))
        {
            return Err(StoreError::InvalidCompilation);
        }
        let source_window = self.compilation_context_sources(project, result.run_id)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let response_payload = format!("rsp_{}", result.run_id);
        let response_digest = result.response.map(Sha256::digest);
        if let Some(response) = result.response {
            insert_protected_payload(
                &transaction,
                project,
                &PayloadId::try_from(response_payload.as_str())
                    .map_err(|_| StoreError::InvalidCompilation)?,
                response,
                response_digest
                    .as_ref()
                    .ok_or(StoreError::InvalidCompilation)?,
            )?;
        }
        transaction.execute(
            "INSERT INTO compilation_results (project_id,run_id,outcome,failure_code,
             response_digest,response_payload_id,completed_at_millis)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                project.as_str(),
                result.run_id,
                if result.failure_code.is_some() {
                    "failed"
                } else if result.needs_context {
                    "needs_context"
                } else {
                    "succeeded"
                },
                result.failure_code,
                response_digest.as_ref().map(AsRef::<[u8]>::as_ref),
                result.response.map(|_| response_payload.as_str()),
                result.completed_at_millis
            ],
        )?;
        insert_assertions(&transaction, project, result, &source_window)?;
        transaction.commit()?;
        Ok(())
    }

    /// Reports semantic coverage without loading or interpreting cold payloads.
    ///
    /// # Errors
    /// Returns a storage error if the project is unavailable.
    pub fn semantic_coverage(&self, project: &ProjectId) -> Result<SemanticCoverage, StoreError> {
        let observation_head = self.source_observation_head(project)?;
        let (required_gaps, required_failed, optional_cold): (i64, i64, i64) =
            self.connection.query_row(
                "SELECT
              COALESCE(SUM(CASE WHEN coverage_requirement='required' AND NOT EXISTS (
                SELECT 1 FROM compilation_runs JOIN compilation_results
                  ON compilation_results.project_id=compilation_runs.project_id
                 AND compilation_results.run_id=compilation_runs.id
                WHERE compilation_runs.project_id=source_versions.project_id
                  AND source_version_id=source_versions.id AND compilation_results.outcome='succeeded' AND mode='live'
              ) THEN 1 ELSE 0 END),0),
              COALESCE(SUM(CASE WHEN coverage_requirement='required' AND EXISTS (
                SELECT 1 FROM compilation_runs JOIN compilation_results
                  ON compilation_results.project_id=compilation_runs.project_id
                 AND compilation_results.run_id=compilation_runs.id
                WHERE compilation_runs.project_id=source_versions.project_id
                  AND source_version_id=source_versions.id AND compilation_results.outcome='failed' AND mode='live'
              ) AND NOT EXISTS (
                SELECT 1 FROM compilation_runs JOIN compilation_results
                  ON compilation_results.project_id=compilation_runs.project_id
                 AND compilation_results.run_id=compilation_runs.id
                WHERE compilation_runs.project_id=source_versions.project_id
                  AND source_version_id=source_versions.id AND compilation_results.outcome='succeeded' AND mode='live'
              ) THEN 1 ELSE 0 END),0),
              COALESCE(SUM(CASE WHEN coverage_requirement='optional' AND NOT EXISTS (
                SELECT 1 FROM compilation_runs JOIN compilation_results
                  ON compilation_results.project_id=compilation_runs.project_id
                 AND compilation_results.run_id=compilation_runs.id
                WHERE compilation_runs.project_id=source_versions.project_id
                  AND source_version_id=source_versions.id AND compilation_results.outcome='succeeded' AND mode='live'
              ) THEN 1 ELSE 0 END),0)
             FROM source_versions WHERE project_id=?1",
                [project.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        Ok(SemanticCoverage {
            observation_head,
            required_gaps: u64::try_from(required_gaps).map_err(|_| StoreError::CorruptHistory)?,
            required_failed: u64::try_from(required_failed)
                .map_err(|_| StoreError::CorruptHistory)?,
            optional_cold: u64::try_from(optional_cold).map_err(|_| StoreError::CorruptHistory)?,
        })
    }

    /// Finds the source associated with a prior run identity, if any.
    ///
    /// # Errors
    /// Returns an error if stored identity is corrupt or SQLite cannot read it.
    pub fn compilation_run_source(
        &self,
        project: &ProjectId,
        id: &str,
    ) -> Result<Option<SourceVersionId>, StoreError> {
        let source: Option<String> = self
            .connection
            .query_row(
                "SELECT source_version_id FROM compilation_runs WHERE project_id=?1 AND id=?2",
                params![project.as_str(), id],
                |row| row.get(0),
            )
            .optional()?;
        source
            .map(|value| {
                SourceVersionId::try_from(value.as_str()).map_err(|_| StoreError::CorruptHistory)
            })
            .transpose()
    }

    /// Returns the provenance and outcome needed to inspect or reuse a run.
    ///
    /// # Errors
    /// Rejects corrupt structural metadata or a failed SQLite read.
    pub fn compilation_run_status(
        &self,
        project: &ProjectId,
        id: &str,
    ) -> Result<Option<CompilationRunStatus>, StoreError> {
        type RawStatus = (
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            i64,
            Vec<u8>,
            String,
            String,
            String,
            Vec<u8>,
            [i64; 9],
            i64,
        );
        let raw: Option<RawStatus> = self.connection.query_row(
            "SELECT source_version_id,mode,compilation_results.outcome,compilation_results.failure_code,interpretation_basis_revision,
                    source_observation_cutoff,context_digest,compiler_id,compiler_version,model_id,prompt_digest,
                    max_input_bytes,max_output_bytes,max_output_tokens,max_assertions,
                    max_context_requests,max_expansion_rounds,max_payload_bytes,
                    max_source_window,max_objects,started_at_millis
             FROM compilation_runs LEFT JOIN compilation_results
               ON compilation_results.project_id=compilation_runs.project_id
              AND compilation_results.run_id=compilation_runs.id
             WHERE compilation_runs.project_id=?1 AND compilation_runs.id=?2",
            params![project.as_str(), id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,
                row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                row.get(8)?, row.get(9)?, row.get(10)?,
                [row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                 row.get(15)?, row.get(16)?, row.get(17)?, row.get(18)?, row.get(19)?], row.get(20)?)),
        ).optional()?;
        raw.map(
            |(
                source,
                mode,
                outcome,
                failure_code,
                basis,
                cutoff,
                digest,
                compiler_id,
                compiler_version,
                model_id,
                prompt_digest,
                limits,
                started_at_millis,
            )| {
                Ok(CompilationRunStatus {
                    source: SourceVersionId::try_from(source.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    mode,
                    succeeded: outcome.as_deref() == Some("succeeded"),
                    needs_context: outcome.as_deref() == Some("needs_context"),
                    completed: outcome.is_some(),
                    started_at_millis,
                    failure_code,
                    interpretation_basis_revision: ProjectRevision::from(
                        u64::try_from(basis).map_err(|_| StoreError::CorruptHistory)?,
                    ),
                    source_observation_cutoff: u64::try_from(cutoff)
                        .map_err(|_| StoreError::CorruptHistory)?,
                    context_digest: digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
                    compiler_id,
                    compiler_version,
                    model_id,
                    prompt_digest: prompt_digest
                        .try_into()
                        .map_err(|_| StoreError::CorruptHistory)?,
                    limits: limits
                        .map(|value| usize::try_from(value).map_err(|_| StoreError::CorruptHistory))
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()?
                        .try_into()
                        .map_err(|_| StoreError::CorruptHistory)?,
                })
            },
        )
        .transpose()
    }

    fn compilation_context_sources(
        &self,
        project: &ProjectId,
        run_id: &str,
    ) -> Result<Vec<SourceVersionId>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT source_version_id FROM compilation_context_sources
             WHERE project_id=?1 AND run_id=?2 ORDER BY window_index",
        )?;
        let rows = statement.query_map(params![project.as_str(), run_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| {
            SourceVersionId::try_from(row?.as_str()).map_err(|_| StoreError::CorruptHistory)
        })
        .collect()
    }

    /// Loads the committed context bytes and manifest without rebuilding history.
    ///
    /// # Errors
    /// Reports unavailable protected bytes or corrupt structural provenance.
    pub fn load_compilation_context(
        &self,
        project: &ProjectId,
        run_id: &str,
    ) -> Result<StoredCompilationContext, StoreError> {
        let (payload_id, expected_digest): (String, Vec<u8>) = self.connection.query_row(
            "SELECT context_payload_id,context_digest FROM compilation_runs
             WHERE project_id=?1 AND id=?2",
            params![project.as_str(), run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let payload_id =
            PayloadId::try_from(payload_id.as_str()).map_err(|_| StoreError::CorruptHistory)?;
        let rendered = match self.read_payload(project, &payload_id)? {
            PayloadRead::Available(bytes) => bytes,
            PayloadRead::Unavailable => return Err(StoreError::InvalidCompilation),
        };
        if Sha256::digest(&rendered).as_slice() != expected_digest {
            return Err(StoreError::CorruptHistory);
        }
        let source_window = self.compilation_context_sources(project, run_id)?;
        let mut statement = self.connection.prepare(
            "SELECT object_id,object_revision FROM compilation_context_objects
             WHERE project_id=?1 AND run_id=?2 ORDER BY object_id",
        )?;
        let rows = statement.query_map(params![project.as_str(), run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let objects = rows
            .map(|row| -> Result<(ObjectId, ObjectRevision), StoreError> {
                let (id, revision) = row?;
                Ok((
                    ObjectId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?,
                    ObjectRevision::try_from(
                        u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                    )
                    .map_err(|_| StoreError::CorruptHistory)?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StoredCompilationContext {
            rendered,
            source_window,
            objects,
        })
    }

    /// Lists durable compiler work that has no recorded result yet.
    ///
    /// # Errors
    /// Returns an error if a stored identity is invalid or SQLite cannot read it.
    pub fn pending_compilation_runs(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<(String, SourceVersionId)>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT compilation_runs.id,source_version_id FROM compilation_runs
             LEFT JOIN compilation_results
               ON compilation_results.project_id=compilation_runs.project_id
              AND compilation_results.run_id=compilation_runs.id
             WHERE compilation_runs.project_id=?1 AND compilation_results.run_id IS NULL
             ORDER BY compilation_runs.started_at_millis,compilation_runs.id",
        )?;
        let rows = statement.query_map([project.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (run_id, source) = row?;
            Ok((
                run_id,
                SourceVersionId::try_from(source.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
            ))
        })
        .collect()
    }

    /// Counts recorded attempts for one source, regardless of delivery fan-out.
    ///
    /// # Errors
    /// Returns a storage error if the query fails.
    pub fn compilation_run_count(
        &self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<u64, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM compilation_runs WHERE project_id=?1 AND source_version_id=?2",
            params![project.as_str(), source.as_str()],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
    }

    /// Counts the typed assertions retained for one compiler run.
    ///
    /// # Errors
    /// Returns a storage error if the query fails.
    pub fn observed_assertion_count(
        &self,
        project: &ProjectId,
        run_id: &str,
    ) -> Result<u64, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM observed_assertions WHERE project_id=?1 AND run_id=?2",
            params![project.as_str(), run_id],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
    }

    /// Reads the structural assertions from one run for later policy evaluation.
    ///
    /// # Errors
    /// Rejects corrupt source identities or numeric fields.
    pub fn observed_assertions(
        &self,
        project: &ProjectId,
        run_id: &str,
    ) -> Result<Vec<StructuralAssertion>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT source_version_id,span_start,span_end,subject_id,predicate_id,value_id,
                    act,epistemic_basis,polarity,confidence_millis,asserted_by,attributed_to,
                    attribution_verified
             FROM observed_assertions WHERE project_id=?1 AND run_id=?2 ORDER BY assertion_index",
        )?;
        let rows = statement.query_map(params![project.as_str(), run_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, i64>(12)?,
            ))
        })?;
        rows.map(|row| {
            let (
                source,
                span_start,
                span_end,
                subject,
                predicate,
                value,
                act,
                epistemic_basis,
                polarity,
                confidence,
                asserted_by,
                attributed_to,
                verified,
            ) = row?;
            Ok(StructuralAssertion {
                source: SourceVersionId::try_from(source.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                span_start: usize::try_from(span_start).map_err(|_| StoreError::CorruptHistory)?,
                span_end: usize::try_from(span_end).map_err(|_| StoreError::CorruptHistory)?,
                subject,
                predicate,
                value,
                act,
                epistemic_basis,
                polarity,
                confidence_millis: u16::try_from(confidence)
                    .map_err(|_| StoreError::CorruptHistory)?,
                asserted_by,
                attributed_to,
                attribution_verified: verified != 0,
            })
        })
        .collect()
    }

    /// Registers an agent to receive reference-only entries for accepted batches.
    ///
    /// # Errors
    /// Returns a storage error when the project is absent or the subscription cannot be saved.
    pub fn subscribe_all(
        &mut self,
        project: &ProjectId,
        agent: &AgentId,
    ) -> Result<(), StoreError> {
        self.connection.execute(
            "INSERT OR IGNORE INTO inbox_subscriptions (project_id, agent_id) VALUES (?1, ?2)",
            params![project.as_str(), agent.as_str()],
        )?;
        Ok(())
    }

    /// Returns durable inbox references after a subscriber's project cursor.
    ///
    /// # Errors
    /// Returns an error if stored identities or revisions are invalid.
    pub fn inbox_after(
        &self,
        project: &ProjectId,
        agent: &AgentId,
        cursor: ProjectRevision,
    ) -> Result<Vec<InboxEntry>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT batch_id, project_revision FROM inbox_entries
             WHERE project_id=?1 AND agent_id=?2 AND project_revision>?3
             ORDER BY project_revision",
        )?;
        statement
            .query_map(
                params![project.as_str(), agent.as_str(), to_sql_revision(cursor)?],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?
            .map(|row| {
                let (batch, revision) = row?;
                Ok(InboxEntry {
                    agent: agent.clone(),
                    batch: merl_core::BatchId::try_from(batch.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    revision: ProjectRevision::from(
                        u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                    ),
                })
            })
            .collect()
    }

    /// Commits an evaluated decision after validating exactly what it read and would write.
    ///
    /// A retry with the same evaluation ID and structural meaning returns its original
    /// revision. External wake-up belongs after this method returns successfully.
    ///
    /// # Errors
    /// Rejects stale dependencies, conflicting input identities, invalid provenance,
    /// or any failed SQLite statement without exposing a partial accepted batch.
    pub fn commit_policy_evaluation(
        &mut self,
        evaluation: &PolicyEvaluation,
        provider: Option<&ProviderObservation>,
    ) -> Result<Option<ProjectRevision>, StoreError> {
        validate_policy_shape(evaluation, provider)?;
        let digest = policy_evaluation_digest(evaluation, provider);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(Vec<u8>, Option<i64>, bool)> = transaction
            .query_row(
                "SELECT evaluation_digest, committed_revision,
                    EXISTS(SELECT 1 FROM policy_conflicts c
                           WHERE c.project_id=e.project_id AND c.evaluation_id=e.id)
                 FROM policy_evaluations e
                 WHERE project_id=?1 AND id=?2",
                params![evaluation.project.as_str(), evaluation.id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((prior_digest, revision, conflicted)) = prior {
            if prior_digest != digest {
                return Err(StoreError::PolicyInputConflict);
            }
            if conflicted {
                return Err(StoreError::PolicyConflict);
            }
            return revision
                .map(|value| {
                    u64::try_from(value)
                        .map(ProjectRevision::from)
                        .map_err(|_| StoreError::CorruptHistory)
                })
                .transpose();
        }
        let current = next_revision(&transaction, &evaluation.project)? - 1;
        let duplicate_inputs = validate_policy_inputs(&transaction, evaluation)?;
        if let Some(overlap) = policy_overlap(evaluation, &duplicate_inputs) {
            let conflict = match &overlap {
                PolicyOverlap::Duplicate => None,
                PolicyOverlap::Conflict(detail) => Some(detail),
            };
            insert_policy_record(
                &transaction,
                evaluation,
                digest,
                None,
                conflict,
                conflict.is_none(),
            )?;
            transaction.commit()?;
            return match overlap {
                PolicyOverlap::Duplicate => Ok(None),
                PolicyOverlap::Conflict(_) => Err(StoreError::PolicyConflict),
            };
        }
        let conflict = if to_sql_revision(evaluation.basis_project_revision)? > current {
            Some(PolicyConflictDetail {
                reason_code: "basis_ahead".into(),
                target_id: None,
                expected_revision: Some(to_sql_revision(evaluation.basis_project_revision)?),
                actual_revision: Some(current),
            })
        } else {
            validate_policy_dependencies(&transaction, evaluation)?
        };
        if let Some(conflict) = conflict {
            insert_policy_record(
                &transaction,
                evaluation,
                digest,
                None,
                Some(&conflict),
                false,
            )?;
            transaction.commit()?;
            return Err(StoreError::PolicyConflict);
        }
        let revision = if let Some(batch) = &evaluation.batch {
            let revision = current.checked_add(1).ok_or(StoreError::CorruptHistory)?;
            insert_accepted_batch(&transaction, batch, provider, revision)?;
            Some(revision)
        } else {
            None
        };
        insert_policy_record(&transaction, evaluation, digest, revision, None, false)?;
        if let (Some(batch), Some(revision)) = (&evaluation.batch, revision) {
            transaction.execute(
                "INSERT INTO inbox_entries (project_id, batch_id, agent_id, project_revision)
                 SELECT project_id, ?2, agent_id, ?3 FROM inbox_subscriptions WHERE project_id=?1",
                params![evaluation.project.as_str(), batch.id.as_str(), revision],
            )?;
        }
        transaction.commit()?;
        revision
            .map(|value| {
                u64::try_from(value)
                    .map(ProjectRevision::from)
                    .map_err(|_| StoreError::CorruptHistory)
            })
            .transpose()
    }

    /// Reads a decision and its typed input dispositions without protected text.
    ///
    /// # Errors
    /// Returns an error if a stored policy identity or disposition is invalid.
    #[expect(
        clippy::too_many_lines,
        reason = "audit expansion joins four immutable policy projections"
    )]
    pub fn policy_evaluation(
        &self,
        project: &ProjectId,
        id: &PolicyEvaluationId,
    ) -> Result<Option<RecordedPolicyEvaluation>, StoreError> {
        let header: Option<PolicyHeader> = self
            .connection
            .query_row(
                "SELECT actor_id, policy_version, policy_config_digest, basis_project_revision, committed_revision
                 FROM policy_evaluations WHERE project_id=?1 AND id=?2",
                params![project.as_str(), id.as_str()],
                |row| Ok(PolicyHeader {
                    actor: row.get(0)?,
                    version: row.get(1)?,
                    configuration_digest: row.get(2)?,
                    basis: row.get(3)?,
                    revision: row.get(4)?,
                }),
            )
            .optional()?;
        let Some(header) = header else {
            return Ok(None);
        };
        let conflict = self
            .connection
            .query_row(
                "SELECT reason_code,target_id,expected_revision,actual_revision
                 FROM policy_conflicts WHERE project_id=?1 AND evaluation_id=?2",
                params![project.as_str(), id.as_str()],
                |row| {
                    Ok(PolicyConflictDetail {
                        reason_code: row.get(0)?,
                        target_id: row.get(1)?,
                        expected_revision: row.get(2)?,
                        actual_revision: row.get(3)?,
                    })
                },
            )
            .optional()?;
        let mut statement = self.connection.prepare(
            "SELECT input_kind,input_id,input_digest,disposition,reason_code
             FROM policy_evaluation_inputs WHERE project_id=?1 AND evaluation_id=?2 ORDER BY input_index",
        )?;
        let inputs = statement
            .query_map(params![project.as_str(), id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .map(|row| {
                let (kind, input_id, digest, disposition, reason) = row?;
                let input_id = merl_core::PolicyInputId::try_from(input_id.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?;
                let input = match kind.as_str() {
                    "command" => merl_core::PolicyInput::Command(input_id),
                    "provider_observation" => merl_core::PolicyInput::ProviderObservation(input_id),
                    "administrative_action" => {
                        merl_core::PolicyInput::AdministrativeAction(input_id)
                    }
                    "observed_assertion" => {
                        // The source reference is loaded below from its immutable assertion link.
                        let (run, index): (String, i64) = self.connection.query_row(
                            "SELECT run_id, assertion_index FROM policy_assertion_inputs
                             WHERE project_id=?1 AND evaluation_id=?2 AND input_id=?3",
                            params![project.as_str(), id.as_str(), input_id.as_str()],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )?;
                        merl_core::PolicyInput::ObservedAssertion {
                            id: input_id,
                            run: merl_core::CompilationRunId::try_from(run.as_str())
                                .map_err(|_| StoreError::CorruptHistory)?,
                            index: u32::try_from(index).map_err(|_| StoreError::CorruptHistory)?,
                        }
                    }
                    _ => return Err(StoreError::CorruptHistory),
                };
                let disposition = match disposition.as_str() {
                    "accepted" => PolicyDisposition::Accepted,
                    "candidate" => PolicyDisposition::Candidate,
                    "rejected" => PolicyDisposition::Rejected,
                    "duplicate" => PolicyDisposition::Duplicate,
                    "conflict" => PolicyDisposition::Conflict,
                    _ => return Err(StoreError::CorruptHistory),
                };
                Ok(merl_core::PolicyInputDecision {
                    input,
                    input_digest: digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
                    disposition,
                    reason: merl_core::ReasonCode::try_from(reason.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let mut read_statement = self.connection.prepare(
            "SELECT read_kind,target_id,expected_revision FROM policy_evaluation_reads
             WHERE project_id=?1 AND evaluation_id=?2 ORDER BY read_index",
        )?;
        let reads = read_statement
            .query_map(params![project.as_str(), id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            })?
            .map(|row| {
                let (kind, target, revision) = row?;
                match kind.as_str() {
                    "object" => Ok(PolicyRead::Object {
                        id: ObjectId::try_from(target.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        revision: revision
                            .map(|value| {
                                ObjectRevision::try_from(
                                    u64::try_from(value).map_err(|_| StoreError::CorruptHistory)?,
                                )
                                .map_err(|_| StoreError::CorruptHistory)
                            })
                            .transpose()?,
                    }),
                    "kind_collection" => Ok(PolicyRead::KindCollection {
                        kind: ObjectKind::try_from(target.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        latest_project_revision: ProjectRevision::from(
                            u64::try_from(revision.ok_or(StoreError::CorruptHistory)?)
                                .map_err(|_| StoreError::CorruptHistory)?,
                        ),
                    }),
                    _ => Err(StoreError::CorruptHistory),
                }
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let mut write_statement = self.connection.prepare(
            "SELECT object_id,expected_revision FROM policy_evaluation_writes
             WHERE project_id=?1 AND evaluation_id=?2 ORDER BY object_id",
        )?;
        let writes = write_statement
            .query_map(params![project.as_str(), id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
            })?
            .map(|row| {
                let (object, revision) = row?;
                Ok(merl_core::PolicyWrite {
                    object: ObjectId::try_from(object.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    expected_revision: revision
                        .map(|value| {
                            ObjectRevision::try_from(
                                u64::try_from(value).map_err(|_| StoreError::CorruptHistory)?,
                            )
                            .map_err(|_| StoreError::CorruptHistory)
                        })
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let mut event_statement = self.connection.prepare(
            "SELECT event_id FROM policy_evaluation_domain_events WHERE project_id=?1 AND evaluation_id=?2 ORDER BY event_id",
        )?;
        let events = event_statement
            .query_map(params![project.as_str(), id.as_str()], |row| {
                row.get::<_, String>(0)
            })?
            .map(|row| {
                merl_core::EventId::try_from(row?.as_str()).map_err(|_| StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(Some(RecordedPolicyEvaluation {
            id: id.clone(),
            actor: ActorId::try_from(header.actor.as_str())
                .map_err(|_| StoreError::CorruptHistory)?,
            version: merl_core::PolicyVersion::try_from(header.version.as_str())
                .map_err(|_| StoreError::CorruptHistory)?,
            configuration_digest: header
                .configuration_digest
                .try_into()
                .map_err(|_| StoreError::CorruptHistory)?,
            basis_project_revision: ProjectRevision::from(
                u64::try_from(header.basis).map_err(|_| StoreError::CorruptHistory)?,
            ),
            committed_revision: header
                .revision
                .map(|value| {
                    u64::try_from(value)
                        .map(ProjectRevision::from)
                        .map_err(|_| StoreError::CorruptHistory)
                })
                .transpose()?,
            conflict,
            inputs,
            reads,
            writes,
            events,
        }))
    }

    /// Finds a previously accepted immutable input for retry handling.
    ///
    /// # Errors
    /// Returns an error if structural receipt data is corrupt or unreadable.
    pub fn accepted_policy_input(
        &self,
        project: &ProjectId,
        input: &merl_core::PolicyInput,
    ) -> Result<Option<([u8; 32], ProjectRevision)>, StoreError> {
        let row: Option<(Vec<u8>, i64)> = self
            .connection
            .query_row(
                "SELECT a.input_digest,e.committed_revision FROM accepted_policy_inputs a
             JOIN policy_evaluations e ON e.project_id=a.project_id AND e.id=a.evaluation_id
             WHERE a.project_id=?1 AND a.input_kind=?2 AND a.input_id=?3",
                params![project.as_str(), input.kind(), input.id().as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(digest, revision)| {
            Ok((
                digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
                ProjectRevision::from(
                    u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                ),
            ))
        })
        .transpose()
    }

    /// Checks whether a compiler assertion already caused an accepted transition.
    ///
    /// # Errors
    /// Returns a storage error if the provenance index cannot be read.
    pub fn accepted_assertion(
        &self,
        project: &ProjectId,
        run: &merl_core::CompilationRunId,
        index: u32,
    ) -> Result<bool, StoreError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accepted_assertions
                 WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3)",
                params![project.as_str(), run.as_str(), i64::from(index)],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    /// Finds the policy decision that created an accepted object's latest revision.
    ///
    /// # Errors
    /// Returns an error if the event or evaluation link is unreadable.
    pub fn object_policy_evaluation(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<PolicyEvaluationId>, StoreError> {
        let value: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT p.evaluation_id FROM domain_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN policy_evaluation_domain_events p ON p.project_id=e.project_id AND p.event_id=e.id
             WHERE e.project_id=?1 AND e.object_id=?2 ORDER BY b.revision DESC, e.event_index DESC LIMIT 1",
                params![project.as_str(), object.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        value
            .flatten()
            .map(|value| {
                PolicyEvaluationId::try_from(value.as_str()).map_err(|_| StoreError::CorruptHistory)
            })
            .transpose()
    }

    /// Finds the exact policy input that produced an object's latest event.
    ///
    /// Kernel bootstrap fixtures have no policy path and return `None`.
    ///
    /// # Errors
    /// Returns an error if an accepted event's policy link is incomplete.
    pub fn object_policy_origin(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Option<ObjectPolicyOrigin>, StoreError> {
        let row: Option<(String, Option<String>, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT e.id,p.evaluation_id,p.input_index FROM domain_events e
                 JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
                 LEFT JOIN policy_evaluation_domain_events p
                    ON p.project_id=e.project_id AND p.event_id=e.id
                 WHERE e.project_id=?1 AND e.object_id=?2
                 ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
                params![project.as_str(), object.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((event, Some(evaluation), Some(input_index))) = row else {
            return Ok(None);
        };
        let evaluation = PolicyEvaluationId::try_from(evaluation.as_str())
            .map_err(|_| StoreError::CorruptHistory)?;
        let record = self
            .policy_evaluation(project, &evaluation)?
            .ok_or(StoreError::CorruptHistory)?;
        let index = usize::try_from(input_index).map_err(|_| StoreError::CorruptHistory)?;
        let input = record.inputs.get(index).ok_or(StoreError::CorruptHistory)?;
        if input.disposition != PolicyDisposition::Accepted {
            return Err(StoreError::CorruptHistory);
        }
        Ok(Some(ObjectPolicyOrigin {
            event: merl_core::EventId::try_from(event.as_str())
                .map_err(|_| StoreError::CorruptHistory)?,
            evaluation,
            input: input.input.clone(),
        }))
    }

    /// Seeds a kernel fixture without policy provenance.
    ///
    /// This bypass exists for pre-policy bootstrap tests. Application mutations
    /// use `commit_policy_evaluation` so accepted objects retain their policy path.
    ///
    /// # Errors
    /// Returns an error for missing projects, invalid references, duplicate IDs,
    /// or a failed SQLite transaction. No partial revision becomes visible.
    pub fn commit_unchecked_bootstrap(
        &mut self,
        batch: &DomainEventBatch,
    ) -> Result<ProjectRevision, StoreError> {
        self.commit_inner(batch)
    }

    fn commit_inner(&mut self, batch: &DomainEventBatch) -> Result<ProjectRevision, StoreError> {
        if batch.events.is_empty() {
            return Err(StoreError::InvalidBatch);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision = next_revision(&transaction, &batch.project)?;
        insert_accepted_batch(&transaction, batch, None, revision)?;
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

fn to_sql_revision(revision: ProjectRevision) -> Result<i64, StoreError> {
    i64::try_from(revision.get()).map_err(|_| StoreError::CorruptHistory)
}

fn validate_policy_shape(
    evaluation: &PolicyEvaluation,
    provider: Option<&ProviderObservation>,
) -> Result<(), StoreError> {
    use std::collections::HashSet;

    if evaluation.inputs.is_empty() {
        return Err(StoreError::InvalidPolicyEvaluation);
    }
    let mut input_ids = HashSet::new();
    for input in &evaluation.inputs {
        if !input_ids.insert((input.input.kind(), input.input.id().as_str())) {
            return Err(StoreError::InvalidPolicyEvaluation);
        }
    }
    let accepted = evaluation
        .inputs
        .iter()
        .any(|input| input.disposition == PolicyDisposition::Accepted);
    if accepted != evaluation.batch.is_some() {
        return Err(StoreError::InvalidPolicyEvaluation);
    }
    match (&evaluation.batch, provider) {
        (Some(batch), provider) => {
            if batch.project != evaluation.project
                || batch.actor != evaluation.actor
                || batch.events.is_empty()
            {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            let event_objects: HashSet<&str> = batch
                .events
                .iter()
                .map(|event| match event {
                    DomainEvent::PutObject { object, .. } => object.as_str(),
                })
                .collect();
            let writes: HashSet<&str> = evaluation
                .writes
                .iter()
                .map(|write| write.object.as_str())
                .collect();
            if event_objects.len() != batch.events.len()
                || writes.len() != evaluation.writes.len()
                || event_objects != writes
            {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            if evaluation.event_origins.len() != batch.events.len() {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            let mut origin_inputs = HashSet::new();
            let mut origin_events = HashSet::new();
            for origin in &evaluation.event_origins {
                let index = usize::try_from(origin.input_index)
                    .map_err(|_| StoreError::InvalidPolicyEvaluation)?;
                if !evaluation
                    .inputs
                    .get(index)
                    .is_some_and(|input| input.disposition == PolicyDisposition::Accepted)
                    || !origin_inputs.insert(index)
                    || !origin_events.insert(origin.event.as_str())
                {
                    return Err(StoreError::InvalidPolicyEvaluation);
                }
            }
            if evaluation
                .inputs
                .iter()
                .filter(|input| input.disposition == PolicyDisposition::Accepted)
                .count()
                != origin_inputs.len()
                || !batch.events.iter().all(|event| {
                    let DomainEvent::PutObject { id, .. } = event;
                    origin_events.contains(id.as_str())
                })
            {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            if let Some(provider) = provider {
                validate_provider_batch(batch, provider)?;
                if !evaluation.inputs.iter().any(|input| {
                    input.input.kind() == "provider_observation"
                        && input.input.id() == &provider.id
                        && input.disposition == PolicyDisposition::Accepted
                }) {
                    return Err(StoreError::InvalidPolicyEvaluation);
                }
            } else if evaluation.inputs.iter().any(|input| {
                input.input.kind() == "provider_observation"
                    && input.disposition == PolicyDisposition::Accepted
            }) {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
        }
        (None, None) if evaluation.writes.is_empty() && evaluation.event_origins.is_empty() => {}
        _ => return Err(StoreError::InvalidPolicyEvaluation),
    }
    Ok(())
}

fn validate_policy_inputs(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
) -> Result<Vec<bool>, StoreError> {
    let mut duplicates = Vec::with_capacity(evaluation.inputs.len());
    for input in &evaluation.inputs {
        let receipt: Option<Vec<u8>> = transaction
            .query_row(
                "SELECT input_digest FROM policy_input_receipts
                 WHERE project_id=?1 AND input_kind=?2 AND input_id=?3",
                params![
                    evaluation.project.as_str(),
                    input.input.kind(),
                    input.input.id().as_str()
                ],
                |row| row.get(0),
            )
            .optional()?;
        if receipt.is_some_and(|digest| digest != input.input_digest) {
            return Err(StoreError::PolicyInputConflict);
        }
        let mut duplicate = false;
        if input.disposition == PolicyDisposition::Accepted {
            let prior: Option<Vec<u8>> = transaction
                .query_row(
                    "SELECT input_digest FROM accepted_policy_inputs
                     WHERE project_id=?1 AND input_kind=?2 AND input_id=?3",
                    params![
                        evaluation.project.as_str(),
                        input.input.kind(),
                        input.input.id().as_str()
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(prior_digest) = prior {
                if prior_digest != input.input_digest {
                    return Err(StoreError::PolicyInputConflict);
                }
                duplicate = true;
            }
        }
        if let merl_core::PolicyInput::ObservedAssertion { run, index, .. } = &input.input {
            validate_observed_input(transaction, &evaluation.project, run, *index)?;
            if input.disposition == PolicyDisposition::Accepted {
                duplicate |= transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM accepted_assertions
                     WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3)",
                    params![evaluation.project.as_str(), run.as_str(), i64::from(*index)],
                    |row| row.get::<_, bool>(0),
                )?;
            }
        }
        duplicates.push(duplicate);
    }
    Ok(duplicates)
}

fn policy_overlap(evaluation: &PolicyEvaluation, duplicates: &[bool]) -> Option<PolicyOverlap> {
    let accepted_count = evaluation
        .inputs
        .iter()
        .filter(|input| input.disposition == PolicyDisposition::Accepted)
        .count();
    let duplicate_count = duplicates.iter().filter(|duplicate| **duplicate).count();
    if duplicate_count == 0 {
        return None;
    }
    if duplicate_count == accepted_count {
        return Some(PolicyOverlap::Duplicate);
    }
    let index = duplicates.iter().position(|duplicate| *duplicate)?;
    Some(PolicyOverlap::Conflict(PolicyConflictDetail {
        reason_code: "accepted_input_overlap".into(),
        target_id: Some(evaluation.inputs[index].input.id().as_str().into()),
        expected_revision: None,
        actual_revision: None,
    }))
}

fn validate_policy_dependencies(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
) -> Result<Option<PolicyConflictDetail>, StoreError> {
    let basis = to_sql_revision(evaluation.basis_project_revision)?;
    for read in &evaluation.reads {
        match read {
            PolicyRead::Object { id, revision } => {
                let actual: Option<i64> = transaction
                    .query_row(
                        "SELECT object_revision FROM objects WHERE project_id=?1 AND id=?2",
                        params![evaluation.project.as_str(), id.as_str()],
                        |row| row.get(0),
                    )
                    .optional()?;
                let expected = revision
                    .map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory))
                    .transpose()?;
                if actual != expected {
                    return Ok(Some(PolicyConflictDetail {
                        reason_code: "object_read_changed".into(),
                        target_id: Some(id.as_str().into()),
                        expected_revision: expected,
                        actual_revision: actual,
                    }));
                }
            }
            PolicyRead::KindCollection {
                kind,
                latest_project_revision,
            } => {
                let actual: i64 = transaction.query_row(
                    "SELECT COALESCE(MAX(project_revision), 0) FROM objects WHERE project_id=?1 AND kind=?2",
                    params![evaluation.project.as_str(), kind.as_str()],
                    |row| row.get(0),
                )?;
                let expected = to_sql_revision(*latest_project_revision)?;
                if actual != expected {
                    return Ok(Some(PolicyConflictDetail {
                        reason_code: "kind_collection_changed".into(),
                        target_id: Some(kind.as_str().into()),
                        expected_revision: Some(expected),
                        actual_revision: Some(actual),
                    }));
                }
            }
        }
    }
    for write in &evaluation.writes {
        let actual: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT object_revision,project_revision FROM objects WHERE project_id=?1 AND id=?2",
                params![evaluation.project.as_str(), write.object.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let expected = write
            .expected_revision
            .map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory))
            .transpose()?;
        if actual.map(|row| row.0) != expected || actual.is_some_and(|row| row.1 > basis) {
            return Ok(Some(PolicyConflictDetail {
                reason_code: "object_write_changed".into(),
                target_id: Some(write.object.as_str().into()),
                expected_revision: expected,
                actual_revision: actual.map(|row| row.0),
            }));
        }
    }
    Ok(None)
}

fn validate_observed_input(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    run: &merl_core::CompilationRunId,
    index: u32,
) -> Result<(), StoreError> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM observed_assertions WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3)",
        params![project.as_str(), run.as_str(), i64::from(index)],
        |row| row.get(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::InvalidPolicyEvaluation)
    }
}

fn insert_policy_record(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    digest: [u8; 32],
    revision: Option<i64>,
    conflict: Option<&PolicyConflictDetail>,
    duplicate: bool,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO policy_evaluations (project_id,id,actor_id,policy_version,policy_config_digest,basis_project_revision,evaluation_digest,batch_id,committed_revision)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![evaluation.project.as_str(), evaluation.id.as_str(), evaluation.actor.as_str(),
            evaluation.version.as_str(), evaluation.configuration_digest.as_slice(), to_sql_revision(evaluation.basis_project_revision)?, digest.as_slice(),
            evaluation.batch.as_ref().filter(|_| conflict.is_none() && !duplicate).map(|batch| batch.id.as_str()), revision],
    )?;
    if let Some(conflict) = conflict {
        insert_policy_conflict(transaction, evaluation, conflict)?;
    }
    for (index, input) in evaluation.inputs.iter().enumerate() {
        let disposition = match (conflict, duplicate, input.disposition) {
            (Some(_), _, PolicyDisposition::Accepted) => PolicyDisposition::Conflict,
            (None, true, PolicyDisposition::Accepted) => PolicyDisposition::Duplicate,
            _ => input.disposition,
        };
        let reason = if disposition == PolicyDisposition::Conflict {
            conflict.map_or(input.reason.as_str(), |detail| detail.reason_code.as_str())
        } else if duplicate && input.disposition == PolicyDisposition::Accepted {
            "already_accepted"
        } else {
            input.reason.as_str()
        };
        transaction.execute(
            "INSERT INTO policy_evaluation_inputs (project_id,evaluation_id,input_index,input_kind,input_id,input_digest,disposition,reason_code)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![evaluation.project.as_str(), evaluation.id.as_str(), i64::try_from(index).map_err(|_| StoreError::InvalidPolicyEvaluation)?,
                input.input.kind(), input.input.id().as_str(), input.input_digest.as_slice(),
                disposition.as_str(), reason],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO policy_input_receipts (project_id,input_kind,input_id,input_digest,first_evaluation_id)
             VALUES (?1,?2,?3,?4,?5)",
            params![evaluation.project.as_str(), input.input.kind(), input.input.id().as_str(), input.input_digest.as_slice(), evaluation.id.as_str()],
        )?;
        if let merl_core::PolicyInput::ObservedAssertion { id, run, index } = &input.input {
            transaction.execute(
                "INSERT INTO policy_assertion_inputs (project_id,evaluation_id,input_id,run_id,assertion_index)
                 VALUES (?1,?2,?3,?4,?5)",
                params![evaluation.project.as_str(), evaluation.id.as_str(), id.as_str(), run.as_str(), i64::from(*index)],
            )?;
        }
        if disposition == PolicyDisposition::Accepted {
            transaction.execute(
                "INSERT INTO accepted_policy_inputs (project_id,input_kind,input_id,input_digest,evaluation_id)
                 VALUES (?1,?2,?3,?4,?5)",
                params![evaluation.project.as_str(), input.input.kind(), input.input.id().as_str(), input.input_digest.as_slice(), evaluation.id.as_str()],
            )?;
            if let merl_core::PolicyInput::ObservedAssertion { run, index, .. } = &input.input {
                transaction.execute(
                    "INSERT INTO accepted_assertions (project_id,run_id,assertion_index,evaluation_id)
                     VALUES (?1,?2,?3,?4)",
                    params![evaluation.project.as_str(), run.as_str(), i64::from(*index), evaluation.id.as_str()],
                )?;
            }
        }
    }
    for (index, read) in evaluation.reads.iter().enumerate() {
        let (kind, target, revision) = match read {
            PolicyRead::Object { id, revision } => (
                "object",
                id.as_str(),
                revision
                    .map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory))
                    .transpose()?,
            ),
            PolicyRead::KindCollection {
                kind,
                latest_project_revision,
            } => (
                "kind_collection",
                kind.as_str(),
                Some(to_sql_revision(*latest_project_revision)?),
            ),
        };
        transaction.execute(
            "INSERT INTO policy_evaluation_reads (project_id,evaluation_id,read_index,read_kind,target_id,expected_revision)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![evaluation.project.as_str(), evaluation.id.as_str(), i64::try_from(index).map_err(|_| StoreError::InvalidPolicyEvaluation)?, kind, target, revision],
        )?;
    }
    for write in &evaluation.writes {
        transaction.execute(
            "INSERT INTO policy_evaluation_writes (project_id,evaluation_id,object_id,expected_revision)
             VALUES (?1,?2,?3,?4)",
            params![evaluation.project.as_str(), evaluation.id.as_str(), write.object.as_str(), write.expected_revision.map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory)).transpose()?],
        )?;
    }
    if conflict.is_none() && !duplicate {
        for origin in &evaluation.event_origins {
            transaction.execute(
                "INSERT INTO policy_evaluation_domain_events (project_id,evaluation_id,event_id,input_index)
                 VALUES (?1,?2,?3,?4)",
                params![evaluation.project.as_str(), evaluation.id.as_str(), origin.event.as_str(), i64::from(origin.input_index)],
            )?;
        }
    }
    Ok(())
}

fn insert_policy_conflict(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    conflict: &PolicyConflictDetail,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO policy_conflicts (project_id,evaluation_id,reason_code,target_id,expected_revision,actual_revision)
         VALUES (?1,?2,?3,?4,?5,?6)",
        params![evaluation.project.as_str(), evaluation.id.as_str(), conflict.reason_code,
            conflict.target_id, conflict.expected_revision, conflict.actual_revision],
    )?;
    Ok(())
}

fn policy_evaluation_digest(
    evaluation: &PolicyEvaluation,
    provider: Option<&ProviderObservation>,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    };
    part(evaluation.id.as_str().as_bytes());
    part(evaluation.project.as_str().as_bytes());
    part(evaluation.actor.as_str().as_bytes());
    part(evaluation.version.as_str().as_bytes());
    part(&evaluation.configuration_digest);
    part(&evaluation.basis_project_revision.get().to_be_bytes());
    part(&(evaluation.inputs.len() as u64).to_be_bytes());
    for input in &evaluation.inputs {
        part(input.input.kind().as_bytes());
        part(input.input.id().as_str().as_bytes());
        if let merl_core::PolicyInput::ObservedAssertion { run, index, .. } = &input.input {
            part(run.as_str().as_bytes());
            part(&index.to_be_bytes());
        }
        part(&input.input_digest);
        part(input.disposition.as_str().as_bytes());
        part(input.reason.as_str().as_bytes());
    }
    part(&(evaluation.reads.len() as u64).to_be_bytes());
    for read in &evaluation.reads {
        match read {
            PolicyRead::Object { id, revision } => {
                part(b"object");
                part(id.as_str().as_bytes());
                part(&revision.map_or(0, ObjectRevision::get).to_be_bytes());
            }
            PolicyRead::KindCollection {
                kind,
                latest_project_revision,
            } => {
                part(b"kind_collection");
                part(kind.as_str().as_bytes());
                part(&latest_project_revision.get().to_be_bytes());
            }
        }
    }
    part(&(evaluation.writes.len() as u64).to_be_bytes());
    for write in &evaluation.writes {
        part(write.object.as_str().as_bytes());
        part(
            &write
                .expected_revision
                .map_or(0, ObjectRevision::get)
                .to_be_bytes(),
        );
    }
    part(&(evaluation.event_origins.len() as u64).to_be_bytes());
    for origin in &evaluation.event_origins {
        part(&origin.input_index.to_be_bytes());
        part(origin.event.as_str().as_bytes());
    }
    part(&[u8::from(evaluation.batch.is_some())]);
    if let Some(batch) = &evaluation.batch {
        part(batch.id.as_str().as_bytes());
        part(&batch.occurred_at_millis.to_be_bytes());
        part(&(batch.events.len() as u64).to_be_bytes());
        for event in &batch.events {
            let DomainEvent::PutObject {
                id,
                object,
                kind,
                payload,
            } = event;
            part(id.as_str().as_bytes());
            part(object.as_str().as_bytes());
            part(kind.as_str().as_bytes());
            part(payload.as_ref().map_or("", PayloadId::as_str).as_bytes());
        }
    }
    part(&[u8::from(provider.is_some())]);
    if let Some(provider) = provider {
        part(provider.id.as_str().as_bytes());
        part(provider.snapshot_payload.as_str().as_bytes());
        part(&provider.observed_at_millis.to_be_bytes());
        part(
            &provider
                .upstream_updated_at_millis
                .unwrap_or(0)
                .to_be_bytes(),
        );
    }
    hash.finalize().into()
}

fn next_revision(transaction: &Transaction<'_>, project: &ProjectId) -> Result<i64, StoreError> {
    let current: Option<i64> = transaction
        .query_row(
            "SELECT current_revision FROM projects WHERE id = ?1",
            [project.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    current
        .ok_or(StoreError::ProjectMissing)?
        .checked_add(1)
        .ok_or(StoreError::CorruptHistory)
}

fn insert_accepted_batch(
    transaction: &Transaction<'_>,
    batch: &DomainEventBatch,
    observation: Option<&ProviderObservation>,
    revision: i64,
) -> Result<(), StoreError> {
    if let Some(observation) = observation {
        ensure_fresh_provider_observation(transaction, &batch.project, observation)?;
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
                    transaction,
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
        insert_provider_observation(transaction, batch, observation)?;
    }
    transaction.execute(
        "UPDATE projects SET current_revision = ?2 WHERE id = ?1",
        params![batch.project.as_str(), revision],
    )?;
    Ok(())
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
    basis: CaptureBasis,
    capture_digest: &[u8; 32],
    payloads: SourcePayloadRefs<'_>,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO source_versions (
             project_id, id, source_id, provider_entity_id, context_scope_id, provider_version_id, binding_id, kind, supersedes_id, ambiguous_order_with_previous, sequence,
             occurred_at_millis, created_at_millis, upstream_updated_at_millis, observed_at_millis, actor_id, provider_actor_id,
             body_digest, edit_diff_digest, capture_digest, payload_id, edit_diff_payload_id, edit_deleted_at_millis, missing_body_reason,
             compilation_mode, coverage_requirement, capture_policy_version, interpretation_basis_revision,
             interpretation_basis_known, source_author_id, provider_source_author_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31)",
        params![
            project.as_str(), capture.version.as_str(), capture.source.as_str(),
            capture.provider_entity_id, capture.context_scope_id, capture.provider_version_id,
            capture.binding.id.as_str(), capture.kind.as_str(),
            capture.supersedes.as_ref().map(SourceVersionId::as_str), i64::from(capture.ambiguous_order_with_previous), sequence,
            capture.occurred_at_millis, capture.created_at_millis, capture.upstream_updated_at_millis, capture.observed_at_millis,
            capture.actor.as_ref().map(ActorId::as_str), capture.provider_actor_id,
            payloads.body_digest.map(AsRef::<[u8]>::as_ref),
            payloads.edit_diff_digest.map(AsRef::<[u8]>::as_ref),
            capture_digest.as_slice(), payloads.body.map(PayloadId::as_str),
            payloads.edit_diff.map(PayloadId::as_str), capture.edit_deleted_at_millis,
            capture.missing_body_reason.map(MissingSourceBody::as_str),
            capture.compilation_mode.as_str(), capture.coverage_requirement.as_str(),
            capture.policy_version.as_str(), basis.revision, i64::from(basis.known),
            capture.source_author.as_ref().map(ActorId::as_str), capture.provider_source_author_id
        ],
    )?;
    Ok(())
}

struct RawSourceVersion {
    source_id: String,
    provider_entity_id: String,
    context_scope_id: String,
    provider_version_id: String,
    binding_id: String,
    kind: String,
    supersedes_id: Option<String>,
    ambiguous_order_with_previous: i64,
    sequence: i64,
    interpretation_basis_revision: i64,
    interpretation_basis_known: i64,
    occurred_at_millis: i64,
    created_at_millis: i64,
    upstream_updated_at_millis: Option<i64>,
    observed_at_millis: i64,
    provider_actor_id: Option<String>,
    source_author_id: Option<String>,
    provider_source_author_id: Option<String>,
    version_actor_id: Option<String>,
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
            interpretation_basis_revision: row.get(8)?,
            interpretation_basis_known: row.get(9)?,
            occurred_at_millis: row.get(10)?,
            created_at_millis: row.get(11)?,
            upstream_updated_at_millis: row.get(12)?,
            observed_at_millis: row.get(13)?,
            provider_actor_id: row.get(14)?,
            compilation_mode: row.get(15)?,
            coverage_requirement: row.get(16)?,
            capture_policy_version: row.get(17)?,
            body_digest: row.get(18)?,
            missing_body_reason: row.get(19)?,
            payload_id: row.get(20)?,
            edit_diff_payload_id: row.get(21)?,
            edit_deleted_at_millis: row.get(22)?,
            context_scope_id: row.get(23)?,
            source_author_id: row.get(24)?,
            provider_source_author_id: row.get(25)?,
            version_actor_id: row.get(26)?,
        })
    }
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

fn current_revision_in_transaction(
    transaction: &Transaction<'_>,
    project: &ProjectId,
) -> Result<i64, StoreError> {
    Ok(transaction.query_row(
        "SELECT current_revision FROM projects WHERE id = ?1",
        [project.as_str()],
        |row| row.get(0),
    )?)
}

#[derive(Clone, Copy)]
struct CaptureBasis {
    revision: i64,
    known: bool,
}

fn validate_source_capture(capture: &SourceCapture<'_>) -> Result<(), StoreError> {
    if capture.created_at_millis > capture.occurred_at_millis
        || capture.occurred_at_millis > capture.observed_at_millis
        || capture.upstream_updated_at_millis.is_some_and(|updated| {
            updated < capture.occurred_at_millis || updated > capture.observed_at_millis
        })
        || capture.edit_deleted_at_millis.is_some_and(|deleted| {
            deleted < capture.occurred_at_millis || deleted > capture.observed_at_millis
        })
        || capture.body.is_some() == capture.missing_body_reason.is_some()
        || !valid_provider_id(&capture.binding.provider_namespace_id)
        || !valid_provider_id(capture.provider_entity_id)
        || !valid_provider_id(capture.context_scope_id)
        || !valid_provider_id(capture.provider_version_id)
        || capture
            .provider_actor_id
            .is_some_and(|id| !valid_provider_id(id))
        || capture
            .provider_source_author_id
            .is_some_and(|id| !valid_provider_id(id))
        || Sha256::digest(capture.binding.provider_namespace_id.as_bytes()).as_slice()
            != capture.binding.namespace_digest
    {
        return Err(StoreError::InvalidSource);
    }
    Ok(())
}

fn validate_compilation_intent(
    store: &Store,
    project: &ProjectId,
    record: &CompilationIntent<'_>,
) -> Result<(), StoreError> {
    let source = store
        .source_version(project, record.source)?
        .ok_or(StoreError::InvalidCompilation)?;
    let causal_basis = if source.interpretation_basis_known {
        Some(source.interpretation_basis_revision)
    } else {
        store.replay_basis(project, record.source)?
    };
    if record.interpretation_basis_revision > store.project_revision(project)?
        || (!source.interpretation_basis_known && record.mode == "live")
        || record.source_observation_cutoff != source.sequence
        || (record.mode != "hindsight"
            && causal_basis != Some(record.interpretation_basis_revision))
        || record.context.len() > record.max_input_bytes
        || record.source_window.last() != Some(record.source)
        || record
            .objects
            .iter()
            .any(|(_, revision)| revision.get() == 0)
        || ![
            record.id,
            record.renderer_version,
            record.selector_version,
            record.compiler_id,
            record.compiler_version,
            record.model_id,
        ]
        .iter()
        .all(|value| valid_record_id(value))
        || !matches!(record.mode, "live" | "replay" | "eval" | "hindsight")
    {
        return Err(StoreError::InvalidCompilation);
    }
    for version in record.source_window {
        let item = store
            .source_version(project, version)?
            .ok_or(StoreError::InvalidCompilation)?;
        if item.sequence > source.sequence {
            return Err(StoreError::InvalidCompilation);
        }
    }
    Ok(())
}

fn insert_context_references(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    record: &CompilationIntent<'_>,
) -> Result<(), StoreError> {
    for (index, version) in record.source_window.iter().enumerate() {
        transaction.execute(
            "INSERT INTO compilation_context_sources (project_id,run_id,window_index,source_version_id) VALUES (?1,?2,?3,?4)",
            params![project.as_str(), record.id,
                i64::try_from(index).map_err(|_| StoreError::InvalidCompilation)?, version.as_str()],
        )?;
    }
    for (object, revision) in record.objects {
        transaction.execute(
            "INSERT INTO compilation_context_objects (project_id,run_id,object_id,object_revision) VALUES (?1,?2,?3,?4)",
            params![project.as_str(), record.id, object.as_str(),
                i64::try_from(revision.get()).map_err(|_| StoreError::InvalidCompilation)?],
        )?;
    }
    Ok(())
}

fn insert_assertions(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    result: &CompilationResult<'_>,
    source_window: &[SourceVersionId],
) -> Result<(), StoreError> {
    for (index, assertion) in result.assertions.iter().enumerate() {
        if !source_window.contains(&assertion.source)
            || assertion.span_start >= assertion.span_end
            || assertion.confidence_millis > 1000
            || ![
                assertion.subject.as_str(),
                assertion.predicate.as_str(),
                assertion.value.as_str(),
                assertion.act.as_str(),
                assertion.epistemic_basis.as_str(),
                assertion.polarity.as_str(),
            ]
            .iter()
            .all(|value| valid_record_id(value))
            || assertion
                .asserted_by
                .as_deref()
                .is_some_and(|value| !valid_record_id(value))
            || assertion
                .attributed_to
                .as_deref()
                .is_some_and(|value| !valid_record_id(value))
        {
            return Err(StoreError::InvalidCompilation);
        }
        transaction.execute(
            "INSERT INTO observed_assertions (project_id,run_id,assertion_index,source_version_id,
              span_start,span_end,subject_id,predicate_id,value_id,act,epistemic_basis,polarity,
              confidence_millis,asserted_by,attributed_to,attribution_verified)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                project.as_str(),
                result.run_id,
                i64::try_from(index).map_err(|_| StoreError::InvalidCompilation)?,
                assertion.source.as_str(),
                i64::try_from(assertion.span_start).map_err(|_| StoreError::InvalidCompilation)?,
                i64::try_from(assertion.span_end).map_err(|_| StoreError::InvalidCompilation)?,
                assertion.subject,
                assertion.predicate,
                assertion.value,
                assertion.act,
                assertion.epistemic_basis,
                assertion.polarity,
                assertion.confidence_millis,
                assertion.asserted_by,
                assertion.attributed_to,
                i64::from(assertion.attribution_verified)
            ],
        )?;
    }
    Ok(())
}

fn valid_record_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':')
        })
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
        capture.context_scope_id,
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
    // Author metadata joins poll time and effective policy as first-capture
    // provenance. Excluding it keeps retries of pre-migration versions stable.
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
        assert!(
            matches!(error, StoreError::UnsupportedSchema(version) if version == SCHEMA_VERSION + 1)
        );
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

    #[test]
    fn a_prior_policy_store_keeps_each_events_exact_input_during_migration() {
        let connection = Connection::open_in_memory().expect("open SQLite");
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_sources.sql"),
            include_str!("../migrations/0003_compilation.sql"),
            include_str!("../migrations/0004_policy.sql"),
            include_str!("../migrations/0005_policy_conflicts.sql"),
        ] {
            connection
                .execute_batch(migration)
                .expect("create prior schema");
        }
        connection
            .execute_batch(
                "INSERT INTO projects (id,current_revision) VALUES ('P1',1);
                 INSERT INTO domain_event_batches (project_id,id,revision,actor_id,occurred_at_millis)
                   VALUES ('P1','B1',1,'alice',1);
                 INSERT INTO domain_events (project_id,batch_id,id,event_index,event_kind,object_id,object_kind)
                   VALUES ('P1','B1','E-z',0,'put_object','D1','decision'),
                          ('P1','B1','E-a',1,'put_object','D2','decision');
                 INSERT INTO policy_evaluations
                   (project_id,id,actor_id,policy_version,policy_config_digest,basis_project_revision,
                    evaluation_digest,batch_id,committed_revision)
                   VALUES ('P1','PE1','alice','v1',zeroblob(32),0,zeroblob(32),'B1',1);
                 INSERT INTO policy_evaluation_inputs
                   (project_id,evaluation_id,input_index,input_kind,input_id,input_digest,disposition,reason_code)
                   VALUES ('P1','PE1',0,'command','C-z',zeroblob(32),'accepted','authorized_command'),
                          ('P1','PE1',1,'command','C-a',zeroblob(32),'accepted','authorized_command');
                 INSERT INTO policy_evaluation_domain_events (project_id,evaluation_id,event_id)
                   VALUES ('P1','PE1','E-z'),('P1','PE1','E-a');",
            )
            .expect("seed prior policy history");
        connection
            .pragma_update(None, "user_version", 5)
            .expect("mark prior schema");

        let store = Store::from_connection(connection).expect("migrate event origins");
        let project = merl_core::ProjectId::try_from("P1").expect("project ID");
        let first = store
            .object_policy_origin(
                &project,
                &merl_core::ObjectId::try_from("D1").expect("object"),
            )
            .expect("first origin")
            .expect("first event");
        let second = store
            .object_policy_origin(
                &project,
                &merl_core::ObjectId::try_from("D2").expect("object"),
            )
            .expect("second origin")
            .expect("second event");
        assert_eq!(first.input.id().as_str(), "C-z");
        assert_eq!(second.input.id().as_str(), "C-a");
    }
}
