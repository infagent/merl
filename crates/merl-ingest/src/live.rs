//! Live GitHub snapshots reuse the source log, provider policy, and compiler intents.

use std::{
    collections::BTreeMap,
    io::Read,
    path::Path,
    process::{Command, Stdio},
};

use merl_compiler::{
    CompilerAdapter, CompilerLimits, RunMode, RunRequest, execute_compilation,
    prepare_eager_compilation, record_compilation_result,
};
use merl_core::{CompilationMode, CoverageRequirement};
use merl_corpus::{
    fixture::{Fixture, Observation},
    github::{ISSUE_QUERY, fixture_from_graphql_pages},
};
use merl_store::{BindingCapturePolicy, StoredSourceVersion};

use super::{
    ImportError, actor_identity, digest_id, fixture_binding, fixture_source_id, fixture_version_id,
    missing_body_reason, observe_terminal_issue_snapshot, utc_millis,
};
use merl_core::{ProjectId, SourceBindingId, SourceKind, SourceVersionId};
use merl_corpus::fixture::{ObservationKind, validate};
use merl_store::{MissingSourceBody, SourceBinding, SourceCapture, Store, StoreError};
use sha2::{Digest, Sha256};

/// Maximum captured GraphQL response size, including pagination.
///
/// The first release accepts 16 MiB per refresh. Larger Issues need a narrower
/// provider adapter; truncating a response could turn unseen comments into deletions.
const MAX_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;

/// Failure at the GitHub capture boundary.
#[derive(Debug)]
pub enum FetchError {
    /// The configured GitHub CLI could not start.
    Unavailable,
    /// GitHub or its CLI rejected the request.
    RequestFailed,
    /// The response cannot prove a complete Issue snapshot.
    Incomplete(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str(
                "GitHub CLI unavailable; install gh and authenticate with gh auth login",
            ),
            Self::RequestFailed => formatter
                .write_str("GitHub request failed; check gh authentication and repository access"),
            Self::Incomplete(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetches complete Issue pages through the installed GitHub CLI.
///
/// The caller chooses the executable so acceptance tests can substitute a provider.
/// Credentials stay with `gh`; Merl does not copy them into project state.
///
/// # Errors
/// Reports unavailable credentials or tools, oversized output, and incomplete pages.
pub fn fetch_issue(
    program: &Path,
    repository: &str,
    number: u64,
    observed_at: &str,
) -> Result<Fixture, FetchError> {
    let (owner, name) = repository
        .split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .ok_or_else(|| FetchError::Incomplete("repository must be owner/name".into()))?;
    let mut child = Command::new(program)
        .args([
            "api",
            "graphql",
            "--paginate",
            "--slurp",
            "-f",
            &format!("query={ISSUE_QUERY}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={number}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| FetchError::Unavailable)?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .ok_or(FetchError::RequestFailed)?
        .take(MAX_CAPTURE_BYTES + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() as u64 > MAX_CAPTURE_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err(FetchError::Incomplete(
            "GitHub response exceeds the capture limit or could not be read".into(),
        ));
    }
    if !child
        .wait()
        .map_err(|_| FetchError::RequestFailed)?
        .success()
    {
        return Err(FetchError::RequestFailed);
    }
    validate_pages(&bytes).map_err(FetchError::Incomplete)?;
    let fixture = fixture_from_graphql_pages("live-capture", observed_at, &bytes)
        .map_err(FetchError::Incomplete)?;
    if fixture.source.issue_number != Some(number) {
        return Err(FetchError::Incomplete(
            "GitHub returned a different Issue".into(),
        ));
    }
    Ok(fixture)
}

fn validate_pages(bytes: &[u8]) -> Result<(), String> {
    let pages: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| "invalid GitHub response")?;
    if pages.is_empty() {
        return Err("GitHub returned no pages".into());
    }
    let mut cursors = std::collections::BTreeSet::new();
    for (index, page) in pages.iter().enumerate() {
        if page
            .get("errors")
            .is_some_and(|errors| errors.as_array().is_none_or(|items| !items.is_empty()))
        {
            return Err("GitHub returned partial data and errors".into());
        }
        let info = &page["data"]["repository"]["issue"]["comments"]["pageInfo"];
        if info["hasNextPage"].as_bool() != Some(index + 1 < pages.len()) {
            return Err("GitHub comments pagination is incomplete".into());
        }
        if index + 1 < pages.len()
            && !info["endCursor"]
                .as_str()
                .is_some_and(|cursor| !cursor.is_empty() && cursors.insert(cursor))
        {
            return Err("GitHub comments pagination cursor is missing or repeated".into());
        }
    }
    Ok(())
}

/// One newly captured version and its eager work identity, when applicable.
#[derive(Debug)]
pub struct CapturedSource {
    /// Stable provider entity ID.
    pub entity: String,
    /// Immutable version to inspect with `source show`.
    pub version: SourceVersionId,
    /// One durable eager intent for this version.
    pub run: Option<String>,
    /// True when a complete refresh no longer exposed the entity.
    pub deleted: bool,
}

/// Capture and compiler outcomes from one Issue refresh.
#[derive(Debug)]
pub struct CaptureReport {
    /// Binding reused by future captures in this provider namespace.
    pub binding: SourceBindingId,
    /// Durable policy, including when the caller omitted policy options.
    pub policy: BindingCapturePolicy,
    /// New immutable observations.
    pub sources: Vec<CapturedSource>,
    /// Existing visible entities whose source version did not change.
    pub unchanged: usize,
    /// Compiler results completed during this refresh.
    pub compiled: usize,
    /// Eager attempts with a recorded failure or context failure.
    pub failures: Vec<String>,
    /// Number of entities observed missing from a complete snapshot.
    pub deleted: usize,
    /// Whether deterministic policy changed the provider mirror.
    pub provider_changed: bool,
    /// Observation head after capture.
    pub observation_head: u64,
    /// Accepted project revision after provider policy.
    pub revision: u64,
}

/// Captures current provider bodies in Merl observation order and dispatches eager work.
///
/// A live observation uses the accepted state available at capture. It makes no
/// claim to reconstruct the state available at an upstream creation timestamp.
/// Deleted entities retain their earlier payloads and acquire a bodyless successor.
///
/// # Errors
/// Rejects invalid snapshots, conflicting authors, and storage or provider-policy errors.
/// A successful prefix survives an error; source and run identities make retries safe.
pub fn capture_issue(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
    default: &BindingCapturePolicy,
    adapter: Option<&impl CompilerAdapter>,
    limits: CompilerLimits,
) -> Result<CaptureReport, ImportError> {
    validate(fixture)?;
    let binding = fixture_binding(fixture)?;
    let policy = store
        .capture_policy(project, &binding.id)?
        .unwrap_or_else(|| default.clone());
    if policy.mode == CompilationMode::Eager && adapter.is_none() {
        return Err(ImportError::CompilerRequired);
    }
    let policy = store.resolve_capture_policy(project, &binding, &policy)?;
    let now = utc_millis(&fixture.capture.captured_at)?;
    let previous =
        store.latest_sources_in_scope(project, &binding.id, &fixture.source.issue_provider_id)?;
    let mut latest = BTreeMap::new();
    for observation in &fixture.observations {
        latest.insert(&observation.provider_id, observation);
    }
    validate_refresh(store, project, fixture, &previous, &latest)?;
    let mut terminal: Vec<_> = latest.values().copied().collect();
    terminal.sort_by_key(|observation| observation.sequence);
    let mut report = CaptureReport {
        binding: binding.id.clone(),
        policy,
        sources: Vec::new(),
        unchanged: 0,
        compiled: 0,
        failures: Vec::new(),
        deleted: 0,
        provider_changed: false,
        observation_head: 0,
        revision: 0,
    };
    for observation in terminal {
        let prior = previous
            .iter()
            .find(|item| item.provider_entity_id == observation.provider_id);
        let source = capture_current(
            store,
            project,
            fixture,
            &binding,
            &report.policy,
            observation,
            prior,
        )?;
        let (version, new) = source;
        let run = if report.policy.mode == CompilationMode::Eager
            && report.policy.coverage == CoverageRequirement::Required
            && observation.body.is_some()
        {
            let run = digest_id("eager", version.as_str());
            dispatch(
                store,
                project,
                &version,
                &run,
                adapter,
                limits,
                now,
                &mut report,
            );
            Some(run)
        } else {
            None
        };
        if new {
            report.sources.push(CapturedSource {
                entity: observation.provider_id.clone(),
                version,
                run,
                deleted: false,
            });
        } else {
            report.unchanged += 1;
        }
    }
    for prior in &previous {
        if !latest.contains_key(&prior.provider_entity_id)
            && prior.missing_body_reason != Some(MissingSourceBody::DeletedByProvider)
        {
            let version = capture_deletion(store, project, &binding, &report.policy, prior, now)?;
            report.sources.push(CapturedSource {
                entity: prior.provider_entity_id.clone(),
                version,
                run: None,
                deleted: true,
            });
            report.deleted += 1;
        }
    }
    report.provider_changed =
        observe_terminal_issue_snapshot(store, project, fixture, &binding, now)?;
    report.observation_head = store.source_observation_head(project)?;
    report.revision = store.project_revision(project)?.get();
    Ok(report)
}

#[expect(
    clippy::too_many_arguments,
    reason = "dispatch binds one source intent to its caller-owned report"
)]
fn dispatch(
    store: &mut Store,
    project: &ProjectId,
    version: &SourceVersionId,
    run: &str,
    adapter: Option<&impl CompilerAdapter>,
    limits: CompilerLimits,
    now: i64,
    report: &mut CaptureReport,
) {
    let result = (|| {
        if let Some(existing) = store.compilation_run_status(project, run)?
            && existing.completed
        {
            return if existing.succeeded {
                Ok(false)
            } else {
                Err(merl_compiler::CompileError::InvalidResponse)
            };
        }
        let adapter = adapter.ok_or_else(|| {
            merl_compiler::CompileError::Adapter(
                "eager capture requires --program and compiler configuration".into(),
            )
        })?;
        let prepared = prepare_eager_compilation(
            store,
            project,
            version,
            adapter,
            RunRequest {
                id: run,
                limits,
                mode: RunMode::Live,
                now_millis: now,
            },
        )?;
        if let Some(prepared) = prepared {
            let response = execute_compilation(&prepared, adapter);
            record_compilation_result(store, project, &prepared, response, now)?;
            Ok(true)
        } else {
            Ok(false)
        }
    })();
    match result {
        Ok(true) => report.compiled += 1,
        Ok(false) => {}
        Err(error) => report.failures.push(format!("{version}: {error}")),
    }
}

fn validate_refresh(
    store: &Store,
    project: &ProjectId,
    fixture: &Fixture,
    previous: &[StoredSourceVersion],
    latest: &BTreeMap<&String, &Observation>,
) -> Result<(), ImportError> {
    let now = utc_millis(&fixture.capture.captured_at)?;
    let updated = fixture
        .provider_snapshot
        .updated_at
        .as_deref()
        .map(utc_millis)
        .transpose()?;
    if let Some(head) = store.provider_issue_head(project, &super::fixture_issue_id(fixture)?)?
        && head
            .input
            .upstream_updated_at_millis
            .zip(updated)
            .is_some_and(|(prior, incoming)| incoming < prior)
    {
        return Err(StoreError::StaleProviderObservation.into());
    }
    for prior in previous {
        if now < prior.observed_at_millis {
            return Err(StoreError::StaleProviderObservation.into());
        }
        if let Some(observation) = latest.get(&prior.provider_entity_id) {
            let updated = observation
                .updated_at
                .as_deref()
                .map(utc_millis)
                .transpose()?;
            if prior
                .upstream_updated_at_millis
                .zip(updated)
                .is_some_and(|(prior, incoming)| incoming < prior)
            {
                return Err(StoreError::StaleProviderObservation.into());
            }
            let author = observation
                .author
                .as_ref()
                .and_then(|actor| actor.provider_id.as_deref());
            if prior
                .provider_source_author_id
                .as_deref()
                .zip(author)
                .is_some_and(|(first, next)| first != next)
            {
                return Err(StoreError::SourceConflict.into());
            }
        }
    }
    Ok(())
}

fn capture_current(
    store: &mut Store,
    project: &ProjectId,
    fixture: &Fixture,
    binding: &SourceBinding,
    policy: &BindingCapturePolicy,
    observation: &Observation,
    prior: Option<&StoredSourceVersion>,
) -> Result<(SourceVersionId, bool), ImportError> {
    let author = observation
        .author
        .as_ref()
        .and_then(|actor| actor.provider_id.as_deref());
    let digest: Option<[u8; 32]> = observation
        .body
        .as_ref()
        .map(|body| Sha256::digest(body.as_bytes()).into());
    if let Some(prior) = prior
        && prior.provider_version_id == observation.version_id
        && prior.body_digest == digest
        && prior.missing_body_reason != Some(MissingSourceBody::DeletedByProvider)
    {
        return Ok((prior.id.clone(), false));
    }
    let now = utc_millis(&fixture.capture.captured_at)?;
    let version = fixture_version_id(&format!(
        "live:{}:{}:{:?}:{}",
        observation.provider_id,
        observation.version_id,
        digest,
        prior.map_or("", |item| item.id.as_str())
    ))?;
    let editor = observation
        .edit
        .as_ref()
        .and_then(|edit| edit.editor.as_ref())
        .or(observation.author.as_ref())
        .and_then(|actor| actor.provider_id.as_deref());
    let capture = SourceCapture {
        binding: binding.clone(),
        source: fixture_source_id(binding, observation)?,
        provider_entity_id: &observation.provider_id,
        context_scope_id: &fixture.source.issue_provider_id,
        version: version.clone(),
        provider_version_id: &observation.version_id,
        kind: SourceKind::try_from(if matches!(observation.kind, ObservationKind::Issue) {
            "issue"
        } else {
            "issue_comment"
        })
        .map_err(|_| ImportError::InvalidIdentity)?,
        supersedes: prior.map(|item| item.id.clone()),
        ambiguous_order_with_previous: false,
        created_at_millis: utc_millis(&observation.created_at)?,
        occurred_at_millis: utc_millis(&observation.occurred_at)?,
        upstream_updated_at_millis: observation
            .updated_at
            .as_deref()
            .map(utc_millis)
            .transpose()?,
        observed_at_millis: now,
        actor: actor_identity(editor)?,
        provider_actor_id: editor,
        source_author: actor_identity(author)?,
        provider_source_author_id: author,
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
        compilation_mode: policy.mode,
        coverage_requirement: policy.coverage,
        policy_version: policy.version.clone(),
    };
    Ok((version, store.capture_source_version(project, &capture)?))
}

fn capture_deletion(
    store: &mut Store,
    project: &ProjectId,
    binding: &SourceBinding,
    policy: &BindingCapturePolicy,
    prior: &StoredSourceVersion,
    now: i64,
) -> Result<SourceVersionId, ImportError> {
    let provider_version = format!("missing:{}", prior.id);
    let version = fixture_version_id(&provider_version)?;
    store.capture_source_version(
        project,
        &SourceCapture {
            binding: binding.clone(),
            source: prior.source.clone(),
            provider_entity_id: &prior.provider_entity_id,
            context_scope_id: &prior.context_scope_id,
            version: version.clone(),
            provider_version_id: &provider_version,
            kind: SourceKind::try_from("observed_deletion")
                .map_err(|_| ImportError::InvalidIdentity)?,
            supersedes: Some(prior.id.clone()),
            ambiguous_order_with_previous: false,
            created_at_millis: prior.created_at_millis,
            occurred_at_millis: now,
            upstream_updated_at_millis: None,
            observed_at_millis: now,
            actor: None,
            provider_actor_id: None,
            source_author: prior.source_author.clone(),
            provider_source_author_id: prior.provider_source_author_id.as_deref(),
            body: None,
            edit_diff: None,
            edit_deleted_at_millis: None,
            missing_body_reason: Some(MissingSourceBody::DeletedByProvider),
            compilation_mode: CompilationMode::CaptureOnly,
            coverage_requirement: policy.coverage,
            policy_version: policy.version.clone(),
        },
    )?;
    Ok(version)
}
