//! The public command-line boundary for local Merl projects.

use std::{error::Error, fmt, fmt::Write as _, fs::File, io::Read, path::Path};

use merl_core::{AgentId, ObjectId, PolicyInput, ProjectId, ProjectRevision, SourceVersionId};
use merl_corpus::fixture::Fixture;
use merl_ingest::{ImportError, import_fixture};
use merl_store::{IssueState, PayloadRead, ProjectDelta, Store, StoreError, SupportStatus};
use serde_json::{Value, json};

/// A CLI failure with a stable machine-readable code.
#[derive(Debug)]
pub struct CliError {
    /// Stable error identity for scripts and agents.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for CliError {}

impl CliError {
    /// Returns one versioned error envelope for a non-interactive caller.
    #[must_use]
    pub fn as_json(&self) -> String {
        format!(
            "{}\n",
            json!({
                "schema": "merl.error/v1",
                "code": self.code,
                "message": self.message
            })
        )
    }
}

impl From<StoreError> for CliError {
    fn from(error: StoreError) -> Self {
        let code = match error {
            StoreError::UnsupportedSchema(_) => "UNSUPPORTED_SCHEMA",
            StoreError::ProjectMissing => "PROJECT_NOT_FOUND",
            StoreError::InvalidBatch => "INVALID_INPUT",
            StoreError::PayloadMissing => "PAYLOAD_NOT_FOUND",
            StoreError::CorruptHistory => "CORRUPT_HISTORY",
            StoreError::SourceConflict => "SOURCE_CONFLICT",
            StoreError::InvalidSource => "INVALID_SOURCE",
            StoreError::StaleProviderObservation => "STALE_PROVIDER_OBSERVATION",
            StoreError::InvalidCompilation => "INVALID_COMPILATION",
            StoreError::PolicyConflict => "POLICY_CONFLICT",
            StoreError::PolicyInputConflict => "POLICY_INPUT_CONFLICT",
            StoreError::InvalidPolicyEvaluation => "INVALID_POLICY_EVALUATION",
            StoreError::InvalidInboxAcknowledgement => "INVALID_INBOX_ACKNOWLEDGEMENT",
            StoreError::Storage(_) => "STORAGE_ERROR",
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<ImportError> for CliError {
    fn from(error: ImportError) -> Self {
        match error {
            ImportError::Store(error) => Self::from(error),
            ImportError::InvalidIdentity => Self {
                code: "INVALID_ID",
                message: error.to_string(),
            },
            ImportError::InvalidFixture(_)
            | ImportError::InvalidTimestamp
            | ImportError::InvalidProviderState => Self {
                code: "INVALID_FIXTURE",
                message: error.to_string(),
            },
            ImportError::Serialization(_) => Self {
                code: "SERIALIZATION_ERROR",
                message: error.to_string(),
            },
            ImportError::Policy(_) => Self {
                code: "POLICY_ERROR",
                message: error.to_string(),
            },
        }
    }
}

/// The command result and the output mode selected by the CLI parser.
#[derive(Debug)]
pub enum CliResponse {
    /// Text for standard output after a successful command.
    Success(String),
    /// An error for standard error in human mode.
    HumanError(CliError),
    /// An error for standard output in JSON mode.
    JsonError(CliError),
}

/// Parses and runs one command without network access.
///
/// The parser keeps the chosen output mode with an error, so the executable
/// does not have to inspect the arguments again after a failure.
#[must_use]
pub fn run(arguments: &[String]) -> CliResponse {
    let mut json_output = false;
    match execute(arguments, &mut json_output) {
        Ok(output) => CliResponse::Success(output),
        Err(error) if json_output => CliResponse::JsonError(error),
        Err(error) => CliResponse::HumanError(error),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the CLI keeps option parsing and explicit command dispatch in one boundary"
)]
fn execute(arguments: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut positional = Vec::new();
    let mut database = None;
    let mut id = None;
    let mut project = None;
    let mut fixture_path = None;
    let mut issue = None;
    let mut scope = None;
    let mut agent = None;
    let mut since = None;
    let mut revision = None;
    let mut version = None;
    let mut history = false;
    let mut expand_source = false;
    let mut role = "general";
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--format" => {
                index += 1;
                let format = arguments.get(index).ok_or_else(missing_value)?;
                *json_output = match format.as_str() {
                    "json" => true,
                    "human" => false,
                    _ => return Err(invalid_input("format must be human or json")),
                };
            }
            "--json" => *json_output = true,
            "--database" => {
                index += 1;
                database = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--id" => {
                index += 1;
                id = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--project" => {
                index += 1;
                project = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--fixture" => {
                index += 1;
                fixture_path = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--issue" => {
                index += 1;
                issue = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--scope" => {
                index += 1;
                scope = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--agent" => {
                index += 1;
                agent = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--since" => {
                index += 1;
                since = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--revision" => {
                index += 1;
                revision = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--version" => {
                index += 1;
                version = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--history" => history = true,
            "--source" => expand_source = true,
            "--role" => {
                index += 1;
                role = arguments.get(index).ok_or_else(missing_value)?.as_str();
                if !matches!(role, "general" | "researcher" | "engineer" | "pm") {
                    return Err(invalid_input("role must be researcher, engineer, or pm"));
                }
            }
            "--help" | "-h" => positional.push("help"),
            option if option.starts_with('-') => {
                return Err(invalid_input(&format!("unknown option {option}")));
            }
            word => positional.push(word),
        }
        index += 1;
    }

    match positional.as_slice() {
        [] | ["help"] => help("", *json_output),
        ["help", "project"] => help("project", *json_output),
        ["help", "project", "init"] => help("project init", *json_output),
        ["help", "project", "revision"] => help("project revision", *json_output),
        ["help", "project", "view"] => help("project view", *json_output),
        ["help", "project", "delta"] => help("project delta", *json_output),
        ["help", "issue"] => help("issue", *json_output),
        ["help", "inbox"] => help("inbox", *json_output),
        ["help", "inbox", "poll"] => help("inbox poll", *json_output),
        ["help", "inbox", "subscribe"] => help("inbox subscribe", *json_output),
        ["help", "inbox", "ack"] => help("inbox ack", *json_output),
        ["help", "show"] => help("show", *json_output),
        ["help", "source"] => help("source", *json_output),
        ["help", "source", "show"] => help("source show", *json_output),
        ["help", "issue", "view"] | ["issue", "view", "help"] => help("issue view", *json_output),
        ["help", "issue", "import-fixture"] | ["issue", "import-fixture", "help"] => {
            help("issue import-fixture", *json_output)
        }
        ["project", "init"] => {
            let id = parse_project(id.ok_or_else(|| invalid_input("--id is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let mut store = Store::open(Path::new(path))?;
            store.create_project(&id)?;
            result("project.init", &id, 0, *json_output)
        }
        ["project", "revision"] => {
            let id = parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let store = Store::open(Path::new(path))?;
            let revision = store.project_revision(&id)?;
            result("project.revision", &id, revision.get(), *json_output)
        }
        ["project", "view"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let store = Store::open(Path::new(database))?;
            render_project_view(&store, &project, role, *json_output)
        }
        ["project", "delta"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let since = parse_revision(since.ok_or_else(|| invalid_input("--since is required"))?)?;
            let store = Store::open(Path::new(database))?;
            let batches = store.project_delta_since(&project, since, 20)?;
            render_delta(
                &project,
                since,
                &batches,
                store.project_revision(&project)?,
                store.semantic_coverage(&project)?,
                *json_output,
            )
        }
        ["issue", "import-fixture"] => {
            let id = parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let fixture_path =
                fixture_path.ok_or_else(|| invalid_input("--fixture is required"))?;
            let fixture = read_fixture(Path::new(fixture_path))?;
            let issue_id = merl_ingest::fixture_issue_id(&fixture)?;
            let mut store = Store::open(Path::new(database))?;
            let report = import_fixture(&mut store, &id, &fixture)?;
            if *json_output {
                render_json(&json!({
                    "schema": "merl.result/v1",
                    "action": "issue.import-fixture",
                    "outcome": if report.captured > 0 || report.provider_changed { "captured" } else { "unchanged" },
                    "project": id.as_str(),
                    "issue": issue_id.as_str(),
                    "scope": fixture.source.issue_provider_id,
                    "revision": report.accepted_revision,
                    "captured": report.captured,
                    "provider_changed": report.provider_changed,
                    "observation_head": report.observation_head,
                    "warnings": []
                }))
            } else {
                Ok(format!(
                    "issue.import-fixture: {id}, {} new observations, head {}, revision {}\n",
                    report.captured, report.observation_head, report.accepted_revision
                ))
            }
        }
        ["issue", "view"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let issue =
                ObjectId::try_from(issue.ok_or_else(|| invalid_input("--issue is required"))?)
                    .map_err(|error| CliError {
                        code: "INVALID_ID",
                        message: error.to_string(),
                    })?;
            let scope = scope.ok_or_else(|| invalid_input("--scope is required"))?;
            let store = Store::open(Path::new(database))?;
            let state = store.issue_state(&project, &issue, scope)?;
            render_issue_view(&store, &project, &issue, &state, role, *json_output)
        }
        ["inbox", "poll"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let agent = parse_agent(agent.ok_or_else(|| invalid_input("--agent is required"))?)?;
            let store = Store::open(Path::new(database))?;
            let cursor = store.inbox_cursor(&project, &agent)?;
            let (entries, has_more) = store.inbox_page_after(&project, &agent, cursor, 20)?;
            let mut rendered = Vec::with_capacity(entries.len());
            for entry in &entries {
                let (changes, changes_truncated) =
                    store.batch_changes(&project, &entry.batch, 20)?;
                rendered.push(json!({
                    "id": format!("{}:{}", entry.batch, entry.agent),
                    "batch": entry.batch.as_str(), "revision": entry.revision.get(),
                    "changes": changes_json(&changes), "changes_truncated": changes_truncated
                }));
            }
            if *json_output {
                render_json(&json!({
                    "schema": "merl.inbox/v1", "project": project.as_str(),
                    "agent": agent.as_str(), "cursor": cursor.get(), "entries": rendered,
                    "has_more": has_more,
                    "coverage": coverage_json(store.semantic_coverage(&project)?)
                }))
            } else {
                let mut output = format!("Inbox for {agent} after revision {}\n", cursor.get());
                for entry in &entries {
                    let (changes, truncated) = store.batch_changes(&project, &entry.batch, 20)?;
                    writeln!(
                        output,
                        "{}: {}",
                        entry.revision.get(),
                        changes
                            .iter()
                            .map(|item| item.reference.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                    .expect("String write");
                    if truncated {
                        output.push_str("  More changes in this batch.\n");
                    }
                }
                Ok(output)
            }
        }
        ["inbox", "subscribe"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let agent = parse_agent(agent.ok_or_else(|| invalid_input("--agent is required"))?)?;
            let mut store = Store::open(Path::new(database))?;
            store.subscribe_all(&project, &agent)?;
            let cursor = store.inbox_cursor(&project, &agent)?;
            if *json_output {
                render_json(
                    &json!({"schema": "merl.inbox-subscription/v1", "project": project.as_str(), "agent": agent.as_str(), "cursor": cursor.get()}),
                )
            } else {
                Ok(format!(
                    "{agent} subscribed to {project} at cursor {}\n",
                    cursor.get()
                ))
            }
        }
        ["inbox", "ack"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let agent = parse_agent(agent.ok_or_else(|| invalid_input("--agent is required"))?)?;
            let revision =
                parse_revision(revision.ok_or_else(|| invalid_input("--revision is required"))?)?;
            let mut store = Store::open(Path::new(database))?;
            let cursor = store.acknowledge_inbox(&project, &agent, revision)?;
            if *json_output {
                render_json(
                    &json!({ "schema": "merl.inbox-ack/v1", "project": project.as_str(), "agent": agent.as_str(), "cursor": cursor.get() }),
                )
            } else {
                Ok(format!(
                    "{agent} acknowledged through revision {}\n",
                    cursor.get()
                ))
            }
        }
        ["show", object] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let object = ObjectId::try_from(*object).map_err(|error| CliError {
                code: "INVALID_ID",
                message: error.to_string(),
            })?;
            let store = Store::open(Path::new(database))?;
            render_object(
                &store,
                &project,
                &object,
                history,
                expand_source,
                *json_output,
            )
        }
        ["source", "show"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let version = SourceVersionId::try_from(
                version.ok_or_else(|| invalid_input("--version is required"))?,
            )
            .map_err(|error| CliError {
                code: "INVALID_ID",
                message: error.to_string(),
            })?;
            let store = Store::open(Path::new(database))?;
            render_source(&store, &project, &version, *json_output)
        }
        _ => Err(invalid_input("unknown command; run `merl help`")),
    }
}

fn role_priority(kind: &str, role: &str) -> u8 {
    match role {
        "researcher" => match kind {
            "hypothesis" | "experiment" | "claim" | "finding" => 0,
            "question" | "decision" | "blocker" => 1,
            _ => 2,
        },
        "engineer" => match kind {
            "task" | "requirement" | "decision" | "contract" | "blocker" => 0,
            "finding" | "question" => 1,
            _ => 2,
        },
        "pm" => match kind {
            "task" | "blocker" | "decision" | "question" => 0,
            "requirement" | "finding" => 1,
            _ => 2,
        },
        _ => 0,
    }
}

fn object_view_value(
    store: &Store,
    project: &ProjectId,
    object: &merl_store::ProjectedObject,
) -> Result<Value, CliError> {
    let mut value = json!({
        "id": object.id.as_str(), "kind": object.kind.as_str(),
        "lifecycle": object.lifecycle.as_str(), "support": support_name(object.support),
        "revision": object.revision.get(), "payload_ref": object.payload.as_ref().map(merl_core::PayloadId::as_str)
    });
    if let Some(payload) = &object.payload {
        match store.read_payload(project, payload)? {
            PayloadRead::Available(bytes) if bytes.len() <= 512 => {
                if let Ok(summary) = String::from_utf8(bytes) {
                    value["summary"] = json!(summary);
                }
            }
            PayloadRead::Unavailable => value["summary_status"] = json!("unavailable"),
            PayloadRead::Available(_) => value["summary_status"] = json!("expand_for_detail"),
        }
    }
    Ok(value)
}

fn render_project_view(
    store: &Store,
    project: &ProjectId,
    role: &str,
    json_output: bool,
) -> Result<String, CliError> {
    let revision = store.project_revision(project)?;
    let selected = store.objects_at_revision(project, revision, 100)?;
    let mut objects = Vec::new();
    for (id, _, _) in &selected.items {
        let object = store
            .object(project, id)?
            .ok_or(StoreError::CorruptHistory)?;
        if object.kind.as_str() != "provider_issue"
            && object.lifecycle != merl_core::ObjectLifecycle::Superseded
        {
            objects.push(object);
        }
    }
    objects.sort_by_key(|object| {
        (
            role_priority(object.kind.as_str(), role),
            object.id.as_str().to_owned(),
        )
    });
    let truncated = selected.truncated || objects.len() > 20;
    objects.truncate(20);
    let summaries = objects
        .iter()
        .map(|object| object_view_value(store, project, object))
        .collect::<Result<Vec<_>, _>>()?;
    let coverage = store.semantic_coverage(project)?;
    if json_output {
        render_json(&json!({
            "schema": "merl.project-view/v1", "project": project.as_str(), "role": role,
            "project_revision": revision.get(), "coverage": coverage_json(coverage),
            "objects": summaries, "truncated": truncated
        }))
    } else {
        let mut output = format!("{project}@{} ({role})\n", revision.get());
        writeln!(
            output,
            "Source head {}; required gaps {}",
            coverage.observation_head, coverage.required_gaps
        )
        .expect("String write");
        for object in &summaries {
            writeln!(
                output,
                "{} {}",
                object["kind"].as_str().unwrap_or("object"),
                object["summary"]
                    .as_str()
                    .unwrap_or(object["id"].as_str().unwrap_or("?"))
            )
            .expect("String write");
        }
        if truncated {
            output.push_str("More objects available by ID.\n");
        }
        Ok(output)
    }
}

fn payload_json(
    store: &Store,
    project: &ProjectId,
    payload: &merl_core::PayloadId,
) -> Result<Value, CliError> {
    match store.read_payload(project, payload)? {
        PayloadRead::Available(bytes) => {
            Ok(json!({"status": "available", "text": String::from_utf8_lossy(&bytes)}))
        }
        PayloadRead::Unavailable => Ok(json!({"status": "unavailable"})),
    }
}

fn render_source(
    store: &Store,
    project: &ProjectId,
    version: &SourceVersionId,
    json_output: bool,
) -> Result<String, CliError> {
    let source = store
        .source_version(project, version)?
        .ok_or_else(|| CliError {
            code: "SOURCE_NOT_FOUND",
            message: "source version does not exist".to_owned(),
        })?;
    let body = match &source.payload {
        Some(payload) => payload_json(store, project, payload)?,
        None => json!({"status": "unavailable"}),
    };
    if json_output {
        render_json(&json!({
            "schema": "merl.source/v1", "project": project.as_str(), "version": version.as_str(),
            "source": source.source.as_str(), "kind": source.kind.as_str(),
            "observation": source.sequence,
            "source_author": source.source_author.as_ref().map(merl_core::ActorId::as_str),
            "version_actor": source.version_actor.as_ref().map(merl_core::ActorId::as_str),
            "supersedes": source.supersedes.as_ref().map(SourceVersionId::as_str), "body": body
        }))
    } else if body["status"] == "available" {
        Ok(format!(
            "{} observation {}\n{}\n",
            version,
            source.sequence,
            body["text"].as_str().unwrap_or("")
        ))
    } else {
        Ok(format!("{version}: source bytes unavailable\n"))
    }
}

fn render_object(
    store: &Store,
    project: &ProjectId,
    object: &ObjectId,
    history: bool,
    expand_source: bool,
    json_output: bool,
) -> Result<String, CliError> {
    let state = store.object(project, object)?.ok_or_else(|| CliError {
        code: "OBJECT_NOT_FOUND",
        message: "object does not exist".to_owned(),
    })?;
    let origin = store.object_policy_origin(project, object)?;
    let mut origin_json = Value::Null;
    if let Some(origin) = &origin {
        let input = &origin.input;
        let mut input_json = json!({"kind": input.kind(), "id": input.id().as_str()});
        if let PolicyInput::ObservedAssertion { run, index, .. } = input {
            input_json["assertion"] = assertion_json(store, project, run, *index, expand_source)?;
        }
        origin_json = json!({
            "event": origin.event.as_str(), "evaluation": origin.evaluation.as_str(), "input": input_json
        });
    }
    let history_json = if history {
        Value::Array(
            store
                .object_history(project, object)?
                .iter()
                .map(|entry| {
                    json!({
                        "event": entry.event.as_str(), "batch": entry.batch.as_str(),
                        "revision": entry.revision.get(), "lifecycle": entry.lifecycle.as_str(),
                        "payload_ref": entry.payload.as_ref().map(merl_core::PayloadId::as_str),
                        "evaluation": entry.evaluation.as_ref().map(merl_core::PolicyEvaluationId::as_str),
                        "input": entry.input.as_ref().map(|input| json!({"kind": input.kind(), "id": input.id().as_str()}))
                    })
                })
                .collect(),
        )
    } else {
        Value::Null
    };
    if json_output {
        let mut result = json!({
            "schema": "merl.object/v1", "project": project.as_str(), "id": object.as_str(),
            "kind": state.kind.as_str(), "lifecycle": state.lifecycle.as_str(),
            "support": support_name(state.support), "revision": state.revision.get(),
            "project_revision": state.project_revision.get(),
            "payload_ref": state.payload.as_ref().map(merl_core::PayloadId::as_str),
            "policy_origin": origin_json
        });
        if history {
            result["history"] = history_json;
        }
        render_json(&result)
    } else {
        let mut output = format!(
            "{} {}: {} ({})\n",
            state.kind,
            object,
            state.lifecycle.as_str(),
            support_name(state.support)
        );
        if let Some(origin) = origin {
            writeln!(
                output,
                "Accepted by {} from {} {}",
                origin.evaluation,
                origin.input.kind(),
                origin.input.id()
            )
            .expect("String write");
        }
        if expand_source {
            let evidence = &origin_json["input"]["assertion"]["evidence"];
            if evidence["status"] == "available" {
                writeln!(
                    output,
                    "Evidence: {}",
                    evidence["text"].as_str().unwrap_or("")
                )
                .expect("String write");
            } else if evidence["status"] == "unavailable" {
                output.push_str("Evidence: source bytes unavailable\n");
            }
        }
        if history {
            for entry in store.object_history(project, object)? {
                writeln!(output, "Revision {}: {}", entry.revision.get(), entry.event)
                    .expect("String write");
            }
        }
        Ok(output)
    }
}

fn assertion_json(
    store: &Store,
    project: &ProjectId,
    run: &merl_core::CompilationRunId,
    index: u32,
    expand_source: bool,
) -> Result<Value, CliError> {
    let assertions = store.observed_assertions(project, run.as_str())?;
    let assertion = assertions
        .get(usize::try_from(index).map_err(|_| StoreError::CorruptHistory)?)
        .ok_or(StoreError::CorruptHistory)?;
    let mut detail = json!({
        "run": run.as_str(), "index": index, "source_version": assertion.source.as_str(),
        "span": {"start": assertion.span_start, "end": assertion.span_end},
        "subject": assertion.subject, "predicate": assertion.predicate, "value": assertion.value,
        "asserted_by": assertion.asserted_by, "attributed_to": assertion.attributed_to,
        "attribution_verified": assertion.attribution_verified
    });
    if expand_source {
        let source = store
            .source_version(project, &assertion.source)?
            .ok_or(StoreError::CorruptHistory)?;
        detail["evidence"] = match source.payload {
            Some(payload) => match store.read_payload(project, &payload)? {
                PayloadRead::Available(bytes) => {
                    let span = bytes
                        .get(assertion.span_start..assertion.span_end)
                        .ok_or(StoreError::CorruptHistory)?;
                    json!({"status": "available", "text": String::from_utf8_lossy(span)})
                }
                PayloadRead::Unavailable => json!({"status": "unavailable"}),
            },
            None => json!({"status": "unavailable"}),
        };
    }
    Ok(detail)
}

fn parse_agent(value: &str) -> Result<AgentId, CliError> {
    AgentId::try_from(value).map_err(|error| CliError {
        code: "INVALID_ID",
        message: error.to_string(),
    })
}

fn parse_revision(value: &str) -> Result<ProjectRevision, CliError> {
    value
        .parse::<u64>()
        .map(ProjectRevision::from)
        .map_err(|_| invalid_input("revision must be a nonnegative integer"))
}

fn coverage_json(coverage: merl_store::SemanticCoverage) -> Value {
    json!({
        "observation_head": coverage.observation_head,
        "processed_through": coverage.processed_through,
        "required_gaps": coverage.required_gaps,
        "required_pending": coverage.required_pending,
        "required_failed": coverage.required_failed,
        "optional_cold": coverage.optional_cold
    })
}

fn changes_json(changes: &[merl_store::DeltaChange]) -> Vec<Value> {
    changes
        .iter()
        .map(|change| {
            json!({
                "event": change.event.as_str(), "ref": change.reference, "kind": change.kind
            })
        })
        .collect()
}

fn render_delta(
    project: &ProjectId,
    since: ProjectRevision,
    batches: &[ProjectDelta],
    head: ProjectRevision,
    coverage: merl_store::SemanticCoverage,
    json_output: bool,
) -> Result<String, CliError> {
    if json_output {
        render_json(&json!({
            "schema": "merl.delta/v1", "project": project.as_str(), "since": since.get(),
            "head_revision": head.get(),
            "has_more": batches.last().is_some_and(|batch| batch.revision < head),
            "batches": batches.iter().map(|batch| json!({
                "batch": batch.batch.as_str(), "revision": batch.revision.get(),
                "changes": changes_json(&batch.changes),
                "changes_truncated": batch.changes_truncated
            })).collect::<Vec<_>>(),
            "coverage": coverage_json(coverage)
        }))
    } else {
        let mut output = format!("{project} changes after {}\n", since.get());
        for batch in batches {
            writeln!(
                output,
                "{}: {}",
                batch.revision.get(),
                batch
                    .changes
                    .iter()
                    .map(|item| item.reference.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .expect("String write");
        }
        Ok(output)
    }
}

fn support_name(value: SupportStatus) -> &'static str {
    match value {
        SupportStatus::Current => "current",
        SupportStatus::RevalidationPending => "revalidation_pending",
        SupportStatus::PartiallySupported => "partially_supported",
        SupportStatus::Unsupported => "unsupported",
    }
}

fn render_issue_view(
    store: &Store,
    project: &ProjectId,
    issue: &ObjectId,
    state: &IssueState,
    role: &str,
    json_output: bool,
) -> Result<String, CliError> {
    let coverage = &state.coverage;
    let mut objects: Vec<_> = state
        .semantics
        .iter()
        .filter(|object| object.lifecycle != merl_core::ObjectLifecycle::Superseded)
        .collect();
    objects.sort_by_key(|object| {
        (
            role_priority(object.kind.as_str(), role),
            object.id.as_str().to_owned(),
        )
    });
    let objects_truncated = objects.len() > 20;
    objects.truncate(20);
    let objects = objects
        .into_iter()
        .map(|object| object_view_value(store, project, object))
        .collect::<Result<Vec<_>, _>>()?;
    if json_output {
        render_json(&json!({
            "schema": "merl.issue-view/v1", "project": project.as_str(), "issue": issue.as_str(), "role": role,
            "project_revision": state.project_revision.get(),
            "coverage": {
                "observation_head": coverage.observation_head,
                "processed_through": coverage.processed_through,
                "required_gaps": coverage.required_gaps,
                "required_pending": coverage.required_pending,
                "required_failed": coverage.required_failed,
                "optional_cold": coverage.optional_cold
            },
            "provider": state.provider.as_ref().map(|provider| json!({
                "authority": "provider", "state": provider.input.state.as_str(),
                "revision": provider.revision.get(),
                "upstream_updated_at_millis": provider.input.upstream_updated_at_millis,
                "labels": provider.input.label_provider_ids,
                "assignees": provider.input.assignee_provider_ids
            })),
            "objects": objects, "objects_truncated": objects_truncated,
            "relations": state.relations.iter().take(20).map(|item| json!({
                "id": item.relation.id.as_str(), "subject": item.relation.subject.as_str(),
                "kind": item.relation.kind.as_str(), "object": item.relation.object.as_str()
            })).collect::<Vec<_>>(), "relations_truncated": state.relations.len() > 20
        }))
    } else {
        let mut output = format!("{project}@{} issue {issue}\n", state.project_revision.get());
        writeln!(
            output,
            "Source head {}; required coverage through {}; gaps {}",
            coverage.observation_head, coverage.processed_through, coverage.required_gaps
        )
        .expect("String write");
        if let Some(provider) = &state.provider {
            writeln!(output, "Provider state: {}", provider.input.state.as_str())
                .expect("String write");
        }
        for object in &objects {
            writeln!(
                output,
                "{} {} ({})",
                object["kind"].as_str().unwrap_or("object"),
                object["summary"]
                    .as_str()
                    .unwrap_or(object["id"].as_str().unwrap_or("?")),
                object["support"].as_str().unwrap_or("unknown")
            )
            .expect("String write");
        }
        if objects_truncated {
            output.push_str("More objects available by ID.\n");
        }
        Ok(output)
    }
}

fn missing_value() -> CliError {
    invalid_input("option needs a value")
}

fn invalid_input(message: &str) -> CliError {
    CliError {
        code: "INVALID_INPUT",
        message: message.to_owned(),
    }
}

fn parse_project(value: &str) -> Result<ProjectId, CliError> {
    ProjectId::try_from(value).map_err(|error| CliError {
        code: "INVALID_ID",
        message: error.to_string(),
    })
}

// This is a local safety ceiling, not a GitHub limit. Raise it only with a
// larger representative fixture and a memory measurement; otherwise a malformed
// import can allocate without bound before validation starts.
const MAX_FIXTURE_BYTES: u64 = 16 * 1024 * 1024;

fn read_fixture(path: &Path) -> Result<Fixture, CliError> {
    let file = File::open(path).map_err(|error| CliError {
        code: "IO_ERROR",
        message: error.to_string(),
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| CliError {
            code: "IO_ERROR",
            message: error.to_string(),
        })?;
    if bytes.len() as u64 > MAX_FIXTURE_BYTES {
        return Err(invalid_input("fixture exceeds the 16 MiB import limit"));
    }
    serde_json::from_slice(&bytes).map_err(|error| CliError {
        code: "INVALID_FIXTURE",
        message: error.to_string(),
    })
}

fn result(
    action: &str,
    project: &ProjectId,
    revision: u64,
    json_output: bool,
) -> Result<String, CliError> {
    if json_output {
        render_json(&json!({
            "schema": "merl.result/v1",
            "action": action,
            "outcome": "accepted",
            "project": project.as_str(),
            "revision": revision,
            "objects": [],
            "warnings": []
        }))
    } else {
        Ok(format!("{action}: {project} at revision {revision}\n"))
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one shallow match keeps command help discoverable without loading the full catalog"
)]
fn help(command: &str, json_output: bool) -> Result<String, CliError> {
    let (usage, summary, related) = match command {
        "" => (
            "merl <group> <command>",
            "Groups: project, issue, source, inbox, show. Run `merl help <group>` for commands.",
            vec!["project", "issue", "source", "inbox", "show"],
        ),
        "project" => (
            "merl project <command>",
            "Commands: init, revision, view, delta.",
            vec![
                "project init",
                "project revision",
                "project view",
                "project delta",
            ],
        ),
        "project init" => (
            "merl project init --id <id> --database <path> [--format json]",
            "Create a local project at revision zero.",
            vec!["project revision"],
        ),
        "project revision" => (
            "merl project revision --project <id> --database <path> [--format json]",
            "Read the accepted project revision.",
            vec!["project init"],
        ),
        "project view" => (
            "merl project view --project <id> --database <path> [--role researcher|engineer|pm] [--format json]",
            "Read current accepted work in role order. Pull full evidence by reference.",
            vec!["issue view", "show", "project delta"],
        ),
        "project delta" => (
            "merl project delta --project <id> --database <path> --since <revision> [--format json]",
            "Read accepted changes after a project revision. Source text is omitted.",
            vec!["project revision", "inbox poll"],
        ),
        "issue" => (
            "merl issue <command>",
            "Commands: import-fixture, view.",
            vec!["issue import-fixture", "issue view"],
        ),
        "issue view" => (
            "merl issue view --project <id> --database <path> --issue <id> --scope <provider-id> [--role researcher|engineer|pm] [--format json]",
            "Read current Issue facts and accepted semantics. Source text stays cold.",
            vec!["issue import-fixture"],
        ),
        "inbox" => (
            "merl inbox <command>",
            "Commands: subscribe, poll, ack.",
            vec!["inbox subscribe", "inbox poll", "inbox ack"],
        ),
        "inbox subscribe" => (
            "merl inbox subscribe --project <id> --database <path> --agent <id> [--format json]",
            "Subscribe an agent to accepted project changes.",
            vec!["inbox poll"],
        ),
        "inbox poll" => (
            "merl inbox poll --project <id> --database <path> --agent <id> [--format json]",
            "Read pending accepted changes for a subscribed agent.",
            vec!["inbox ack", "project delta"],
        ),
        "inbox ack" => (
            "merl inbox ack --project <id> --database <path> --agent <id> --revision <n> [--format json]",
            "Acknowledge the oldest unread entry. Repeating an acknowledgement is safe.",
            vec!["inbox poll"],
        ),
        "show" => (
            "merl show <object-id> --project <id> --database <path> [--history] [--source] [--format json]",
            "Inspect one accepted object, its policy input, and requested evidence.",
            vec!["issue view", "source show"],
        ),
        "source" => (
            "merl source <command>",
            "Commands: show.",
            vec!["source show"],
        ),
        "source show" => (
            "merl source show --project <id> --database <path> --version <id> [--format json]",
            "Read one captured source version. Erased bytes report unavailable.",
            vec!["show"],
        ),
        "issue import-fixture" => (
            "merl issue import-fixture --project <id> --database <path> --fixture <path> [--format json]",
            "Import a versioned Issue fixture without GitHub access. The fixture must pass corpus validation.",
            vec!["project init", "project revision"],
        ),
        _ => return Err(invalid_input("unknown help topic")),
    };
    let example = match command {
        "project init" => Some("merl project init --id P1 --database project.sqlite"),
        "project revision" => {
            Some("merl project revision --project P1 --database project.sqlite --json")
        }
        "project view" => {
            Some("merl project view --project P1 --database project.sqlite --role pm --json")
        }
        "project delta" => {
            Some("merl project delta --project P1 --database project.sqlite --since 42 --json")
        }
        "issue import-fixture" => Some(
            "merl issue import-fixture --project P1 --database project.sqlite --fixture issue.json --json",
        ),
        "issue view" => Some(
            "merl issue view --project P1 --database project.sqlite --issue I204 --scope github-issue-204 --role engineer --json",
        ),
        "inbox subscribe" => {
            Some("merl inbox subscribe --project P1 --database project.sqlite --agent dev --json")
        }
        "inbox poll" => {
            Some("merl inbox poll --project P1 --database project.sqlite --agent dev --json")
        }
        "inbox ack" => Some(
            "merl inbox ack --project P1 --database project.sqlite --agent dev --revision 43 --json",
        ),
        "show" => {
            Some("merl show D18 --project P1 --database project.sqlite --history --source --json")
        }
        "source show" => {
            Some("merl source show --project P1 --database project.sqlite --version SV1 --json")
        }
        _ => None,
    };
    if json_output {
        let errors = if command == "issue import-fixture" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "INVALID_FIXTURE",
                "IO_ERROR",
                "PROJECT_NOT_FOUND",
                "SOURCE_CONFLICT",
                "STALE_PROVIDER_OBSERVATION",
                "STORAGE_ERROR",
            ]
        } else if command == "show" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "PROJECT_NOT_FOUND",
                "OBJECT_NOT_FOUND",
                "CORRUPT_HISTORY",
                "STORAGE_ERROR",
            ]
        } else if command == "source show" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "PROJECT_NOT_FOUND",
                "SOURCE_NOT_FOUND",
                "STORAGE_ERROR",
            ]
        } else if command == "inbox ack" || command == "inbox poll" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "INVALID_INBOX_ACKNOWLEDGEMENT",
                "STORAGE_ERROR",
            ]
        } else {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "PROJECT_NOT_FOUND",
                "UNSUPPORTED_SCHEMA",
                "STORAGE_ERROR",
            ]
        };
        render_json(&json!({
            "schema": "merl.help/v1",
            "command": command,
            "usage": usage,
            "summary": summary,
            "related": related,
            "errors": errors,
            "example": example
        }))
    } else {
        let mut output = format!("{usage}\n{summary}\n");
        if let Some(example) = example {
            writeln!(output, "Example: {example}").expect("String write");
        }
        Ok(output)
    }
}

fn render_json(value: &Value) -> Result<String, CliError> {
    serde_json::to_string(&value)
        .map(|encoded| format!("{encoded}\n"))
        .map_err(|error| CliError {
            code: "SERIALIZATION_ERROR",
            message: error.to_string(),
        })
}
