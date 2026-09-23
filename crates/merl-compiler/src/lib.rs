//! Causal source selection and a bounded, authority-free compiler boundary.

use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    io::{Read, Write},
    process::{Command, Stdio},
};

use merl_core::{ObjectId, ObjectRevision, ProjectId, ProjectRevision, SourceVersionId};
use merl_store::{
    CompilationIntent, CompilationResult, PayloadRead, SelectedObjects, Store, StoreError,
    StructuralAssertion,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Failure to build or persist one source interpretation.
#[derive(Debug)]
pub enum CompileError {
    /// The selected source or accepted history is unavailable.
    Store(StoreError),
    /// A historical version lacks exact bytes or has unresolved causal order.
    NonCausalHistory,
    /// A retained source or compiler input was erased after the run was recorded.
    MissingEvidence,
    /// The configured limit cannot fit the selected context.
    InputBudget,
    /// The adapter could not produce a valid bounded response.
    OutputBudget,
    /// The adapter emitted invalid or unauthorized output.
    InvalidResponse,
    /// The bounded run needs another context round before coverage is complete.
    ContextRequired,
    /// Live on-demand work has no matching accepted spend authorization.
    UnauthorizedCompilation,
    /// This binary cannot reconstruct an older selector or renderer contract.
    UnsupportedReplayVersion(String),
    /// The external compiler process failed to start or complete.
    Adapter(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::NonCausalHistory => f.write_str("exact causal source history is unavailable"),
            Self::MissingEvidence => {
                f.write_str("protected source or compiler input bytes were erased")
            }
            Self::InputBudget => f.write_str("compiler context exceeds its input budget"),
            Self::OutputBudget => f.write_str("compiler response exceeds its output budget"),
            Self::InvalidResponse => {
                f.write_str("compiler returned an invalid structured response")
            }
            Self::ContextRequired => f.write_str("compiler requested more context"),
            Self::UnauthorizedCompilation => {
                f.write_str("compiler run lacks matching accepted authorization")
            }
            Self::UnsupportedReplayVersion(version) => {
                write!(f, "unsupported compiler replay version: {version}")
            }
            Self::Adapter(message) => write!(f, "compiler adapter failed: {message}"),
        }
    }
}

impl Error for CompileError {}

impl From<StoreError> for CompileError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Limits applied before context construction and after the adapter responds.
#[derive(Clone, Copy, Debug, JsonSchema, Serialize)]
pub struct CompilerLimits {
    /// Maximum encoded input size, including selected source text.
    pub input_bytes: usize,
    /// Maximum encoded output size.
    pub output_bytes: usize,
    /// Provider-facing output token cap.
    pub output_tokens: usize,
    /// Maximum number of assertions in one response.
    pub assertions: usize,
    /// Maximum context requests in one response.
    pub context_requests: usize,
    /// Maximum requested context expansion rounds.
    pub expansion_rounds: usize,
    /// Maximum bytes of source or referenced payload text in the context.
    pub payload_bytes: usize,
    /// Maximum number of recent source versions selected.
    pub source_window: usize,
    /// Maximum number of historical objects selected.
    pub objects: usize,
}

impl CompilerLimits {
    /// Returns the stable field order used by run and authorization records.
    #[must_use]
    pub const fn as_array(self) -> [usize; 9] {
        [
            self.input_bytes,
            self.output_bytes,
            self.output_tokens,
            self.assertions,
            self.context_requests,
            self.expansion_rounds,
            self.payload_bytes,
            self.source_window,
            self.objects,
        ]
    }
}

/// A historically bounded compiler input and its manifest.
#[derive(Clone, Debug)]
pub struct CompilationContext {
    /// The source whose meaning is being interpreted.
    pub trigger: SourceVersionId,
    /// Project revision visible at its capture.
    pub interpretation_basis_revision: ProjectRevision,
    /// Inclusive source observation cutoff.
    pub source_observation_cutoff: u64,
    /// Exact source versions selected in observation order.
    pub source_window: Vec<SourceVersionId>,
    /// Object revisions selected from the historical accepted state.
    pub objects: Vec<(ObjectId, ObjectRevision)>,
    /// Exact bytes handed to the adapter.
    pub rendered: Vec<u8>,
}

#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct RenderedContext {
    schema: &'static str,
    context_scope_id: String,
    interpretation_basis_revision: u64,
    source_observation_cutoff: u64,
    trigger: String,
    sources: Vec<RenderedSource>,
    objects: Vec<RenderedObject>,
    objects_truncated: bool,
}

#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct RenderedSource {
    id: String,
    observation: u64,
    source_author_id: Option<String>,
    version_actor_id: Option<String>,
    created_at_millis: i64,
    occurred_at_millis: i64,
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_deletion_of: Option<String>,
}

#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct RenderedObject {
    id: String,
    revision: u64,
    body: Option<String>,
}

struct ObjectContext {
    references: Vec<(ObjectId, ObjectRevision)>,
    views: Vec<RenderedObject>,
    truncated: bool,
}

#[derive(Clone, Copy)]
struct ContextPosition {
    basis: ProjectRevision,
    cutoff: u64,
}

#[derive(Clone, Copy)]
enum SelectorVersion {
    ObjectIdPrefixV1,
    IssueContextV1,
}

impl SelectorVersion {
    fn for_replay(version: &str) -> Result<Self, CompileError> {
        match version {
            "object_id_prefix_v1" => Ok(Self::ObjectIdPrefixV1),
            "issue_context_v1" => Ok(Self::IssueContextV1),
            other => Err(CompileError::UnsupportedReplayVersion(other.to_owned())),
        }
    }
}

/// Selects only observations and accepted state available at the trigger's position.
///
/// # Errors
/// Rejects missing source bytes, ambiguous ordering, or an over-budget context.
pub fn build_context(
    store: &Store,
    project: &ProjectId,
    trigger: &SourceVersionId,
    limits: CompilerLimits,
) -> Result<CompilationContext, CompileError> {
    let source = store
        .source_version(project, trigger)?
        .ok_or(CompileError::NonCausalHistory)?;
    let basis = if source.interpretation_basis_known {
        source.interpretation_basis_revision
    } else {
        store
            .replay_basis(project, trigger)?
            .ok_or(CompileError::NonCausalHistory)?
    };
    build_context_with_basis(
        store,
        project,
        trigger,
        basis,
        limits,
        false,
        SelectorVersion::IssueContextV1,
    )
}

/// Rebuilds an earlier compiler input from causal evidence and checks its saved manifest.
///
/// This checks source bytes again; reading the retained rendered payload alone would
/// hide a later purge or a renderer change from replay.
///
/// # Errors
/// Reports missing evidence, changed selection, or a digest mismatch as noncausal history.
pub fn rebuild_recorded_context(
    store: &Store,
    project: &ProjectId,
    run_id: &str,
    limits: CompilerLimits,
) -> Result<CompilationContext, CompileError> {
    let status = store
        .compilation_run_status(project, run_id)?
        .ok_or(CompileError::NonCausalHistory)?;
    if status.mode == "hindsight" || status.limits != limits.as_array() {
        return Err(CompileError::NonCausalHistory);
    }
    if status.renderer_version != "json_v1" {
        return Err(CompileError::UnsupportedReplayVersion(
            status.renderer_version,
        ));
    }
    let selector = SelectorVersion::for_replay(&status.selector_version)?;
    let saved = store
        .load_compilation_context(project, run_id)
        .map_err(|error| {
            if matches!(error, StoreError::InvalidCompilation) {
                CompileError::MissingEvidence
            } else {
                CompileError::Store(error)
            }
        })?;
    let rebuilt = build_context_with_basis(
        store,
        project,
        &status.source,
        status.interpretation_basis_revision,
        limits,
        false,
        selector,
    )?;
    let mut rebuilt_objects = rebuilt.objects.clone();
    let mut saved_objects = saved.objects;
    rebuilt_objects.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    saved_objects.sort_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    if rebuilt.interpretation_basis_revision != status.interpretation_basis_revision
        || rebuilt.source_observation_cutoff != status.source_observation_cutoff
        || rebuilt.source_window != saved.source_window
        || rebuilt_objects != saved_objects
        || Sha256::digest(&rebuilt.rendered).as_slice() != status.context_digest
        || rebuilt.rendered != saved.rendered
    {
        return Err(CompileError::NonCausalHistory);
    }
    Ok(rebuilt)
}

/// Binds historical observations in order, recording the accepted revision at each step.
pub struct SequentialReplay {
    project: ProjectId,
}

impl SequentialReplay {
    #[must_use]
    pub fn new(project: ProjectId) -> Self {
        Self { project }
    }

    /// # Errors
    /// Rejects skipped observations or a first observation bound after state advanced.
    pub fn next_context(
        &mut self,
        store: &mut Store,
        source: &SourceVersionId,
        limits: CompilerLimits,
    ) -> Result<CompilationContext, CompileError> {
        store.bind_next_replay_position(&self.project, source)?;
        build_context(store, &self.project, source, limits)
    }
}

fn build_context_with_basis(
    store: &Store,
    project: &ProjectId,
    trigger: &SourceVersionId,
    basis: ProjectRevision,
    limits: CompilerLimits,
    hindsight: bool,
    selector: SelectorVersion,
) -> Result<CompilationContext, CompileError> {
    let source = store
        .source_version(project, trigger)?
        .ok_or(CompileError::NonCausalHistory)?;
    verify_basis(store, project, trigger, &source, basis, hindsight)?;
    if limits.source_window == 0 || limits.objects == 0 {
        return Err(CompileError::InputBudget);
    }
    let selected = store.recent_source_versions_in_scope(
        project,
        &source.context_scope_id,
        source.sequence,
        limits.source_window,
    )?;
    build_context_from_sources(
        store,
        project,
        trigger,
        ContextPosition {
            basis,
            cutoff: source.sequence,
        },
        selected,
        limits,
        selector,
    )
}

fn source_context_body(
    store: &Store,
    project: &ProjectId,
    source: &merl_store::StoredSourceVersion,
    observed_deletion: bool,
) -> Result<Vec<u8>, CompileError> {
    // A live deletion describes an observation, not replacement prose. Its
    // predecessor remains in the manifest so later interpretations can see the loss.
    if observed_deletion {
        return Ok(Vec::new());
    }
    match store.read_payload(
        project,
        source
            .payload
            .as_ref()
            .ok_or(CompileError::NonCausalHistory)?,
    )? {
        PayloadRead::Available(bytes) => Ok(bytes),
        PayloadRead::Unavailable => Err(CompileError::MissingEvidence),
    }
}

fn build_context_from_sources(
    store: &Store,
    project: &ProjectId,
    trigger: &SourceVersionId,
    position: ContextPosition,
    selected: Vec<SourceVersionId>,
    limits: CompilerLimits,
    selector: SelectorVersion,
) -> Result<CompilationContext, CompileError> {
    if selected.is_empty() || selected.len() > limits.source_window || limits.objects == 0 {
        return Err(CompileError::InputBudget);
    }
    let source = store
        .source_version(project, trigger)?
        .ok_or(CompileError::NonCausalHistory)?;
    if !selected.iter().any(|item| item == trigger) {
        return Err(CompileError::NonCausalHistory);
    }
    let mut sources = Vec::new();
    let mut source_window = Vec::new();
    let mut payload_bytes = 0usize;
    let mut previous_sequence = 0_u64;
    for version in selected {
        let item = store
            .source_version(project, &version)?
            .ok_or(CompileError::NonCausalHistory)?;
        let observed_deletion = item.kind.as_str() == "observed_deletion"
            && item.missing_body_reason == Some(merl_store::MissingSourceBody::DeletedByProvider)
            && item.supersedes.is_some()
            && item.payload.is_none();
        if item.ambiguous_order_with_previous
            || (item.payload.is_none() && !observed_deletion)
            || item.sequence > position.cutoff
            || item.sequence <= previous_sequence
            || item.context_scope_id != source.context_scope_id
        {
            return Err(CompileError::NonCausalHistory);
        }
        previous_sequence = item.sequence;
        let body = source_context_body(store, project, &item, observed_deletion)?;
        payload_bytes = payload_bytes
            .checked_add(body.len())
            .ok_or(CompileError::InputBudget)?;
        if payload_bytes > limits.payload_bytes {
            return Err(CompileError::InputBudget);
        }
        source_window.push(item.id.clone());
        sources.push(RenderedSource {
            id: item.id.to_string(),
            observation: item.sequence,
            source_author_id: item.source_author.map(|actor| actor.to_string()),
            version_actor_id: item.version_actor.map(|actor| actor.to_string()),
            created_at_millis: item.created_at_millis,
            occurred_at_millis: item.occurred_at_millis,
            body: if observed_deletion {
                None
            } else {
                Some(String::from_utf8(body).map_err(|_| CompileError::InvalidResponse)?)
            },
            observed_deletion_of: if observed_deletion {
                item.supersedes.map(|id| id.to_string())
            } else {
                None
            },
        });
    }
    let selection = match selector {
        SelectorVersion::ObjectIdPrefixV1 => {
            store.objects_at_revision(project, position.basis, limits.objects)?
        }
        SelectorVersion::IssueContextV1 => select_objects(
            store,
            project,
            position.basis,
            trigger,
            &source.context_scope_id,
            &sources,
            limits.objects,
        )?,
    };
    let object_context = render_objects(store, project, selection, &mut payload_bytes, limits)?;
    let rendered = serde_json::to_vec(&RenderedContext {
        schema: "merl.compilation-context/v1",
        context_scope_id: source.context_scope_id,
        interpretation_basis_revision: position.basis.get(),
        source_observation_cutoff: position.cutoff,
        trigger: trigger.to_string(),
        sources,
        objects: object_context.views,
        objects_truncated: object_context.truncated,
    })
    .map_err(|_| CompileError::InvalidResponse)?;
    if rendered.len() > limits.input_bytes {
        return Err(CompileError::InputBudget);
    }
    Ok(CompilationContext {
        trigger: trigger.clone(),
        interpretation_basis_revision: position.basis,
        source_observation_cutoff: position.cutoff,
        source_window,
        objects: object_context.references,
        rendered,
    })
}

fn build_revalidation_context(
    store: &Store,
    project: &ProjectId,
    trigger: &SourceVersionId,
    impact: &merl_store::EvidenceImpact,
    limits: CompilerLimits,
) -> Result<CompilationContext, CompileError> {
    if impact.next_action != "recompile" || impact.revalidated_by.is_some() {
        return Err(CompileError::NonCausalHistory);
    }
    let replacement = impact
        .replacement
        .as_ref()
        .ok_or(CompileError::NonCausalHistory)?;
    let replacement_source = store
        .source_version(project, replacement)?
        .ok_or(CompileError::NonCausalHistory)?;
    if replacement_source.supersedes.as_ref() != Some(&impact.changed_source) {
        return Err(CompileError::NonCausalHistory);
    }
    let expected_trigger = if impact.trigger == impact.changed_source {
        replacement
    } else {
        &impact.trigger
    };
    if trigger != expected_trigger {
        return Err(CompileError::NonCausalHistory);
    }
    let mut selected = store.compilation_context_sources(project, impact.affected_run.as_str())?;
    let changed = selected
        .iter_mut()
        .find(|version| **version == impact.changed_source)
        .ok_or(CompileError::NonCausalHistory)?;
    *changed = replacement.clone();
    let mut ordered = Vec::with_capacity(selected.len());
    for version in selected {
        let source = store
            .source_version(project, &version)?
            .ok_or(CompileError::NonCausalHistory)?;
        ordered.push((source.sequence, version));
    }
    ordered.sort_by_key(|item| item.0);
    let selected = ordered.into_iter().map(|(_, version)| version).collect();
    build_context_from_sources(
        store,
        project,
        trigger,
        ContextPosition {
            basis: store.project_revision(project)?,
            cutoff: store.source_observation_head(project)?,
        },
        selected,
        limits,
        SelectorVersion::IssueContextV1,
    )
}

fn render_objects(
    store: &Store,
    project: &ProjectId,
    selection: SelectedObjects,
    payload_bytes: &mut usize,
    limits: CompilerLimits,
) -> Result<ObjectContext, CompileError> {
    let mut references = Vec::new();
    let mut views = Vec::new();
    for (id, revision, payload) in selection.items {
        let body = match payload {
            Some(payload) => match store.read_payload(project, &payload)? {
                PayloadRead::Available(bytes) => {
                    *payload_bytes = payload_bytes
                        .checked_add(bytes.len())
                        .ok_or(CompileError::InputBudget)?;
                    if *payload_bytes > limits.payload_bytes {
                        return Err(CompileError::InputBudget);
                    }
                    Some(String::from_utf8(bytes).map_err(|_| CompileError::InvalidResponse)?)
                }
                PayloadRead::Unavailable => return Err(CompileError::MissingEvidence),
            },
            None => None,
        };
        views.push(RenderedObject {
            id: id.to_string(),
            revision: revision.get(),
            body,
        });
        references.push((id, revision));
    }
    Ok(ObjectContext {
        references,
        views,
        truncated: selection.truncated,
    })
}

fn select_objects(
    store: &Store,
    project: &ProjectId,
    basis: ProjectRevision,
    trigger: &SourceVersionId,
    scope: &str,
    sources: &[RenderedSource],
    budget: usize,
) -> Result<SelectedObjects, CompileError> {
    let mut named = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(trigger_source) = sources.iter().find(|item| item.id == trigger.as_str()) {
        for token in trigger_source
            .body
            .as_deref()
            .unwrap_or_default()
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        {
            // An uppercase handle with a number is a structural hint, not a semantic claim.
            if !token.starts_with(|character: char| character.is_ascii_uppercase())
                || !token.bytes().any(|byte| byte.is_ascii_digit())
                || !seen.insert(token.to_owned())
            {
                continue;
            }
            if let Ok(id) = ObjectId::try_from(token)
                && let Some((revision, payload)) = store.object_at_revision(project, &id, basis)?
            {
                named.push((id, revision, payload));
                if named.len() > budget {
                    return Err(CompileError::InputBudget);
                }
            }
        }
    }
    let historical = store.objects_at_revision_for_scope(
        project,
        basis,
        budget.saturating_add(named.len()),
        scope,
    )?;
    let mut selected = named;
    let total_visible = selected.len()
        + historical
            .items
            .iter()
            .filter(|item| !selected.iter().any(|(id, _, _)| id == &item.0))
            .count();
    for item in historical.items {
        if selected.len() == budget {
            break;
        }
        if !selected.iter().any(|(id, _, _)| id == &item.0) {
            selected.push(item);
        }
    }
    Ok(SelectedObjects {
        items: selected,
        truncated: historical.truncated || total_visible > budget,
    })
}

fn verify_basis(
    store: &Store,
    project: &ProjectId,
    trigger: &SourceVersionId,
    source: &merl_store::StoredSourceVersion,
    basis: ProjectRevision,
    hindsight: bool,
) -> Result<(), CompileError> {
    if basis > store.project_revision(project)?
        || (!hindsight
            && source.interpretation_basis_known
            && basis != source.interpretation_basis_revision)
        || (!hindsight
            && !source.interpretation_basis_known
            && store.replay_basis(project, trigger)? != Some(basis))
    {
        return Err(CompileError::NonCausalHistory);
    }
    Ok(())
}

/// A typed compiler response; unknown prose fields are rejected.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerResponse {
    /// Protocol discriminator.
    pub schema: String,
    /// Independent assertions; policy later decides whether any become accepted.
    pub assertions: Vec<Assertion>,
    /// Limited requests for another explicit context-selection round.
    #[serde(default)]
    pub context_required: Vec<ContextRequest>,
    /// Explicit inability to resolve a referent from the supplied context.
    #[serde(default)]
    pub unresolved: Vec<Unresolved>,
    /// Typed semantic relations, without copied source prose.
    #[serde(default)]
    pub relations: Vec<TypedRelation>,
}

/// A bounded relation inferred from a source span.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TypedRelation {
    /// Source or object identifier at the start of the relation.
    pub subject: String,
    /// Relation predicate, such as `supports` or `disputes`.
    pub predicate: String,
    /// Source or object identifier at the end of the relation.
    pub object: String,
}

/// A source-grounded structural assertion with separate attribution axes.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Assertion {
    /// Source version that contains the cited span.
    pub source: String,
    /// Inclusive UTF-8 byte offset in the source body.
    pub span_start: usize,
    /// Exclusive UTF-8 byte offset in the source body.
    pub span_end: usize,
    /// Bounded subject identifier.
    pub subject: String,
    /// Bounded predicate identifier.
    pub predicate: String,
    /// Bounded value identifier or payload reference.
    pub value: String,
    /// Claim, propose, request, ask, or report.
    pub act: String,
    /// Observed, inferred, or reported.
    pub epistemic_basis: String,
    /// Positive or negative.
    pub polarity: String,
    /// Confidence in thousandths.
    pub confidence_millis: u16,
    /// Quoted or relayed actor, if any.
    pub attributed_to: Option<String>,
}

/// A bounded request for more source context.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    /// Source or object handle to consider in another run.
    pub reference: String,
}

/// A referent the compiler cannot resolve without guessing.
#[derive(Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Unresolved {
    /// Source version containing the ambiguous text.
    pub source: String,
    /// Start of the ambiguous span.
    pub span_start: usize,
    /// End of the ambiguous span.
    pub span_end: usize,
}

/// A compiler cannot read Merl state; it receives only the rendered context.
pub trait CompilerAdapter {
    /// Stable implementation identity recorded with the run.
    fn id(&self) -> &str;
    /// Stable implementation version recorded with the run.
    fn version(&self) -> &str;
    /// Model identity, or `deterministic` for a fake.
    fn model(&self) -> &str;
    /// Digest of the adapter's prompt or ruleset.
    fn prompt_digest(&self) -> [u8; 32];
    /// Digest of the executable artifact and fixed adapter configuration.
    fn configuration_digest(&self) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(self.id().as_bytes());
        digest.update(self.version().as_bytes());
        digest.update(self.model().as_bytes());
        digest.update(self.prompt_digest());
        digest.finalize().into()
    }
    /// Runs the compiler and returns encoded structured output.
    ///
    /// # Errors
    /// Reports process or model failures without mutating accepted project state.
    fn compile(&self, context: &[u8], limits: CompilerLimits) -> Result<Vec<u8>, CompileError>;
}

#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct ProcessRequest<'a> {
    schema: &'static str,
    #[schemars(with = "RenderedContext")]
    context: &'a serde_json::Value,
    limits: CompilerLimits,
}

/// Machine-readable schemas for the versioned external compiler contract.
///
/// The Rust types are authoritative; adapters can use these schemas to
/// validate their input and output without linking a Rust crate.
#[must_use]
pub fn protocol_schemas() -> serde_json::Value {
    serde_json::json!({
        "request": schemars::schema_for!(ProcessRequest<'static>),
        "response": schemars::schema_for!(CompilerResponse),
    })
}

/// A versioned process adapter for a configured model-backed compiler.
#[derive(Debug)]
pub struct ProcessCompiler {
    /// Executable chosen by the evaluation runner.
    program: String,
    /// Fixed arguments; the exact context is supplied only on stdin.
    args: Vec<String>,
    /// Stable implementation version.
    version: String,
    /// Provider/model identity for evaluation provenance.
    model: String,
    /// Hash of the configured prompt, which remains outside the database.
    prompt_digest: [u8; 32],
    configuration_digest: [u8; 32],
}

impl ProcessCompiler {
    /// Freezes a process adapter to the executable bytes and fixed arguments on disk now.
    ///
    /// # Errors
    /// Returns an adapter error when the executable cannot be read.
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        version: impl Into<String>,
        model: impl Into<String>,
        prompt_digest: [u8; 32],
    ) -> Result<Self, CompileError> {
        let program = program.into();
        let artifact = std::fs::read(&program)
            .map_err(|error| CompileError::Adapter(format!("cannot read compiler: {error}")))?;
        let configuration_digest = process_configuration_digest(&artifact, &args)?;
        Ok(Self {
            program,
            args,
            version: version.into(),
            model: model.into(),
            prompt_digest,
            configuration_digest,
        })
    }

    fn verify_configuration(&self) -> Result<(), CompileError> {
        let artifact = std::fs::read(&self.program)
            .map_err(|error| CompileError::Adapter(format!("cannot read compiler: {error}")))?;
        if process_configuration_digest(&artifact, &self.args)? != self.configuration_digest {
            return Err(CompileError::UnauthorizedCompilation);
        }
        Ok(())
    }
}

fn process_configuration_digest(
    artifact: &[u8],
    args: &[String],
) -> Result<[u8; 32], CompileError> {
    let mut digest = Sha256::new();
    digest.update(b"merl.process-compiler-config/v1");
    digest.update(
        u64::try_from(artifact.len())
            .map_err(|_| CompileError::Adapter("compiler artifact is too large".into()))?
            .to_le_bytes(),
    );
    digest.update(artifact);
    for argument in args {
        digest.update(
            u64::try_from(argument.len())
                .map_err(|_| CompileError::Adapter("compiler argument is too large".into()))?
                .to_le_bytes(),
        );
        digest.update(argument.as_bytes());
    }
    Ok(digest.finalize().into())
}

impl CompilerAdapter for ProcessCompiler {
    fn id(&self) -> &'static str {
        "process"
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn model(&self) -> &str {
        &self.model
    }
    fn prompt_digest(&self) -> [u8; 32] {
        self.prompt_digest
    }
    fn configuration_digest(&self) -> [u8; 32] {
        self.configuration_digest
    }
    fn compile(&self, context: &[u8], limits: CompilerLimits) -> Result<Vec<u8>, CompileError> {
        // Queued work may outlive the file originally authorized at this path.
        self.verify_configuration()?;
        let context_value: serde_json::Value =
            serde_json::from_slice(context).map_err(|_| CompileError::InvalidResponse)?;
        let request = serde_json::to_vec(&ProcessRequest {
            schema: "merl.compiler-request/v1",
            context: &context_value,
            limits,
        })
        .map_err(|_| CompileError::InvalidResponse)?;
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| CompileError::Adapter(error.to_string()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CompileError::Adapter("stdin unavailable".into()))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| CompileError::Adapter("stdout unavailable".into()))?;
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                let result = stdin.write_all(&request);
                drop(stdin);
                result
            });
            let mut bytes = Vec::new();
            let read = stdout
                .by_ref()
                .take(limits.output_bytes.saturating_add(1) as u64)
                .read_to_end(&mut bytes);
            if read.is_err() || bytes.len() > limits.output_bytes {
                let _ = child.kill();
            }
            let written = writer
                .join()
                .map_err(|_| CompileError::Adapter("stdin writer panicked".into()))?;
            let status = child
                .wait()
                .map_err(|error| CompileError::Adapter(error.to_string()))?;
            read.map_err(|error| CompileError::Adapter(error.to_string()))?;
            if bytes.len() > limits.output_bytes {
                return Err(CompileError::OutputBudget);
            }
            written.map_err(|error| CompileError::Adapter(error.to_string()))?;
            if !status.success() {
                return Err(CompileError::Adapter(
                    "process exited unsuccessfully".into(),
                ));
            }
            Ok(bytes)
        })
    }
}

/// Offline fake for protocol tests; it never invents project assertions.
#[derive(Debug)]
pub struct FakeCompiler;

impl CompilerAdapter for FakeCompiler {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn model(&self) -> &'static str {
        "deterministic"
    }
    fn prompt_digest(&self) -> [u8; 32] {
        Sha256::digest(b"fake-v1").into()
    }
    fn compile(&self, context: &[u8], _limits: CompilerLimits) -> Result<Vec<u8>, CompileError> {
        let context: serde_json::Value =
            serde_json::from_slice(context).map_err(|_| CompileError::InvalidResponse)?;
        let trigger = context["trigger"]
            .as_str()
            .ok_or(CompileError::InvalidResponse)?;
        serde_json::to_vec(&CompilerResponse {
            schema: "merl.compiler-response/v1".into(),
            assertions: Vec::new(),
            context_required: Vec::new(),
            unresolved: vec![Unresolved {
                source: trigger.into(),
                span_start: 0,
                span_end: 0,
            }],
            relations: Vec::new(),
        })
        .map_err(|_| CompileError::InvalidResponse)
    }
}

/// Why a compiler attempt exists; only live successes contribute to live coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunMode {
    /// Normal source processing.
    Live,
    /// Reproduce a recorded causal input without changing accepted state.
    Replay,
    /// Run a corpus experiment without changing live coverage.
    Eval,
    /// Reinterpret an old source using currently accepted project state.
    Hindsight,
}

impl RunMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Replay => "replay",
            Self::Eval => "eval",
            Self::Hindsight => "hindsight",
        }
    }
}

/// Caller-supplied attempt identity, limits, purpose, and clock.
#[derive(Clone, Copy, Debug)]
pub struct RunRequest<'a> {
    /// Stable run identity used for retry deduplication.
    pub id: &'a str,
    /// Input and output budgets.
    pub limits: CompilerLimits,
    /// Live, replay, evaluation, or hindsight purpose.
    pub mode: RunMode,
    /// Current UTC time in Unix milliseconds.
    pub now_millis: i64,
}

/// A durable attempt whose compiler input is owned independently of the store.
#[derive(Debug)]
pub struct PreparedCompilation {
    id: String,
    context: CompilationContext,
    limits: CompilerLimits,
    compiler_id: String,
    compiler_version: String,
    model_id: String,
    prompt_digest: [u8; 32],
    adapter_config_digest: [u8; 32],
}

#[derive(Clone, Copy)]
struct ContextVersions<'a> {
    renderer: &'a str,
    selector: &'a str,
}

impl PreparedCompilation {
    /// Stable identity used to reconcile a result after process restart.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Persists eager source work selected by the source's capture policy.
///
/// # Errors
/// Rejects non-eager sources and non-live requests before creating a run.
pub fn prepare_eager_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    let captured = store
        .source_version(project, source)?
        .ok_or(CompileError::NonCausalHistory)?;
    if request.mode != RunMode::Live
        || captured.compilation_mode != merl_core::CompilationMode::Eager
    {
        return Err(CompileError::UnauthorizedCompilation);
    }
    prepare_compilation(store, project, source, adapter, request)
}

/// Persists a corpus-evaluation attempt that cannot satisfy live coverage.
///
/// # Errors
/// Rejects non-evaluation requests and invalid compiler context.
pub fn prepare_evaluation_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    if request.mode != RunMode::Eval {
        return Err(CompileError::InvalidResponse);
    }
    prepare_compilation(store, project, source, adapter, request)
}

/// Persists deliberate reinterpretation using current accepted state.
///
/// # Errors
/// Rejects non-hindsight requests and invalid compiler context.
pub fn prepare_hindsight_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    if request.mode != RunMode::Hindsight {
        return Err(CompileError::InvalidResponse);
    }
    prepare_compilation(store, project, source, adapter, request)
}

/// Persists an immutable run intent before external compiler work begins.
///
/// `None` means this identity already has a successful result. A pending
/// identity returns the same prepared work so recovery can run it again.
///
/// # Errors
/// Fails on missing causal history, a previous failed result, or an incompatible run ID.
fn prepare_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    let RunRequest {
        id: run_id,
        limits,
        mode,
        now_millis: _,
    } = request;
    if let Some(existing) = store.compilation_run_status(project, run_id)? {
        if existing.source != *source
            || existing.mode != mode.as_str()
            || existing.compiler_id != adapter.id()
            || existing.compiler_version != adapter.version()
            || existing.model_id != adapter.model()
            || existing.prompt_digest != adapter.prompt_digest()
            || existing.adapter_config_digest != adapter.configuration_digest()
            || existing.limits != limits.as_array()
        {
            return Err(CompileError::InvalidResponse);
        }
        if existing.completed {
            return if existing.succeeded {
                Ok(None)
            } else if existing.needs_context {
                Err(CompileError::ContextRequired)
            } else {
                Err(error_from_code(existing.failure_code.as_deref()))
            };
        }
        let saved = store.load_compilation_context(project, run_id)?;
        return Ok(Some(PreparedCompilation {
            id: run_id.into(),
            context: CompilationContext {
                trigger: source.clone(),
                interpretation_basis_revision: existing.interpretation_basis_revision,
                source_observation_cutoff: existing.source_observation_cutoff,
                source_window: saved.source_window,
                objects: saved.objects,
                rendered: saved.rendered,
            },
            limits,
            compiler_id: adapter.id().into(),
            compiler_version: adapter.version().into(),
            model_id: adapter.model().into(),
            prompt_digest: adapter.prompt_digest(),
            adapter_config_digest: existing.adapter_config_digest,
        }));
    }
    if mode == RunMode::Replay {
        return Err(CompileError::InvalidResponse);
    }
    let context = if mode == RunMode::Hindsight {
        if let Some(impact) = store.evidence_impact(project, run_id)? {
            build_revalidation_context(store, project, source, &impact, limits)?
        } else {
            build_context_with_basis(
                store,
                project,
                source,
                store.project_revision(project)?,
                limits,
                true,
                SelectorVersion::IssueContextV1,
            )?
        }
    } else {
        build_context(store, project, source, limits)?
    };
    persist_compilation(
        store,
        project,
        source,
        adapter,
        request,
        context,
        ContextVersions {
            renderer: "json_v1",
            selector: "issue_context_v1",
        },
    )
}

/// Persists live on-demand work only after policy accepted the exact compiler request.
///
/// # Errors
/// Rejects missing or mismatched authorization before a compiler run is created.
pub fn prepare_authorized_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    if request.mode != RunMode::Live {
        return Err(CompileError::UnauthorizedCompilation);
    }
    let Some(authorization) = store.compilation_authorization(project, request.id)? else {
        return Err(CompileError::UnauthorizedCompilation);
    };
    if authorization.source != *source
        || authorization.compiler_id != adapter.id()
        || authorization.compiler_version != adapter.version()
        || authorization.model_id != adapter.model()
        || authorization.prompt_digest != adapter.prompt_digest()
        || authorization.adapter_config_digest != adapter.configuration_digest()
        || authorization.limits != request.limits.as_array()
    {
        return Err(CompileError::UnauthorizedCompilation);
    }
    prepare_compilation(store, project, source, adapter, request)
}

/// Persists a new replay attempt with the exact input verified for an earlier run.
///
/// # Errors
/// Rejects changed or missing evidence, incompatible retry identities, and non-replay requests.
pub fn prepare_replay_compilation(
    store: &mut Store,
    project: &ProjectId,
    original_run_id: &str,
    rebuilt: CompilationContext,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    if request.mode != RunMode::Replay || request.id == original_run_id {
        return Err(CompileError::InvalidResponse);
    }
    let original = store
        .compilation_run_status(project, original_run_id)?
        .ok_or(CompileError::NonCausalHistory)?;
    let verified = rebuild_recorded_context(store, project, original_run_id, request.limits)?;
    if rebuilt.trigger != verified.trigger
        || rebuilt.interpretation_basis_revision != verified.interpretation_basis_revision
        || rebuilt.source_observation_cutoff != verified.source_observation_cutoff
        || rebuilt.source_window != verified.source_window
        || rebuilt.objects != verified.objects
        || rebuilt.rendered != verified.rendered
    {
        return Err(CompileError::NonCausalHistory);
    }
    if let Some(existing) = store.compilation_run_status(project, request.id)? {
        if existing.context_digest != original.context_digest
            || existing.renderer_version != original.renderer_version
            || existing.selector_version != original.selector_version
            || existing.interpretation_basis_revision != original.interpretation_basis_revision
            || existing.source_observation_cutoff != original.source_observation_cutoff
        {
            return Err(CompileError::InvalidResponse);
        }
        return prepare_compilation(store, project, &original.source, adapter, request);
    }
    persist_compilation(
        store,
        project,
        &original.source,
        adapter,
        request,
        rebuilt,
        ContextVersions {
            renderer: &original.renderer_version,
            selector: &original.selector_version,
        },
    )
}

fn persist_compilation(
    store: &mut Store,
    project: &ProjectId,
    source: &SourceVersionId,
    adapter: &impl CompilerAdapter,
    request: RunRequest<'_>,
    context: CompilationContext,
    versions: ContextVersions<'_>,
) -> Result<Option<PreparedCompilation>, CompileError> {
    let RunRequest {
        id: run_id,
        limits,
        mode,
        now_millis,
    } = request;
    let intent = CompilationIntent {
        id: run_id,
        source,
        context: &context.rendered,
        source_window: &context.source_window,
        objects: &context.objects,
        interpretation_basis_revision: context.interpretation_basis_revision,
        source_observation_cutoff: context.source_observation_cutoff,
        renderer_version: versions.renderer,
        selector_version: versions.selector,
        compiler_id: adapter.id(),
        compiler_version: adapter.version(),
        model_id: adapter.model(),
        prompt_digest: adapter.prompt_digest(),
        adapter_config_digest: adapter.configuration_digest(),
        mode: mode.as_str(),
        max_input_bytes: limits.input_bytes,
        max_output_bytes: limits.output_bytes,
        max_output_tokens: limits.output_tokens,
        max_assertions: limits.assertions,
        max_context_requests: limits.context_requests,
        max_expansion_rounds: limits.expansion_rounds,
        max_payload_bytes: limits.payload_bytes,
        max_source_window: limits.source_window,
        max_objects: limits.objects,
        started_at_millis: now_millis,
    };
    store.prepare_compilation(project, &intent)?;
    Ok(Some(PreparedCompilation {
        id: run_id.into(),
        context,
        limits,
        compiler_id: adapter.id().into(),
        compiler_version: adapter.version().into(),
        model_id: adapter.model().into(),
        prompt_digest: adapter.prompt_digest(),
        adapter_config_digest: adapter.configuration_digest(),
    }))
}

/// Calls the external compiler without borrowing the project store.
///
/// # Errors
/// Rejects an adapter that does not match the durable run configuration.
pub fn execute_compilation(
    prepared: &PreparedCompilation,
    adapter: &impl CompilerAdapter,
) -> Result<Vec<u8>, CompileError> {
    if prepared.compiler_id != adapter.id()
        || prepared.compiler_version != adapter.version()
        || prepared.model_id != adapter.model()
        || prepared.prompt_digest != adapter.prompt_digest()
        || prepared.adapter_config_digest != adapter.configuration_digest()
    {
        return Err(CompileError::UnauthorizedCompilation);
    }
    adapter.compile(&prepared.context.rendered, prepared.limits)
}

/// Persists a result in a separate transaction after external work completes.
///
/// # Errors
/// A failed adapter or invalid output is recorded as a failed result. No
/// assertion is partially persisted or accepted into project state.
pub fn record_compilation_result(
    store: &mut Store,
    project: &ProjectId,
    prepared: &PreparedCompilation,
    raw: Result<Vec<u8>, CompileError>,
    completed_at_millis: i64,
) -> Result<(), CompileError> {
    if let Some(existing) = store.compilation_run_status(project, &prepared.id)? {
        if existing.completed {
            return if existing.succeeded {
                Ok(())
            } else if existing.needs_context {
                Err(CompileError::ContextRequired)
            } else {
                Err(error_from_code(existing.failure_code.as_deref()))
            };
        }
    } else {
        return Err(CompileError::InvalidResponse);
    }
    let validated = raw.and_then(|bytes| {
        validate_response(&bytes, &prepared.context, prepared.limits)
            .map(|(assertions, needs_context)| (bytes, assertions, needs_context))
    });
    let (response, assertions, needs_context, failure): (
        Option<&[u8]>,
        &[StructuralAssertion],
        bool,
        Option<&str>,
    ) = match &validated {
        Ok((bytes, assertions, needs_context)) => (Some(bytes), assertions, *needs_context, None),
        Err(error) => (None, &[], false, Some(error_code(error))),
    };
    let result = CompilationResult {
        run_id: &prepared.id,
        failure_code: failure,
        response,
        assertions,
        needs_context,
        completed_at_millis,
    };
    store.complete_compilation(project, &result)?;
    match validated {
        Ok((_, _, true)) => Err(CompileError::ContextRequired),
        Ok(_) => Ok(()),
        Err(error) => Err(error),
    }
}

fn error_code(error: &CompileError) -> &'static str {
    match error {
        CompileError::OutputBudget => "output_budget",
        CompileError::Adapter(_) => "adapter_failure",
        CompileError::InputBudget => "input_budget",
        CompileError::NonCausalHistory => "noncausal_history",
        CompileError::MissingEvidence => "missing_evidence",
        CompileError::ContextRequired => "context_required",
        CompileError::UnauthorizedCompilation => "unauthorized_compilation",
        CompileError::UnsupportedReplayVersion(_) => "unsupported_replay_version",
        CompileError::Store(_) | CompileError::InvalidResponse => "invalid_response",
    }
}

fn error_from_code(code: Option<&str>) -> CompileError {
    match code {
        Some("output_budget") => CompileError::OutputBudget,
        Some("adapter_failure") => CompileError::Adapter("previous run failed".into()),
        _ => CompileError::InvalidResponse,
    }
}

fn validate_response(
    bytes: &[u8],
    context: &CompilationContext,
    limits: CompilerLimits,
) -> Result<(Vec<StructuralAssertion>, bool), CompileError> {
    if bytes.len() > limits.output_bytes {
        return Err(CompileError::OutputBudget);
    }
    let response: CompilerResponse =
        serde_json::from_slice(bytes).map_err(|_| CompileError::InvalidResponse)?;
    let rendered: serde_json::Value =
        serde_json::from_slice(&context.rendered).map_err(|_| CompileError::InvalidResponse)?;
    if response.schema != "merl.compiler-response/v1"
        || response.assertions.len() > limits.assertions
        || response.context_required.len() > limits.context_requests
        || (!response.context_required.is_empty() && limits.expansion_rounds == 0)
        || response.relations.len() > limits.assertions.saturating_mul(4)
        || response.relations.iter().any(|relation| {
            !valid_id(&relation.subject)
                || !valid_id(&relation.predicate)
                || !valid_id(&relation.object)
        })
        || response
            .context_required
            .iter()
            .any(|request| !valid_id(&request.reference))
        || response.unresolved.iter().any(|item| {
            !context
                .source_window
                .iter()
                .any(|id| id.as_str() == item.source)
                || (item.span_start != item.span_end
                    && !source_span_valid(&rendered, &item.source, item.span_start, item.span_end))
        })
    {
        return Err(CompileError::InvalidResponse);
    }
    let needs_context = !response.context_required.is_empty();
    let assertions = response
        .assertions
        .into_iter()
        .map(|item| {
            let source = SourceVersionId::try_from(item.source.as_str())
                .map_err(|_| CompileError::InvalidResponse)?;
            if !context.source_window.contains(&source)
                || item.span_start >= item.span_end
                || !source_span_valid(&rendered, &item.source, item.span_start, item.span_end)
                || item.confidence_millis > 1000
                || ![&item.subject, &item.predicate, &item.value]
                    .iter()
                    .all(|value| valid_id(value))
                || item
                    .attributed_to
                    .as_ref()
                    .is_some_and(|value| !valid_id(value))
                || !matches!(
                    item.act.as_str(),
                    "claim" | "propose" | "request" | "ask" | "report"
                )
                || !matches!(
                    item.epistemic_basis.as_str(),
                    "observed" | "inferred" | "reported"
                )
                || !matches!(item.polarity.as_str(), "positive" | "negative")
            {
                return Err(CompileError::InvalidResponse);
            }
            let asserted_by = rendered["sources"]
                .as_array()
                .and_then(|sources| {
                    sources
                        .iter()
                        .find(|candidate| candidate["id"] == item.source)
                })
                .and_then(|source| source["source_author_id"].as_str())
                .map(str::to_owned);
            Ok(StructuralAssertion {
                source,
                span_start: item.span_start,
                span_end: item.span_end,
                subject: item.subject,
                predicate: item.predicate,
                value: item.value,
                act: item.act,
                epistemic_basis: item.epistemic_basis,
                polarity: item.polarity,
                confidence_millis: item.confidence_millis,
                asserted_by,
                attributed_to: item.attributed_to,
                attribution_verified: false,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((assertions, needs_context))
}

fn source_span_valid(rendered: &serde_json::Value, source: &str, start: usize, end: usize) -> bool {
    rendered["sources"].as_array().is_some_and(|sources| {
        sources.iter().any(|item| {
            item["id"].as_str() == Some(source)
                && item["body"]
                    .as_str()
                    .is_some_and(|body| start < end && body.get(start..end).is_some())
        })
    })
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::{
        CompilationContext, CompileError, CompilerLimits, ProcessCompiler,
        process_configuration_digest, validate_response,
    };
    use merl_core::{ProjectRevision, SourceVersionId};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMPORARY_COMPILER_ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn prose_fields_and_out_of_range_spans_cannot_enter_assertions() {
        let context = CompilationContext {
            trigger: SourceVersionId::try_from("s1").expect("source"),
            interpretation_basis_revision: ProjectRevision::initial(),
            source_observation_cutoff: 1,
            source_window: vec![SourceVersionId::try_from("s1").expect("source")],
            objects: Vec::new(),
            rendered: br#"{"sources":[{"id":"s1","body":"fixed gain"}]}"#.to_vec(),
        };
        let limits = CompilerLimits {
            input_bytes: 1024,
            output_bytes: 1024,
            output_tokens: 100,
            assertions: 2,
            context_requests: 1,
            expansion_rounds: 1,
            payload_bytes: 1024,
            source_window: 1,
            objects: 1,
        };
        let valid = serde_json::json!({"schema":"merl.compiler-response/v1", "assertions":[{
            "source":"s1", "span_start":0, "span_end":5, "subject":"capture",
            "predicate":"gain", "value":"fixed", "act":"report",
            "epistemic_basis":"observed", "polarity":"positive", "confidence_millis":900,
            "attributed_to":null
        }]});
        assert_eq!(
            validate_response(&serde_json::to_vec(&valid).expect("JSON"), &context, limits)
                .expect("valid assertion")
                .0
                .len(),
            1
        );
        for invalid in [
            {
                let mut value = valid.clone();
                value["rationale"] = serde_json::json!("essay");
                value
            },
            {
                let mut value = valid.clone();
                value["assertions"][0]["attribution_verified"] = serde_json::json!(true);
                value
            },
            {
                let mut value = valid.clone();
                value["assertions"][0]["span_end"] = serde_json::json!(500);
                value
            },
        ] {
            assert!(matches!(
                validate_response(
                    &serde_json::to_vec(&invalid).expect("JSON"),
                    &context,
                    limits
                ),
                Err(CompileError::InvalidResponse)
            ));
        }
    }

    #[test]
    fn process_identity_changes_with_the_executable_or_fixed_arguments() {
        let base =
            process_configuration_digest(b"compiler-a", &["--strict".into()]).expect("base digest");
        let other_artifact = process_configuration_digest(b"compiler-b", &["--strict".into()])
            .expect("artifact digest");
        let other_arguments = process_configuration_digest(b"compiler-a", &["--fast".into()])
            .expect("argument digest");

        assert_ne!(base, other_artifact);
        assert_ne!(base, other_arguments);
    }

    #[test]
    fn process_adapter_rejects_an_executable_replaced_after_construction() {
        let id = TEMPORARY_COMPILER_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "merl-compiler-configuration-{}-{id}",
            std::process::id()
        ));
        std::fs::write(&path, b"compiler-a").expect("initial compiler");
        let compiler =
            ProcessCompiler::new(path.to_string_lossy(), Vec::new(), "v1", "model-a", [1; 32])
                .expect("process compiler");
        std::fs::write(&path, b"compiler-b").expect("replacement compiler");

        let result = compiler.verify_configuration();

        let _ = std::fs::remove_file(path);
        assert!(matches!(result, Err(CompileError::UnauthorizedCompilation)));
    }
}
