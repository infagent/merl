//! Inspect and resume bounded compiler work without accepting semantic state.

use crate::{CliError, invalid_input, parse_project, parse_sha256};
use merl_core::ProjectId;
use merl_store::Store;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) struct Options<'a> {
    pub project: Option<&'a str>,
    pub database: Option<&'a str>,
    pub run: Option<&'a str>,
    pub program: Option<&'a str>,
    pub compiler_version: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt_digest: Option<&'a str>,
    pub compiler_args: &'a [String],
    pub offset: Option<&'a str>,
    pub json: bool,
}

pub(super) fn execute(
    operation: &str,
    options: Options<'_>,
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
    store.project_revision(&project)?;
    let value = match operation {
        "list" => {
            let offset = crate::parse_offset(options.offset)?;
            let requests = store.expansion_requests(&project, offset)?;
            let work = requests
                .iter()
                .map(|run| work_json(&store, &project, run))
                .collect::<Result<Vec<_>, _>>()?;
            json!({"schema":"merl.compilation-work/v1", "project":project.as_str(), "work":work,"next_offset":(work.len()==merl_store::COMPILATION_WORK_PAGE_SIZE).then_some(offset+work.len())})
        }
        "show" | "context" | "expand" => {
            let run = options
                .run
                .ok_or_else(|| invalid_input("--run is required"))?;
            if operation == "expand" {
                expand(&mut store, &project, run, options, clock)?;
            }
            let mut value = run_json(&store, &project, run)?;
            value["schema"] = json!("merl.compilation/v1");
            value["project"] = json!(project.as_str());
            value["expansion"] = work_json(&store, &project, run)?;
            value["context_rounds"] = lineage(&store, &project, run)?;
            if operation == "context" {
                let context = store.load_compilation_context(&project, run)?;
                value["rendered"] = json!(String::from_utf8_lossy(&context.rendered));
            }
            value
        }
        _ => return Err(invalid_input("unknown compilation command")),
    };
    if options.json {
        return Ok(format!("{value}\n"));
    }
    if operation == "list" {
        let lines = value["work"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|w| {
                format!(
                    "{}: round {} {}, successor {}{}",
                    w["parent_run"].as_str().unwrap_or_default(),
                    w["round"],
                    w["status"].as_str().unwrap_or_default(),
                    w["child_run"].as_str().unwrap_or_default(),
                    w["failure_code"]
                        .as_str()
                        .map_or(String::new(), |c| format!(" ({c})"))
                )
            })
            .collect::<Vec<_>>();
        return Ok(format!("{}\n", lines.join("\n")));
    }
    Ok(format!(
        "Run {}: {}\nExpansion: {}\n{}",
        value["run"].as_str().unwrap_or_default(),
        value["outcome"].as_str().unwrap_or_default(),
        value["expansion"],
        value["rendered"].as_str().unwrap_or_default()
    ))
}

fn expand(
    store: &mut Store,
    project: &ProjectId,
    run: &str,
    options: Options<'_>,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<(), CliError> {
    let adapter = merl_compiler::ProcessCompiler::new(
        options
            .program
            .ok_or_else(|| invalid_input("--program is required"))?,
        options.compiler_args.to_vec(),
        options
            .compiler_version
            .ok_or_else(|| invalid_input("--compiler-version is required"))?,
        options
            .model
            .ok_or_else(|| invalid_input("--model is required"))?,
        parse_sha256(
            options
                .prompt_digest
                .ok_or_else(|| invalid_input("--prompt-digest is required"))?,
        )?,
    )?;
    let prepared =
        match merl_compiler::prepare_context_expansion(store, project, run, &adapter, clock()?) {
            Ok(prepared) => prepared,
            Err(merl_compiler::CompileError::ContextRequired) => None,
            Err(error) => return Err(error.into()),
        };
    if let Some(prepared) = prepared {
        let response = merl_compiler::execute_compilation(&prepared, &adapter);
        match merl_compiler::record_compilation_result(
            store,
            project,
            &prepared,
            response,
            clock()?,
        ) {
            Ok(()) | Err(merl_compiler::CompileError::ContextRequired) => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn work_json(store: &Store, project: &ProjectId, run: &str) -> Result<Value, CliError> {
    Ok(store.expansion_work(project,run)?.map_or(Value::Null, |w|json!({"parent_run":w.parent_run,"child_run":w.child_run,"round":w.round,"references":w.references,"status":w.status,"failure_code":w.failure_code})))
}

fn run_json(store: &Store, project: &ProjectId, run: &str) -> Result<Value, CliError> {
    let status = store
        .compilation_run_status(project, run)?
        .ok_or_else(|| invalid_input("compiler run does not exist"))?;
    Ok(
        json!({"run":run,"mode":status.mode,"outcome":if !status.completed {"pending"} else if status.needs_context {"needs_context"} else if status.succeeded {"succeeded"} else {"failed"},
        "failure_code":status.failure_code,"source":status.source.as_str(),"interpretation_basis_revision":status.interpretation_basis_revision.get(),"source_observation_cutoff":status.source_observation_cutoff,
        "input_digest":crate::digest_text(&status.context_digest),"limits":status.limits}),
    )
}

pub(super) fn lineage(store: &Store, project: &ProjectId, run: &str) -> Result<Value, CliError> {
    let mut rounds = Vec::new();
    let mut current = Some(run.to_owned());
    while let Some(id) = current {
        let mut detail = run_json(store, project, &id)?;
        detail["expansion"] = work_json(store, project, &id)?;
        rounds.push(detail);
        current = store
            .expansion_parent(project, &id)?
            .or(store.compilation_replay_origin(project, &id)?);
    }
    rounds.reverse();
    Ok(json!(rounds))
}

pub(super) fn help(operation: Option<&str>, json_output: bool) -> Result<String, CliError> {
    let (name, usage, description) = match operation {
        None => (
            "compilation",
            "merl compilation <list|show|context|expand>",
            "Inspect pending, blocked, and exhausted context requests, or execute one bounded expansion round.",
        ),
        Some("list") => (
            "compilation list",
            "merl compilation list --project <id> --database <path> [--offset <n>] [--json]",
            "List 100 durable expansion requests per page, including completed and exhausted work.",
        ),
        Some("show") => (
            "compilation show",
            "merl compilation show --project <id> --database <path> --run <id> [--json]",
            "Inspect a run, its causal context lineage, and its next expansion request.",
        ),
        Some("context") => (
            "compilation context",
            "merl compilation context --project <id> --database <path> --run <id> [--json]",
            "Read the exact retained compiler input. Erased input remains unavailable.",
        ),
        Some("expand") => (
            "compilation expand",
            "merl compilation expand --project <id> --database <path> --run <parent-id> --program <path> --compiler-version <version> --model <id> --prompt-digest sha256:<hex> [--compiler-arg <arg>] [--json]",
            "Resume one round with the original compiler configuration and limits. Retrying the parent resumes its reserved successor; acceptance still requires source apply.",
        ),
        _ => return Err(invalid_input("unknown compilation help topic")),
    };
    if json_output {
        Ok(format!(
            "{}\n",
            json!({"schema":"merl.help/v1","command":name,"usage":usage,"summary":description,"examples":["merl compilation list --project P1 --database project.sqlite --json","merl compilation show --project P1 --database project.sqlite --run CR42 --json"],
        "errors":["COMPILER_EXPANSION_REFERENCE","COMPILER_EXPANSION_LOOP","COMPILER_EXPANSION_ROUND_BUDGET","COMPILER_INPUT_BUDGET","COMPILER_UNAUTHORIZED","MISSING_EVIDENCE"],
        "related":["compilation list","compilation show","compilation context","compilation expand","source assertions","source apply"]})
        ))
    } else {
        Ok(format!("{usage}\n{description}\n"))
    }
}
