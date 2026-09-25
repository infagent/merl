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
    let mut reason = task
        .reason
        .as_ref()
        .map(|p| crate::payload_json(store, project, p))
        .transpose()?;
    if let (Some(span), Some(value)) = (&task.reason_span, &mut reason)
        && let Some(text) = value["text"].as_str()
    {
        value["text"] = json!(
            text.get(span.start..span.end)
                .ok_or(merl_store::StoreError::CorruptHistory)?
        );
    }
    let mut value = json!({"commitment":task.commitment.as_str(),"scheduling":task.scheduling.as_str(),"execution":task.execution.as_str(),"reason_ref":task.reason.as_ref().map(merl_core::PayloadId::as_str),"reason":reason,"review_at":task.review_at,"review_when":task.review_when,"start_after":task.start_after});
    if let Some(needed_by) = task
        .temporal
        .iter()
        .find(|temporal| temporal.original.role == merl_core::temporal::TemporalRole::NeededBy)
    {
        value["needed_by"] = json!(needed_by.value);
    }
    Ok(value)
}
pub(super) fn task_lines(task: &Value) -> String {
    if task.is_null() {
        return String::new();
    }
    let mut output = format!(
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
    );
    if !task["needed_by"].is_null() {
        writeln!(
            output,
            "Needed by: {}",
            temporal_value_text(&task["needed_by"])
        )
        .expect("String write");
    }
    for (field, label) in [
        ("review_when", "Review after"),
        ("start_after", "Start after"),
    ] {
        if !task[field].is_null() {
            writeln!(output, "{label}: {}", predicate_text(&task[field])).expect("String write");
        }
    }
    output
}

fn predicate_text(value: &Value) -> String {
    let state = if value["kind"] == "provider_merged" {
        "merges"
    } else {
        "resolves"
    };
    format!("{} {state}", value["subject"].as_str().unwrap_or("unknown"))
}

// Task constraints and generic temporal evidence use the same wording for dates
// and unevaluated predicates; neither renderer turns a need into a commitment.
fn temporal_value_text(value: &Value) -> String {
    match value["kind"].as_str() {
        Some("date") => value["date"].as_str().unwrap_or("unknown").to_owned(),
        Some("predicate") => format!("after {}", predicate_text(&value["predicate"])),
        _ => format!(
            "unresolved ({})",
            value["reason"].as_str().unwrap_or("unknown")
        ),
    }
}

pub(super) fn temporal_lines(values: &Value) -> String {
    let mut output = String::new();
    if let Some(values) = values.as_array() {
        for temporal in values {
            let text = temporal_value_text(&temporal["value"]);
            writeln!(
                output,
                "{}: {text}",
                temporal["original"]["role"].as_str().unwrap_or("time")
            )
            .expect("String write");
        }
    }
    output
}
pub(super) fn decorate(
    store: &Store,
    project: &ProjectId,
    object: &ObjectId,
    value: &mut Value,
) -> Result<(), CliError> {
    if let Some(assertion) = store.object_assertion(project, object)?
        && !assertion.temporal.is_empty()
    {
        value["temporal"] = json!(assertion.temporal);
    }
    if let Some(task) = store.task_state(project, object)? {
        value["task"] = task_value(store, project, &task)?;
        if !task.temporal.is_empty() {
            value["temporal"] = json!(task.temporal);
        }
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
    if operation.is_some_and(|op| !operations.contains(&op)) {
        return Err(invalid_input("unknown semantic command"));
    }
    let name = operation.map_or_else(|| group.to_owned(), |op| format!("{group} {op}"));
    let Some(operation) = operation else {
        return crate::help::render(
            &name,
            &format!("merl {group} <{}>", operations.join("|")),
            "Submit semantic state through durable command authority.",
            None,
            &["project authority", "show", "source show"],
            json_output,
        );
    };
    let (arguments, example) = match operation {
        "create" | "request" => (
            "--subject <object> --summary <text> [--issue <scope>]",
            "--subject X1 --summary 'Recorded project state'",
        ),
        "resolve" => (
            "<object> --summary <answer>",
            "Q1 --summary 'Use fixed gain'",
        ),
        "defer" => (
            "<object> --reason <text> --review-at <YYYY-MM-DD>",
            "T1 --reason 'Migration has priority' --review-at 2026-10-01",
        ),
        _ => ("<object>", "T1"),
    };
    let summary = match (group, operation) {
        ("decision", _) => "Record a decision under command_actor authority.",
        ("question", "create") => "Record an open question under command_actor authority.",
        ("question", _) => "Resolve an existing question with its answer.",
        ("finding", "create") => "Record an open finding under command_actor authority.",
        ("finding", _) => "Resolve an existing finding with its resolution.",
        ("hypothesis", _) => "Record a hypothesis under command_actor authority.",
        ("claim", _) => "Record a claim under command_actor authority.",
        ("task", "request") => "Request a task without accepting or starting it.",
        ("task", "accept") => "Accept responsibility for an existing task without starting it.",
        ("task", "defer") => {
            "Defer a task that has not started, with a reason and review date; preserve its commitment and execution state."
        }
        ("task", "start") => "Start execution of an accepted, eligible task.",
        _ => "Complete execution of a started task.",
    };
    let summary = format!(
        "{summary} Reuse --id for identical retries. --dry-run previews policy without committing. Optional --note or --note-file adds at most 8192 UTF-8 bytes of supplemental evidence."
    );
    let usage = format!(
        "merl {name} {arguments} --project <id> --database <path> --actor <id> --id <request> [--note <text> | --note-file <path>] [--dry-run] [--format human|json] [--json]"
    );
    let example = format!(
        "merl {name} {example} --project P1 --database project.sqlite --actor alice --id request1"
    );
    crate::help::render(
        &name,
        &usage,
        &summary,
        Some(&example),
        &["project authority", "show", "source show", "project view"],
        json_output,
    )
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
    if value["task"].is_null() {
        output.push_str(&temporal_lines(&value["temporal"]));
    }
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
    if value["task"].is_null() {
        output.push_str(&temporal_lines(&value["temporal"]));
    }
    if let Some(status) = value["status"].as_str() {
        writeln!(output, "Status: {status}").expect("String write");
    }
    if let Some(source) = value["command"]["source"].as_str() {
        writeln!(output, "Supplemental source: {source}").expect("String write");
    }
    output
}
