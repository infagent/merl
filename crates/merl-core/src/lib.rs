//! Structural identities and accepted changes shared by Merl adapters.

use std::{error::Error, fmt};

/// A structural identifier rejected before it reaches persistent history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidIdentifier;

impl fmt::Display for InvalidIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("identifier must be 1–128 ASCII letters, digits, '_' or '-'")
    }
}

impl Error for InvalidIdentifier {}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

macro_rules! identifier {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl $name {
            /// Returns the validated identifier used in structural records.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<&str> for $name {
            type Error = InvalidIdentifier;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                if valid_identifier(value) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(InvalidIdentifier)
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(
    ProjectId,
    "The stable identity of one accepted project history."
);
identifier!(
    ActorId,
    "The stable identity of the actor responsible for a batch."
);
identifier!(BatchId, "The idempotency identity of one accepted batch.");
identifier!(EventId, "The stable identity of one accepted domain event.");
identifier!(ObjectId, "The stable identity of one materialized object.");
identifier!(
    ObjectKind,
    "A bounded object classification, such as `decision`."
);
identifier!(
    PayloadId,
    "A reference to independently erasable protected bytes."
);
identifier!(RelationId, "The stable identity of one accepted relation.");
identifier!(
    RelationKind,
    "A bounded relation predicate, such as `supersedes`."
);
identifier!(
    SourceId,
    "The stable identity of one captured source entity."
);
identifier!(
    SourceVersionId,
    "The stable identity of one immutable source version."
);
identifier!(
    SourceBindingId,
    "The project-specific identity of one attached source namespace."
);
identifier!(
    SourceKind,
    "A bounded source classification, such as `issue_comment`."
);
identifier!(SourceProvider, "A bounded provider name, such as `github`.");
identifier!(
    CapturePolicyVersion,
    "The version of the binding policy effective at source capture."
);

/// Whether capture schedules semantic extraction from prose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompilationMode {
    /// Keep the source cold until an authorized later action selects it.
    CaptureOnly,
    /// Compile only after an explicit authorized request.
    OnDemand,
    /// Schedule compilation after capture.
    Eager,
}

impl CompilationMode {
    /// Returns the bounded value stored with an immutable source capture.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CaptureOnly => "capture_only",
            Self::OnDemand => "on_demand",
            Self::Eager => "eager",
        }
    }
}

impl TryFrom<&str> for CompilationMode {
    type Error = InvalidIdentifier;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "capture_only" => Ok(Self::CaptureOnly),
            "on_demand" => Ok(Self::OnDemand),
            "eager" => Ok(Self::Eager),
            _ => Err(InvalidIdentifier),
        }
    }
}

/// Whether an unprocessed source leaves a semantic-coverage gap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageRequirement {
    /// Processing is needed before claiming semantic completeness.
    Required,
    /// Retain the source without weakening completeness.
    Optional,
}

impl CoverageRequirement {
    /// Returns the bounded value stored with an immutable source capture.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
        }
    }
}

impl TryFrom<&str> for CoverageRequirement {
    type Error = InvalidIdentifier;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "required" => Ok(Self::Required),
            "optional" => Ok(Self::Optional),
            _ => Err(InvalidIdentifier),
        }
    }
}
identifier!(PolicyInputId, "The identity of one immutable policy input.");
identifier!(
    CompilationRunId,
    "The stable identity of one compiler attempt."
);
identifier!(PolicyEvaluationId, "The identity of one policy decision.");
identifier!(
    PolicyVersion,
    "The bounded version of authority rules used for evaluation."
);
identifier!(AgentId, "The stable identity of one inbox subscriber.");
identifier!(
    ReasonCode,
    "A bounded explanation code for one policy disposition."
);

/// A project-scoped link between two accepted objects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Relation {
    /// Stable identity of this link.
    pub id: RelationId,
    /// Project that owns both endpoints.
    pub project: ProjectId,
    /// Object the predicate starts from.
    pub subject: ObjectId,
    /// Bounded predicate, such as `supersedes` or `supports`.
    pub kind: RelationKind,
    /// Object the predicate points to.
    pub object: ObjectId,
}

/// A precise captured source version, independent of its protected bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRef {
    /// Stable source entity identity.
    pub source: SourceId,
    /// The source version actually observed or cited.
    pub version: SourceVersionId,
}

/// The kind and identity of a semantic proposal presented to policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyInput {
    /// Meaning extracted from a captured source by a compiler.
    ObservedAssertion {
        /// Stable identity when policy sees this assertion.
        id: PolicyInputId,
        /// Compiler run that recorded the original assertion.
        run: CompilationRunId,
        /// Position of the assertion in its immutable compiler result.
        index: u32,
    },
    /// A structured action submitted by an authenticated actor.
    Command(PolicyInputId),
    /// A provider-owned fact checked through a deterministic policy path.
    ProviderObservation(PolicyInputId),
    /// An authorized maintenance action, including future purge operations.
    AdministrativeAction(PolicyInputId),
}

impl PolicyInput {
    /// Returns the immutable input identity without changing its authority.
    #[must_use]
    pub const fn id(&self) -> &PolicyInputId {
        match self {
            Self::ObservedAssertion { id, .. }
            | Self::Command(id)
            | Self::ProviderObservation(id)
            | Self::AdministrativeAction(id) => id,
        }
    }

    /// Returns a bounded kind used to distinguish equal IDs from different inputs.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ObservedAssertion { .. } => "observed_assertion",
            Self::Command(_) => "command",
            Self::ProviderObservation(_) => "provider_observation",
            Self::AdministrativeAction(_) => "administrative_action",
        }
    }
}

/// One input's independently recorded policy outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDisposition {
    /// Its proposed change may enter the accepted batch.
    Accepted,
    /// It remains visible for review but does not change accepted state.
    Candidate,
    /// The authority will not accept this proposal.
    Rejected,
    /// Its semantic origin has already been handled.
    Duplicate,
    /// Current accepted state conflicts with this proposal.
    Conflict,
}

impl PolicyDisposition {
    /// Returns the bounded storage value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Candidate => "candidate",
            Self::Rejected => "rejected",
            Self::Duplicate => "duplicate",
            Self::Conflict => "conflict",
        }
    }
}

/// An input decision retains its immutable identity and a digest of its submitted meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyInputDecision {
    /// The input's typed identity.
    pub input: PolicyInput,
    /// Digest of its immutable command, assertion, observation, or action.
    pub input_digest: [u8; 32],
    /// Independent disposition for this input.
    pub disposition: PolicyDisposition,
    /// Bounded audit reason; longer notes belong in a protected payload.
    pub reason: ReasonCode,
}

/// Semantic permission an administrator can grant within one project.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorityPermission {
    /// The source author may make explicit decisions under assertion policy.
    DecisionAuthor,
    /// The actor may submit structured semantic commands.
    CommandActor,
}

impl AuthorityPermission {
    /// Stable value used by the CLI and stored grant records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DecisionAuthor => "decision_author",
            Self::CommandActor => "command_actor",
        }
    }
}

impl TryFrom<&str> for AuthorityPermission {
    type Error = InvalidAuthorityPermission;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "decision_author" => Ok(Self::DecisionAuthor),
            "command_actor" => Ok(Self::CommandActor),
            _ => Err(InvalidAuthorityPermission),
        }
    }
}

/// The requested permission is outside the first-release semantic grant set.
#[derive(Debug)]
pub struct InvalidAuthorityPermission;

impl std::fmt::Display for InvalidAuthorityPermission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("permission must be decision_author or command_actor")
    }
}

impl std::error::Error for InvalidAuthorityPermission {}

/// Accepted-state dependency read while policy made its decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyRead {
    /// A named object had this revision, or was absent.
    Object {
        /// Object looked up by policy.
        id: ObjectId,
        /// `None` also guards an absence-based decision against insertion.
        revision: Option<ObjectRevision>,
    },
    /// No object of this kind changed after the recorded revision.
    KindCollection {
        /// Classification whose members affected policy.
        kind: ObjectKind,
        /// Latest matching object's project revision when evaluated.
        latest_project_revision: ProjectRevision,
    },
}

/// Accepted target policy proposes to change after checking concurrent writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyWrite {
    /// Object creation or replacement.
    Object {
        /// Target object.
        object: ObjectId,
        /// Revision visible when policy evaluated, or `None` for creation.
        expected_revision: Option<ObjectRevision>,
    },
    /// Relation creation or replacement.
    Relation {
        /// Target relation.
        relation: RelationId,
        /// Revision visible when policy evaluated, or `None` for creation.
        expected_revision: Option<ObjectRevision>,
    },
}

/// One accepted input's exact contribution to an event in its batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyEventOrigin {
    /// Position of the accepted input in the evaluation.
    pub input_index: u32,
    /// Event produced by that input.
    pub event: EventId,
}

/// Auditable decision prepared against accepted state before the final transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyEvaluation {
    /// Stable retry identity.
    pub id: PolicyEvaluationId,
    /// Project whose authority owns this decision.
    pub project: ProjectId,
    /// Authenticated evaluator, not a model-attributed actor.
    pub actor: ActorId,
    /// Ruleset that produced this decision.
    pub version: PolicyVersion,
    /// Digest of the exact authority configuration used with that ruleset.
    pub configuration_digest: [u8; 32],
    /// Accepted state against which the rules ran.
    pub basis_project_revision: ProjectRevision,
    /// Typed inputs and their separate dispositions.
    pub inputs: Vec<PolicyInputDecision>,
    /// State whose value or absence affected the decision.
    pub reads: Vec<PolicyRead>,
    /// Targets proposed for accepted mutation.
    pub writes: Vec<PolicyWrite>,
    /// Exact input-to-event links for accepted changes.
    pub event_origins: Vec<PolicyEventOrigin>,
    /// Accepted events, if this evaluation changes project state.
    pub batch: Option<DomainEventBatch>,
}

/// Provider-owned Issue state observed through an attached source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderIssueState {
    /// The provider reports the Issue as open.
    Open,
    /// The provider reports the Issue as closed.
    Closed,
}

impl ProviderIssueState {
    /// Returns the bounded value used in structural provider records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}

impl TryFrom<&str> for ProviderIssueState {
    type Error = InvalidIdentifier;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            _ => Err(InvalidIdentifier),
        }
    }
}

/// A typed provider fact proposed for deterministic acceptance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderObservation {
    /// Stable identity of the policy input.
    pub id: PolicyInputId,
    /// Project binding through which the provider fact arrived.
    pub binding: SourceBindingId,
    /// Provider-backed Issue mirror affected by the observation.
    pub issue: ObjectId,
    /// Provider-owned open or closed state.
    pub state: ProviderIssueState,
    /// Provider update time, independent of the local poll time.
    pub upstream_updated_at_millis: Option<i64>,
    /// Provider close time, if the Issue is closed.
    pub closed_at_millis: Option<i64>,
    /// Stable provider node IDs of current labels, or `None` for an older capture without IDs.
    pub label_provider_ids: Option<Vec<String>>,
    /// Stable provider node IDs of current assignees, or `None` when identity was unavailable.
    pub assignee_provider_ids: Option<Vec<String>>,
    /// Protected snapshot payload containing display names and prose.
    pub snapshot_payload: PayloadId,
    /// Time the authority observed this provider snapshot.
    pub observed_at_millis: i64,
}

/// Monotonic sequence of accepted batches within one project.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectRevision(u64);

impl ProjectRevision {
    /// Returns revision zero for a project without accepted batches.
    #[must_use]
    pub const fn initial() -> Self {
        Self(0)
    }

    /// Returns the numeric revision for storage and machine output.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advances exactly one accepted batch without wrapping the sequence.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl From<u64> for ProjectRevision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A zero object revision cannot represent an accepted object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidObjectRevision;

impl fmt::Display for InvalidObjectRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("object revision must be positive")
    }
}

impl Error for InvalidObjectRevision {}

/// Monotonic revision of one materialized object.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ObjectRevision(u64);

impl ObjectRevision {
    /// Returns the first accepted version of an object.
    #[must_use]
    pub const fn initial() -> Self {
        Self(1)
    }

    /// Returns the numeric revision for storage and machine output.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advances after another accepted change without wrapping the sequence.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl TryFrom<u64> for ObjectRevision {
    type Error = InvalidObjectRevision;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(InvalidObjectRevision)
        } else {
            Ok(Self(value))
        }
    }
}

/// A structural accepted change; prose remains behind a payload reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainEvent {
    /// Resolve reviewed evidence support without changing semantic content or lifecycle.
    ResolveSupport {
        /// Stable accepted-effect identity.
        id: EventId,
        /// Existing object whose support is affected.
        object: ObjectId,
        /// Immutable review containing the impact, outcome, and optional assertion.
        review: PolicyInputId,
    },
    /// Create or replace the current representation of an object.
    PutObject {
        /// Stable event identity.
        id: EventId,
        /// Object affected by this event.
        object: ObjectId,
        /// Bounded object classification.
        kind: ObjectKind,
        /// Protected content, if this object has any.
        payload: Option<PayloadId>,
        /// Source conversation this semantic object belongs to, when known.
        issue_scope: Option<String>,
        /// Accepted lifecycle, independent of evidence health.
        lifecycle: ObjectLifecycle,
    },
    /// Create or replace one structural edge between accepted objects.
    PutRelation {
        /// Stable event identity.
        id: EventId,
        /// Relation owned by the same project as the batch.
        relation: Relation,
    },
}

/// Accepted status of an object; source edits affect support separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectLifecycle {
    /// The object remains part of current accepted state.
    Active,
    /// A later accepted object superseded this one.
    Superseded,
    /// The project explicitly invalidated this object.
    Invalidated,
}

impl ObjectLifecycle {
    /// Stable structural value used in accepted event records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Invalidated => "invalidated",
        }
    }
}

impl TryFrom<&str> for ObjectLifecycle {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "active" => Ok(Self::Active),
            "superseded" => Ok(Self::Superseded),
            "invalidated" => Ok(Self::Invalidated),
            _ => Err(()),
        }
    }
}

/// Events accepted together under exactly one project revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEventBatch {
    /// Stable batch identity for retries and provenance.
    pub id: BatchId,
    /// Project whose authority accepted the batch.
    pub project: ProjectId,
    /// Actor responsible for the accepted transition.
    pub actor: ActorId,
    /// Caller-supplied UTC Unix milliseconds; tests may use a fixed value.
    pub occurred_at_millis: i64,
    /// Ordered events to commit atomically.
    pub events: Vec<DomainEvent>,
}

#[cfg(test)]
mod tests {
    use super::{ObjectKind, ObjectRevision, ProjectRevision};

    #[test]
    fn structural_identifiers_cannot_carry_prose_or_wrap_revisions() {
        for invalid in ["", "decision with spaces", "source/body", "a\nsecret"] {
            assert!(ObjectKind::try_from(invalid).is_err());
        }
        assert!(ObjectRevision::try_from(0).is_err());
        assert!(ProjectRevision::from(u64::MAX).next().is_none());
    }
}
