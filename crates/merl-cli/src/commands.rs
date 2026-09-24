//! Domain commands keep semantic content separate from optional source prose.

use crate::{CliError, invalid_input};
use merl_core::{ObjectId, PolicyInput, ProjectId};
use merl_policy::SemanticCommand;
use merl_store::{CommandOperation, SemanticCommandRecord, Store};
use serde_json::{Value, json};
use std::{fmt::Write as _, io::Read, path::Path};

#[derive(Default)]
pub(super) struct Options<'a> {
    pub summary: Option<&'a str>,
    pub note: Option<&'a str>,
    pub note_file: Option<&'a str>,
    pub review_at: Option<&'a str>,
}

pub(super) fn is_group(group: &str) -> bool {
    matches!(
        group,
        "decision" | "question" | "finding" | "hypothesis" | "claim" | "task"
    )
}

pub(super) fn execute(
    store: &mut Store,
    project: &ProjectId,
    command: &SemanticCommand,
    dry_run: bool,
    now: i64,
    json_output: bool,
) -> Result<String, CliError> {
    let evaluation = if dry_run {
        let result = merl_policy::preview_semantic_command(store, project, command, now)?;
        json!({"schema":"merl.semantic-command-preview/v1","project":project.as_str(),"request":command.id.as_str(),"object":command.object.as_str(),"operation":command.operation.as_str(),"outcome":result.inputs[0].disposition.as_str(),"reason":result.inputs[0].reason.as_str(),"basis_project_revision":result.basis_project_revision.get()})
    } else {
        let result = merl_policy::submit_semantic_command(store, project, command, now)?;
        let receipt = store
            .semantic_command(project, &command.id)?
            .ok_or(merl_store::StoreError::CorruptHistory)?;
        json!({"schema":"merl.semantic-command/v1","project":project.as_str(),"request":command.id.as_str(),"object":command.object.as_str(),"operation":command.operation.as_str(),"actor":command.actor.as_str(),"outcome":result.inputs[0].disposition.as_str(),"reason":result.inputs[0].reason.as_str(),"revision":result.committed_revision.map(merl_core::ProjectRevision::get),"evaluation":result.id.as_str(),"source":receipt.source.as_ref().map(merl_core::SourceVersionId::as_str),"task":receipt.task.as_ref().filter(|_|result.committed_revision.is_some()).map(|t|task_value(store,project,t)).transpose()?,"policy_version":result.version.as_str(),"configuration_digest":crate::digest_text(&result.configuration_digest),"conflict":result.conflict.as_ref().map(|c|json!({"reason":c.reason_code,"target":c.target_id,"expected_revision":c.expected_revision,"actual_revision":c.actual_revision}))})
    };
    if json_output {
        return Ok(format!("{evaluation}\n"));
    }
    let mut output = format!(
        "{} {}: {} (request {})\n",
        evaluation["outcome"].as_str().unwrap_or("unknown"),
        command.object,
        evaluation["reason"].as_str().unwrap_or(""),
        command.id
    );
    output.push_str(&task_lines(&evaluation["task"]));
    if let Some(source) = evaluation["source"].as_str() {
        writeln!(output, "Supplemental source: {source}").expect("String write");
    }
    Ok(output)
}

pub(super) fn note(options: &Options<'_>) -> Result<Option<String>, CliError> {
    if options.note.is_some() && options.note_file.is_some() {
        return Err(invalid_input("choose --note or --note-file"));
    }
    if let Some(path) = options.note_file {
        let file = std::fs::File::open(Path::new(path))
            .map_err(|_| invalid_input("cannot read note file"))?;
        let mut bytes = Vec::new();
        file.take((merl_policy::MAX_COMMAND_TEXT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid_input("cannot read note file"))?;
        if bytes.len() > merl_policy::MAX_COMMAND_TEXT_BYTES {
            return Err(invalid_input("note exceeds 8192 bytes"));
        }
        return String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| invalid_input("note must be UTF-8"));
    }
    Ok(options.note.map(str::to_owned))
}

pub(super) fn task_value(
    store: &Store,
    project: &ProjectId,
    task: &merl_store::TaskState,
) -> Result<Value, CliError> {
    Ok(
        json!({"commitment":task.commitment.as_str(),"scheduling":task.scheduling.as_str(),"execution":task.execution.as_str(),"reason_ref":task.reason.as_ref().map(merl_core::PayloadId::as_str),"reason":task.reason.as_ref().map(|p|crate::payload_json(store,project,p)).transpose()?,"review_at":task.review_at}),
    )
}
pub(super) fn task_lines(task: &Value) -> String {
    if task.is_null() {
        return String::new();
    }
    format!(
        "Commitment: {}; scheduling: {}; execution: {}\n{}{}",
        task["commitment"].as_str().unwrap_or("unknown"),
        task["scheduling"].as_str().unwrap_or("unknown"),
        task["execution"].as_str().unwrap_or("unknown"),
        task["review_at"]
            .as_str()
            .map_or(String::new(), |date| format!("Review: {date}\n")),
        task["reason"]["text"]
            .as_str()
            .map_or(String::new(), |text| format!("Reason: {text}\n"))
    )
}
pub(super) fn decorate(
    store: &Store,
    project: &ProjectId,
    object: &ObjectId,
    value: &mut Value,
) -> Result<(), CliError> {
    if let Some(task) = store.task_state(project, object)? {
        value["task"] = task_value(store, project, &task)?;
    }
    if let Some(command) = store.object_semantic_command(project, object)? {
        value["command"] = lineage(store, project, &command, false)?;
        if command.operation == CommandOperation::Resolve {
            value["status"] = json!("resolved");
        }
    }
    Ok(())
}
pub(super) fn lineage(
    store: &Store,
    project: &ProjectId,
    command: &SemanticCommandRecord,
    expand: bool,
) -> Result<Value, CliError> {
    let accepted = store
        .accepted_policy_input(project, &PolicyInput::Command(command.id.clone()))?
        .is_some();
    let batch = store.semantic_command_batch(project, &command.id)?;
    let mut value = json!({"batch":batch.as_ref().map(merl_core::BatchId::as_str),"id":command.id.as_str(),"operation":command.operation.as_str(),"subject":command.object.as_str(),"kind":command.kind.as_str(),"source":command.source.as_ref().map(merl_core::SourceVersionId::as_str),"supplements":accepted.then_some(command.object.as_str()),"represented_value":command.payload.as_ref().map(merl_core::PayloadId::as_str)});
    if expand
        && let Some(source) = &command.source
        && let Some(source) = store.source_version(project, source)?
    {
        value["note"] = source
            .payload
            .as_ref()
            .map(|p| crate::payload_json(store, project, p))
            .transpose()?
            .unwrap_or(Value::Null);
    }
    Ok(value)
}

pub(super) fn help(
    group: &str,
    operation: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    let operations: &[&str] = match group {
        "task" => &["request", "accept", "defer", "start", "complete"],
        "question" | "finding" => &["create", "resolve"],
        _ => &["create"],
    };
    if let Some(op) = operation
        && !operations.contains(&op)
    {
        return Err(invalid_input("unknown semantic command"));
    }
    let name = format!(
        "{group}{}",
        operation.map_or(String::new(), |o| format!(" {o}"))
    );
    let mut arguments = vec!["--project ID --database PATH --actor ID --id REQUEST"];
    let example = match operation {
        Some("create" | "request") => {
            arguments.push("--subject OBJECT --summary TEXT [--issue SCOPE]");
            "--subject X1 --summary 'Recorded project state'"
        }
        Some("resolve") => {
            arguments.push("OBJECT --summary ANSWER");
            "Q1 --summary 'Use fixed gain'"
        }
        Some("defer") => {
            arguments.push("OBJECT --reason TEXT --review-at YYYY-MM-DD");
            "T1 --reason 'Migration has priority' --review-at 2026-10-01"
        }
        Some(_) => {
            arguments.push("OBJECT");
            "T1"
        }
        None => "--subject X1 --summary 'Recorded project state'",
    };
    arguments.extend([
        "--note TEXT or --note-file PATH (optional, at most 8192 UTF-8 bytes)",
        "--dry-run --json",
    ]);
    let example = format!(
        "merl {group} {} {example} --project P1 --database merl.db --actor alice --id request1",
        operation.unwrap_or(operations[0])
    );
    let value = json!({"schema":"merl.help/v1","command":name,"summary":"Submit semantic state through durable command authority.","commands":if operation.is_none(){operations}else{&[]},"arguments":arguments,"outcomes":["accepted","rejected","conflict"],"errors":["INVALID_INPUT","POLICY_ERROR","POLICY_INPUT_CONFLICT"],"examples":[example],"related":["project authority","show","source show"]});
    if json_output {
        Ok(format!("{value}\n"))
    } else {
        Ok(format!(
            "{name}: submit semantic state through command authority\nOperations: {}\n{}\nOutcomes: accepted, rejected, conflict. Errors: INVALID_INPUT, POLICY_INPUT_CONFLICT.\nExample: {example}\nRelated: project authority, show, source show\n",
            operations.join(", "),
            value["arguments"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

// At most eight notes expand here: 8 × 8 KiB caps new historical prose at
// 64 KiB. Remaining notes keep source references for explicit source expansion.
const MAX_EXPANDED_NOTES: usize = 8;
pub(super) fn history(
    store: &Store,
    project: &ProjectId,
    entries: &[merl_store::ObjectHistoryEntry],
) -> Result<Vec<Value>, CliError> {
    let mut result = Vec::new();
    let mut expanded = 0;
    for entry in entries {
        if let Some(PolicyInput::Command(id)) = &entry.input
            && let Some(command) = store.semantic_command(project, id)?
        {
            let expand = expanded < MAX_EXPANDED_NOTES;
            let mut item = lineage(store, project, &command, expand)?;
            if command.source.is_some() {
                if expand {
                    expanded += 1;
                } else {
                    item["note"] = json!({"status":"expand_by_reference"});
                }
            }
            result.push(item);
        }
    }
    Ok(result)
}

pub(super) fn object_details(
    store: &Store,
    project: &ProjectId,
    object: &merl_store::ProjectedObject,
) -> Result<String, CliError> {
    let mut value = json!({});
    decorate(store, project, &object.id, &mut value)?;
    let mut output = task_lines(&value["task"]);
    if let Some(status) = value["status"].as_str() {
        writeln!(output, "Status: {status}").expect("String write");
    }
    if let Some(payload) = &object.payload {
        let content = crate::payload_json(store, project, payload)?;
        if let Some(text) = content["text"].as_str() {
            writeln!(output, "{text}").expect("String write");
        } else {
            output.push_str("Content unavailable\n");
        }
    }
    Ok(output)
}

pub(super) fn view_lines(value: &Value) -> String {
    let mut output = task_lines(&value["task"]);
    if let Some(status) = value["status"].as_str() {
        writeln!(output, "Status: {status}").expect("String write");
    }
    if let Some(source) = value["command"]["source"].as_str() {
        writeln!(output, "Supplemental source: {source}").expect("String write");
    }
    output
}
