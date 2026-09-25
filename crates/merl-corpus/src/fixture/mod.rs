//! Frozen evaluation histories and their temporal gold-state labels.

use std::collections::HashSet;
use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Schema understood by this version of the corpus tooling.
/// Frozen corpus contract used by existing development and natural fixtures.
pub const FIXTURE_SCHEMA_V1: &str = "merl.corpus-fixture/v1";
/// Captures upstream update time and stable label and assignee identities.
pub const FIXTURE_SCHEMA_V2: &str = "merl.corpus-fixture/v2";
/// States when retained body bytes were available, rather than implying that
/// terminal capture bytes existed at every earlier source cutoff.
pub const FIXTURE_SCHEMA_V3: &str = "merl.corpus-fixture/v3";
/// Schema emitted by new corpus captures.
pub const FIXTURE_SCHEMA: &str = FIXTURE_SCHEMA_V3;

/// A versioned evaluation fixture.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Fixture {
    /// Stable schema identifier.
    pub schema: String,
    /// Stable corpus identifier.
    pub id: String,
    /// Corpus partition.
    pub partition: Partition,
    /// Whether the history occurred naturally or was staged for one behavior.
    pub origin: Origin,
    /// External identity of the captured source.
    pub source: Source,
    /// Capture provenance and integrity metadata.
    pub capture: Capture,
    /// Redistribution review for retained public text.
    pub provenance: Provenance,
    /// Provider-owned Issue facts at capture time.
    pub provider_snapshot: ProviderSnapshot,
    /// Ordered source observations.
    pub observations: Vec<Observation>,
    /// Expected state at selected causal cutoffs.
    #[serde(default)]
    pub gold_states: Vec<GoldState>,
}

/// Corpus partition.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Partition {
    /// Visible material used while implementing the compiler.
    Development,
    /// Visible cases designed to expose known failure modes.
    Adversarial,
    /// Material sealed from implementation agents until release evaluation.
    HeldOut,
}

/// Fixture origin.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A captured project history.
    Natural,
    /// A deliberately staged history with known ground truth.
    Controlled,
}

/// Provider identity for the captured history.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    /// Source system, such as `github` or `controlled`.
    pub provider: String,
    /// Provider repository name when the source belongs to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Rename-stable provider repository ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_provider_id: Option<String>,
    /// Human-facing Issue number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    /// Rename-stable provider Issue ID.
    pub issue_provider_id: String,
    /// URL used to audit or recapture the source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Capture provenance and integrity metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Capture {
    /// Time the provider snapshot was taken.
    pub captured_at: String,
    /// Tool version that wrote this shape.
    pub capture_tool_version: String,
    /// Digest of the normalized observations.
    pub source_sha256: String,
    /// Strength of the retained historical reconstruction.
    pub history_fidelity: HistoryFidelity,
    /// Number of ordered observations retained in the fixture.
    pub provider_observation_count: usize,
}

/// Fidelity available for historical source versions.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFidelity {
    /// Merl observed each source version when it was current.
    ExactObserved,
    /// Provider diffs were sufficient to verify each prior body.
    DiffReconstructable,
    /// Provider diffs exist but do not establish exact prior bodies.
    DiffOnly,
    /// Only the terminal provider body was available.
    TerminalSnapshotOnly,
    /// A controlled fixture records each staged version exactly.
    StagedExact,
}

/// Earliest point at which retained bytes may be disclosed to a causal reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyAvailability {
    /// Merl observed this body at its version event, or reconstructed it exactly.
    AtObservation,
    /// Only the terminal capture proves these bytes; earlier cutoffs must hide them.
    AtCapture,
}

/// Provenance needed before corpus text can be redistributed.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Provenance {
    /// License identifier observed in the source repository.
    pub repository_license_at_capture: String,
    /// Current review state for retaining the source text in this repository.
    pub redistribution_review: RedistributionReview,
}

/// Review state for redistributing retained source text.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedistributionReview {
    /// A maintainer must review the retained text before release.
    Pending,
    /// A maintainer approved retaining the text with its provenance.
    Approved,
    /// The payload must remain outside the public corpus.
    Restricted,
}

/// Provider-owned Issue facts at capture time.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderSnapshot {
    /// Terminal Issue title at capture time, not a historical fact at every cutoff.
    pub title: String,
    /// Provider state, such as `OPEN` or `CLOSED`.
    pub state: String,
    /// Provider update time for this terminal snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Time the provider closed the Issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// Current provider labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Stable IDs paired with the display labels above, when captured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_refs: Vec<ProviderLabelRef>,
    /// Current provider assignees.
    #[serde(default)]
    pub assignees: Vec<ActorRef>,
    /// Current provider milestone title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
}

/// Rename-stable label identity and its display name at capture.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderLabelRef {
    /// Provider node ID.
    pub provider_id: String,
    /// Provider label name at capture time.
    pub name: String,
}

/// Rename-stable provider actor identity and its display login at capture.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActorRef {
    /// Provider actor type; absent in older captures and never inferred from a login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// Provider node ID, when the actor also implements GitHub's Node interface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// Login shown to users at capture time.
    pub login: String,
}

/// One immutable source version in causal order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Observation {
    /// One-based order assigned when the fixture was captured.
    pub sequence: u64,
    /// Kind of provider content.
    pub kind: ObservationKind,
    /// Provider-stable external entity identifier, shared by its versions.
    pub provider_id: String,
    /// Provider-stable ID of this version or edit event.
    pub version_id: String,
    /// Earlier observation sequence for the same external entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<u64>,
    /// A timestamp tie with the preceding observation lacks a causal order.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ambiguous_order_with_previous: bool,
    /// Author recorded by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<ActorRef>,
    /// Time this version became visible according to the provider.
    pub occurred_at: String,
    /// Authored RFC 3339 timestamp with the author's recorded local offset.
    /// Provider UTC transport timestamps do not establish this evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authored_at: Option<String>,
    /// Named author timezone, when the source records one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_timezone: Option<String>,
    /// Original provider creation time for the external entity.
    pub created_at: String,
    /// Entity update time at capture, which may follow this body edit.
    /// Earlier versions leave it unknown unless a separate capture established it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Exact source text, absent when the provider cannot reconstruct it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// When the captured bytes became available to an evaluator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_availability: Option<BodyAvailability>,
    /// Hash of the exact captured version, absent with the body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_sha256: Option<String>,
    /// Why exact bytes are absent, if they are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_body_reason: Option<MissingBodyReason>,
    /// Edit metadata, when this observation represents an edit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<ContentEdit>,
}

/// Kind of source observation.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationKind {
    /// Initial Issue body.
    Issue,
    /// A comment on the Issue.
    IssueComment,
    /// A controlled source with no external provider type.
    Controlled,
}

/// Reason a historical source version cannot be replayed exactly.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingBodyReason {
    /// Only an edit diff, not the complete old body, was available.
    PriorVersionUnavailable,
    /// The provider removed this version's retained content.
    DeletedByProvider,
}

/// Provider metadata for an edit to user-authored content.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentEdit {
    /// Provider-stable edit ID.
    pub provider_id: String,
    /// Actor who made the edit when the provider exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<ActorRef>,
    /// Time of the edit.
    pub edited_at: String,
    /// Provider diff. A diff does not by itself prove the exact prior body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// Time the provider removed this edit's retained content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

/// Expected state and unresolved candidates after a source observation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldState {
    /// Highest observation visible at this point.
    pub source_observation_cutoff: u64,
    /// Expected semantic objects.
    pub objects: Vec<GoldObject>,
    /// Expected typed relationships between objects at this cutoff.
    #[serde(default)]
    pub relations: Vec<GoldRelation>,
}

/// One expected semantic object with separate state and evidence health.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldObject {
    /// Stable fixture-local name.
    pub key: String,
    /// Domain object kind.
    pub kind: String,
    /// Expected state for this object kind.
    pub lifecycle: GoldObjectState,
    /// Whether the object's original evidence is still sound.
    pub support_status: SupportStatus,
    /// Evidence with explicit role rather than an undifferentiated support list.
    pub evidence: Vec<GoldEvidence>,
    /// Task planning facets, present only for a task object.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_plan: Option<GoldTaskPlan>,
}

/// A Task's commitment, schedule, and execution at one cutoff.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldTaskPlan {
    /// Whether the work owner accepted the request.
    pub commitment: TaskCommitment,
    /// When the owner intends to revisit or start it.
    pub scheduling: TaskScheduling,
    /// How far work has progressed.
    pub execution: TaskExecution,
    /// Why the owner deferred the task, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deferral_reason: Option<String>,
    /// Earliest provider or project state that permits the task to start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_after: Option<GoldPredicate>,
    /// Date when the owner will reconsider a deferred task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_at: Option<String>,
}

/// Commitment to a known Task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCommitment {
    /// The owner has not accepted or declined the request.
    Pending,
    /// The owner has taken responsibility for the work.
    Accepted,
    /// The owner will not take the work.
    Declined,
}

/// Scheduling of a known Task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskScheduling {
    /// The owner has not placed the work on a schedule.
    Unscheduled,
    /// The owner will reconsider the work when its trigger arrives.
    Deferred,
    /// The owner has set a time to perform the work.
    Scheduled,
}

/// Execution of a known Task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecution {
    /// Nobody has started the work.
    NotStarted,
    /// Work is under way.
    InProgress,
    /// An accepted constraint prevents progress.
    Blocked,
    /// The work is done.
    Completed,
    /// The owner stopped the work.
    Cancelled,
}

/// State predicate that must hold before a Task can start.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldPredicate {
    /// Fully qualified provider or project object reference.
    pub subject: String,
    /// Field or state to check on that object.
    pub predicate: String,
    /// Truth value required before work may start.
    pub expected: bool,
}

/// Corpus state at a causal cutoff, including type-specific states such as open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldObjectState {
    /// Proposed but not yet accepted.
    Candidate,
    /// Unresolved question or work item.
    Open,
    /// Current object; a Task may still await owner commitment.
    Active,
    /// Answered question or completed work item.
    Resolved,
    /// Replaced by a later object.
    Superseded,
    /// Withdrawn as incorrect.
    Invalidated,
}

/// Health of the evidence supporting an object, independent of its state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportStatus {
    /// Evidence still supports the object.
    Current,
    /// An underlying source changed but has not yet been reconsidered.
    EvidenceChanged,
    /// Reconsideration is queued or running.
    RevalidationPending,
    /// Some, but not all, evidence remains supportive.
    PartiallySupported,
    /// No retained evidence currently supports the object.
    Unsupported,
}

/// How an observation bears on a gold object.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldEvidence {
    /// Source observation sequence.
    pub observation: u64,
    /// Supports, disputes, or motivates reconsideration of the object.
    pub role: EvidenceRole,
}

/// Typed evidence role.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    /// Supports the object.
    Supports,
    /// Disputes the object.
    Disputes,
    /// Changes its source evidence without yet changing its lifecycle.
    EvidenceChanged,
}

/// Expected relation between two gold objects.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldRelation {
    /// Fixture-local source object key.
    pub from: String,
    /// Typed semantic relation.
    pub kind: RelationKind,
    /// Fixture-local destination object key.
    pub to: String,
    /// Source observation establishing the relation.
    pub observation: u64,
}

/// Relations needed by the first Issue fixtures.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// A new object replaces an earlier object.
    Supersedes,
    /// An object supports another.
    Supports,
    /// An object disputes another.
    Disputes,
    /// A decision or fact answers a question.
    Answers,
}

/// A fixture violates a corpus invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// The fixture declares a schema this binary does not understand.
    UnsupportedSchema(String),
    /// Observation sequences must be contiguous and one-based.
    ObservationSequence {
        /// Sequence expected at this position.
        expected: u64,
        /// Sequence found in the fixture.
        actual: u64,
    },
    /// Capture metadata disagrees with the retained observation count.
    ObservationCount {
        /// Count declared by capture metadata.
        declared: usize,
        /// Count found in the fixture.
        actual: usize,
    },
    /// A version does not point to the preceding version of its entity.
    InvalidVersionLineage(u64),
    /// Exact bytes and their digest must appear together; gaps need a reason.
    InvalidBodyCapture(u64),
    /// Edit metadata does not describe its enclosing source version.
    InvalidContentEdit(u64),
    /// Terminal provider facts are incomplete or inconsistent.
    InvalidProviderSnapshot,
    /// A provenance timestamp is not RFC 3339.
    InvalidTimestamp(String),
    /// A source version is placed before its external entity existed.
    ObservationBeforeCreation(u64),
    /// A source version is placed after the fixture was captured.
    ObservationAfterCapture(u64),
    /// A recorded upstream update predates its source version or follows capture.
    InvalidSourceUpdate(u64),
    /// Two observations claim the same immutable version identity.
    DuplicateVersionIdentity(u64),
    /// A source version occurs before the observation preceding it.
    CausalOrder(u64),
    /// A natural capture claims exact ordering across a timestamp tie.
    InvalidOrderingClaim(u64),
    /// Exact causal replay crosses a timestamp tie with no established order.
    AmbiguousCausalOrder(u64),
    /// Exact historical replay crosses a missing source version.
    MissingHistoricalBody(u64),
    /// Capture digest disagrees with the normalized observations.
    SourceDigest {
        /// Digest declared by capture metadata.
        declared: String,
        /// Digest calculated during validation.
        actual: String,
    },
    /// A gold-state cutoff does not identify a retained observation.
    UnknownCutoff(u64),
    /// Two gold states claim the same causal cutoff.
    DuplicateGoldCutoff(u64),
    /// One gold state defines the same local object twice.
    DuplicateGoldObject(String),
    /// A task's gold planning facets are absent or inconsistent.
    InvalidTaskPlan(String),
    /// A gold object cites an observation that does not exist.
    UnknownSupport {
        /// Fixture-local object name.
        object: String,
        /// Missing observation sequence.
        support_observation: u64,
    },
    /// A gold-state label uses information that had not happened at its cutoff.
    FutureLeakage {
        /// Fixture-local object name.
        object: String,
        /// Gold-state source cutoff.
        cutoff: u64,
        /// Later observation cited by the object.
        support_observation: u64,
    },
    /// A relation names a gold object absent at its cutoff.
    UnknownRelationObject(String),
}

impl fmt::Display for ValidationError {
    #[expect(
        clippy::too_many_lines,
        reason = "each fixture error has one precise diagnostic"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => write!(formatter, "unsupported schema {schema}"),
            Self::ObservationSequence { expected, actual } => {
                write!(formatter, "expected observation {expected}, found {actual}")
            }
            Self::ObservationCount { declared, actual } => write!(
                formatter,
                "capture declares {declared} observations, fixture contains {actual}"
            ),
            Self::InvalidVersionLineage(sequence) => {
                write!(
                    formatter,
                    "invalid source-version lineage at observation {sequence}"
                )
            }
            Self::InvalidBodyCapture(sequence) => {
                write!(
                    formatter,
                    "inconsistent body capture at observation {sequence}"
                )
            }
            Self::InvalidContentEdit(sequence) => {
                write!(formatter, "invalid edit metadata at observation {sequence}")
            }
            Self::InvalidProviderSnapshot => {
                formatter.write_str("invalid terminal provider snapshot")
            }
            Self::InvalidTimestamp(value) => {
                write!(formatter, "invalid RFC 3339 timestamp {value}")
            }
            Self::ObservationBeforeCreation(sequence) => {
                write!(formatter, "observation {sequence} precedes source creation")
            }
            Self::ObservationAfterCapture(sequence) => {
                write!(formatter, "observation {sequence} follows fixture capture")
            }
            Self::InvalidSourceUpdate(sequence) => {
                write!(
                    formatter,
                    "source update for observation {sequence} is out of time"
                )
            }
            Self::DuplicateVersionIdentity(sequence) => {
                write!(
                    formatter,
                    "duplicate source version at observation {sequence}"
                )
            }
            Self::CausalOrder(sequence) => {
                write!(
                    formatter,
                    "source observation {sequence} is out of causal order"
                )
            }
            Self::InvalidOrderingClaim(sequence) => {
                write!(
                    formatter,
                    "invalid causal ordering claim at observation {sequence}"
                )
            }
            Self::AmbiguousCausalOrder(sequence) => {
                write!(
                    formatter,
                    "ambiguous causal order at observation {sequence}"
                )
            }
            Self::MissingHistoricalBody(sequence) => {
                write!(
                    formatter,
                    "exact replay crosses missing body at observation {sequence}"
                )
            }
            Self::SourceDigest { declared, actual } => {
                write!(
                    formatter,
                    "source digest is {declared}, calculated {actual}"
                )
            }
            Self::UnknownCutoff(cutoff) => {
                write!(formatter, "gold-state cutoff {cutoff} does not exist")
            }
            Self::DuplicateGoldCutoff(cutoff) => {
                write!(formatter, "duplicate gold-state cutoff {cutoff}")
            }
            Self::DuplicateGoldObject(key) => {
                write!(formatter, "duplicate gold object {key}")
            }
            Self::InvalidTaskPlan(key) => {
                write!(formatter, "invalid gold task plan for {key}")
            }
            Self::UnknownSupport {
                object,
                support_observation,
            } => write!(
                formatter,
                "gold object {object} cites missing observation {support_observation}"
            ),
            Self::FutureLeakage {
                object,
                cutoff,
                support_observation,
            } => write!(
                formatter,
                "gold object {object} at cutoff {cutoff} cites later observation {support_observation}"
            ),
            Self::UnknownRelationObject(key) => {
                write!(formatter, "gold relation names absent object {key}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// Calculates the digest stored in [`Capture::source_sha256`].
///
/// # Panics
///
/// Panics only if serializing the strongly typed observations fails, which
/// would indicate a programming error in their `Serialize` implementation.
#[must_use]
pub fn source_digest(provider_snapshot: &ProviderSnapshot, observations: &[Observation]) -> String {
    let bytes = serde_json::to_vec(&(provider_snapshot, observations))
        .expect("serializing corpus source material should be infallible");
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a string should be infallible");
    }
    encoded
}

/// Hashes exact source bytes without incorporating a mutable display field.
#[must_use]
pub fn body_digest(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a string should be infallible");
    }
    encoded
}

/// Checks whether every source version through a cutoff has exact bytes.
///
/// # Errors
///
/// Returns the first missing version. Capture gaps cannot be replayed as if
/// the terminal body had been available at an earlier cutoff.
pub fn require_exact_source_bodies_through(
    fixture: &Fixture,
    cutoff: u64,
) -> Result<(), ValidationError> {
    require_known_cutoff(fixture, cutoff)?;
    for observation in fixture
        .observations
        .iter()
        .filter(|item| item.sequence <= cutoff)
    {
        if observation.body.is_none()
            || body_availability(fixture, observation) != Some(BodyAvailability::AtObservation)
        {
            return Err(ValidationError::MissingHistoricalBody(observation.sequence));
        }
    }
    Ok(())
}

/// Returns retained bytes only when the fixture proves they were available at
/// the observation cutoff or at an explicitly requested terminal capture.
///
/// Older natural fixtures did not label individual bodies. For those, terminal
/// capture bytes stay hidden until the final observation unless the fixture
/// declares an exact observed or reconstructed history.
#[must_use]
pub fn available_body_at<'a>(
    fixture: &Fixture,
    observation: &'a Observation,
    cutoff: u64,
    capture_phase: bool,
) -> Option<&'a str> {
    let body = observation.body.as_deref()?;
    match body_availability(fixture, observation)? {
        BodyAvailability::AtObservation if observation.sequence <= cutoff => Some(body),
        BodyAvailability::AtCapture
            if capture_phase
                && cutoff == fixture.observations.last().map_or(0, |last| last.sequence) =>
        {
            Some(body)
        }
        BodyAvailability::AtObservation | BodyAvailability::AtCapture => None,
    }
}

/// Effective body-availability claim, including conservative legacy inference.
#[must_use]
pub fn body_availability(fixture: &Fixture, observation: &Observation) -> Option<BodyAvailability> {
    observation.body.as_ref()?;
    observation.body_availability.or({
        if matches!(fixture.origin, Origin::Controlled)
            || matches!(
                fixture.capture.history_fidelity,
                HistoryFidelity::ExactObserved
                    | HistoryFidelity::DiffReconstructable
                    | HistoryFidelity::StagedExact
            )
        {
            Some(BodyAvailability::AtObservation)
        } else {
            Some(BodyAvailability::AtCapture)
        }
    })
}

/// Refuses causal replay through a timestamp tie without established order.
///
/// # Errors
///
/// Returns the first ambiguous observation or an unknown cutoff. A cutoff just
/// before a tied observation is ambiguous too: that later-listed event may have
/// happened first. This checks ordering only; callers also need exact source
/// bytes and suitable fidelity.
pub fn require_unambiguous_order_through(
    fixture: &Fixture,
    cutoff: u64,
) -> Result<(), ValidationError> {
    require_known_cutoff(fixture, cutoff)?;
    for observation in fixture
        .observations
        .iter()
        .filter(|item| item.sequence <= cutoff)
    {
        if observation.ambiguous_order_with_previous {
            return Err(ValidationError::AmbiguousCausalOrder(observation.sequence));
        }
    }
    if let Some(next) = usize::try_from(cutoff)
        .ok()
        .and_then(|index| fixture.observations.get(index))
        && next.ambiguous_order_with_previous
    {
        return Err(ValidationError::AmbiguousCausalOrder(next.sequence));
    }
    Ok(())
}

fn require_known_cutoff(fixture: &Fixture, cutoff: u64) -> Result<(), ValidationError> {
    if cutoff == 0
        || usize::try_from(cutoff).map_or(true, |index| index > fixture.observations.len())
    {
        return Err(ValidationError::UnknownCutoff(cutoff));
    }
    Ok(())
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde skip_serializing_if requires a borrowed value"
)]
fn is_false(value: &bool) -> bool {
    !value
}

/// Checks the integrity and temporal invariants required by a fixture.
///
/// # Errors
///
/// Returns a [`ValidationError`] when capture metadata is inconsistent or a
/// gold-state label uses missing or future evidence.
#[expect(
    clippy::too_many_lines,
    reason = "fixture integrity is checked as one public boundary"
)]
pub fn validate(fixture: &Fixture) -> Result<(), ValidationError> {
    if fixture.schema != FIXTURE_SCHEMA_V1
        && fixture.schema != FIXTURE_SCHEMA_V2
        && fixture.schema != FIXTURE_SCHEMA_V3
    {
        return Err(ValidationError::UnsupportedSchema(fixture.schema.clone()));
    }

    let captured_at = parse_timestamp(&fixture.capture.captured_at)?;
    let snapshot = &fixture.provider_snapshot;
    if fixture.schema != FIXTURE_SCHEMA_V1
        && (snapshot
            .updated_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?
            .is_some_and(|time| time > captured_at)
            || snapshot
                .closed_at
                .as_deref()
                .map(parse_timestamp)
                .transpose()?
                .is_some_and(|time| time > captured_at)
            || (matches!(fixture.origin, Origin::Natural)
                && (snapshot.updated_at.is_none()
                    || snapshot.labels.len() != snapshot.label_refs.len()
                    || snapshot
                        .assignees
                        .iter()
                        .any(|actor| actor.provider_id.is_none()))))
    {
        return Err(ValidationError::InvalidProviderSnapshot);
    }
    let mut label_ids = HashSet::new();
    if fixture.schema != FIXTURE_SCHEMA_V1
        && (snapshot.label_refs.iter().any(|label| {
            label.provider_id.is_empty() || !label_ids.insert(label.provider_id.as_str())
        }) || snapshot
            .labels
            .iter()
            .zip(&snapshot.label_refs)
            .any(|(name, label)| name != &label.name))
    {
        return Err(ValidationError::InvalidProviderSnapshot);
    }
    let mut last_time = None;
    let mut version_ids = HashSet::new();
    for (expected, observation) in (1_u64..).zip(&fixture.observations) {
        if observation.sequence != expected {
            return Err(ValidationError::ObservationSequence {
                expected,
                actual: observation.sequence,
            });
        }
        let body_is_exact = observation.body.is_some();
        if body_is_exact != observation.body_sha256.is_some()
            || body_is_exact == observation.missing_body_reason.is_some()
            || (fixture.schema == FIXTURE_SCHEMA_V3
                && body_is_exact != observation.body_availability.is_some())
            || observation
                .body
                .as_ref()
                .zip(observation.body_sha256.as_ref())
                .is_some_and(|(body, digest)| body_digest(body) != *digest)
        {
            return Err(ValidationError::InvalidBodyCapture(observation.sequence));
        }
        let occurred_at = parse_timestamp(&observation.occurred_at)?;
        let created_at = parse_timestamp(&observation.created_at)?;
        let updated_at = observation
            .updated_at
            .as_deref()
            .map(parse_timestamp)
            .transpose()?;
        if occurred_at < created_at {
            return Err(ValidationError::ObservationBeforeCreation(
                observation.sequence,
            ));
        }
        if occurred_at > captured_at {
            return Err(ValidationError::ObservationAfterCapture(
                observation.sequence,
            ));
        }
        if updated_at.is_some_and(|updated| updated < occurred_at || updated > captured_at) {
            return Err(ValidationError::InvalidSourceUpdate(observation.sequence));
        }
        if let Some(edit) = &observation.edit {
            let edited_at = parse_timestamp(&edit.edited_at)?;
            if edit.provider_id != observation.version_id || edited_at != occurred_at {
                return Err(ValidationError::InvalidContentEdit(observation.sequence));
            }
            if let Some(deleted_at) = edit
                .deleted_at
                .as_deref()
                .map(parse_timestamp)
                .transpose()?
                && (deleted_at < edited_at || deleted_at > captured_at)
            {
                return Err(ValidationError::InvalidContentEdit(observation.sequence));
            }
        }
        if !version_ids.insert(observation.version_id.as_str()) {
            return Err(ValidationError::DuplicateVersionIdentity(
                observation.sequence,
            ));
        }
        if last_time.is_some_and(|time| occurred_at < time) {
            return Err(ValidationError::CausalOrder(observation.sequence));
        }
        let tied = last_time.is_some_and(|time| occurred_at == time);
        if observation.ambiguous_order_with_previous
            != (tied && matches!(fixture.origin, Origin::Natural))
        {
            return Err(ValidationError::InvalidOrderingClaim(observation.sequence));
        }
        last_time = Some(occurred_at);
    }
    let mut latest_by_entity = std::collections::HashMap::new();
    for observation in &fixture.observations {
        if latest_by_entity.insert(&observation.provider_id, observation.sequence)
            != observation.supersedes
        {
            return Err(ValidationError::InvalidVersionLineage(observation.sequence));
        }
    }

    if fixture.capture.provider_observation_count != fixture.observations.len() {
        return Err(ValidationError::ObservationCount {
            declared: fixture.capture.provider_observation_count,
            actual: fixture.observations.len(),
        });
    }

    let actual_digest = source_digest(&fixture.provider_snapshot, &fixture.observations);
    if fixture.capture.source_sha256 != actual_digest {
        return Err(ValidationError::SourceDigest {
            declared: fixture.capture.source_sha256.clone(),
            actual: actual_digest,
        });
    }

    let last_observation = fixture.observations.last().map_or(0, |item| item.sequence);
    let mut gold_cutoffs = HashSet::new();
    for state in &fixture.gold_states {
        if state.source_observation_cutoff == 0
            || state.source_observation_cutoff > last_observation
        {
            return Err(ValidationError::UnknownCutoff(
                state.source_observation_cutoff,
            ));
        }
        if !gold_cutoffs.insert(state.source_observation_cutoff) {
            return Err(ValidationError::DuplicateGoldCutoff(
                state.source_observation_cutoff,
            ));
        }

        let mut keys = HashSet::new();
        for object in &state.objects {
            if !keys.insert(object.key.as_str()) {
                return Err(ValidationError::DuplicateGoldObject(object.key.clone()));
            }
            if (object.kind == "task") != object.task_plan.is_some() {
                return Err(ValidationError::InvalidTaskPlan(object.key.clone()));
            }
            if let Some(plan) = &object.task_plan {
                let has_reason = plan
                    .deferral_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty());
                let has_condition = plan.start_after.as_ref().is_some_and(|condition| {
                    !condition.subject.trim().is_empty() && !condition.predicate.trim().is_empty()
                });
                let has_review = plan.review_at.as_deref().is_some();
                if let Some(review_at) = &plan.review_at {
                    parse_timestamp(review_at)?;
                }
                if plan.start_after.is_some() && !has_condition
                    || plan.scheduling == TaskScheduling::Deferred
                        && (!has_reason || !(has_condition || has_review))
                {
                    return Err(ValidationError::InvalidTaskPlan(object.key.clone()));
                }
            }
            for support_observation in object.evidence.iter().map(|item| &item.observation) {
                if *support_observation == 0 || *support_observation > last_observation {
                    return Err(ValidationError::UnknownSupport {
                        object: object.key.clone(),
                        support_observation: *support_observation,
                    });
                }
                if *support_observation > state.source_observation_cutoff {
                    return Err(ValidationError::FutureLeakage {
                        object: object.key.clone(),
                        cutoff: state.source_observation_cutoff,
                        support_observation: *support_observation,
                    });
                }
            }
        }
        for relation in &state.relations {
            for key in [&relation.from, &relation.to] {
                if !keys.contains(key.as_str()) {
                    return Err(ValidationError::UnknownRelationObject(key.clone()));
                }
            }
            if relation.observation == 0 || relation.observation > last_observation {
                return Err(ValidationError::UnknownSupport {
                    object: relation.from.clone(),
                    support_observation: relation.observation,
                });
            }
            if relation.observation > state.source_observation_cutoff {
                return Err(ValidationError::FutureLeakage {
                    object: relation.from.clone(),
                    cutoff: state.source_observation_cutoff,
                    support_observation: relation.observation,
                });
            }
        }
    }

    Ok(())
}

fn parse_timestamp(value: &str) -> Result<OffsetDateTime, ValidationError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| ValidationError::InvalidTimestamp(value.to_owned()))
}
