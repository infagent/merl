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
    ObservedAssertion(PolicyInputId),
    /// A structured action submitted by an authenticated actor.
    Command(PolicyInputId),
    /// A provider-owned fact checked through a deterministic policy path.
    ProviderObservation(PolicyInputId),
    /// An authorized maintenance action, including future purge operations.
    AdministrativeAction(PolicyInputId),
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
    /// Protected snapshot payload containing title, labels, and assignees.
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
    },
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
