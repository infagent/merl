//! SQLite authority for accepted events and independently erasable payloads.

use std::{collections::BTreeSet, error::Error, fmt, fmt::Write as _, path::Path};

use merl_core::{
    ActorId, AgentId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, ObjectId, ObjectKind, ObjectLifecycle, ObjectRevision, PayloadId,
    PolicyDisposition, PolicyEvaluation, PolicyEvaluationId, PolicyRead, ProjectId,
    ProjectRevision, ProviderIssueState, ProviderObservation, Relation, RelationId, RelationKind,
    SourceBindingId, SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 14;

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
    /// An inbox acknowledgement skipped an earlier entry or named no entry.
    InvalidInboxAcknowledgement,
    /// The preview changed, the actor omitted a reason, or this source was purged already.
    InvalidPurge,
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
            Self::InvalidInboxAcknowledgement => {
                formatter.write_str("inbox entry is absent or out of order")
            }
            Self::InvalidPurge => formatter.write_str("purge request does not match its preview"),
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

/// One protected payload named in an administrative purge preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgePayload {
    /// Project-scoped payload identity.
    pub id: PayloadId,
    /// Digest retained after the bytes disappear.
    pub digest: [u8; 32],
}

/// Exact dependency set an operator must confirm before source erasure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgePreview {
    /// Captured version whose retained bytes initiated this request.
    pub source: SourceVersionId,
    /// Source and derived payloads that will become unavailable.
    pub payloads: Vec<PurgePayload>,
    /// Compiler inputs whose exact replay will become unavailable.
    pub runs: Vec<merl_core::CompilationRunId>,
    /// Assertions whose recorded input will lose retained bytes.
    pub assertions: u64,
    /// Accepted events whose supporting evidence will change.
    pub events: Vec<merl_core::EventId>,
    /// Accepted objects needing reconsideration.
    pub objects: Vec<ObjectId>,
    /// Accepted relations whose assertion provenance becomes unavailable.
    pub relations: Vec<RelationId>,
    /// Digest of the complete structural preview, used at confirmation.
    pub confirm_digest: [u8; 32],
}

/// Durable receipt for an administrative source-content purge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PurgeAudit {
    /// Source version whose bytes were selected.
    pub source: SourceVersionId,
    /// Administrator recorded by the local authority.
    pub actor: ActorId,
    /// UTC request time in Unix milliseconds.
    pub requested_at_millis: i64,
    /// Protected reason text, retained separately from structural history.
    pub reason: PayloadId,
    /// Confirmed dependency and payload digest.
    pub preview_digest: [u8; 32],
    /// Active-store scrubbing finished after logical erasure.
    pub completed: bool,
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
    /// Issue conversation that owns this semantic object, when known.
    pub issue_scope: Option<String>,
    /// Accepted lifecycle, independent of evidence support.
    pub lifecycle: ObjectLifecycle,
    /// Evidence health, independent of the object's accepted revision.
    pub support: SupportStatus,
    /// Revision of this object, independent of the project revision.
    pub revision: ObjectRevision,
    /// Project revision in which the object last changed.
    pub project_revision: ProjectRevision,
}

type ObjectRow = (String, Option<String>, Option<String>, String, i64, i64);

/// Provider-owned facts and accepted semantic objects for one Issue thread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueState {
    /// Project revision shared by provider and semantic changes.
    pub project_revision: ProjectRevision,
    /// Latest accepted provider observation, if one has arrived.
    pub provider: Option<AcceptedProviderObservation>,
    /// Merl-owned objects attached to this Issue, excluding provider mirror records.
    pub semantics: Vec<ProjectedObject>,
    /// Accepted structural links touching the Issue or its scoped objects.
    pub relations: Vec<ProjectedRelation>,
    /// Scope-specific coverage; accepted revision alone does not imply completeness.
    pub coverage: SemanticCoverage,
}

/// Current projection of one accepted structural relation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectedRelation {
    /// Its accepted endpoints and predicate.
    pub relation: Relation,
    /// Revision of this relation, independent of the project revision.
    pub revision: ObjectRevision,
    /// Accepted batch that last changed the relation.
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

/// One accepted event reference in a compact batch delta.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeltaChange {
    /// Stable event identity for provenance expansion.
    pub event: merl_core::EventId,
    /// Object or relation changed by the event.
    pub reference: String,
    /// Bounded event kind; no protected text enters a delta.
    pub kind: String,
}

/// One accepted batch and its reference-only changes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDelta {
    /// Batch identity used by the inbox.
    pub batch: merl_core::BatchId,
    /// Accepted revision of this batch.
    pub revision: ProjectRevision,
    /// Changed structural references in event order.
    pub changes: Vec<DeltaChange>,
    /// More changes exist in this batch than the read budget allowed.
    pub changes_truncated: bool,
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

/// One accepted revision of an object, oldest first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectHistoryEntry {
    /// Event that changed this object.
    pub event: merl_core::EventId,
    /// Batch that accepted the event.
    pub batch: merl_core::BatchId,
    /// Project revision of that batch.
    pub revision: ProjectRevision,
    /// Lifecycle accepted at this revision.
    pub lifecycle: ObjectLifecycle,
    /// Protected text reference, if this version has one.
    pub payload: Option<PayloadId>,
    /// Evaluation that accepted this event, absent for bootstrap history.
    pub evaluation: Option<PolicyEvaluationId>,
    /// Exact typed input that produced this event.
    pub input: Option<merl_core::PolicyInput>,
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
    /// Highest observation before the first unprocessed required source.
    pub processed_through: u64,
    /// Required versions whose latest live attempt has not succeeded.
    pub required_gaps: u64,
    /// Required versions awaiting a result or requested context.
    pub required_pending: u64,
    /// Required versions whose latest live compiler attempt failed.
    pub required_failed: u64,
    /// Required versions whose captured body was erased after capture.
    pub required_purged: u64,
    /// Optional versions whose latest live attempt has not succeeded.
    pub optional_cold: u64,
}

/// Evidence health is separate from the accepted object's lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportStatus {
    /// No supporting source has changed since acceptance or revalidation.
    Current,
    /// Every known support is awaiting reconsideration.
    RevalidationPending,
    /// Some support remains current while other support awaits reconsideration.
    PartiallySupported,
    /// No retained support currently justifies the object.
    Unsupported,
}

/// Immutable `evidence_changed` fact and the resulting reconsideration intent.
/// An unresolved `recompile` impact means revalidation is pending.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceImpact {
    /// Stable audit identity.
    pub id: String,
    /// Accepted object whose support needs reconsideration.
    pub object: ObjectId,
    /// Accepted event whose support may have changed.
    pub support_event: merl_core::EventId,
    /// Earlier compilation whose interpretation must be reconsidered.
    pub affected_run: merl_core::CompilationRunId,
    /// Source that triggered the earlier compilation.
    pub trigger: SourceVersionId,
    /// Source version available when that event was accepted.
    pub changed_source: SourceVersionId,
    /// New source version, absent when retained source bytes were erased.
    pub replacement: Option<SourceVersionId>,
    /// Recompile when new bytes exist; reevaluate when evidence is unavailable.
    pub next_action: String,
    /// A later accepted event that resolved this impact, if any.
    pub revalidated_by: Option<merl_core::EventId>,
}

/// Inspectable outcome of a retained compiler attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompilationRunStatus {
    /// Project-local order in which the authority accepted this attempt.
    pub attempt_order: u64,
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
        connection.pragma_update(None, "secure_delete", "ON")?;
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
            if version < 6 {
                transaction
                    .execute_batch(include_str!("../migrations/0006_policy_event_origins.sql"))?;
            }
            if version < 7 {
                transaction.execute_batch(include_str!("../migrations/0007_issue_scope.sql"))?;
            }
            if version < 8 {
                transaction
                    .execute_batch(include_str!("../migrations/0008_evidence_health.sql"))?;
            }
            if version < 9 {
                transaction.execute_batch(include_str!("../migrations/0009_relations.sql"))?;
            }
            if version < 10 {
                transaction
                    .execute_batch(include_str!("../migrations/0010_object_lifecycle.sql"))?;
            }
            if version < 11 {
                transaction
                    .execute_batch(include_str!("../migrations/0011_impact_derivation.sql"))?;
            }
            if version < 12 {
                transaction.execute_batch(include_str!("../migrations/0012_inbox_cursor.sql"))?;
            }
            if version < 13 {
                transaction.execute_batch(include_str!(
                    "../migrations/0013_compilation_attempt_order.sql"
                ))?;
            }
            if version < 14 {
                transaction.execute_batch(include_str!("../migrations/0014_purge_audit.sql"))?;
            }
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        let mut store = Self { connection };
        store.finish_pending_purges()?;
        Ok(store)
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source: Option<String> = transaction
            .query_row(
                "SELECT id FROM source_versions WHERE project_id=?1 AND payload_id=?2",
                params![project.as_str(), id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let changed = transaction.execute(
            "UPDATE payloads SET bytes = NULL, erased = 1 WHERE project_id = ?1 AND id = ?2 AND erased=0",
            params![project.as_str(), id.as_str()],
        )?;
        if changed == 0 {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM payloads WHERE project_id=?1 AND id=?2)",
                params![project.as_str(), id.as_str()],
                |row| row.get(0),
            )?;
            return if exists {
                Ok(())
            } else {
                Err(StoreError::PayloadMissing)
            };
        }
        if let Some(source) = source {
            record_evidence_impacts(&transaction, project, &source, None, "reevaluate", 0)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Lists source and derived bytes, accepted support, and the digest to confirm.
    ///
    /// # Errors
    /// Rejects an unknown source or damaged structural lineage.
    pub fn preview_source_purge(
        &self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<PurgePreview, StoreError> {
        Self::preview_source_purge_on(&self.connection, project, source)
    }

    fn preview_source_purge_on(
        connection: &Connection,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<PurgePreview, StoreError> {
        let captured: Option<(Option<String>, Option<String>)> = connection.query_row(
            "SELECT payload_id,edit_diff_payload_id FROM source_versions WHERE project_id=?1 AND id=?2",
            params![project.as_str(), source.as_str()],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        let (body, diff) = captured.ok_or(StoreError::InvalidSource)?;
        let mut payload_ids = BTreeSet::new();
        if let Some(body) = body {
            payload_ids.insert(body);
        }
        if let Some(diff) = diff {
            payload_ids.insert(diff);
        }
        let mut run_statement = connection.prepare(
            "SELECT DISTINCT r.id,r.context_payload_id,c.response_payload_id
             FROM compilation_runs r
             JOIN compilation_context_sources s ON s.project_id=r.project_id AND s.run_id=r.id
             LEFT JOIN compilation_results c ON c.project_id=r.project_id AND c.run_id=r.id
             WHERE r.project_id=?1 AND s.source_version_id=?2 ORDER BY r.id",
        )?;
        let run_rows =
            run_statement.query_map(params![project.as_str(), source.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
        let mut runs = Vec::new();
        let mut assertions = 0_u64;
        let mut event_ids = BTreeSet::new();
        let mut object_ids = BTreeSet::new();
        let mut relation_ids = BTreeSet::new();
        for row in run_rows {
            let (run, context, response) = row?;
            payload_ids.insert(context);
            if let Some(response) = response {
                payload_ids.insert(response);
            }
            assertions += collect_purge_consequences(
                connection,
                project,
                &run,
                &mut payload_ids,
                &mut event_ids,
                &mut object_ids,
                &mut relation_ids,
            )?;
            runs.push(
                merl_core::CompilationRunId::try_from(run.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
            );
        }
        let mut payloads = Vec::new();
        for id in payload_ids {
            let digest: Vec<u8> = connection.query_row(
                "SELECT digest FROM payloads WHERE project_id=?1 AND id=?2",
                params![project.as_str(), id],
                |row| row.get(0),
            )?;
            payloads.push(PurgePayload {
                id: PayloadId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?,
                digest: digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
            });
        }
        let events = event_ids
            .into_iter()
            .map(|id| {
                merl_core::EventId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let objects = object_ids
            .into_iter()
            .map(|id| ObjectId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory))
            .collect::<Result<Vec<_>, _>>()?;
        let relations = relation_ids
            .into_iter()
            .map(|id| RelationId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory))
            .collect::<Result<Vec<_>, _>>()?;
        let mut preview = PurgePreview {
            source: source.clone(),
            payloads,
            runs,
            assertions,
            events,
            objects,
            relations,
            confirm_digest: [0; 32],
        };
        preview.confirm_digest = purge_preview_digest(project, &preview);
        Ok(preview)
    }

    /// Erases a confirmed source and derived payload set, then scrubs the active store.
    ///
    /// The local authority records the supplied actor; same-user process isolation
    /// remains outside Merl's first-release security boundary.
    ///
    /// # Errors
    /// Rejects a stale preview, missing reason, prior purge, or failed SQLite scrub.
    pub fn purge_source(
        &mut self,
        project: &ProjectId,
        source: &SourceVersionId,
        actor: &ActorId,
        reason: &[u8],
        now_millis: i64,
        confirm_digest: [u8; 32],
    ) -> Result<PurgeAudit, StoreError> {
        if reason.is_empty() || reason.len() > 4096 || self.purge_audit(project, source)?.is_some()
        {
            return Err(StoreError::InvalidPurge);
        }
        let reason_id = purge_reason_id(project, source)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let preview = Self::preview_source_purge_on(&transaction, project, source)?;
        if preview.confirm_digest != confirm_digest || preview.payloads.is_empty() {
            return Err(StoreError::InvalidPurge);
        }
        insert_protected_payload(
            &transaction,
            project,
            &reason_id,
            reason,
            &Sha256::digest(reason),
        )?;
        transaction.execute(
            "INSERT INTO purge_intents
             (project_id,source_version_id,actor_id,requested_at_millis,reason_payload_id,preview_digest)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![project.as_str(),source.as_str(),actor.as_str(),now_millis,reason_id.as_str(),confirm_digest.as_slice()],
        )?;
        for payload in &preview.payloads {
            transaction.execute(
                "INSERT INTO purge_intent_payloads
                 (project_id,source_version_id,payload_id,digest) VALUES (?1,?2,?3,?4)",
                params![
                    project.as_str(),
                    source.as_str(),
                    payload.id.as_str(),
                    payload.digest.as_slice()
                ],
            )?;
            transaction.execute(
                "UPDATE payloads SET bytes=NULL,erased=1 WHERE project_id=?1 AND id=?2 AND erased=0",
                params![project.as_str(),payload.id.as_str()],
            )?;
        }
        record_evidence_impacts(
            &transaction,
            project,
            source.as_str(),
            None,
            "reevaluate",
            now_millis,
        )?;
        transaction.commit()?;
        self.finish_pending_purges()?;
        self.purge_audit(project, source)?
            .ok_or(StoreError::CorruptHistory)
    }

    /// Returns the durable purge receipt even if physical scrubbing is pending.
    ///
    /// # Errors
    /// Rejects damaged audit metadata or an unreadable store.
    pub fn purge_audit(
        &self,
        project: &ProjectId,
        source: &SourceVersionId,
    ) -> Result<Option<PurgeAudit>, StoreError> {
        let row: Option<(String, i64, String, Vec<u8>, bool)> = self
            .connection
            .query_row(
                "SELECT i.actor_id,i.requested_at_millis,i.reason_payload_id,i.preview_digest,
                    c.source_version_id IS NOT NULL
             FROM purge_intents i LEFT JOIN purge_completions c
               ON c.project_id=i.project_id AND c.source_version_id=i.source_version_id
             WHERE i.project_id=?1 AND i.source_version_id=?2",
                params![project.as_str(), source.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(actor, requested_at_millis, reason, digest, completed)| {
            Ok(PurgeAudit {
                source: source.clone(),
                actor: ActorId::try_from(actor.as_str()).map_err(|_| StoreError::CorruptHistory)?,
                requested_at_millis,
                reason: PayloadId::try_from(reason.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                preview_digest: digest.try_into().map_err(|_| StoreError::CorruptHistory)?,
                completed,
            })
        })
        .transpose()
    }

    fn finish_pending_purges(&mut self) -> Result<(), StoreError> {
        let pending = {
            let mut statement = self.connection.prepare(
                "SELECT i.project_id,i.source_version_id,i.requested_at_millis
                 FROM purge_intents i LEFT JOIN purge_completions c
                   ON c.project_id=i.project_id AND c.source_version_id=i.source_version_id
                 WHERE c.source_version_id IS NULL ORDER BY i.project_id,i.source_version_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
        };
        if pending.is_empty() {
            return Ok(());
        }
        // Scrub after the logical erasure commits. A crash before completion leaves
        // the intent pending so the next open repeats the scrub before serving reads.
        let busy: i64 =
            self.connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(StoreError::Storage(
                "SQLite checkpoint is busy during purge".into(),
            ));
        }
        self.connection.execute_batch("VACUUM")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (project, source, at) in pending {
            transaction.execute(
                "INSERT INTO purge_completions (project_id,source_version_id,completed_at_millis)
                 VALUES (?1,?2,?3)",
                params![project, source, at],
            )?;
        }
        transaction.commit()?;
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
        if let Some(previous) = previous {
            record_evidence_impacts(
                &transaction,
                project,
                &previous,
                Some(capture.version.as_str()),
                if capture.body.is_some() {
                    "recompile"
                } else {
                    "reevaluate"
                },
                capture.observed_at_millis,
            )?;
        }
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
        self.objects_at_revision_with_scope(project, revision, limit, None)
    }

    /// Selects current semantic objects by caller-provided relevance before a bounded view.
    ///
    /// The scan does not impose an ID prefix: a high-priority object remains visible
    /// even when the project has many lower-priority objects.
    ///
    /// # Errors
    /// Rejects invalid limits or damaged projected object identities.
    pub fn current_objects_ranked(
        &self,
        project: &ProjectId,
        limit: usize,
        priority: impl Fn(&str, &str) -> u8,
    ) -> Result<(Vec<ProjectedObject>, bool), StoreError> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidBatch);
        }
        let mut statement = self.connection.prepare(
            "SELECT id,kind FROM objects WHERE project_id=?1 AND kind!='provider_issue' AND lifecycle!='superseded'",
        )?;
        let rows = statement.query_map(params![project.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut selected = Vec::with_capacity(limit + 1);
        for row in rows {
            let (id, kind) = row?;
            selected.push((priority(&id, &kind), id));
            selected.sort_unstable();
            selected.truncate(limit + 1);
        }
        let truncated = selected.len() > limit;
        selected.truncate(limit);
        let objects = selected
            .into_iter()
            .map(|(_, id)| {
                let id = ObjectId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?;
                self.object(project, &id)?.ok_or(StoreError::CorruptHistory)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((objects, truncated))
    }

    /// Returns direct neighbors so a task view can surface its blockers and requirements
    /// before unrelated work of the same kind.
    ///
    /// # Errors
    /// Rejects invalid limits or damaged relation endpoints.
    pub fn focus_neighbors(
        &self,
        project: &ProjectId,
        focus: &ObjectId,
        limit: usize,
    ) -> Result<(Vec<ObjectId>, bool), StoreError> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidBatch);
        }
        let mut statement = self.connection.prepare(
            "SELECT subject_id,object_id FROM relations WHERE project_id=?1 AND (subject_id=?2 OR object_id=?2) ORDER BY id LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![
                project.as_str(),
                focus.as_str(),
                i64::try_from(limit + 1).map_err(|_| StoreError::InvalidBatch)?
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let mut endpoints = rows.collect::<Result<Vec<_>, _>>()?;
        let truncated = endpoints.len() > limit;
        endpoints.truncate(limit);
        let mut neighbors = Vec::new();
        for (subject, object) in endpoints {
            let other = if subject == focus.as_str() {
                object
            } else {
                subject
            };
            let id = ObjectId::try_from(other.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            if !neighbors.contains(&id) {
                neighbors.push(id);
            }
        }
        Ok((neighbors, truncated))
    }

    /// Selects Issue-owned accepted objects first, without crossing the causal revision.
    ///
    /// # Errors
    /// Rejects future revisions or damaged accepted history.
    pub fn objects_at_revision_for_scope(
        &self,
        project: &ProjectId,
        revision: ProjectRevision,
        limit: usize,
        scope: &str,
    ) -> Result<SelectedObjects, StoreError> {
        if !valid_provider_id(scope) {
            return Err(StoreError::InvalidCompilation);
        }
        self.objects_at_revision_with_scope(project, revision, limit, Some(scope))
    }

    fn objects_at_revision_with_scope(
        &self,
        project: &ProjectId,
        revision: ProjectRevision,
        limit: usize,
        scope: Option<&str>,
    ) -> Result<SelectedObjects, StoreError> {
        if revision > self.project_revision(project)? || limit == 0 {
            return Err(StoreError::InvalidCompilation);
        }
        let limit = i64::try_from(limit).map_err(|_| StoreError::InvalidCompilation)?;
        let lookahead = limit.checked_add(1).ok_or(StoreError::InvalidCompilation)?;
        let mut statement = self.connection.prepare(
            "WITH history AS (
               SELECT domain_events.object_id, domain_events.payload_id, domain_events.issue_scope_id,
                      COUNT(*) OVER (PARTITION BY domain_events.object_id) AS object_revision,
                      ROW_NUMBER() OVER (PARTITION BY domain_events.object_id
                        ORDER BY domain_event_batches.revision DESC, domain_events.event_index DESC) AS rank
               FROM domain_events JOIN domain_event_batches
                 ON domain_event_batches.project_id = domain_events.project_id
                AND domain_event_batches.id = domain_events.batch_id
               WHERE domain_events.project_id = ?1 AND domain_event_batches.revision <= ?2
             ) SELECT object_id, payload_id, object_revision FROM history
               WHERE rank = 1 ORDER BY CASE WHEN ?4 IS NOT NULL AND issue_scope_id=?4 THEN 0 ELSE 1 END, object_id LIMIT ?3",
        )?;
        let basis = i64::try_from(revision.get()).map_err(|_| StoreError::InvalidCompilation)?;
        let mut rows = statement.query(params![project.as_str(), basis, lookahead, scope])?;
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

    /// Reads one object's accepted version at a causal project revision.
    ///
    /// # Errors
    /// Rejects future revisions or damaged accepted history.
    pub fn object_at_revision(
        &self,
        project: &ProjectId,
        object: &ObjectId,
        revision: ProjectRevision,
    ) -> Result<Option<(ObjectRevision, Option<PayloadId>)>, StoreError> {
        if revision > self.project_revision(project)? {
            return Err(StoreError::InvalidCompilation);
        }
        let basis = i64::try_from(revision.get()).map_err(|_| StoreError::InvalidCompilation)?;
        let row: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT COUNT(*) OVER (), e.payload_id FROM domain_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             WHERE e.project_id=?1 AND e.object_id=?2 AND b.revision<=?3
             ORDER BY b.revision DESC, e.event_index DESC LIMIT 1",
                params![project.as_str(), object.as_str(), basis],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        row.map(|(count, payload)| {
            let revision = ObjectRevision::try_from(
                u64::try_from(count).map_err(|_| StoreError::CorruptHistory)?,
            )
            .map_err(|_| StoreError::CorruptHistory)?;
            let payload = payload
                .map(|id| PayloadId::try_from(id.as_str()))
                .transpose()
                .map_err(|_| StoreError::CorruptHistory)?;
            Ok((revision, payload))
        })
        .transpose()
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
        let attempt_order: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(attempt_order),0)+1 FROM compilation_runs WHERE project_id=?1",
            [project.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO compilation_runs (
              project_id,id,source_version_id,context_digest,context_payload_id,
              interpretation_basis_revision,source_observation_cutoff,renderer_version,selector_version,
              max_input_bytes,max_output_bytes,max_output_tokens,max_assertions,max_context_requests,max_expansion_rounds,max_payload_bytes,
              max_source_window,max_objects,
              compiler_id,compiler_version,model_id,prompt_digest,mode,started_at_millis,attempt_order
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            params![project.as_str(), record.id, record.source.as_str(), context_digest.as_slice(),
                context_payload, interpretation_basis, cutoff,
                record.renderer_version, record.selector_version, max_input, max_output,
                max_tokens, max_assertions, max_requests, max_rounds, max_payload,
                max_source_window, max_objects,
                record.compiler_id, record.compiler_version, record.model_id, record.prompt_digest.as_slice(),
                record.mode, record.started_at_millis, attempt_order],
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
        self.semantic_coverage_for_scope(project, None)
    }

    /// Reports only the required and optional observations attached to one Issue thread.
    ///
    /// # Errors
    /// Returns a storage error if the project or source metadata is unavailable.
    pub fn semantic_coverage_in_scope(
        &self,
        project: &ProjectId,
        scope: &str,
    ) -> Result<SemanticCoverage, StoreError> {
        if scope.is_empty() || scope.len() > 512 {
            return Err(StoreError::InvalidSource);
        }
        self.semantic_coverage_for_scope(project, Some(scope))
    }

    fn semantic_coverage_for_scope(
        &self,
        project: &ProjectId,
        scope: Option<&str>,
    ) -> Result<SemanticCoverage, StoreError> {
        let observation_head: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM source_versions
             WHERE project_id=?1 AND (?2 IS NULL OR context_scope_id=?2)",
            params![project.as_str(), scope],
            |row| row.get(0),
        )?;
        let (required_gaps, required_failed, optional_cold, first_gap, required_pending, required_purged):
            (i64, i64, i64, Option<i64>, i64, i64) = self.connection.query_row(
            "WITH scoped AS (
               SELECT s.sequence,s.coverage_requirement,
                 (COALESCE(p.erased,0)=1 OR COALESCE((
                   SELECT cp.erased FROM compilation_runs cr
                   JOIN payloads cp ON cp.project_id=cr.project_id AND cp.id=cr.context_payload_id
                   WHERE cr.project_id=s.project_id AND cr.source_version_id=s.id AND cr.mode='live'
                   ORDER BY cr.attempt_order DESC LIMIT 1),0)=1) AS unavailable,
                 COALESCE((SELECT COALESCE(c.outcome,'pending')
                   FROM compilation_runs r LEFT JOIN compilation_results c
                     ON c.project_id=r.project_id AND c.run_id=r.id
                   WHERE r.project_id=s.project_id AND r.source_version_id=s.id
                     AND r.mode='live'
                   ORDER BY r.attempt_order DESC LIMIT 1),'unprocessed') AS latest_outcome
               FROM source_versions s LEFT JOIN payloads p
                 ON p.project_id=s.project_id AND p.id=s.payload_id
               WHERE s.project_id=?1 AND (?2 IS NULL OR s.context_scope_id=?2)
             )
             SELECT
               COALESCE(SUM(coverage_requirement='required' AND (latest_outcome!='succeeded' OR unavailable)),0),
               COALESCE(SUM(coverage_requirement='required' AND latest_outcome='failed' AND NOT unavailable),0),
               COALESCE(SUM(coverage_requirement='optional' AND (latest_outcome!='succeeded' OR unavailable)),0),
               MIN(CASE WHEN coverage_requirement='required' AND (latest_outcome!='succeeded' OR unavailable)
                   THEN sequence END),
               COALESCE(SUM(coverage_requirement='required' AND latest_outcome IN ('pending','needs_context') AND NOT unavailable),0),
               COALESCE(SUM(coverage_requirement='required' AND unavailable),0)
             FROM scoped",
            params![project.as_str(), scope],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )?;
        let head = u64::try_from(observation_head).map_err(|_| StoreError::CorruptHistory)?;
        Ok(SemanticCoverage {
            observation_head: head,
            processed_through: first_gap
                .map(|gap| u64::try_from(gap - 1).map_err(|_| StoreError::CorruptHistory))
                .transpose()?
                .unwrap_or(head),
            required_gaps: u64::try_from(required_gaps).map_err(|_| StoreError::CorruptHistory)?,
            required_pending: u64::try_from(required_pending)
                .map_err(|_| StoreError::CorruptHistory)?,
            required_failed: u64::try_from(required_failed)
                .map_err(|_| StoreError::CorruptHistory)?,
            required_purged: u64::try_from(required_purged)
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
            i64,
        );
        let raw: Option<RawStatus> = self.connection.query_row(
            "SELECT source_version_id,mode,compilation_results.outcome,compilation_results.failure_code,interpretation_basis_revision,
                    source_observation_cutoff,context_digest,compiler_id,compiler_version,model_id,prompt_digest,
                    max_input_bytes,max_output_bytes,max_output_tokens,max_assertions,
                    max_context_requests,max_expansion_rounds,max_payload_bytes,
                    max_source_window,max_objects,started_at_millis,attempt_order
             FROM compilation_runs LEFT JOIN compilation_results
               ON compilation_results.project_id=compilation_runs.project_id
              AND compilation_results.run_id=compilation_runs.id
             WHERE compilation_runs.project_id=?1 AND compilation_runs.id=?2",
            params![project.as_str(), id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,
                row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?,
                row.get(8)?, row.get(9)?, row.get(10)?,
                [row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?,
                 row.get(15)?, row.get(16)?, row.get(17)?, row.get(18)?, row.get(19)?], row.get(20)?,row.get(21)?)),
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
                attempt_order,
            )| {
                Ok(CompilationRunStatus {
                    attempt_order: u64::try_from(attempt_order)
                        .map_err(|_| StoreError::CorruptHistory)?,
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

    /// Reads the source-version manifest without expanding retained prose.
    ///
    /// # Errors
    /// Returns an error if the saved manifest is unreadable.
    pub fn compilation_context_sources(
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: i64 = transaction.query_row(
            "SELECT current_revision FROM projects WHERE id=?1",
            params![project.as_str()],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO inbox_subscriptions (project_id, agent_id) VALUES (?1, ?2)",
            params![project.as_str(), agent.as_str()],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO inbox_cursors (project_id, agent_id, revision) VALUES (?1, ?2, ?3)",
            params![project.as_str(), agent.as_str(), revision],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Reads the last acknowledged project revision for one subscribed agent.
    ///
    /// # Errors
    /// Returns an error when the agent has no subscription or storage is unreadable.
    pub fn inbox_cursor(
        &self,
        project: &ProjectId,
        agent: &AgentId,
    ) -> Result<ProjectRevision, StoreError> {
        let revision: Option<i64> = self
            .connection
            .query_row(
                "SELECT revision FROM inbox_cursors WHERE project_id=?1 AND agent_id=?2",
                params![project.as_str(), agent.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let revision = revision.ok_or(StoreError::InvalidInboxAcknowledgement)?;
        Ok(ProjectRevision::from(
            u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
        ))
    }

    /// Acknowledges the oldest unread entry; repeating the acknowledgement is harmless.
    ///
    /// # Errors
    /// Rejects an unknown or skipped entry without advancing the cursor.
    pub fn acknowledge_inbox(
        &mut self,
        project: &ProjectId,
        agent: &AgentId,
        revision: ProjectRevision,
    ) -> Result<ProjectRevision, StoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM inbox_cursors WHERE project_id=?1 AND agent_id=?2",
                params![project.as_str(), agent.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let current = current.ok_or(StoreError::InvalidInboxAcknowledgement)?;
        let requested = to_sql_revision(revision)?;
        if requested <= current {
            let known: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM inbox_entries WHERE project_id=?1 AND agent_id=?2 AND project_revision=?3)",
                params![project.as_str(), agent.as_str(), requested],
                |row| row.get(0),
            )?;
            if !known {
                return Err(StoreError::InvalidInboxAcknowledgement);
            }
            return Ok(ProjectRevision::from(
                u64::try_from(current).map_err(|_| StoreError::CorruptHistory)?,
            ));
        }
        let next: Option<i64> = transaction.query_row(
            "SELECT MIN(project_revision) FROM inbox_entries WHERE project_id=?1 AND agent_id=?2 AND project_revision>?3",
            params![project.as_str(), agent.as_str(), current], |row| row.get(0),
        )?;
        if next != Some(requested) {
            return Err(StoreError::InvalidInboxAcknowledgement);
        }
        transaction.execute(
            "UPDATE inbox_cursors SET revision=?3 WHERE project_id=?1 AND agent_id=?2",
            params![project.as_str(), agent.as_str(), requested],
        )?;
        transaction.commit()?;
        Ok(revision)
    }

    /// Lists accepted batches after a project cursor, without copying source text.
    ///
    /// # Errors
    /// Rejects a future cursor or damaged event identities.
    pub fn project_delta_since(
        &self,
        project: &ProjectId,
        since: ProjectRevision,
        limit: usize,
    ) -> Result<Vec<ProjectDelta>, StoreError> {
        if since > self.project_revision(project)? || limit == 0 || limit > 100 {
            return Err(StoreError::InvalidBatch);
        }
        let mut statement = self.connection.prepare(
            "SELECT id,revision FROM domain_event_batches WHERE project_id=?1 AND revision>?2 ORDER BY revision LIMIT ?3",
        )?;
        let batches = statement
            .query_map(
                params![
                    project.as_str(),
                    to_sql_revision(since)?,
                    i64::try_from(limit).map_err(|_| StoreError::InvalidBatch)?
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        batches
            .into_iter()
            .map(|(id, revision)| {
                let batch = merl_core::BatchId::try_from(id.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?;
                let revision = ProjectRevision::from(
                    u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                );
                let (changes, changes_truncated) = self.batch_changes(project, &batch, 20)?;
                Ok(ProjectDelta {
                    batch,
                    revision,
                    changes,
                    changes_truncated,
                })
            })
            .collect()
    }

    /// Resolves one accepted batch to structural references in event order.
    ///
    /// # Errors
    /// Rejects damaged event identities or unreadable storage.
    pub fn batch_changes(
        &self,
        project: &ProjectId,
        batch: &merl_core::BatchId,
        limit: usize,
    ) -> Result<(Vec<DeltaChange>, bool), StoreError> {
        self.batch_changes_page(project, batch, 0, limit)
    }

    /// Keeps omitted changes reachable after a caller acknowledges a truncated inbox entry.
    ///
    /// # Errors
    /// Rejects invalid bounds or damaged accepted event identities.
    pub fn batch_changes_page(
        &self,
        project: &ProjectId,
        batch: &merl_core::BatchId,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<DeltaChange>, bool), StoreError> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidBatch);
        }
        let offset = i64::try_from(offset).map_err(|_| StoreError::InvalidBatch)?;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM domain_event_batches WHERE project_id=?1 AND id=?2)",
            params![project.as_str(), batch.as_str()],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(StoreError::InvalidBatch);
        }
        let mut statement = self.connection.prepare(
            "SELECT id,object_id,event_kind,event_index FROM domain_events WHERE project_id=?1 AND batch_id=?2
             UNION ALL
             SELECT id,relation_id,'put_relation',event_index FROM relation_events WHERE project_id=?1 AND batch_id=?2
             ORDER BY event_index LIMIT ?3 OFFSET ?4",
        )?;
        let mut changes = statement
            .query_map(
                params![
                    project.as_str(),
                    batch.as_str(),
                    i64::try_from(limit + 1).map_err(|_| StoreError::InvalidBatch)?,
                    offset
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?
            .map(|row| {
                let (event, reference, kind) = row?;
                Ok(DeltaChange {
                    event: merl_core::EventId::try_from(event.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    reference,
                    kind,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let truncated = changes.len() > limit;
        if truncated {
            changes.pop();
        }
        Ok((changes, truncated))
    }

    /// Finds a subscribed agent's accepted batch at a specific revision, even after ack.
    ///
    /// # Errors
    /// Rejects unreadable storage or invalid stored batch identities.
    pub fn inbox_entry_at(
        &self,
        project: &ProjectId,
        agent: &AgentId,
        revision: ProjectRevision,
    ) -> Result<Option<InboxEntry>, StoreError> {
        let batch: Option<String> = self.connection.query_row(
            "SELECT batch_id FROM inbox_entries WHERE project_id=?1 AND agent_id=?2 AND project_revision=?3",
            params![project.as_str(), agent.as_str(), to_sql_revision(revision)?],
            |row| row.get(0),
        ).optional()?;
        batch
            .map(|batch| {
                Ok(InboxEntry {
                    agent: agent.clone(),
                    batch: merl_core::BatchId::try_from(batch.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    revision,
                })
            })
            .transpose()
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

    /// Reads one bounded inbox page after the durable cursor.
    ///
    /// Callers need a next-page signal without loading the full subscriber history.
    ///
    /// # Errors
    /// Rejects invalid limits or damaged accepted identities.
    pub fn inbox_page_after(
        &self,
        project: &ProjectId,
        agent: &AgentId,
        cursor: ProjectRevision,
        limit: usize,
    ) -> Result<(Vec<InboxEntry>, bool), StoreError> {
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidInboxAcknowledgement);
        }
        let limit =
            i64::try_from(limit + 1).map_err(|_| StoreError::InvalidInboxAcknowledgement)?;
        let mut statement = self.connection.prepare(
            "SELECT batch_id,project_revision FROM inbox_entries
             WHERE project_id=?1 AND agent_id=?2 AND project_revision>?3
             ORDER BY project_revision LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![
                project.as_str(),
                agent.as_str(),
                to_sql_revision(cursor)?,
                limit
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        let mut entries = rows
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
            .collect::<Result<Vec<_>, StoreError>>()?;
        let has_more =
            entries.len() > usize::try_from(limit - 1).map_err(|_| StoreError::CorruptHistory)?;
        if has_more {
            entries.pop();
        }
        Ok((entries, has_more))
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
            "SELECT object_id,expected_revision,target_kind FROM policy_evaluation_writes
             WHERE project_id=?1 AND evaluation_id=?2 ORDER BY target_kind,object_id",
        )?;
        let writes = write_statement
            .query_map(params![project.as_str(), id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .map(|row| {
                let (target, revision, kind) = row?;
                let expected_revision = revision
                    .map(|value| {
                        ObjectRevision::try_from(
                            u64::try_from(value).map_err(|_| StoreError::CorruptHistory)?,
                        )
                        .map_err(|_| StoreError::CorruptHistory)
                    })
                    .transpose()?;
                match kind.as_str() {
                    "object" => Ok(merl_core::PolicyWrite::Object {
                        object: ObjectId::try_from(target.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        expected_revision,
                    }),
                    "relation" => Ok(merl_core::PolicyWrite::Relation {
                        relation: RelationId::try_from(target.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        expected_revision,
                    }),
                    _ => Err(StoreError::CorruptHistory),
                }
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let mut event_statement = self.connection.prepare(
            "SELECT event_id FROM policy_evaluation_domain_events WHERE project_id=?1 AND evaluation_id=?2
             UNION ALL SELECT event_id FROM policy_evaluation_relation_events WHERE project_id=?1 AND evaluation_id=?2 ORDER BY event_id",
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

    /// Lists accepted versions of one object without expanding protected text.
    ///
    /// # Errors
    /// Rejects corrupt event identities, revisions, or lifecycle values.
    pub fn object_history(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Vec<ObjectHistoryEntry>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT e.id,e.batch_id,b.revision,e.lifecycle,e.payload_id,p.evaluation_id,p.input_index FROM domain_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN policy_evaluation_domain_events p ON p.project_id=e.project_id AND p.event_id=e.id
             WHERE e.project_id=?1 AND e.object_id=?2 ORDER BY b.revision,e.event_index",
        )?;
        statement
            .query_map(params![project.as_str(), object.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })?
            .map(|row| {
                let (event, batch, revision, lifecycle, payload, evaluation, input_index) = row?;
                let evaluation = evaluation
                    .map(|id| PolicyEvaluationId::try_from(id.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?;
                let input = match (&evaluation, input_index) {
                    (Some(evaluation), Some(index)) => {
                        let record = self
                            .policy_evaluation(project, evaluation)?
                            .ok_or(StoreError::CorruptHistory)?;
                        Some(
                            record
                                .inputs
                                .get(
                                    usize::try_from(index)
                                        .map_err(|_| StoreError::CorruptHistory)?,
                                )
                                .ok_or(StoreError::CorruptHistory)?
                                .input
                                .clone(),
                        )
                    }
                    (None, None) => None,
                    _ => return Err(StoreError::CorruptHistory),
                };
                Ok(ObjectHistoryEntry {
                    event: merl_core::EventId::try_from(event.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    batch: merl_core::BatchId::try_from(batch.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    revision: ProjectRevision::from(
                        u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                    ),
                    lifecycle: ObjectLifecycle::try_from(lifecycle.as_str())
                        .map_err(|()| StoreError::CorruptHistory)?,
                    payload: payload
                        .map(|id| PayloadId::try_from(id.as_str()))
                        .transpose()
                        .map_err(|_| StoreError::CorruptHistory)?,
                    evaluation,
                    input,
                })
            })
            .collect()
    }

    /// Finds the accepted input responsible for a relation's latest event.
    ///
    /// # Errors
    /// Returns an error if an accepted relation's policy link is incomplete.
    pub fn relation_policy_origin(
        &self,
        project: &ProjectId,
        relation: &RelationId,
    ) -> Result<Option<ObjectPolicyOrigin>, StoreError> {
        let row: Option<(String, Option<String>, Option<i64>)> = self
            .connection
            .query_row(
                "SELECT e.id,p.evaluation_id,p.input_index FROM relation_events e
             JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN policy_evaluation_relation_events p
               ON p.project_id=e.project_id AND p.event_id=e.id
             WHERE e.project_id=?1 AND e.relation_id=?2
             ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
                params![project.as_str(), relation.as_str()],
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
        let row: Option<ObjectRow> = self
            .connection
            .query_row(
                "SELECT kind, payload_id, issue_scope_id, lifecycle, object_revision, project_revision FROM objects WHERE project_id = ?1 AND id = ?2",
                params![project.as_str(), id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .optional()?;
        row.map(
            |(kind, payload, issue_scope, lifecycle, revision, project_revision)| {
                Ok(ProjectedObject {
                    id: id.clone(),
                    kind: ObjectKind::try_from(kind.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    payload: payload
                        .map(|value| PayloadId::try_from(value.as_str()))
                        .transpose()
                        .map_err(|_| StoreError::CorruptHistory)?,
                    issue_scope,
                    lifecycle: ObjectLifecycle::try_from(lifecycle.as_str())
                        .map_err(|()| StoreError::CorruptHistory)?,
                    support: self.object_support_status(project, id)?,
                    revision: ObjectRevision::try_from(
                        u64::try_from(revision).map_err(|_| StoreError::CorruptHistory)?,
                    )
                    .map_err(|_| StoreError::CorruptHistory)?,
                    project_revision: ProjectRevision::from(
                        u64::try_from(project_revision).map_err(|_| StoreError::CorruptHistory)?,
                    ),
                })
            },
        )
        .transpose()
    }

    /// Reads a structural edge without expanding either endpoint's payload.
    ///
    /// # Errors
    /// Rejects corrupt relation metadata or storage failures.
    pub fn relation(
        &self,
        project: &ProjectId,
        id: &RelationId,
    ) -> Result<Option<ProjectedRelation>, StoreError> {
        let row: Option<(String, String, String, i64, i64)> = self
            .connection
            .query_row(
                "SELECT subject_id,relation_kind,object_id,relation_revision,project_revision
             FROM relations WHERE project_id=?1 AND id=?2",
                params![project.as_str(), id.as_str()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(subject, kind, object, revision, project_revision)| {
            Ok(ProjectedRelation {
                relation: Relation {
                    id: id.clone(),
                    project: project.clone(),
                    subject: ObjectId::try_from(subject.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    kind: RelationKind::try_from(kind.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                    object: ObjectId::try_from(object.as_str())
                        .map_err(|_| StoreError::CorruptHistory)?,
                },
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

    /// Reports evidence health without changing the accepted object's lifecycle.
    ///
    /// # Errors
    /// Returns a storage error if support history is unreadable.
    pub fn object_support_status(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<SupportStatus, StoreError> {
        let (supports, pending, current, unsupported): (i64, i64, i64, i64) =
            self.connection.query_row(
                "SELECT COUNT(*),
              COALESCE(SUM(CASE WHEN EXISTS (
                SELECT 1 FROM evidence_impacts i WHERE i.project_id=s.project_id
                  AND i.object_id=s.object_id AND i.support_event_id=s.event_id
                  AND i.next_action='recompile'
                  AND NOT EXISTS (SELECT 1 FROM evidence_revalidations r
                    WHERE r.project_id=i.project_id AND r.impact_id=i.id)
              ) THEN 1 ELSE 0 END),0),
              COALESCE(SUM(CASE WHEN NOT EXISTS (
                SELECT 1 FROM evidence_impacts i WHERE i.project_id=s.project_id
                  AND i.object_id=s.object_id AND i.support_event_id=s.event_id
                  AND (NOT EXISTS (SELECT 1 FROM evidence_revalidations r
                    WHERE r.project_id=i.project_id AND r.impact_id=i.id)
                    OR EXISTS (SELECT 1 FROM evidence_revalidations r
                    WHERE r.project_id=i.project_id AND r.impact_id=i.id
                      AND r.outcome='unsupported'))
              ) THEN 1 ELSE 0 END),0),
              COALESCE(SUM(CASE WHEN EXISTS (
                SELECT 1 FROM evidence_impacts i WHERE i.project_id=s.project_id
                  AND i.object_id=s.object_id AND i.support_event_id=s.event_id
                  AND ((i.next_action='reevaluate' AND NOT EXISTS
                    (SELECT 1 FROM evidence_revalidations r
                     WHERE r.project_id=i.project_id AND r.impact_id=i.id))
                    OR EXISTS (SELECT 1 FROM evidence_revalidations r
                     WHERE r.project_id=i.project_id AND r.impact_id=i.id
                       AND r.outcome='unsupported'))
              ) THEN 1 ELSE 0 END),0)
             FROM evidence_supports s WHERE s.project_id=?1 AND s.object_id=?2",
                params![project.as_str(), object.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        Ok(match (supports, pending, current, unsupported) {
            (0, _, _, _) | (_, 0, _, 0) => SupportStatus::Current,
            (_, 0, 0, _) => SupportStatus::Unsupported,
            (_, _, 0, _) => SupportStatus::RevalidationPending,
            _ => SupportStatus::PartiallySupported,
        })
    }

    /// Counts durable evidence impacts still waiting for revalidation.
    ///
    /// # Errors
    /// Returns a storage error if the impact log is unreadable.
    pub fn pending_revalidation_count(&self, project: &ProjectId) -> Result<u64, StoreError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM evidence_impacts i WHERE i.project_id=?1
               AND NOT EXISTS (SELECT 1 FROM evidence_revalidations r
                 WHERE r.project_id=i.project_id AND r.impact_id=i.id)",
            [project.as_str()],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
    }

    /// Returns the append-only impact trail for one accepted object.
    ///
    /// # Errors
    /// Rejects corrupt structural identities or unreadable storage.
    pub fn evidence_impacts_for_object(
        &self,
        project: &ProjectId,
        object: &ObjectId,
    ) -> Result<Vec<EvidenceImpact>, StoreError> {
        self.evidence_impacts_query(project, Some(object))
    }

    /// Lists unresolved reconsideration work after restart without scanning objects.
    ///
    /// # Errors
    /// Rejects damaged lineage or unreadable storage.
    pub fn pending_evidence_impacts(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<EvidenceImpact>, StoreError> {
        self.evidence_impacts_query(project, None)
    }

    fn evidence_impacts_query(
        &self,
        project: &ProjectId,
        object: Option<&ObjectId>,
    ) -> Result<Vec<EvidenceImpact>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT i.id,i.object_id,i.support_event_id,i.source_version_id,i.replacement_version_id,
                    i.next_action,r.event_id,i.affected_run_id,cr.source_version_id
             FROM evidence_impacts i LEFT JOIN evidence_revalidations r
               ON r.project_id=i.project_id AND r.impact_id=i.id
             LEFT JOIN compilation_runs cr
               ON cr.project_id=i.project_id AND cr.id=i.affected_run_id
             WHERE i.project_id=?1 AND (?2 IS NOT NULL AND i.object_id=?2
               OR ?2 IS NULL AND r.impact_id IS NULL)
             ORDER BY i.id",
        )?;
        let rows = statement.query_map(
            params![project.as_str(), object.map(ObjectId::as_str)],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (
                id,
                object,
                support_event,
                changed_source,
                replacement,
                next_action,
                revalidated_by,
                affected_run,
                trigger,
            ) = row?;
            Ok(EvidenceImpact {
                id,
                object: ObjectId::try_from(object.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                support_event: merl_core::EventId::try_from(support_event.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                affected_run: merl_core::CompilationRunId::try_from(
                    affected_run.as_deref().ok_or(StoreError::CorruptHistory)?,
                )
                .map_err(|_| StoreError::CorruptHistory)?,
                trigger: SourceVersionId::try_from(
                    trigger.as_deref().ok_or(StoreError::CorruptHistory)?,
                )
                .map_err(|_| StoreError::CorruptHistory)?,
                changed_source: SourceVersionId::try_from(changed_source.as_str())
                    .map_err(|_| StoreError::CorruptHistory)?,
                replacement: replacement
                    .map(|id| SourceVersionId::try_from(id.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
                next_action,
                revalidated_by: revalidated_by
                    .map(|id| merl_core::EventId::try_from(id.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?,
            })
        })
        .collect()
    }

    /// Finds one durable revalidation intent, including the run to revisit.
    ///
    /// # Errors
    /// Rejects damaged lineage or unreadable storage.
    pub fn evidence_impact(
        &self,
        project: &ProjectId,
        impact_id: &str,
    ) -> Result<Option<EvidenceImpact>, StoreError> {
        let object: Option<String> = self
            .connection
            .query_row(
                "SELECT object_id FROM evidence_impacts WHERE project_id=?1 AND id=?2",
                params![project.as_str(), impact_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object) = object else {
            return Ok(None);
        };
        let object = ObjectId::try_from(object.as_str()).map_err(|_| StoreError::CorruptHistory)?;
        self.evidence_impacts_for_object(project, &object)
            .map(|items| items.into_iter().find(|item| item.id == impact_id))
    }

    /// Reads the current Issue state without mixing provider facts into semantic objects.
    ///
    /// # Errors
    /// Rejects an invalid scope or corrupt accepted metadata.
    pub fn issue_state(
        &self,
        project: &ProjectId,
        issue: &ObjectId,
        scope: &str,
    ) -> Result<IssueState, StoreError> {
        if !valid_provider_id(scope) {
            return Err(StoreError::InvalidSource);
        }
        let mut statement = self.connection.prepare(
            "SELECT id FROM objects WHERE project_id=?1 AND issue_scope_id=?2
               AND kind!='provider_issue' ORDER BY id",
        )?;
        let ids = statement
            .query_map(params![project.as_str(), scope], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut semantics = Vec::with_capacity(ids.len());
        for id in ids {
            let id = ObjectId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            semantics.push(
                self.object(project, &id)?
                    .ok_or(StoreError::CorruptHistory)?,
            );
        }
        let mut relation_statement = self.connection.prepare(
            "SELECT id FROM relations WHERE project_id=?1 AND
             (subject_id=?2 OR object_id=?2
               OR subject_id IN (SELECT id FROM objects WHERE project_id=?1 AND issue_scope_id=?3)
               OR object_id IN (SELECT id FROM objects WHERE project_id=?1 AND issue_scope_id=?3))
             ORDER BY id",
        )?;
        let relation_ids = relation_statement
            .query_map(params![project.as_str(), issue.as_str(), scope], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut relations = Vec::with_capacity(relation_ids.len());
        for id in relation_ids {
            let id = RelationId::try_from(id.as_str()).map_err(|_| StoreError::CorruptHistory)?;
            relations.push(
                self.relation(project, &id)?
                    .ok_or(StoreError::CorruptHistory)?,
            );
        }
        Ok(IssueState {
            project_revision: self.project_revision(project)?,
            provider: self.provider_issue_head(project, issue)?,
            semantics,
            relations,
            coverage: self.semantic_coverage_in_scope(project, scope)?,
        })
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
    #[expect(
        clippy::too_many_lines,
        reason = "replay keeps the cross-kind event order visible in one pass"
    )]
    pub fn rebuild_projection(&mut self, project: &ProjectId) -> Result<(), StoreError> {
        let expected_revision = self.project_revision(project)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "DELETE FROM relations WHERE project_id = ?1",
            [project.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM objects WHERE project_id = ?1",
            [project.as_str()],
        )?;
        let mut statement = transaction.prepare(
            "SELECT b.revision, e.event_index, 'object', e.object_id, e.object_kind,
                    e.payload_id, e.issue_scope_id, NULL, NULL, e.lifecycle
             FROM domain_events e JOIN domain_event_batches b
               ON e.project_id=b.project_id AND e.batch_id=b.id
             WHERE e.project_id=?1
             UNION ALL
             SELECT b.revision, r.event_index, 'relation', r.relation_id, r.relation_kind,
                    NULL, NULL, r.subject_id, r.object_id, NULL
             FROM relation_events r JOIN domain_event_batches b
               ON r.project_id=b.project_id AND r.batch_id=b.id
             WHERE r.project_id=?1
             ORDER BY 1,2",
        )?;
        let rows = statement.query_map([project.as_str()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        let mut latest_revision = 0_i64;
        for row in rows {
            let (
                revision,
                event_kind,
                object,
                kind,
                payload,
                issue_scope,
                subject,
                target,
                lifecycle,
            ) = row?;
            latest_revision = revision;
            if event_kind == "relation" {
                apply_relation(
                    &transaction,
                    &Relation {
                        id: RelationId::try_from(object.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        project: project.clone(),
                        subject: ObjectId::try_from(
                            subject.as_deref().ok_or(StoreError::CorruptHistory)?,
                        )
                        .map_err(|_| StoreError::CorruptHistory)?,
                        kind: RelationKind::try_from(kind.as_str())
                            .map_err(|_| StoreError::CorruptHistory)?,
                        object: ObjectId::try_from(
                            target.as_deref().ok_or(StoreError::CorruptHistory)?,
                        )
                        .map_err(|_| StoreError::CorruptHistory)?,
                    },
                    revision,
                )?;
            } else if event_kind == "object" {
                let object =
                    ObjectId::try_from(object.as_str()).map_err(|_| StoreError::CorruptHistory)?;
                let kind =
                    ObjectKind::try_from(kind.as_str()).map_err(|_| StoreError::CorruptHistory)?;
                let payload = payload
                    .map(|value| PayloadId::try_from(value.as_str()))
                    .transpose()
                    .map_err(|_| StoreError::CorruptHistory)?;
                let lifecycle = ObjectLifecycle::try_from(
                    lifecycle.as_deref().ok_or(StoreError::CorruptHistory)?,
                )
                .map_err(|()| StoreError::CorruptHistory)?;
                apply_put(
                    &transaction,
                    project,
                    &object,
                    &kind,
                    payload.as_ref(),
                    issue_scope.as_deref(),
                    lifecycle,
                    revision,
                )?;
            } else {
                return Err(StoreError::CorruptHistory);
            }
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
            "SELECT (SELECT COUNT(*) FROM domain_events WHERE project_id=?1)
                  + (SELECT COUNT(*) FROM relation_events WHERE project_id=?1)",
            [project.as_str()],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
    }
}

fn collect_purge_consequences(
    connection: &Connection,
    project: &ProjectId,
    run: &str,
    payloads: &mut BTreeSet<String>,
    events: &mut BTreeSet<String>,
    objects: &mut BTreeSet<String>,
    relations: &mut BTreeSet<String>,
) -> Result<u64, StoreError> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM observed_assertions WHERE project_id=?1 AND run_id=?2",
        params![project.as_str(), run],
        |row| row.get(0),
    )?;
    let mut object_statement = connection.prepare(
        "SELECT DISTINCT e.id,e.object_id,e.payload_id
         FROM policy_assertion_inputs a
         JOIN policy_evaluation_inputs i
           ON i.project_id=a.project_id AND i.evaluation_id=a.evaluation_id AND i.input_id=a.input_id
         JOIN policy_evaluation_domain_events o
           ON o.project_id=i.project_id AND o.evaluation_id=i.evaluation_id AND o.input_index=i.input_index
         JOIN domain_events e ON e.project_id=o.project_id AND e.id=o.event_id
         WHERE a.project_id=?1 AND a.run_id=?2 ORDER BY e.id",
    )?;
    let object_rows = object_statement.query_map(params![project.as_str(), run], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in object_rows {
        let (event, object, payload) = row?;
        events.insert(event);
        objects.insert(object);
        if let Some(payload) = payload {
            payloads.insert(payload);
        }
    }
    let mut relation_statement = connection.prepare(
        "SELECT DISTINCT e.id,e.relation_id
         FROM policy_assertion_inputs a
         JOIN policy_evaluation_inputs i
           ON i.project_id=a.project_id AND i.evaluation_id=a.evaluation_id AND i.input_id=a.input_id
         JOIN policy_evaluation_relation_events o
           ON o.project_id=i.project_id AND o.evaluation_id=i.evaluation_id AND o.input_index=i.input_index
         JOIN relation_events e ON e.project_id=o.project_id AND e.id=o.event_id
         WHERE a.project_id=?1 AND a.run_id=?2 ORDER BY e.id",
    )?;
    let relation_rows = relation_statement.query_map(params![project.as_str(), run], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in relation_rows {
        let (event, relation) = row?;
        events.insert(event);
        relations.insert(relation);
    }
    u64::try_from(count).map_err(|_| StoreError::CorruptHistory)
}

fn purge_preview_digest(project: &ProjectId, preview: &PurgePreview) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(project.as_str().as_bytes());
    hash.update([0]);
    hash.update(preview.source.as_str().as_bytes());
    for payload in &preview.payloads {
        hash.update([0]);
        hash.update(payload.id.as_str().as_bytes());
        hash.update(payload.digest);
    }
    for run in &preview.runs {
        hash.update([1]);
        hash.update(run.as_str().as_bytes());
    }
    for event in &preview.events {
        hash.update([2]);
        hash.update(event.as_str().as_bytes());
    }
    for object in &preview.objects {
        hash.update([3]);
        hash.update(object.as_str().as_bytes());
    }
    for relation in &preview.relations {
        hash.update([4]);
        hash.update(relation.as_str().as_bytes());
    }
    hash.update(preview.assertions.to_be_bytes());
    hash.finalize().into()
}

fn purge_reason_id(project: &ProjectId, source: &SourceVersionId) -> Result<PayloadId, StoreError> {
    let digest = Sha256::digest(format!("{project}/{source}").as_bytes());
    let mut id = String::from("purge_reason_");
    for byte in digest {
        write!(&mut id, "{byte:02x}").expect("writing a digest is infallible");
    }
    PayloadId::try_from(id.as_str()).map_err(|_| StoreError::InvalidPurge)
}

fn to_sql_revision(revision: ProjectRevision) -> Result<i64, StoreError> {
    i64::try_from(revision.get()).map_err(|_| StoreError::CorruptHistory)
}

#[expect(
    clippy::too_many_lines,
    reason = "the shape guard checks one atomic evaluation and its event origins"
)]
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
                || (provider.is_none()
                    && batch.events.iter().any(|event| {
                        matches!(event, DomainEvent::PutObject { kind, .. } if kind.as_str() == "provider_issue")
                    }))
            {
                return Err(StoreError::InvalidPolicyEvaluation);
            }
            let event_targets: HashSet<(&str, &str)> = batch
                .events
                .iter()
                .map(|event| match event {
                    DomainEvent::PutObject { object, .. } => ("object", object.as_str()),
                    DomainEvent::PutRelation { relation, .. } => ("relation", relation.id.as_str()),
                })
                .collect();
            let writes: HashSet<(&str, &str)> = evaluation
                .writes
                .iter()
                .map(|write| match write {
                    merl_core::PolicyWrite::Object { object, .. } => ("object", object.as_str()),
                    merl_core::PolicyWrite::Relation { relation, .. } => {
                        ("relation", relation.as_str())
                    }
                })
                .collect();
            if event_targets.len() != batch.events.len()
                || writes.len() != evaluation.writes.len()
                || event_targets != writes
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
                || !batch.events.iter().all(|event| match event {
                    DomainEvent::PutObject { id, .. } | DomainEvent::PutRelation { id, .. } => {
                        origin_events.contains(id.as_str())
                    }
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
        let (table, target, reason_code) = match write {
            merl_core::PolicyWrite::Object { object, .. } => {
                ("objects", object.as_str(), "object_write_changed")
            }
            merl_core::PolicyWrite::Relation { relation, .. } => {
                ("relations", relation.as_str(), "relation_write_changed")
            }
        };
        let actual: Option<(i64, i64)> = transaction
            .query_row(
                if table == "objects" {
                    "SELECT object_revision,project_revision FROM objects WHERE project_id=?1 AND id=?2"
                } else {
                    "SELECT relation_revision,project_revision FROM relations WHERE project_id=?1 AND id=?2"
                },
                params![evaluation.project.as_str(), target],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let revision = match write {
            merl_core::PolicyWrite::Object {
                expected_revision, ..
            }
            | merl_core::PolicyWrite::Relation {
                expected_revision, ..
            } => expected_revision,
        };
        let expected = revision
            .map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory))
            .transpose()?;
        if actual.map(|row| row.0) != expected || actual.is_some_and(|row| row.1 > basis) {
            return Ok(Some(PolicyConflictDetail {
                reason_code: reason_code.into(),
                target_id: Some(target.into()),
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

#[expect(
    clippy::too_many_lines,
    reason = "the audit record is written as one transaction-local unit"
)]
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
        let (kind, target, revision) = match write {
            merl_core::PolicyWrite::Object {
                object,
                expected_revision,
            } => ("object", object.as_str(), expected_revision),
            merl_core::PolicyWrite::Relation {
                relation,
                expected_revision,
            } => ("relation", relation.as_str(), expected_revision),
        };
        transaction.execute(
            "INSERT INTO policy_evaluation_writes (project_id,evaluation_id,object_id,expected_revision,target_kind)
             VALUES (?1,?2,?3,?4,?5)",
            params![evaluation.project.as_str(), evaluation.id.as_str(), target, revision.map(|value| i64::try_from(value.get()).map_err(|_| StoreError::CorruptHistory)).transpose()?, kind],
        )?;
    }
    if conflict.is_none() && !duplicate {
        for origin in &evaluation.event_origins {
            let event = evaluation
                .batch
                .as_ref()
                .and_then(|batch| {
                    batch.events.iter().find(|event| match event {
                        DomainEvent::PutObject { id, .. } | DomainEvent::PutRelation { id, .. } => {
                            id == &origin.event
                        }
                    })
                })
                .ok_or(StoreError::InvalidPolicyEvaluation)?;
            let link_table = match event {
                DomainEvent::PutObject { .. } => "policy_evaluation_domain_events",
                DomainEvent::PutRelation { .. } => "policy_evaluation_relation_events",
            };
            transaction.execute(
                if link_table == "policy_evaluation_domain_events" {
                    "INSERT INTO policy_evaluation_domain_events (project_id,evaluation_id,event_id,input_index) VALUES (?1,?2,?3,?4)"
                } else {
                    "INSERT INTO policy_evaluation_relation_events (project_id,evaluation_id,event_id,input_index) VALUES (?1,?2,?3,?4)"
                },
                params![evaluation.project.as_str(), evaluation.id.as_str(), origin.event.as_str(), i64::from(origin.input_index)],
            )?;
            if matches!(event, DomainEvent::PutObject { .. }) {
                record_accepted_support(transaction, evaluation, origin)?;
            }
        }
    }
    Ok(())
}

fn record_accepted_support(
    transaction: &Transaction<'_>,
    evaluation: &PolicyEvaluation,
    origin: &merl_core::PolicyEventOrigin,
) -> Result<(), StoreError> {
    let Some(merl_core::PolicyInputDecision {
        input: merl_core::PolicyInput::ObservedAssertion { run, index, .. },
        ..
    }) = evaluation.inputs.get(origin.input_index as usize)
    else {
        return Ok(());
    };
    let source: String = transaction.query_row(
        "SELECT source_version_id FROM observed_assertions
         WHERE project_id=?1 AND run_id=?2 AND assertion_index=?3",
        params![evaluation.project.as_str(), run.as_str(), i64::from(*index)],
        |row| row.get(0),
    )?;
    let object: String = transaction.query_row(
        "SELECT object_id FROM domain_events WHERE project_id=?1 AND id=?2",
        params![evaluation.project.as_str(), origin.event.as_str()],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO evidence_supports (project_id,object_id,source_version_id,event_id)
         VALUES (?1,?2,?3,?4)",
        params![
            evaluation.project.as_str(),
            object,
            source,
            origin.event.as_str()
        ],
    )?;
    transaction.execute(
        "INSERT INTO evidence_revalidations (project_id,impact_id,event_id,outcome)
         SELECT i.project_id,i.id,?4,'current' FROM evidence_impacts i
         JOIN compilation_runs prior
           ON prior.project_id=i.project_id AND prior.id=i.affected_run_id
         WHERE i.project_id=?1 AND i.object_id=?2 AND i.replacement_version_id=?3
           AND i.source_version_id=prior.source_version_id
           AND NOT EXISTS (SELECT 1 FROM evidence_revalidations r
             WHERE r.project_id=i.project_id AND r.impact_id=i.id)",
        params![
            evaluation.project.as_str(),
            object,
            source,
            origin.event.as_str()
        ],
    )?;
    transaction.execute(
        "INSERT INTO evidence_revalidations (project_id,impact_id,event_id,outcome)
         SELECT i.project_id,i.id,?4,'current' FROM evidence_impacts i
         JOIN compilation_runs prior
           ON prior.project_id=i.project_id AND prior.id=i.affected_run_id
         JOIN compilation_runs revised
           ON revised.project_id=i.project_id AND revised.id=?3
         JOIN compilation_context_sources cs
           ON cs.project_id=revised.project_id AND cs.run_id=revised.id
          AND cs.source_version_id=i.replacement_version_id
         WHERE i.project_id=?1 AND i.object_id=?2 AND i.id=?3
           AND i.next_action='recompile' AND revised.mode='hindsight'
           AND revised.source_version_id=prior.source_version_id
           AND NOT EXISTS (SELECT 1 FROM evidence_revalidations r
             WHERE r.project_id=i.project_id AND r.impact_id=i.id)",
        params![
            evaluation.project.as_str(),
            object,
            run.as_str(),
            origin.event.as_str()
        ],
    )?;
    Ok(())
}

fn record_evidence_impacts(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    previous: &str,
    replacement: Option<&str>,
    next_action: &str,
    recorded_at_millis: i64,
) -> Result<(), StoreError> {
    let mut statement = transaction.prepare(
        "SELECT DISTINCT s.object_id,s.event_id,pa.run_id FROM evidence_supports s
         JOIN policy_evaluation_domain_events pe
           ON pe.project_id=s.project_id AND pe.event_id=s.event_id
         JOIN policy_evaluation_inputs pi
           ON pi.project_id=pe.project_id AND pi.evaluation_id=pe.evaluation_id
          AND pi.input_index=pe.input_index
         JOIN policy_assertion_inputs pa
           ON pa.project_id=pi.project_id AND pa.evaluation_id=pi.evaluation_id
          AND pa.input_id=pi.input_id
         WHERE s.project_id=?1 AND (s.source_version_id=?2 OR EXISTS (
           SELECT 1 FROM compilation_context_sources cs
           WHERE cs.project_id=pa.project_id AND cs.run_id=pa.run_id
             AND cs.source_version_id=?2))
         ORDER BY s.object_id,s.event_id",
    )?;
    let supports = statement
        .query_map(params![project.as_str(), previous], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (object, support_event, affected_run) in supports {
        let digest = Sha256::digest(
            format!(
                "{previous}/{}/{support_event}",
                replacement.unwrap_or("purged")
            )
            .as_bytes(),
        );
        let mut id = String::from("impact_");
        for byte in digest {
            write!(&mut id, "{byte:02x}").expect("writing a digest is infallible");
        }
        transaction.execute(
            "INSERT OR IGNORE INTO evidence_impacts
             (project_id,id,object_id,support_event_id,source_version_id,replacement_version_id,next_action,recorded_at_millis,affected_run_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![project.as_str(), id, object, support_event, previous, replacement, next_action, recorded_at_millis, affected_run],
        )?;
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

#[expect(
    clippy::too_many_lines,
    reason = "the digest covers all structural policy inputs in a fixed order"
)]
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
        match write {
            merl_core::PolicyWrite::Object {
                object,
                expected_revision,
            } => {
                part(b"object");
                part(object.as_str().as_bytes());
                part(
                    &expected_revision
                        .map_or(0, ObjectRevision::get)
                        .to_be_bytes(),
                );
            }
            merl_core::PolicyWrite::Relation {
                relation,
                expected_revision,
            } => {
                part(b"relation");
                part(relation.as_str().as_bytes());
                part(
                    &expected_revision
                        .map_or(0, ObjectRevision::get)
                        .to_be_bytes(),
                );
            }
        }
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
            match event {
                DomainEvent::PutObject {
                    id,
                    object,
                    kind,
                    payload,
                    issue_scope,
                    lifecycle,
                } => {
                    part(id.as_str().as_bytes());
                    part(object.as_str().as_bytes());
                    part(kind.as_str().as_bytes());
                    part(payload.as_ref().map_or("", PayloadId::as_str).as_bytes());
                    if let Some(scope) = issue_scope {
                        part(b"issue_scope_v1");
                        part(scope.as_bytes());
                    }
                    part(lifecycle.as_str().as_bytes());
                }
                DomainEvent::PutRelation { id, relation } => {
                    part(b"put_relation_v1");
                    part(id.as_str().as_bytes());
                    part(relation.id.as_str().as_bytes());
                    part(relation.subject.as_str().as_bytes());
                    part(relation.kind.as_str().as_bytes());
                    part(relation.object.as_str().as_bytes());
                }
            }
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
        let event_id = match event {
            DomainEvent::PutObject { id, .. } | DomainEvent::PutRelation { id, .. } => id,
        };
        ensure_event_id_unused(transaction, &batch.project, event_id)?;
        match event {
            DomainEvent::PutObject {
                id,
                object,
                kind,
                payload,
                issue_scope,
                lifecycle,
            } => {
                let existing_kind: Option<String> = transaction
                    .query_row(
                        "SELECT kind FROM objects WHERE project_id=?1 AND id=?2",
                        params![batch.project.as_str(), object.as_str()],
                        |row| row.get(0),
                    )
                    .optional()?;
                if !provider_kind_transition_allowed(
                    existing_kind.as_deref(),
                    kind,
                    observation.is_some(),
                ) {
                    return Err(StoreError::InvalidBatch);
                }
                if issue_scope
                    .as_deref()
                    .is_some_and(|scope| !valid_provider_id(scope))
                {
                    return Err(StoreError::InvalidBatch);
                }
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
                    "INSERT INTO domain_events (project_id, batch_id, id, event_index, event_kind, object_id, object_kind, payload_id, issue_scope_id, lifecycle) VALUES (?1, ?2, ?3, ?4, 'put_object', ?5, ?6, ?7, ?8, ?9)",
                    params![batch.project.as_str(), batch.id.as_str(), id.as_str(), i64::try_from(index).map_err(|_| StoreError::InvalidBatch)?, object.as_str(), kind.as_str(), payload.as_ref().map(PayloadId::as_str), issue_scope, lifecycle.as_str()],
                )?;
                apply_put(
                    transaction,
                    &batch.project,
                    object,
                    kind,
                    payload.as_ref(),
                    issue_scope.as_deref(),
                    *lifecycle,
                    revision,
                )?;
            }
            DomainEvent::PutRelation { id, relation } => {
                if relation.project != batch.project {
                    return Err(StoreError::InvalidBatch);
                }
                transaction.execute(
                    "INSERT INTO relation_events
                     (project_id,batch_id,id,event_index,relation_id,subject_id,relation_kind,object_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![batch.project.as_str(), batch.id.as_str(), id.as_str(),
                        i64::try_from(index).map_err(|_| StoreError::InvalidBatch)?,
                        relation.id.as_str(), relation.subject.as_str(),
                        relation.kind.as_str(), relation.object.as_str()],
                )?;
                apply_relation(transaction, relation, revision)?;
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

fn ensure_event_id_unused(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    event: &merl_core::EventId,
) -> Result<(), StoreError> {
    let reused: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM domain_events WHERE project_id=?1 AND id=?2
         UNION ALL SELECT 1 FROM relation_events WHERE project_id=?1 AND id=?2)",
        params![project.as_str(), event.as_str()],
        |row| row.get(0),
    )?;
    if reused {
        Err(StoreError::InvalidBatch)
    } else {
        Ok(())
    }
}

fn provider_kind_transition_allowed(
    previous: Option<&str>,
    next: &ObjectKind,
    has_observation: bool,
) -> bool {
    !(previous == Some("provider_issue") && !has_observation
        || next.as_str() == "provider_issue"
            && previous.is_some_and(|kind| kind != "provider_issue"))
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
            lifecycle,
            ..
        },
    ] = batch.events.as_slice()
    else {
        return Err(StoreError::InvalidBatch);
    };
    if object != &observation.issue
        || kind.as_str() != "provider_issue"
        || payload != &observation.snapshot_payload
        || *lifecycle != ObjectLifecycle::Active
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

#[expect(
    clippy::too_many_arguments,
    reason = "projection replay passes a decoded object event without an intermediate allocation"
)]
fn apply_put(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    object: &ObjectId,
    kind: &ObjectKind,
    payload: Option<&PayloadId>,
    issue_scope: Option<&str>,
    lifecycle: ObjectLifecycle,
    revision: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO objects (project_id, id, kind, payload_id, issue_scope_id, lifecycle, object_revision, project_revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)
         ON CONFLICT(project_id, id) DO UPDATE SET
           kind = excluded.kind,
           payload_id = excluded.payload_id,
           issue_scope_id = COALESCE(excluded.issue_scope_id, objects.issue_scope_id),
           lifecycle = excluded.lifecycle,
           object_revision = objects.object_revision + 1,
           project_revision = excluded.project_revision",
        params![
            project.as_str(),
            object.as_str(),
            kind.as_str(),
            payload.map(PayloadId::as_str),
            issue_scope,
            lifecycle.as_str(),
            revision
        ],
    )?;
    Ok(())
}

fn apply_relation(
    transaction: &Transaction<'_>,
    relation: &Relation,
    revision: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "INSERT INTO relations
         (project_id,id,subject_id,relation_kind,object_id,relation_revision,project_revision)
         VALUES (?1,?2,?3,?4,?5,1,?6)
         ON CONFLICT(project_id,id) DO UPDATE SET
           subject_id=excluded.subject_id,
           relation_kind=excluded.relation_kind,
           object_id=excluded.object_id,
           relation_revision=relations.relation_revision+1,
           project_revision=excluded.project_revision",
        params![
            relation.project.as_str(),
            relation.id.as_str(),
            relation.subject.as_str(),
            relation.kind.as_str(),
            relation.object.as_str(),
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
    let hindsight = record.mode == "hindsight";
    if record.interpretation_basis_revision > store.project_revision(project)?
        || (!source.interpretation_basis_known && record.mode == "live")
        || record.source_observation_cutoff > store.source_observation_head(project)?
        || (!hindsight && record.source_observation_cutoff != source.sequence)
        || (hindsight && record.source_observation_cutoff < source.sequence)
        || (!hindsight && causal_basis != Some(record.interpretation_basis_revision))
        || record.context.len() > record.max_input_bytes
        || (hindsight && !record.source_window.contains(record.source))
        || (!hindsight && record.source_window.last() != Some(record.source))
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
        if item.sequence > record.source_observation_cutoff {
            return Err(StoreError::InvalidCompilation);
        }
    }
    if let Some(impact) = store.evidence_impact(project, record.id)? {
        validate_revalidation_intent(store, project, record, &impact)?;
    }
    Ok(())
}

fn validate_revalidation_intent(
    store: &Store,
    project: &ProjectId,
    record: &CompilationIntent<'_>,
    impact: &EvidenceImpact,
) -> Result<(), StoreError> {
    let replacement = impact
        .replacement
        .as_ref()
        .ok_or(StoreError::InvalidCompilation)?;
    let expected_trigger = if impact.trigger == impact.changed_source {
        replacement
    } else {
        &impact.trigger
    };
    let mut expected_window =
        store.compilation_context_sources(project, impact.affected_run.as_str())?;
    let changed = expected_window
        .iter_mut()
        .find(|version| **version == impact.changed_source)
        .ok_or(StoreError::InvalidCompilation)?;
    *changed = replacement.clone();
    let mut expected_order = Vec::with_capacity(expected_window.len());
    for version in expected_window {
        let source = store
            .source_version(project, &version)?
            .ok_or(StoreError::InvalidCompilation)?;
        expected_order.push((source.sequence, version));
    }
    expected_order.sort_by_key(|item| item.0);
    let expected_window: Vec<_> = expected_order
        .into_iter()
        .map(|(_, version)| version)
        .collect();
    if record.mode != "hindsight"
        || record.source != expected_trigger
        || record.source_window != expected_window
        || record.interpretation_basis_revision != store.project_revision(project)?
        || record.source_observation_cutoff != store.source_observation_head(project)?
        || impact.next_action != "recompile"
        || impact.revalidated_by.is_some()
    {
        return Err(StoreError::InvalidCompilation);
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

    #[test]
    fn relation_migration_preserves_existing_object_write_guards() {
        let connection = Connection::open_in_memory().expect("open SQLite");
        for migration in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_sources.sql"),
            include_str!("../migrations/0003_compilation.sql"),
            include_str!("../migrations/0004_policy.sql"),
            include_str!("../migrations/0005_policy_conflicts.sql"),
            include_str!("../migrations/0006_policy_event_origins.sql"),
            include_str!("../migrations/0007_issue_scope.sql"),
            include_str!("../migrations/0008_evidence_health.sql"),
        ] {
            connection
                .execute_batch(migration)
                .expect("create prior schema");
        }
        connection
            .execute_batch(
                "INSERT INTO projects (id) VALUES ('P1');
             INSERT INTO policy_evaluations
               (project_id,id,actor_id,policy_version,policy_config_digest,
                basis_project_revision,evaluation_digest)
               VALUES ('P1','PE1','alice','v1',zeroblob(32),0,zeroblob(32));
             INSERT INTO policy_evaluation_writes
               (project_id,evaluation_id,object_id,expected_revision)
               VALUES ('P1','PE1','D1',NULL);",
            )
            .expect("seed prior write guard");
        connection
            .pragma_update(None, "user_version", 8)
            .expect("mark prior schema");

        let store = Store::from_connection(connection).expect("migrate relation schema");
        let kind: String = store
            .connection
            .query_row(
                "SELECT target_kind FROM policy_evaluation_writes
             WHERE project_id='P1' AND evaluation_id='PE1' AND object_id='D1'",
                [],
                |row| row.get(0),
            )
            .expect("preserved write guard");
        assert_eq!(kind, "object");
    }

    #[test]
    fn storage_rejects_provider_replacement_of_a_semantic_object() {
        use merl_core::{
            ActorId, BatchId, DomainEvent, DomainEventBatch, EventId, ObjectId, ObjectKind,
            ObjectLifecycle, ProjectId,
        };

        let mut store = Store::open_in_memory().expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let object = ObjectId::try_from("D1").expect("object");
        store.create_project(&project).expect("project");
        let batch = |batch, event, kind| DomainEventBatch {
            id: BatchId::try_from(batch).expect("batch"),
            project: project.clone(),
            actor: ActorId::try_from("owner").expect("actor"),
            occurred_at_millis: 1,
            events: vec![DomainEvent::PutObject {
                id: EventId::try_from(event).expect("event"),
                object: object.clone(),
                kind: ObjectKind::try_from(kind).expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: ObjectLifecycle::Active,
            }],
        };
        store
            .commit_unchecked_bootstrap(&batch("B1", "E1", "decision"))
            .expect("decision");
        assert!(matches!(
            store.commit_unchecked_bootstrap(&batch("B2", "E2", "provider_issue")),
            Err(StoreError::InvalidBatch)
        ));
        assert_eq!(
            store
                .object(&project, &object)
                .expect("object")
                .expect("present")
                .kind
                .as_str(),
            "decision"
        );
        assert_eq!(store.project_revision(&project).expect("revision").get(), 1);
    }
}
