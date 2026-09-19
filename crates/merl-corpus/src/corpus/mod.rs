//! Frozen evaluation histories and their temporal gold-state labels.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Schema understood by this version of the corpus tooling.
pub const FIXTURE_SCHEMA: &str = "merl.corpus-fixture/v1";

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
    /// Provider transitions retained for questions that depend on them.
    #[serde(default)]
    pub provider_events: Vec<ProviderEvent>,
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
    /// Current Issue title.
    pub title: String,
    /// Provider state, such as `OPEN` or `CLOSED`.
    pub state: String,
    /// Time the provider closed the Issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    /// Current provider labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Current provider assignees.
    #[serde(default)]
    pub assignees: Vec<String>,
    /// Current provider milestone title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
}

/// One provider transition retained outside prose compilation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderEvent {
    /// Stable provider event ID.
    pub provider_id: String,
    /// Provider event kind.
    pub kind: String,
    /// Time the transition occurred.
    pub occurred_at: String,
    /// Actor recorded by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Provider-specific structural values needed to interpret the event.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, serde_json::Value>,
}

/// One source observation in provider order.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Observation {
    /// One-based order assigned when the fixture was captured.
    pub sequence: u64,
    /// Kind of provider content.
    pub kind: ObservationKind,
    /// Provider-stable source identifier.
    pub provider_id: String,
    /// Author login recorded by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Provider creation time.
    pub created_at: String,
    /// Provider update time.
    pub updated_at: String,
    /// Current source text at capture.
    pub body: String,
    /// Provider edit records available at capture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<ContentEdit>,
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

/// Provider metadata for an edit to user-authored content.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ContentEdit {
    /// Provider-stable edit ID.
    pub provider_id: String,
    /// Actor who made the edit when the provider exposes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
    /// Time of the edit.
    pub edited_at: String,
    /// Provider diff. A diff does not by itself prove the exact prior body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    /// Time the provider removed this edit's retained content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

/// Expected accepted state after a source observation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldState {
    /// Highest observation visible at this point.
    pub source_observation_cutoff: u64,
    /// Expected semantic objects.
    pub objects: Vec<GoldObject>,
}

/// One expected semantic object and the observations that support it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoldObject {
    /// Stable fixture-local name.
    pub key: String,
    /// Domain object kind.
    pub kind: String,
    /// Expected lifecycle state.
    pub state: String,
    /// Observations used to justify this expectation.
    pub support_observations: Vec<u64>,
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
    /// Capture digest disagrees with the normalized observations.
    SourceDigest {
        /// Digest declared by capture metadata.
        declared: String,
        /// Digest calculated during validation.
        actual: String,
    },
    /// A gold-state cutoff does not identify a retained observation.
    UnknownCutoff(u64),
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
}

impl fmt::Display for ValidationError {
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
            Self::SourceDigest { declared, actual } => {
                write!(
                    formatter,
                    "source digest is {declared}, calculated {actual}"
                )
            }
            Self::UnknownCutoff(cutoff) => {
                write!(formatter, "gold-state cutoff {cutoff} does not exist")
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
pub fn source_digest(
    provider_snapshot: &ProviderSnapshot,
    observations: &[Observation],
    provider_events: &[ProviderEvent],
) -> String {
    let bytes = serde_json::to_vec(&(provider_snapshot, observations, provider_events))
        .expect("serializing corpus source material should be infallible");
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(71);
    encoded.push_str("sha256:");
    for byte in digest {
        write!(encoded, "{byte:02x}").expect("writing to a string should be infallible");
    }
    encoded
}

/// Checks the integrity and temporal invariants required by a fixture.
///
/// # Errors
///
/// Returns a [`ValidationError`] when capture metadata is inconsistent or a
/// gold-state label uses missing or future evidence.
pub fn validate(fixture: &Fixture) -> Result<(), ValidationError> {
    if fixture.schema != FIXTURE_SCHEMA {
        return Err(ValidationError::UnsupportedSchema(fixture.schema.clone()));
    }

    for (expected, observation) in (1_u64..).zip(&fixture.observations) {
        if observation.sequence != expected {
            return Err(ValidationError::ObservationSequence {
                expected,
                actual: observation.sequence,
            });
        }
    }

    if fixture.capture.provider_observation_count != fixture.observations.len() {
        return Err(ValidationError::ObservationCount {
            declared: fixture.capture.provider_observation_count,
            actual: fixture.observations.len(),
        });
    }

    let actual_digest = source_digest(
        &fixture.provider_snapshot,
        &fixture.observations,
        &fixture.provider_events,
    );
    if fixture.capture.source_sha256 != actual_digest {
        return Err(ValidationError::SourceDigest {
            declared: fixture.capture.source_sha256.clone(),
            actual: actual_digest,
        });
    }

    let last_observation = fixture.observations.last().map_or(0, |item| item.sequence);
    for state in &fixture.gold_states {
        if state.source_observation_cutoff == 0
            || state.source_observation_cutoff > last_observation
        {
            return Err(ValidationError::UnknownCutoff(
                state.source_observation_cutoff,
            ));
        }

        for object in &state.objects {
            for support_observation in &object.support_observations {
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
    }

    Ok(())
}
