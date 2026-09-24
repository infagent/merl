//! Inspection and administration of defaults for an existing source binding.

use crate::{CliError, invalid_input, parse_project, render_json};
use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, PolicyInputId,
    SourceBindingId,
};
use merl_policy::{BindingPolicyChange, change_binding_policy};
use merl_store::Store;
use serde_json::json;
use std::path::Path;

#[derive(Clone, Copy, Default)]
pub(super) struct Options<'a> {
    pub database: Option<&'a str>,
    pub project: Option<&'a str>,
    pub id: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub expected: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub coverage: Option<&'a str>,
}

pub(super) fn execute(
    binding: &str,
    set: bool,
    options: Options<'_>,
    json_output: bool,
    now: i64,
) -> Result<String, CliError> {
    let project = parse_project(required(options.project, "--project")?)?;
    let binding = SourceBindingId::try_from(binding).map_err(|e| invalid_input(&e.to_string()))?;
    let mut store = Store::open(Path::new(required(options.database, "--database")?))?;
    if !set {
        let state = store.binding_policy(&project, &binding)?;
        let change = state.change.as_ref().map(|object| {
            let evaluation = store.object_policy_evaluation(&project, &object.id)?.ok_or(merl_store::StoreError::CorruptHistory)?;
            let evaluation = store.policy_evaluation(&project, &evaluation)?.ok_or(merl_store::StoreError::CorruptHistory)?;
            Ok::<_, merl_store::StoreError>(json!({"object":object.id.as_str(),"revision":object.project_revision.get(),"actor":evaluation.actor.as_str(),"evaluation":evaluation.id.as_str(),"authorization_policy":evaluation.version.as_str(),"reason":object.payload.as_ref().map(merl_core::PayloadId::as_str)}))
        }).transpose()?;
        return if json_output {
            render_json(
                &json!({"schema":"merl.binding-policy/v1","action":"source.compilation-policy","project":project.as_str(),"binding":binding.as_str(),"policy":{"mode":state.policy.mode.as_str(),"coverage":state.policy.coverage.as_str(),"version":state.policy.version.as_str()},"change":change}),
            )
        } else {
            let provenance = change.map_or_else(
                || "Established by initial capture.".to_owned(),
                |change| {
                    format!(
                        "Changed by {} at revision {}; evaluation {}; reason {}.",
                        change["actor"].as_str().unwrap_or(""),
                        change["revision"],
                        change["evaluation"].as_str().unwrap_or(""),
                        change["reason"].as_str().unwrap_or("")
                    )
                },
            );
            Ok(format!(
                "Binding {binding}: {}, {}, version {}.\n{provenance}\n",
                state.policy.mode.as_str(),
                state.policy.coverage.as_str(),
                state.policy.version
            ))
        };
    }
    let change = BindingPolicyChange {
        id: PolicyInputId::try_from(required(options.id, "--id")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        actor: ActorId::try_from(required(options.actor, "--actor")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        binding,
        expected_version: CapturePolicyVersion::try_from(required(
            options.expected,
            "--expected-version",
        )?)
        .map_err(|e| invalid_input(&e.to_string()))?,
        mode: match required(options.mode, "--mode")? {
            "capture_only" => CompilationMode::CaptureOnly,
            "on_demand" => CompilationMode::OnDemand,
            "eager" => CompilationMode::Eager,
            _ => {
                return Err(invalid_input(
                    "--mode must be capture_only, on_demand, or eager",
                ));
            }
        },
        coverage: match required(options.coverage, "--coverage")? {
            "optional" => CoverageRequirement::Optional,
            "required" => CoverageRequirement::Required,
            _ => return Err(invalid_input("--coverage must be optional or required")),
        },
        reason: required(options.reason, "--reason")?.to_owned(),
    };
    let record = change_binding_policy(&mut store, &project, &change, now)?;
    let outcome = record.inputs[0].disposition.as_str();
    if json_output {
        render_json(
            &json!({"schema":"merl.binding-policy-change/v1","action":"source.compilation-policy.set","project":project.as_str(),"binding":change.binding.as_str(),"request":change.id.as_str(),"actor":change.actor.as_str(),"outcome":outcome,"reason_code":record.inputs[0].reason.as_str(),"revision":record.committed_revision.map(merl_core::ProjectRevision::get),"evaluation":record.id.as_str(),"authorization_policy":record.version.as_str()}),
        )
    } else {
        Ok(format!(
            "{outcome}: binding {} policy change by {}; {}; accepted revision {}.\n",
            change.binding,
            change.actor,
            record.inputs[0].reason,
            record
                .committed_revision
                .map_or_else(|| "none".to_owned(), |r| r.get().to_string())
        ))
    }
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, CliError> {
    value.ok_or_else(|| invalid_input(&format!("{name} is required")))
}

pub(super) fn help(set: bool, json_output: bool) -> Result<String, CliError> {
    let (command, usage, summary) = if set {
        (
            "source compilation-policy set",
            "merl source compilation-policy set <binding> --project <id> --database <path> --id <request> --actor <administrator> --expected-version <version> --mode <capture_only|on_demand|eager> --coverage <optional|required> --reason <text> [--json]",
            "Change defaults for future captures. Earlier versions and compiler intents keep their policy; use source require or source compile for explicit historical work.",
        )
    } else {
        (
            "source compilation-policy",
            "merl source compilation-policy <binding> --project <id> --database <path> [--json]",
            "Inspect the effective policy version and accepted change. Use the binding ID returned by issue capture; set changes this binding's defaults.",
        )
    };
    if json_output {
        render_json(
            &json!({"schema":"merl.help/v1","command":command,"usage":usage,"summary":summary,"related":["source compilation-policy set","issue capture","source require","source compile"],"outcomes":["accepted","rejected","conflict"],"errors":["INVALID_INPUT","BINDING_NOT_FOUND","POLICY_INPUT_CONFLICT","POLICY_CONFLICT","STORAGE_ERROR"]}),
        )
    } else {
        Ok(format!("{usage}\n\n{summary}\n"))
    }
}
