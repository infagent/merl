//! Human and machine output for recorded compiler assertions and their policy outcomes.

use crate::{CliError, assertion_json, invalid_input};
use merl_core::{CompilationRunId, PolicyInput, ProjectId};
use merl_store::{PayloadRead, RecordedPolicyEvaluation, Store, StoreError};
use serde_json::json;

pub(super) fn inspect(
    store: &Store,
    project: &ProjectId,
    run: &CompilationRunId,
    json_output: bool,
) -> Result<String, CliError> {
    let status = store
        .compilation_run_status(project, run.as_str())?
        .ok_or_else(|| invalid_input("compiler run does not exist"))?;
    let assertions = store.observed_assertions(project, run.as_str())?;
    let mut details = Vec::with_capacity(assertions.len());
    for (index, assertion) in assertions.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| StoreError::CorruptHistory)?;
        let mut detail = assertion_json(store, project, run, index, false)?;
        detail["act"] = json!(assertion.act);
        detail["epistemic_basis"] = json!(assertion.epistemic_basis);
        detail["polarity"] = json!(assertion.polarity);
        detail["confidence_millis"] = json!(assertion.confidence_millis);
        detail["accepted"] = json!(store.accepted_assertion(project, run, index)?);
        details.push(detail);
    }
    let response = store.compilation_response(project, run)?;
    let retained = matches!(response, Some(PayloadRead::Available(_)));
    let response: Option<merl_compiler::CompilerResponse> = match response {
        Some(PayloadRead::Available(bytes)) => {
            Some(serde_json::from_slice(&bytes).map_err(|_| StoreError::CorruptHistory)?)
        }
        _ => None,
    };
    let result = json!({"schema":"merl.assertions/v1", "project":project.as_str(), "run":run.as_str(),
        "mode":status.mode, "completed":status.completed, "succeeded":status.succeeded,
        "needs_context":status.needs_context, "failure_code":status.failure_code,
        "response_available":retained, "assertions":details,
        "unresolved":response.as_ref().map(|r| &r.unresolved),
        "context_required":response.as_ref().map(|r| &r.context_required),
        "deferred_relations":response.as_ref().map(|r| &r.relations)});
    if json_output {
        return Ok(format!("{result}\n"));
    }
    let mut lines = vec![format!(
        "Run {run}: {} assertions, mode {}, completed {}",
        assertions.len(),
        status.mode,
        status.completed
    )];
    for (index, assertion) in assertions.iter().enumerate() {
        lines.push(format!(
            "{index}: {} {} ({}, {}, {})",
            assertion.subject,
            assertion.predicate,
            assertion.act,
            assertion.epistemic_basis,
            assertion.polarity
        ));
    }
    if let Some(response) = response {
        lines.push(format!(
            "Unresolved spans: {}; context requests: {}; deferred relations: {}",
            response.unresolved.len(),
            response.context_required.len(),
            response.relations.len()
        ));
    } else {
        lines.push("Compiler response unavailable".into());
    }
    Ok(format!("{}\n", lines.join("\n")))
}

pub(super) fn render_application(
    project: &ProjectId,
    run: &CompilationRunId,
    result: Option<&RecordedPolicyEvaluation>,
    json_output: bool,
) -> Result<String, CliError> {
    let mut inputs = Vec::new();
    if let Some(result) = result {
        for input in &result.inputs {
            let PolicyInput::ObservedAssertion { index, .. } = &input.input else {
                return Err(StoreError::CorruptHistory.into());
            };
            inputs.push(json!({"index":index, "input":input.input.id().as_str(), "outcome":input.disposition.as_str(), "reason":input.reason.as_str()}));
        }
    }
    let value = json!({"schema":"merl.assertion-application/v1", "project":project.as_str(), "run":run.as_str(),
        "evaluation":result.map(|r| r.id.as_str()), "revision":result.and_then(|r| r.committed_revision).map(merl_core::ProjectRevision::get),
        "basis_project_revision":result.map(|r| r.basis_project_revision.get()),
        "policy_version":result.map(|r| r.version.as_str()),
        "configuration_digest":result.map(|r| crate::digest_text(&r.configuration_digest)),
        "conflict":result.and_then(|r| r.conflict.as_ref()).map(|c| json!({"reason":c.reason_code,"target":c.target_id,"expected_revision":c.expected_revision,"actual_revision":c.actual_revision})),
        "inputs":inputs});
    if json_output {
        return Ok(format!("{value}\n"));
    }
    let mut lines = vec![format!("Run {run}: {} assertion outcomes", inputs.len())];
    for input in inputs {
        lines.push(format!(
            "{}: {} ({})",
            input["index"],
            input["outcome"].as_str().unwrap_or_default(),
            input["reason"].as_str().unwrap_or_default()
        ));
    }
    if let Some(revision) = result.and_then(|r| r.committed_revision) {
        lines.push(format!("Accepted at {project}@{}", revision.get()));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

/// Renders a current-policy preview without recording an evaluation or inbox entry.
pub(super) fn render_preview(
    project: &ProjectId,
    run: &CompilationRunId,
    prepared: Option<&merl_policy::PreparedPolicy>,
    json_output: bool,
) -> Result<String, CliError> {
    let evaluation = prepared.map(|p| &p.evaluation);
    let mut inputs = Vec::new();
    if let Some(evaluation) = evaluation {
        for input in &evaluation.inputs {
            let PolicyInput::ObservedAssertion { index, .. } = &input.input else {
                return Err(StoreError::CorruptHistory.into());
            };
            inputs.push(json!({"index":index, "input":input.input.id().as_str(), "outcome":input.disposition.as_str(), "reason":input.reason.as_str()}));
        }
    }
    let value = json!({"schema":"merl.assertion-preview/v1", "project":project.as_str(), "run":run.as_str(),
        "basis_project_revision":evaluation.map(|e| e.basis_project_revision.get()),
        "policy_version":evaluation.map(|e| e.version.as_str()),
        "configuration_digest":evaluation.map(|e| crate::digest_text(&e.configuration_digest)),
        "inputs":inputs,
        "objects":evaluation.and_then(|e| e.batch.as_ref()).map(|b| b.events.iter().filter_map(|e| match e {
            merl_core::DomainEvent::PutObject { object, .. } => Some(object.as_str()),
            merl_core::DomainEvent::PutRelation { .. } | merl_core::DomainEvent::ResolveSupport { .. } => None,
        }).collect::<Vec<_>>()).unwrap_or_default()});
    if json_output {
        return Ok(format!("{value}\n"));
    }
    let mut lines = vec![format!(
        "Preview for {run}; no evaluation or state committed"
    )];
    for input in inputs {
        lines.push(format!(
            "{}: {} ({})",
            input["index"],
            input["outcome"].as_str().unwrap_or_default(),
            input["reason"].as_str().unwrap_or_default()
        ));
    }
    Ok(format!("{}\n", lines.join("\n")))
}
