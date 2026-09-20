//! Offline Issue-history ingestion into a project's source log.

use std::{error::Error, fmt, fmt::Write as _};

use merl_core::{
    ActorId, BatchId, CapturePolicyVersion, CompilationMode, CoverageRequirement, DomainEvent,
    DomainEventBatch, EventId, ObjectId, ObjectKind, PayloadId, PolicyEvaluationId, PolicyInputId,
    PolicyVersion, ProjectId, ProviderIssueState, ProviderObservation, SourceBindingId, SourceId,
    SourceKind, SourceProvider, SourceVersionId,
};
use merl_corpus::fixture::{
    Fixture, MissingBodyReason, ObservationKind, ValidationError, validate,
};
use merl_policy::{PolicyError, PolicyRules, Proposal, evaluate};
use merl_store::{MissingSourceBody, PayloadRead, SourceBinding, SourceCapture, Store, StoreError};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Failure to import a captured Issue history.
#[derive(Debug)]
pub enum ImportError {
    /// The fixture violates its versioned corpus contract.
    InvalidFixture(ValidationError),
    /// A provider identifier cannot be represented structurally.
    InvalidIdentity,
    /// A timestamp cannot be normalized into UTC milliseconds.
    InvalidTimestamp,
    /// The provider snapshot does not contain a known Issue state.
    InvalidProviderState,
    /// A typed provider snapshot could not be encoded behind a payload reference.
    Serialization(String),
    /// The local project authority rejected or could not store a capture.
    Store(StoreError),
    /// A provider fact did not pass the deterministic policy boundary.
    Policy(PolicyError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFixture(error) => write!(formatter, "invalid fixture: {error}"),
            Self::InvalidIdentity => formatter.write_str("invalid provider identity"),
            Self::InvalidTimestamp => formatter.write_str("invalid provider timestamp"),
            Self::InvalidProviderState => formatter.write_str("invalid provider Issue state"),
            Self::Serialization(error) => write!(formatter, "snapshot encoding failed: {error}"),
            Self::Store(error) => write!(formatter, "{error}"),
            Self::Policy(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for ImportError {}

impl From<StoreError> for ImportError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<PolicyError> for ImportError {
    fn from(error: PolicyError) -> Self {
        Self::Policy(error)
    }
}

impl From<ValidationError> for ImportError {
    fn from(error: ValidationError) -> Self {
        Self::InvalidFixture(error)
    }
}

/// Number of new source versions captured during one fixture import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportReport {
    /// Versions not seen before by this project.
    pub captured: u64,
    /// Observation head after import, including earlier imports.
    pub observation_head: u64,
    /// Accepted project revision after a deterministic terminal provider observation.
    pub accepted_revision: u64,
    /// Whether the provider mirror changed during this import.
    pub provider_changed: bool,
}

/// The fixed capture policy used by the offline first-release fixture importer.
/// General source-binding policy selection belongs to the compiler slice.
pub const FIXTURE_CAPTURE_POLICY_VERSION: &str = "fixture_import_v1";

/// Imports source versions in fixture order without treating their prose as accepted state.
///
/// The corpus fixture carries a terminal provider snapshot, not a historical
/// provider-transition stream. This function captures its source versions
/// only; it does not invent earlier Issue open/closed or label transitions.
/// Each captured version is eager and coverage-required under
/// `fixture_import_v1`. This fixed offline policy is not a default for other
/// source bindings.
///
/// # Errors
/// Returns an error if the fixture is invalid or a version conflicts with
/// previously captured source history. A successful prefix may be retained
/// after a later error; retrying the same fixture is idempotent.
pub fn import_fixture(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
) -> Result<ImportReport, ImportError> {
    import_fixture_inner(store, project, fixture, false)
}

/// Captures a historical fixture without accepting its terminal provider snapshot.
///
/// A replay runner can then compile each observation in order against the
/// accepted state built from earlier observations. Use a fresh project: a
/// terminal snapshot or unrelated accepted event would leak future state.
///
/// # Errors
/// Rejects invalid fixtures or conflicting source versions.
pub fn import_fixture_for_causal_replay(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
) -> Result<ImportReport, ImportError> {
    if store.project_revision(project)?.get() != 0 || store.source_observation_head(project)? != 0 {
        return Err(ImportError::Store(StoreError::InvalidCompilation));
    }
    import_fixture_inner(store, project, fixture, true)
}

fn import_fixture_inner(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
    historical: bool,
) -> Result<ImportReport, ImportError> {
    validate(fixture)?;
    let binding = fixture_binding(fixture)?;
    let observed_at_millis = utc_millis(&fixture.capture.captured_at)?;
    let policy_version = CapturePolicyVersion::try_from(FIXTURE_CAPTURE_POLICY_VERSION)
        .map_err(|_| ImportError::InvalidIdentity)?;
    let mut captured = 0;
    for observation in &fixture.observations {
        let source = SourceId::try_from(
            digest_id(
                "so",
                &format!("{}:{}", binding.provider, observation.provider_id),
            )
            .as_str(),
        )
        .map_err(|_| ImportError::InvalidIdentity)?;
        let version = fixture_version_id(&observation.version_id)?;
        let supersedes = observation
            .supersedes
            .map(|sequence| {
                fixture
                    .observations
                    .get(usize::try_from(sequence - 1).map_err(|_| ImportError::InvalidIdentity)?)
                    .ok_or(ImportError::InvalidIdentity)
                    .and_then(|prior| fixture_version_id(&prior.version_id))
            })
            .transpose()?;
        let actor_ref = observation
            .edit
            .as_ref()
            .map_or(observation.author.as_ref(), |edit| edit.editor.as_ref());
        let provider_actor_id = actor_ref.and_then(|actor| actor.provider_id.as_deref());
        let actor = actor_identity(provider_actor_id)?;
        let provider_source_author_id = observation
            .author
            .as_ref()
            .and_then(|author| author.provider_id.as_deref());
        let source_author = actor_identity(provider_source_author_id)?;
        let kind = match observation.kind {
            ObservationKind::Issue => "issue",
            ObservationKind::IssueComment => "issue_comment",
            ObservationKind::Controlled => "controlled",
        };
        let capture = SourceCapture {
            binding: binding.clone(),
            source,
            provider_entity_id: &observation.provider_id,
            context_scope_id: &fixture.source.issue_provider_id,
            version,
            provider_version_id: &observation.version_id,
            kind: SourceKind::try_from(kind).map_err(|_| ImportError::InvalidIdentity)?,
            supersedes,
            ambiguous_order_with_previous: observation.ambiguous_order_with_previous,
            created_at_millis: utc_millis(&observation.created_at)?,
            occurred_at_millis: utc_millis(&observation.occurred_at)?,
            upstream_updated_at_millis: observation
                .updated_at
                .as_deref()
                .map(utc_millis)
                .transpose()?,
            observed_at_millis,
            actor,
            provider_actor_id,
            source_author,
            provider_source_author_id,
            body: observation.body.as_deref().map(str::as_bytes),
            edit_diff: observation
                .edit
                .as_ref()
                .and_then(|edit| edit.diff.as_deref())
                .map(str::as_bytes),
            edit_deleted_at_millis: observation
                .edit
                .as_ref()
                .and_then(|edit| edit.deleted_at.as_deref())
                .map(utc_millis)
                .transpose()?,
            missing_body_reason: observation.missing_body_reason.map(missing_body_reason),
            compilation_mode: CompilationMode::Eager,
            coverage_requirement: CoverageRequirement::Required,
            policy_version: policy_version.clone(),
        };
        captured += u64::from(if historical {
            store.capture_historical_source_version(project, &capture)?
        } else {
            store.capture_source_version(project, &capture)?
        });
    }
    let provider_changed = if historical {
        false
    } else {
        observe_terminal_issue_snapshot(store, project, fixture, &binding, observed_at_millis)?
    };
    Ok(ImportReport {
        captured,
        observation_head: store.source_observation_head(project)?,
        accepted_revision: store.project_revision(project)?.get(),
        provider_changed,
    })
}

fn actor_identity(provider_id: Option<&str>) -> Result<Option<ActorId>, ImportError> {
    provider_id
        .map(|id| {
            ActorId::try_from(digest_id("actor", id).as_str())
                .map_err(|_| ImportError::InvalidIdentity)
        })
        .transpose()
}

fn fixture_binding(fixture: &Fixture) -> Result<SourceBinding, ImportError> {
    let provider = SourceProvider::try_from(fixture.source.provider.as_str())
        .map_err(|_| ImportError::InvalidIdentity)?;
    let namespace = fixture
        .source
        .repository_provider_id
        .as_deref()
        .unwrap_or(&fixture.source.issue_provider_id);
    Ok(SourceBinding {
        id: SourceBindingId::try_from(digest_id("sb", &format!("{provider}:{namespace}")).as_str())
            .map_err(|_| ImportError::InvalidIdentity)?,
        provider,
        provider_namespace_id: namespace.to_owned(),
        namespace_digest: Sha256::digest(namespace.as_bytes()).into(),
    })
}

fn missing_body_reason(reason: MissingBodyReason) -> MissingSourceBody {
    match reason {
        MissingBodyReason::PriorVersionUnavailable => MissingSourceBody::PriorVersionUnavailable,
        MissingBodyReason::DeletedByProvider => MissingSourceBody::DeletedByProvider,
    }
}

/// Returns a rename-stable Issue mirror identity for one fixture.
///
/// # Errors
/// Returns an error only if a derived structural ID is invalid.
pub fn fixture_issue_id(fixture: &Fixture) -> Result<ObjectId, ImportError> {
    ObjectId::try_from(
        digest_id(
            "pi",
            &format!(
                "{}:{}",
                fixture.source.provider, fixture.source.issue_provider_id
            ),
        )
        .as_str(),
    )
    .map_err(|_| ImportError::InvalidIdentity)
}

/// Converts a stable fixture edit/version identity to a bounded Merl ID.
///
/// # Errors
/// Returns an error only if a derived ID violates Merl's structural format.
pub fn fixture_version_id(provider_version_id: &str) -> Result<SourceVersionId, ImportError> {
    SourceVersionId::try_from(digest_id("sv", provider_version_id).as_str())
        .map_err(|_| ImportError::InvalidIdentity)
}

fn digest_id(prefix: &str, value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut id = String::with_capacity(prefix.len() + 65);
    id.push_str(prefix);
    id.push('_');
    for byte in digest {
        write!(id, "{byte:02x}").expect("writing a digest to a string is infallible");
    }
    id
}

fn utc_millis(value: &str) -> Result<i64, ImportError> {
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ImportError::InvalidTimestamp)?;
    i64::try_from(parsed.unix_timestamp_nanos() / 1_000_000)
        .map_err(|_| ImportError::InvalidTimestamp)
}

fn observe_terminal_issue_snapshot(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
    binding: &SourceBinding,
    captured_at_millis: i64,
) -> Result<bool, ImportError> {
    let state = match fixture.provider_snapshot.state.as_str() {
        "OPEN" => ProviderIssueState::Open,
        "CLOSED" => ProviderIssueState::Closed,
        _ => return Err(ImportError::InvalidProviderState),
    };
    let issue = fixture_issue_id(fixture)?;
    let bytes = serde_json::to_vec(&fixture.provider_snapshot)
        .map_err(|error| ImportError::Serialization(error.to_string()))?;
    if let Some(current) = store.object(project, &issue)? {
        if current.kind.as_str() != "provider_issue" {
            return Err(StoreError::SourceConflict.into());
        }
        if let Some(payload) = current.payload
            && store.read_payload(project, &payload)? == PayloadRead::Available(bytes.clone())
        {
            store.note_provider_seen(project, &issue, captured_at_millis)?;
            return Ok(false);
        }
    }
    let encoded = std::str::from_utf8(&bytes)
        .map_err(|error| ImportError::Serialization(error.to_string()))?;
    let identity = format!(
        "{}:{}:{}",
        issue,
        fixture.capture.captured_at,
        digest_id("snap", encoded)
    );
    let payload = PayloadId::try_from(digest_id("ps", &identity).as_str())
        .map_err(|_| ImportError::InvalidIdentity)?;
    match store.read_payload(project, &payload) {
        Ok(PayloadRead::Available(existing)) if existing == bytes => {}
        Ok(_) => return Err(StoreError::SourceConflict.into()),
        Err(StoreError::PayloadMissing) => store.put_payload(project, &payload, &bytes)?,
        Err(error) => return Err(error.into()),
    }
    let batch = DomainEventBatch {
        id: BatchId::try_from(digest_id("pb", &identity).as_str())
            .map_err(|_| ImportError::InvalidIdentity)?,
        project: project.clone(),
        actor: ActorId::try_from("provider_observation")
            .map_err(|_| ImportError::InvalidIdentity)?,
        occurred_at_millis: captured_at_millis,
        events: vec![DomainEvent::PutObject {
            id: EventId::try_from(digest_id("pe", &identity).as_str())
                .map_err(|_| ImportError::InvalidIdentity)?,
            object: issue.clone(),
            kind: ObjectKind::try_from("provider_issue")
                .map_err(|_| ImportError::InvalidIdentity)?,
            payload: Some(payload.clone()),
            issue_scope: None,
        }],
    };
    let observation = ProviderObservation {
        id: PolicyInputId::try_from(digest_id("po", &identity).as_str())
            .map_err(|_| ImportError::InvalidIdentity)?,
        binding: binding.id.clone(),
        issue: issue.clone(),
        state,
        upstream_updated_at_millis: fixture
            .provider_snapshot
            .updated_at
            .as_deref()
            .map(utc_millis)
            .transpose()?,
        closed_at_millis: fixture
            .provider_snapshot
            .closed_at
            .as_deref()
            .map(utc_millis)
            .transpose()?,
        label_provider_ids: (fixture.provider_snapshot.labels.len()
            == fixture.provider_snapshot.label_refs.len())
        .then(|| {
            fixture
                .provider_snapshot
                .label_refs
                .iter()
                .map(|label| label.provider_id.clone())
                .collect()
        }),
        assignee_provider_ids: fixture
            .provider_snapshot
            .assignees
            .iter()
            .map(|actor| actor.provider_id.clone())
            .collect(),
        snapshot_payload: payload,
        observed_at_millis: captured_at_millis,
    };
    accept_provider_snapshot(store, project, &batch, observation, &identity)
}

fn accept_provider_snapshot(
    store: &mut Store,
    project: &ProjectId,
    batch: &DomainEventBatch,
    observation: ProviderObservation,
    identity: &str,
) -> Result<bool, ImportError> {
    let prepared = evaluate(
        store,
        project,
        &batch.actor,
        PolicyEvaluationId::try_from(digest_id("pv", identity).as_str())
            .map_err(|_| ImportError::InvalidIdentity)?,
        batch.id.clone(),
        batch.occurred_at_millis,
        &PolicyRules {
            version: PolicyVersion::try_from("fixture_provider_v1")
                .map_err(|_| ImportError::InvalidIdentity)?,
            decision_authors: Vec::new(),
            command_actors: Vec::new(),
            administrators: Vec::new(),
        },
        &[Proposal::ProviderObservation {
            observation,
            event: batch.events[0].clone(),
        }],
    )?;
    Ok(prepared.commit(store)?.is_some())
}
