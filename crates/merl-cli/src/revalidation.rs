//! Public evidence work keeps reinterpretation separate from accepted support.

use crate::{CliError, invalid_input, parse_project};
use merl_store::Store;
use serde_json::json;
use std::path::Path;

#[derive(Clone, Copy, Default)]
pub(super) struct Options<'a> {
    pub project: Option<&'a str>,
    pub database: Option<&'a str>,
    pub after: Option<&'a str>,
    pub id: Option<&'a str>,
    pub impact: Option<&'a str>,
    pub action: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub run: Option<&'a str>,
    pub assertion_index: Option<&'a str>,
    pub program: Option<&'a str>,
    pub compiler_version: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt_digest: Option<&'a str>,
    pub compiler_args: &'a [String],
}

pub(super) fn execute(
    operation: &str,
    options: Options<'_>,
    json_output: bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    let project = parse_project(
        options
            .project
            .ok_or_else(|| invalid_input("--project is required"))?,
    )?;
    let mut store = Store::open(Path::new(
        options
            .database
            .ok_or_else(|| invalid_input("--database is required"))?,
    ))?;
    if operation == "run" || operation == "resolve" {
        let value = if operation == "run" {
            run(&mut store, &project, &options, clock)?
        } else {
            resolve(&mut store, &project, &options, clock()?)?
        };
        return if json_output {
            Ok(format!("{value}\n"))
        } else if operation == "run" {
            Ok(format!(
                "Run {}: {} (hindsight)\n",
                value["run"].as_str().unwrap_or_default(),
                value["outcome"].as_str().unwrap_or_default()
            ))
        } else {
            Ok(format!(
                "{} {}: {} ({})\n",
                value["disposition"].as_str().unwrap_or_default(),
                value["impact"].as_str().unwrap_or_default(),
                value["action"].as_str().unwrap_or_default(),
                value["reason"].as_str().unwrap_or_default()
            ))
        };
    }
    if operation != "list" {
        return Err(invalid_input("unknown revalidation command"));
    }
    let (impacts, more) = store.evidence_impact_page(&project, options.after)?;
    let work = impacts.iter().map(|i| {
        let run=store.latest_revalidation_attempt(&project,&i.id)?;
        let status=store.compilation_run_status(&project,run.as_ref().map_or(i.id.as_str(),merl_core::CompilationRunId::as_str))?;
        Ok(json!({"impact":i.id,"object":i.object.as_str(),"support_event":i.support_event.as_str(),"affected_run":i.affected_run.as_str(),"trigger":i.trigger.as_str(),"changed_source":i.changed_source.as_str(),"replacement":i.replacement.as_ref().map(merl_core::SourceVersionId::as_str),"next_action":i.next_action,"evidence_unavailable":store.revalidation_evidence_unavailable(&project,&i.id)?,"latest_attempt":run.as_ref().map(merl_core::CompilationRunId::as_str),"run_outcome":status.map(|s|if !s.completed {"pending"} else if s.needs_context {"needs_context"} else if s.succeeded {"succeeded"} else {"failed"})}))
    }).collect::<Result<Vec<_>,CliError>>()?;
    if json_output {
        return Ok(format!(
            "{}\n",
            json!({"schema":"merl.revalidation-work/v1","project":project.as_str(),"work":work,"next_after":more.then(||impacts.last().map(|i|i.id.as_str())).flatten()})
        ));
    }
    Ok(format!(
        "{}\n",
        impacts
            .iter()
            .map(|i| format!("{}: {} {}", i.id, i.object, i.next_action))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn required<'a>(value: Option<&'a str>, message: &str) -> Result<&'a str, CliError> {
    value.ok_or_else(|| invalid_input(message))
}
fn identifier<T: for<'a> TryFrom<&'a str>>(value: &str) -> Result<T, CliError> {
    T::try_from(value).map_err(|_| invalid_input("invalid structural identity"))
}

fn run(
    store: &mut Store,
    project: &merl_core::ProjectId,
    options: &Options<'_>,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<serde_json::Value, CliError> {
    let actor = identifier(required(options.actor, "--actor is required")?)?;
    if !merl_policy::PolicyRules::from_store(store, project)?
        .command_actors
        .contains(&actor)
    {
        return Err(merl_compiler::CompileError::UnauthorizedCompilation.into());
    }
    let impact = store
        .evidence_impact(project, required(options.id, "--id is required")?)?
        .ok_or_else(|| invalid_input("evidence impact does not exist"))?;
    let prior = store
        .compilation_run_status(project, impact.affected_run.as_str())?
        .ok_or_else(|| invalid_input("affected run does not exist"))?;
    let adapter = merl_compiler::ProcessCompiler::new(
        required(options.program, "--program is required")?,
        options.compiler_args.to_vec(),
        required(options.compiler_version, "--compiler-version is required")?,
        required(options.model, "--model is required")?,
        crate::parse_sha256(required(
            options.prompt_digest,
            "--prompt-digest is required",
        )?)?,
    )?;
    let run = identifier::<merl_core::CompilationRunId>(options.run.unwrap_or(&impact.id))?;
    let source = match store.compilation_run_status(project, run.as_str())? {
        Some(saved) => saved.source,
        None => store.latest_source_version(project, &impact.trigger)?,
    };
    store.record_revalidation_attempt(project, &impact.id, &run)?;
    let work = match merl_compiler::prepare_hindsight_compilation(
        store,
        project,
        &source,
        &adapter,
        merl_compiler::RunRequest {
            id: run.as_str(),
            limits: merl_compiler::CompilerLimits::from_array(prior.limits),
            mode: merl_compiler::RunMode::Hindsight,
            now_millis: clock()?,
        },
    ) {
        Ok(work) => work,
        Err(merl_compiler::CompileError::ContextRequired) => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(work) = work {
        let response = merl_compiler::execute_compilation(&work, &adapter);
        match merl_compiler::record_compilation_result(store, project, &work, response, clock()?) {
            Ok(()) | Err(merl_compiler::CompileError::ContextRequired) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let status = store
        .compilation_run_status(project, run.as_str())?
        .ok_or_else(|| invalid_input("missing run"))?;
    Ok(
        json!({"schema":"merl.revalidation-run/v1","impact":impact.id,"run":run.as_str(),"mode":status.mode,"outcome":if status.succeeded {"succeeded"} else if status.needs_context {"needs_context"} else {"failed"}}),
    )
}

fn resolve(
    store: &mut Store,
    project: &merl_core::ProjectId,
    options: &Options<'_>,
    now: i64,
) -> Result<serde_json::Value, CliError> {
    let review = merl_store::RevalidationReview {
        id: identifier(required(options.id, "--id is required")?)?,
        impact: required(options.impact, "--impact is required")?.into(),
        actor: identifier(required(options.actor, "--actor is required")?)?,
        action: merl_store::RevalidationAction::try_from(required(
            options.action,
            "--action is required",
        )?)
        .map_err(|_| {
            invalid_input("action must be confirm, weaken, supersede, invalidate, or unavailable")
        })?,
        run: options.run.map(identifier).transpose()?,
        assertion_index: options
            .assertion_index
            .map(|s| {
                s.parse::<u32>()
                    .map_err(|_| invalid_input("invalid assertion index"))
            })
            .transpose()?,
    };
    let result = merl_policy::resolve_revalidation(store, project, &review, now)?;
    Ok(
        json!({"schema":"merl.revalidation-resolution/v1","impact":review.impact,"request":review.id.as_str(),"action":review.action.as_str(),"evaluation":result.id.as_str(),"revision":result.committed_revision.map(merl_core::ProjectRevision::get),"disposition":result.inputs[0].disposition.as_str(),"reason":result.inputs[0].reason.as_str()}),
    )
}

pub(super) fn lineage(
    store: &Store,
    project: &merl_core::ProjectId,
    review: &merl_store::RevalidationReview,
    expand_source: bool,
) -> Result<serde_json::Value, CliError> {
    let impact = store
        .evidence_impact(project, &review.impact)?
        .ok_or_else(|| invalid_input("missing impact"))?;
    let mut value = json!({"request":review.id.as_str(),"actor":review.actor.as_str(),"action":review.action.as_str(),"impact":impact.id,"support_event":impact.support_event.as_str(),"affected_run":impact.affected_run.as_str(),"changed_source":impact.changed_source.as_str(),"replacement":impact.replacement.as_ref().map(merl_core::SourceVersionId::as_str),"resolved_by":impact.revalidated_by.as_ref().map(merl_core::EventId::as_str),"run":review.run.as_ref().map(merl_core::CompilationRunId::as_str)});
    if let Some(run) = &review.run {
        value["context_rounds"] = crate::compilations::lineage(store, project, run.as_str())?;
        if let Some(index) = review.assertion_index {
            value["assertion"] = crate::assertion_json(store, project, run, index, expand_source)?;
        }
    }
    Ok(value)
}

// History expands structural review lineage only. Source bodies remain behind
// the explicit run and source commands, even when an object has many reviews.
pub(super) fn history(
    store: &Store,
    project: &merl_core::ProjectId,
    object: &merl_core::ObjectId,
) -> Result<Vec<serde_json::Value>, CliError> {
    store
        .object_revalidation_reviews(project, object)?
        .iter()
        .map(|review| lineage(store, project, review, false))
        .collect()
}

pub(super) fn help(operation: Option<&str>, json_output: bool) -> Result<String, CliError> {
    let (command, usage, summary) = match operation {
        None => (
            "project revalidation",
            "merl project revalidation <list|run|resolve>",
            "Inspect changed evidence, recompile its recorded derivation, and review the resulting support.",
        ),
        Some("list") => (
            "project revalidation list",
            "merl project revalidation list --project <id> --database <path> [--after <impact>] [--json]",
            "List up to 100 pending impacts, with the affected object, support event, and compiler trigger.",
        ),
        Some("run") => (
            "project revalidation run",
            "merl project revalidation run --project <id> --database <path> --id <impact> --actor <id> [--run <new-attempt>] --program <path> --compiler-version <version> --model <id> --prompt-digest sha256:<hex> [--compiler-arg <arg>] [--json]",
            "Record hindsight work with the affected run's limits and latest versions of its recorded sources. Retry resumes saved input; a new run ID starts another attempt. Requires command_actor.",
        ),
        Some("resolve") => (
            "project revalidation resolve",
            "merl project revalidation resolve --project <id> --database <path> --id <request> --impact <id> --actor <id> --action <confirm|weaken|supersede|invalidate|unavailable> [--run <id>] [--assertion-index <n>] [--json]",
            "Review under current command authority. Confirm and supersede require a completed run and assertion index; weaken and invalidate require a completed run. Unavailable requires erased or bodyless evidence and takes no run. Exact retries return the recorded result.",
        ),
        _ => return Err(invalid_input("unknown revalidation help topic")),
    };
    if json_output {
        Ok(format!(
            "{}\n",
            json!({"schema":"merl.help/v1","command":command,"usage":usage,"summary":summary,"related":["compilation show","compilation expand","source assertions","show"],"errors":["INVALID_INPUT","COMPILER_UNAUTHORIZED","REVALIDATION_RUN_INELIGIBLE","POLICY_INPUT_CONFLICT","MISSING_EVIDENCE"]})
        ))
    } else {
        Ok(format!("{usage}\n{summary}\n"))
    }
}
