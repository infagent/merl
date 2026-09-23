//! Bounded successors preserve the original interpretation basis and spend policy.

use super::{
    CompilationContext, CompileError, CompilerAdapter, CompilerLimits, ContextVersions,
    PreparedCompilation, RunMode, RunRequest, persist_compilation, prepare_compilation,
    rebuild_recorded_context,
};
use merl_core::{CompilationRunId, ObjectId, ProjectId, SourceVersionId};
use merl_store::{ExpansionWork, PayloadRead, Store, StoreError};
use serde_json::{Value, json};

/// Prepares one committed context request using its original compiler and limits.
///
/// The parent response reserves the successor identity. Retries load the same
/// intent; they cannot add references, change the compiler, or buy more rounds.
/// `None` means the successor already completed successfully.
///
/// # Errors
/// Rejects changed compiler identity, unavailable references, repeated requests,
/// exhausted budgets, and erased evidence. Reference and budget failures remain
/// inspectable after restart without changing the parent's immutable result.
pub fn prepare_context_expansion(
    store: &mut Store,
    project: &ProjectId,
    parent: &str,
    adapter: &impl CompilerAdapter,
    now_millis: i64,
) -> Result<Option<PreparedCompilation>, CompileError> {
    let work = store
        .expansion_work(project, parent)?
        .ok_or(CompileError::InvalidResponse)?;
    let original = store
        .compilation_run_status(project, parent)?
        .ok_or(CompileError::InvalidResponse)?;
    if original.compiler_id != adapter.id()
        || original.compiler_version != adapter.version()
        || original.model_id != adapter.model()
        || original.prompt_digest != adapter.prompt_digest()
        || original.adapter_config_digest != adapter.configuration_digest()
    {
        return Err(CompileError::UnauthorizedCompilation);
    }
    if let Some(code) = work.failure_code.as_deref() {
        return Err(expansion_error(code));
    }
    let mode = match original.mode.as_str() {
        "live" => RunMode::Live,
        "replay" => RunMode::Replay,
        "eval" => RunMode::Eval,
        "hindsight" => RunMode::Hindsight,
        _ => return Err(CompileError::InvalidResponse),
    };
    let limits = CompilerLimits::from_array(original.limits);
    let request = RunRequest {
        id: &work.child_run,
        limits,
        mode,
        now_millis,
    };
    if store
        .compilation_run_status(project, &work.child_run)?
        .is_some()
    {
        return prepare_compilation(store, project, &original.source, adapter, request);
    }
    let result = (|| {
        let saved = store
            .load_compilation_context(project, parent)
            .map_err(context_error)?;
        let basis = CompilationContext {
            trigger: original.source.clone(),
            interpretation_basis_revision: original.interpretation_basis_revision,
            source_observation_cutoff: original.source_observation_cutoff,
            source_window: saved.source_window,
            objects: saved.objects,
            rendered: saved.rendered,
        };
        expand(store, project, &work, basis, limits)
    })();
    let context = match result {
        Ok(context) => context,
        Err(
            error @ (CompileError::InputBudget
            | CompileError::MissingEvidence
            | CompileError::ExpansionReference
            | CompileError::ExpansionLoop),
        ) => {
            store.fail_expansion(project, parent, super::error_code(&error))?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    persist_compilation(
        store,
        project,
        &original.source,
        adapter,
        request,
        context,
        ContextVersions {
            replay_of: None,
            renderer: "json_v1",
            selector: "context_expansion_v1",
        },
    )
}

fn expansion_error(code: &str) -> CompileError {
    match code {
        "expansion_round_budget" => CompileError::ExpansionRoundBudget,
        "expansion_reference" => CompileError::ExpansionReference,
        "expansion_loop" => CompileError::ExpansionLoop,
        "input_budget" => CompileError::InputBudget,
        "missing_evidence" => CompileError::MissingEvidence,
        other => super::error_from_code(Some(other)),
    }
}

pub(super) fn rebuild(
    store: &Store,
    project: &ProjectId,
    run: &str,
    limits: CompilerLimits,
) -> Result<CompilationContext, CompileError> {
    if let Some(original) = store.compilation_replay_origin(project, run)? {
        let rebuilt = rebuild_recorded_context(store, project, &original, limits)?;
        let saved = store
            .load_compilation_context(project, run)
            .map_err(context_error)?;
        if rebuilt.rendered != saved.rendered {
            return Err(CompileError::NonCausalHistory);
        }
        return Ok(rebuilt);
    }
    let parent = store
        .expansion_parent(project, run)?
        .ok_or(CompileError::NonCausalHistory)?;
    let work = store
        .expansion_work(project, &parent)?
        .ok_or(CompileError::NonCausalHistory)?;
    let basis = rebuild_recorded_context(store, project, &parent, limits)?;
    let rebuilt = expand(store, project, &work, basis, limits)?;
    let saved = store
        .load_compilation_context(project, run)
        .map_err(context_error)?;
    if rebuilt.rendered != saved.rendered {
        return Err(CompileError::NonCausalHistory);
    }
    Ok(rebuilt)
}

fn expand(
    store: &Store,
    project: &ProjectId,
    work: &ExpansionWork,
    mut basis: CompilationContext,
    limits: CompilerLimits,
) -> Result<CompilationContext, CompileError> {
    let parent_id = CompilationRunId::try_from(work.parent_run.as_str())
        .map_err(|_| CompileError::InvalidResponse)?;
    if !matches!(
        store.compilation_response(project, &parent_id)?,
        Some(PayloadRead::Available(_))
    ) {
        return Err(CompileError::MissingEvidence);
    }
    let mut rendered: Value =
        serde_json::from_slice(&basis.rendered).map_err(|_| CompileError::InvalidResponse)?;
    let mut seen = std::collections::BTreeSet::new();
    for reference in &work.references {
        if !seen.insert(reference)
            || basis.objects.iter().any(|(id, _)| id.as_str() == reference)
            || basis
                .source_window
                .iter()
                .any(|id| id.as_str() == reference)
        {
            return Err(CompileError::ExpansionLoop);
        }
        let object =
            ObjectId::try_from(reference.as_str()).map_err(|_| CompileError::ExpansionReference)?;
        let version = SourceVersionId::try_from(reference.as_str())
            .map_err(|_| CompileError::ExpansionReference)?;
        let selected = store.compiler_object_at_revision(
            project,
            &object,
            basis.interpretation_basis_revision,
        )?;
        let source = store.source_version(project, &version)?;
        if selected.is_some() && source.is_some() {
            return Err(CompileError::ExpansionReference);
        }
        if let Some((revision, payload)) = selected {
            let body = payload
                .map(|id| match store.read_payload(project, &id)? {
                    PayloadRead::Available(bytes) => {
                        String::from_utf8(bytes).map_err(|_| CompileError::InvalidResponse)
                    }
                    PayloadRead::Unavailable => Err(CompileError::MissingEvidence),
                })
                .transpose()?;
            rendered["objects"]
                .as_array_mut()
                .ok_or(CompileError::InvalidResponse)?
                .push(json!({"id":reference,"revision":revision.get(),"body":body}));
            basis.objects.push((object, revision));
        } else if let Some(source) = source {
            if source.payload.is_none()
                || source.sequence > basis.source_observation_cutoff
                || source.ambiguous_order_with_previous
                || source.kind.as_str() == "generated_projection"
            {
                return Err(CompileError::ExpansionReference);
            }
            let body = super::source_context_body(store, project, &source, false)?;
            let body = String::from_utf8(body).map_err(|_| CompileError::InvalidResponse)?;
            rendered["sources"].as_array_mut().ok_or(CompileError::InvalidResponse)?.push(json!({
                "id":reference,"observation":source.sequence,"source_author_id":source.source_author.map(|id|id.to_string()),
                "version_actor_id":source.version_actor.map(|id|id.to_string()),"created_at_millis":source.created_at_millis,
                "occurred_at_millis":source.occurred_at_millis,"body":body
            }));
            if let Some(origin) = super::render_command_origin(store, project, &version)? {
                let sources = rendered["sources"]
                    .as_array_mut()
                    .ok_or(CompileError::InvalidResponse)?;
                sources.last_mut().ok_or(CompileError::InvalidResponse)?["semantic_origin"] =
                    serde_json::to_value(origin).map_err(|_| CompileError::InvalidResponse)?;
            }
        } else {
            return Err(CompileError::ExpansionReference);
        }
    }
    let sources = rendered["sources"]
        .as_array_mut()
        .ok_or(CompileError::InvalidResponse)?;
    sources.sort_by_key(|s| s["observation"].as_u64().unwrap_or(0));
    basis.source_window = sources
        .iter()
        .map(|s| {
            SourceVersionId::try_from(s["id"].as_str().unwrap_or_default())
                .map_err(|_| CompileError::InvalidResponse)
        })
        .collect::<Result<_, _>>()?;
    if basis.objects.len() > limits.objects || basis.source_window.len() > limits.source_window {
        return Err(CompileError::InputBudget);
    }
    let payload_bytes = ["sources", "objects"]
        .iter()
        .flat_map(|key| rendered[key].as_array().into_iter().flatten())
        .try_fold(0usize, |sum, item| {
            sum.checked_add(item["body"].as_str().map_or(0, str::len))
        })
        .ok_or(CompileError::InputBudget)?;
    basis.rendered = serde_json::to_vec(&rendered).map_err(|_| CompileError::InvalidResponse)?;
    if payload_bytes > limits.payload_bytes || basis.rendered.len() > limits.input_bytes {
        return Err(CompileError::InputBudget);
    }
    Ok(basis)
}

fn context_error(error: StoreError) -> CompileError {
    match error {
        StoreError::InvalidCompilation => CompileError::MissingEvidence,
        other => CompileError::Store(other),
    }
}
