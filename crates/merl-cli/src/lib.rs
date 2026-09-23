//! The public command-line boundary for local Merl projects.

mod assertions;
mod authority;
mod candidates;
mod capture;
mod commands;

use std::{
    collections::BTreeSet, error::Error, fmt, fmt::Write as _, fs::File, io::Read, path::Path,
};

use merl_core::{AgentId, ObjectId, PolicyInput, ProjectId, ProjectRevision, SourceVersionId};
use merl_corpus::fixture::Fixture;
use merl_ingest::{ImportError, import_fixture};
use merl_policy::{Proposal, apply_current};
use merl_store::{
    CompilationAuthorizationConfig, IssueState, ObjectHistoryEntry, PayloadRead, ProjectDelta,
    PurgeAudit, PurgePreview, Store, StoreError, SupportStatus,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

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
            StoreError::InvalidPurge => "INVALID_PURGE",
            StoreError::InvalidCoveragePromotion => "INVALID_COVERAGE_PROMOTION",
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
            ImportError::CompilerRequired => Self {
                code: "COMPILER_REQUIRED",
                message: error.to_string(),
            },
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

impl From<merl_policy::PolicyError> for CliError {
    fn from(error: merl_policy::PolicyError) -> Self {
        match error {
            merl_policy::PolicyError::Store(error) => Self::from(error),
            merl_policy::PolicyError::CandidateMissing => Self {
                code: "CANDIDATE_NOT_FOUND",
                message: error.to_string(),
            },
            merl_policy::PolicyError::RunIneligible => Self {
                code: "ASSERTION_RUN_INELIGIBLE",
                message: error.to_string(),
            },
            merl_policy::PolicyError::InvalidCommand(message) => invalid_input(message),
            error @ merl_policy::PolicyError::InvalidProposal => Self {
                code: "POLICY_ERROR",
                message: error.to_string(),
            },
        }
    }
}

impl From<merl_compiler::CompileError> for CliError {
    fn from(error: merl_compiler::CompileError) -> Self {
        let code = match &error {
            merl_compiler::CompileError::NonCausalHistory => "NON_CAUSAL_HISTORY",
            merl_compiler::CompileError::MissingEvidence => "MISSING_EVIDENCE",
            merl_compiler::CompileError::InputBudget => "COMPILER_INPUT_BUDGET",
            merl_compiler::CompileError::OutputBudget => "COMPILER_OUTPUT_BUDGET",
            merl_compiler::CompileError::InvalidResponse => "INVALID_COMPILER_RESPONSE",
            merl_compiler::CompileError::ContextRequired => "COMPILER_CONTEXT_REQUIRED",
            merl_compiler::CompileError::UnauthorizedCompilation => "COMPILER_UNAUTHORIZED",
            merl_compiler::CompileError::UnsupportedReplayVersion(_) => {
                "UNSUPPORTED_REPLAY_VERSION"
            }
            merl_compiler::CompileError::Adapter(_) => "COMPILER_ADAPTER_ERROR",
            merl_compiler::CompileError::Store(_) => "STORAGE_ERROR",
        };
        Self {
            code,
            message: error.to_string(),
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

/// Parses and runs one command through its configured local or provider boundary.
///
/// The parser keeps the chosen output mode with an error, so the executable
/// does not have to inspect the arguments again after a failure.
#[must_use]
pub fn run(arguments: &[String]) -> CliResponse {
    run_with_clock(arguments, &utc_now_millis)
}

/// Runs a command with the authority clock supplied by an embedded host.
///
/// The clock returns UTC milliseconds since the Unix epoch. Acceptance drivers
/// use a fixed clock to exercise the CLI without giving callers a timestamp flag.
/// The standalone executable supplies its node clock through [`run`].
#[must_use]
pub fn run_with_clock(
    arguments: &[String],
    clock: &impl Fn() -> Result<i64, CliError>,
) -> CliResponse {
    let mut json_output = false;
    match execute(arguments, &mut json_output, clock) {
        Ok(output) => CliResponse::Success(output),
        Err(error) if json_output => CliResponse::JsonError(error),
        Err(error) => CliResponse::HumanError(error),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the CLI keeps option parsing and explicit command dispatch in one boundary"
)]
fn execute(
    arguments: &[String],
    json_output: &mut bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    let mut command_options = commands::Options::default();
    let mut candidate_kind = None;
    let mut candidate_value = None;
    let mut candidate_after = None;
    let mut positional = Vec::new();
    let mut database = None;
    let mut id = None;
    let mut project = None;
    let mut capture_options = capture::Options::default();
    let mut fixture_path = None;
    let mut issue = None;
    let mut scope = None;
    let mut agent = None;
    let mut since = None;
    let mut revision = None;
    let mut batch = None;
    let mut offset = None;
    let mut focus = None;
    let mut version = None;
    let mut reason = None;
    let mut actor = None;
    let mut administrator = None;
    let mut subject = None;
    let mut permission = None;
    let mut confirm_digest = None;
    let mut dry_run = false;
    let mut run_id = None;
    let mut new_run = None;
    let mut program = None;
    let mut compiler_args = Vec::new();
    let mut compiler_version = None;
    let mut model = None;
    let mut prompt_digest = None;
    let mut history = false;
    let mut expand_source = false;
    let mut role = "general";
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--summary" | "--statement" | "--note" | "--note-file" | "--review-at" => {
                let option = arguments[index].as_str();
                index += 1;
                let value = arguments.get(index).ok_or_else(missing_value)?.as_str();
                match option {
                    "--summary" | "--statement" => command_options.summary = Some(value),
                    "--note" => command_options.note = Some(value),
                    "--note-file" => command_options.note_file = Some(value),
                    _ => command_options.review_at = Some(value),
                }
            }
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
            "--repository" | "--github-program" | "--mode" | "--coverage" => {
                let option = arguments[index].as_str();
                index += 1;
                let value = arguments.get(index).ok_or_else(missing_value)?.as_str();
                capture_options.set(option, value);
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
            "--batch" => {
                index += 1;
                batch = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--offset" => {
                index += 1;
                offset = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--focus" => {
                index += 1;
                focus = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--version" => {
                index += 1;
                version = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--reason" => {
                index += 1;
                reason = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--actor" => {
                index += 1;
                actor = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--subject" => {
                index += 1;
                subject = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--permission" => {
                index += 1;
                permission = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--administrator" => {
                index += 1;
                administrator = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--confirm-digest" => {
                index += 1;
                confirm_digest = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--kind" => {
                index += 1;
                candidate_kind = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--value" => {
                index += 1;
                candidate_value = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--after" => {
                index += 1;
                candidate_after = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--dry-run" => dry_run = true,
            "--run" => {
                index += 1;
                run_id = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--new-run" => {
                index += 1;
                new_run = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--program" => {
                index += 1;
                program = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--compiler-arg" => {
                index += 1;
                compiler_args.push(arguments.get(index).ok_or_else(missing_value)?.clone());
            }
            "--compiler-version" => {
                index += 1;
                compiler_version = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--model" => {
                index += 1;
                model = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
            }
            "--prompt-digest" => {
                index += 1;
                prompt_digest = Some(arguments.get(index).ok_or_else(missing_value)?.as_str());
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
        ["help", group] | [group, "help"] if commands::is_group(group) => {
            commands::help(group, None, *json_output)
        }
        ["help", group, operation] | [group, operation, "help"] if commands::is_group(group) => {
            commands::help(group, Some(operation), *json_output)
        }
        [group, operation, tail @ ..] if commands::is_group(group) => {
            let project = ProjectId::try_from(project.ok_or_else(missing_value)?)
                .map_err(|_| invalid_input("invalid project"))?;
            let create = matches!(*operation, "create" | "request");
            if (create && !tail.is_empty())
                || (!create && (tail.len() != 1 || subject.is_some()))
                || (*operation == "request" && *group != "task")
            {
                return Err(invalid_input("invalid command target"));
            }
            let operation = merl_store::CommandOperation::try_from(if *operation == "request" {
                "create"
            } else {
                operation
            })
            .map_err(|_| invalid_input("unknown semantic operation"))?;
            let command = merl_policy::SemanticCommand {
                id: merl_core::PolicyInputId::try_from(id.ok_or_else(missing_value)?)
                    .map_err(|_| invalid_input("invalid request"))?,
                actor: merl_core::ActorId::try_from(actor.ok_or_else(missing_value)?)
                    .map_err(|_| invalid_input("invalid actor"))?,
                operation,
                object: ObjectId::try_from(if create {
                    subject.ok_or_else(missing_value)?
                } else {
                    tail[0]
                })
                .map_err(|_| invalid_input("invalid object"))?,
                kind: merl_core::ObjectKind::try_from(*group)
                    .map_err(|_| invalid_input("invalid kind"))?,
                issue_scope: issue.map(str::to_owned),
                summary: command_options.summary.map(str::to_owned),
                reason: reason.map(str::to_owned),
                review_at: command_options.review_at.map(str::to_owned),
                note: commands::note(&command_options)?,
            };
            let mut store = Store::open(Path::new(database.ok_or_else(missing_value)?))?;
            commands::execute(
                &mut store,
                &project,
                &command,
                dry_run,
                clock()?,
                *json_output,
            )
        }

        ["help", "candidate"] | ["candidate", "help"] => candidates::help(None, *json_output),
        ["help", "candidate", operation] | ["candidate", operation, "help"] => {
            candidates::help(Some(operation), *json_output)
        }
        ["candidate", operation, tail @ ..] => {
            let project = ProjectId::try_from(project.ok_or_else(missing_value)?)
                .map_err(|_| invalid_input("invalid project"))?;
            let mut store = Store::open(Path::new(database.ok_or_else(missing_value)?))?;
            candidates::execute(
                &mut store,
                &project,
                operation,
                tail,
                candidates::Options {
                    id,
                    actor,
                    subject,
                    kind: candidate_kind,
                    value: candidate_value,
                    reason,
                    after: candidate_after,
                    offset,
                    dry_run,
                    json_output: *json_output,
                },
                clock()?,
            )
        }
        [] | ["help"] => help("", *json_output),
        ["help", "project"] => help("project", *json_output),
        ["help", "project", "init"] => help("project init", *json_output),
        ["help", "project", "revision"] => help("project revision", *json_output),
        ["help", "project", "rebuild"] => help("project rebuild", *json_output),
        ["help", "project", "view"] => help("project view", *json_output),
        ["help", "project", "delta"] => help("project delta", *json_output),
        ["help", "project", "batch"] => help("project batch", *json_output),
        ["help", "issue"] => help("issue", *json_output),
        ["help", "inbox"] => help("inbox", *json_output),
        ["help", "inbox", "poll"] => help("inbox poll", *json_output),
        ["help", "inbox", "show"] => help("inbox show", *json_output),
        ["help", "inbox", "subscribe"] => help("inbox subscribe", *json_output),
        ["help", "inbox", "ack"] => help("inbox ack", *json_output),
        ["help", "show"] => help("show", *json_output),
        ["help", "source"] => help("source", *json_output),
        ["help", "source", "show"] => help("source show", *json_output),
        ["help", "source", "assertions"] => help("source assertions", *json_output),
        ["help", "source", "apply"] => help("source apply", *json_output),
        ["help", "source", "compile"] => help("source compile", *json_output),
        ["help", "source", "require"] => help("source require", *json_output),
        ["help", "source", "purge"] => help("source purge", *json_output),
        ["help", "source", "purge-audit"] => help("source purge-audit", *json_output),
        ["help", "source", "replay"] => help("source replay", *json_output),
        ["help", "issue", "view"] | ["issue", "view", "help"] => help("issue view", *json_output),
        ["help", "issue", "import-fixture"] | ["issue", "import-fixture", "help"] => {
            help("issue import-fixture", *json_output)
        }
        ["help", "project", "authority"] | ["project", "authority", "help"] => {
            authority::help(None, *json_output)
        }
        ["help", "project", "authority", operation]
        | ["project", "authority", operation, "help"] => {
            authority::help(Some(operation), *json_output)
        }
        ["project", "authority", operation] => authority::execute(
            operation,
            authority::Options {
                database,
                project,
                actor,
                subject,
                permission,
                id,
                reason,
            },
            *json_output,
            clock,
        ),
        ["project", "init"] => {
            let id = parse_project(id.ok_or_else(|| invalid_input("--id is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let mut store = Store::open(Path::new(path))?;
            store.create_project(&id)?;
            if let Some(administrator) = administrator {
                let administrator = merl_core::ActorId::try_from(administrator)
                    .map_err(|error| invalid_input(&error.to_string()))?;
                store.grant_administrator_unchecked_bootstrap(&id, &administrator)?;
            }
            result("project.init", &id, 0, *json_output)
        }
        ["project", "revision"] => {
            let id = parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let store = Store::open(Path::new(path))?;
            let revision = store.project_revision(&id)?;
            result("project.revision", &id, revision.get(), *json_output)
        }
        ["project", "rebuild"] => {
            let id = parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let path = database.ok_or_else(|| invalid_input("--database is required"))?;
            let mut store = Store::open(Path::new(path))?;
            store.rebuild_projection(&id)?;
            let revision = store.project_revision(&id)?;
            let accepted_events = store.accepted_event_count(&id)?;
            let erased_payloads = store.erased_payload_count(&id)?;
            if *json_output {
                render_json(&json!({
                    "schema": "merl.result/v1", "action": "project.rebuild",
                    "project": id.as_str(), "revision": revision.get(),
                    "accepted_events": accepted_events, "erased_payloads": erased_payloads,
                    "provenance": if erased_payloads == 0 { "complete" } else { "degraded" }
                }))
            } else {
                Ok(format!(
                    "Rebuilt {id} at revision {} from {accepted_events} accepted events; {erased_payloads} payloads unavailable\n",
                    revision.get()
                ))
            }
        }
        ["project", "view"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let store = Store::open(Path::new(database))?;
            render_project_view(&store, &project, role, focus, *json_output)
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
        ["project", "batch"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let batch = merl_core::BatchId::try_from(
                batch.ok_or_else(|| invalid_input("--batch is required"))?,
            )
            .map_err(|error| CliError {
                code: "INVALID_ID",
                message: error.to_string(),
            })?;
            let offset = parse_offset(offset)?;
            let store = Store::open(Path::new(database))?;
            render_batch_page(&store, &project, &batch, offset, *json_output)
        }
        ["help", "issue", "capture"] | ["issue", "capture", "help"] => {
            help("issue capture", *json_output)
        }
        ["issue", "capture"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let number = issue
                .ok_or_else(|| invalid_input("--issue is required"))?
                .parse::<u64>()
                .map_err(|_| invalid_input("--issue must be a positive number"))?;
            if number == 0 {
                return Err(invalid_input("--issue must be a positive number"));
            }
            let adapter = if let Some(program) = program {
                Some(merl_compiler::ProcessCompiler::new(
                    program,
                    compiler_args,
                    compiler_version
                        .ok_or_else(|| invalid_input("--compiler-version is required"))?,
                    model.ok_or_else(|| invalid_input("--model is required"))?,
                    parse_sha256(
                        prompt_digest
                            .ok_or_else(|| invalid_input("--prompt-digest is required"))?,
                    )?,
                )?)
            } else {
                None
            };
            capture::execute(
                Path::new(database),
                &project,
                number,
                &capture_options,
                adapter.as_ref(),
                *json_output,
                clock,
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
                    "changes": changes_json(&changes), "changes_truncated": changes_truncated,
                    "next_offset": changes_truncated.then_some(changes.len())
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
                        writeln!(
                            output,
                            "  More changes: merl inbox show --revision {} --offset 20",
                            entry.revision.get()
                        )
                        .expect("String write");
                    }
                }
                Ok(output)
            }
        }
        ["inbox", "show"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let agent = parse_agent(agent.ok_or_else(|| invalid_input("--agent is required"))?)?;
            let revision =
                parse_revision(revision.ok_or_else(|| invalid_input("--revision is required"))?)?;
            let offset = parse_offset(offset)?;
            let store = Store::open(Path::new(database))?;
            let entry = store
                .inbox_entry_at(&project, &agent, revision)?
                .ok_or_else(|| invalid_input("inbox entry does not exist"))?;
            render_batch_page(&store, &project, &entry.batch, offset, *json_output)
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
        ["source", action @ ("assertions" | "apply")] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let run = merl_core::CompilationRunId::try_from(
                run_id.ok_or_else(|| invalid_input("--run is required"))?,
            )
            .map_err(|error| invalid_input(&error.to_string()))?;
            let mut store = Store::open(Path::new(database))?;
            if *action == "assertions" {
                assertions::inspect(&store, &project, &run, *json_output)
            } else {
                let actor = merl_core::ActorId::try_from(
                    actor.ok_or_else(|| invalid_input("--actor is required"))?,
                )
                .map_err(|error| invalid_input(&error.to_string()))?;
                let request = merl_core::PolicyInputId::try_from(
                    id.ok_or_else(|| invalid_input("--id is required"))?,
                )
                .map_err(|error| invalid_input(&error.to_string()))?;
                if dry_run {
                    let prepared = merl_policy::prepare_assertions(
                        &store,
                        &project,
                        &run,
                        &actor,
                        &request,
                        clock()?,
                    )?;
                    assertions::render_preview(&project, &run, prepared.as_ref(), *json_output)
                } else {
                    let result = merl_policy::apply_assertions(
                        &mut store,
                        &project,
                        &run,
                        &actor,
                        &request,
                        clock()?,
                    )?;
                    assertions::render_application(&project, &run, result.as_ref(), *json_output)
                }
            }
        }
        ["source", "compile"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let version = SourceVersionId::try_from(
                version.ok_or_else(|| invalid_input("--version is required"))?,
            )
            .map_err(|error| invalid_input(&error.to_string()))?;
            let run = run_id.ok_or_else(|| invalid_input("--run is required"))?;
            let actor = merl_core::ActorId::try_from(
                actor.ok_or_else(|| invalid_input("--actor is required"))?,
            )
            .map_err(|error| invalid_input(&error.to_string()))?;
            let reason = reason.ok_or_else(|| invalid_input("--reason is required"))?;
            let compiler_version =
                compiler_version.ok_or_else(|| invalid_input("--compiler-version is required"))?;
            let model = model.ok_or_else(|| invalid_input("--model is required"))?;
            let prompt_digest = parse_sha256(
                prompt_digest.ok_or_else(|| invalid_input("--prompt-digest is required"))?,
            )?;
            let adapter = merl_compiler::ProcessCompiler::new(
                program.ok_or_else(|| invalid_input("--program is required"))?,
                compiler_args,
                compiler_version,
                model,
                prompt_digest,
            )?;
            let limits = first_release_compiler_limits();
            let mut store = Store::open(Path::new(database))?;
            let source = store
                .source_version(&project, &version)?
                .ok_or_else(|| invalid_input("source version does not exist"))?;
            let intent = store.prepare_compilation_authorization(
                &project,
                &version,
                run,
                reason.as_bytes(),
                CompilationAuthorizationConfig {
                    compiler_id: "process",
                    compiler_version,
                    model_id: model,
                    prompt_digest,
                    adapter_config_digest: merl_compiler::CompilerAdapter::configuration_digest(
                        &adapter,
                    ),
                    limits: limits.as_array(),
                },
            )?;
            let identity = format!("{project}/{version}/{run}/{actor}/{reason}");
            let proposal = Proposal::AdministrativeAction {
                id: stable_id("source_compile_input", &identity)?,
                event: merl_core::DomainEvent::PutObject {
                    id: stable_id("source_compile_event", &identity)?,
                    object: intent.object,
                    kind: merl_core::ObjectKind::try_from("source_compilation_request")
                        .map_err(|error| invalid_input(&error.to_string()))?,
                    payload: Some(intent.reason.clone()),
                    issue_scope: Some(source.context_scope_id),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            };
            let record = apply_current(
                &mut store,
                &project,
                &actor,
                stable_id("source_compile_evaluation", &identity)?,
                stable_id("source_compile_batch", &identity)?,
                clock()?,
                &[proposal],
            )?;
            let disposition = record.inputs[0].disposition;
            if disposition == merl_core::PolicyDisposition::Rejected {
                let coverage = store.semantic_coverage(&project)?;
                return if *json_output {
                    render_json(&json!({
                        "schema": "merl.source-compile/v1", "action": "source.compile",
                        "project": project.as_str(), "source": version.as_str(), "run": run,
                        "actor": actor.as_str(), "outcome": "rejected",
                        "required_gaps": coverage.required_gaps
                    }))
                } else {
                    Ok(format!(
                        "Compilation of {version} was rejected for {actor}.\n"
                    ))
                };
            }
            let prepared = merl_compiler::prepare_authorized_compilation(
                &mut store,
                &project,
                &version,
                &adapter,
                merl_compiler::RunRequest {
                    id: run,
                    limits,
                    mode: merl_compiler::RunMode::Live,
                    now_millis: clock()?,
                },
            )?;
            let outcome = if let Some(prepared) = prepared {
                let response = merl_compiler::execute_compilation(&prepared, &adapter);
                merl_compiler::record_compilation_result(
                    &mut store,
                    &project,
                    &prepared,
                    response,
                    clock()?,
                )?;
                "compiled"
            } else {
                "unchanged"
            };
            let coverage = store.semantic_coverage(&project)?;
            if *json_output {
                render_json(&json!({
                    "schema": "merl.source-compile/v1", "action": "source.compile",
                    "project": project.as_str(), "source": version.as_str(), "run": run,
                    "outcome": outcome, "required_gaps": coverage.required_gaps
                }))
            } else {
                Ok(format!("Compiled {version} as {run} ({outcome}).\n"))
            }
        }
        ["source", "require"] => {
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
            let scope = scope.ok_or_else(|| invalid_input("--scope is required"))?;
            let actor = merl_core::ActorId::try_from(
                actor.ok_or_else(|| invalid_input("--actor is required"))?,
            )
            .map_err(|error| CliError {
                code: "INVALID_ID",
                message: error.to_string(),
            })?;
            let reason = reason.ok_or_else(|| invalid_input("--reason is required"))?;
            let mut store = Store::open(Path::new(database))?;
            let existed = store
                .coverage_promotion(&project, &version, scope)?
                .is_some();
            let intent =
                store.prepare_source_requirement(&project, &version, scope, reason.as_bytes())?;
            if existed {
                let promotion = store
                    .coverage_promotion(&project, &version, scope)?
                    .ok_or(StoreError::CorruptHistory)?;
                if *json_output {
                    return render_json(&json!({
                        "schema": "merl.source-coverage/v1",
                        "action": "source.require",
                        "project": project.as_str(),
                        "source": promotion.source.as_str(),
                        "scope": promotion.scope,
                        "coverage": "required",
                        "actor": promotion.actor.as_str(),
                        "reason_payload": promotion.reason.as_str(),
                        "promoted_at_millis": promotion.promoted_at_millis,
                        "outcome": "unchanged"
                    }));
                }
                return Ok(format!(
                    "{} is required for {} (unchanged)\n",
                    promotion.source, promotion.scope
                ));
            }
            let identity = format!(
                "{}/{}/{}/{}/{}",
                project, intent.source, scope, actor, reason
            );
            let proposal = Proposal::AdministrativeAction {
                id: stable_id("source_requirement_input", &identity)?,
                event: merl_core::DomainEvent::PutObject {
                    id: stable_id("source_requirement_event", &identity)?,
                    object: intent.object,
                    kind: merl_core::ObjectKind::try_from("source_coverage_requirement")
                        .map_err(|error| invalid_input(&error.to_string()))?,
                    payload: Some(intent.reason.clone()),
                    issue_scope: Some(scope.to_owned()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            };
            let now = clock()?;
            let record = apply_current(
                &mut store,
                &project,
                &actor,
                stable_id("source_requirement_evaluation", &identity)?,
                stable_id("source_requirement_batch", &identity)?,
                now,
                &[proposal],
            )?;
            let disposition = record.inputs[0].disposition;
            let promotion = store.coverage_promotion(&project, &version, scope)?;
            let outcome = match disposition {
                merl_core::PolicyDisposition::Accepted => "promoted",
                merl_core::PolicyDisposition::Duplicate => "unchanged",
                merl_core::PolicyDisposition::Rejected => "rejected",
                merl_core::PolicyDisposition::Candidate => "candidate",
                merl_core::PolicyDisposition::Conflict => "conflict",
            };
            if *json_output {
                render_json(&json!({
                    "schema": "merl.source-coverage/v1",
                    "action": "source.require",
                    "project": project.as_str(),
                    "source": intent.source.as_str(),
                    "scope": scope,
                    "coverage": if promotion.is_some() { "required" } else { "optional" },
                    "actor": actor.as_str(),
                    "reason_payload": intent.reason.as_str(),
                    "promoted_at_millis": promotion.as_ref().map(|value| value.promoted_at_millis),
                    "outcome": outcome
                }))
            } else {
                Ok(format!(
                    "{} is required for {} ({})\n",
                    intent.source, scope, outcome
                ))
            }
        }
        ["source", "purge"] => {
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
            let reason = reason.ok_or_else(|| invalid_input("--reason is required"))?;
            let mut store = Store::open(Path::new(database))?;
            if dry_run {
                if confirm_digest.is_some() {
                    return Err(invalid_input(
                        "--dry-run cannot be combined with --confirm-digest",
                    ));
                }
                let preview = store.preview_source_purge(&project, &version)?;
                render_purge_preview(&project, &preview, *json_output)
            } else {
                let actor = merl_core::ActorId::try_from(
                    actor.ok_or_else(|| invalid_input("--actor is required"))?,
                )
                .map_err(|error| CliError {
                    code: "INVALID_ID",
                    message: error.to_string(),
                })?;
                let digest = parse_sha256(
                    confirm_digest.ok_or_else(|| invalid_input("--confirm-digest is required"))?,
                )?;
                let now_millis = clock()?;
                let audit = store.purge_source(
                    &project,
                    &version,
                    &actor,
                    reason.as_bytes(),
                    now_millis,
                    digest,
                )?;
                render_purge_audit(&project, &audit, *json_output)
            }
        }
        ["source", "purge-audit"] => {
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
            let audit = store
                .purge_audit(&project, &version)?
                .ok_or_else(|| CliError {
                    code: "PURGE_AUDIT_NOT_FOUND",
                    message: "source has no purge audit".into(),
                })?;
            let receipts = store.purge_receipt_payloads(&project, &version)?;
            let reason = payload_json(&store, &project, &audit.reason)?;
            if *json_output {
                render_json(&json!({
                    "schema": "merl.purge-audit/v1", "project": project.as_str(),
                    "source": version.as_str(), "actor": audit.actor.as_str(),
                    "requested_at_millis": audit.requested_at_millis,
                    "reason": reason, "completed": audit.completed,
                    "preview_digest": digest_text(&audit.preview_digest),
                    "payloads": receipts.iter().map(|item| json!({
                        "id": item.id.as_str(), "digest": digest_text(&item.digest)
                    })).collect::<Vec<_>>(),
                    "covered_scopes": ["active_store"],
                    "outside_scope": ["provider_systems", "unmanaged_backups", "prior_exports"]
                }))
            } else {
                Ok(format!(
                    "{}: purge {} by {}; {} payload receipts\n",
                    version,
                    if audit.completed {
                        "complete"
                    } else {
                        "pending"
                    },
                    audit.actor,
                    receipts.len()
                ))
            }
        }
        ["source", "replay"] => {
            let project =
                parse_project(project.ok_or_else(|| invalid_input("--project is required"))?)?;
            let database = database.ok_or_else(|| invalid_input("--database is required"))?;
            let run_id = run_id.ok_or_else(|| invalid_input("--run is required"))?;
            let mut store = Store::open(Path::new(database))?;
            let original = store
                .compilation_run_status(&project, run_id)?
                .ok_or_else(|| invalid_input("recorded compiler run does not exist"))?;
            let limits = merl_compiler::CompilerLimits {
                input_bytes: original.limits[0],
                output_bytes: original.limits[1],
                output_tokens: original.limits[2],
                assertions: original.limits[3],
                context_requests: original.limits[4],
                expansion_rounds: original.limits[5],
                payload_bytes: original.limits[6],
                source_window: original.limits[7],
                objects: original.limits[8],
            };
            let rebuilt =
                merl_compiler::rebuild_recorded_context(&store, &project, run_id, limits)?;
            let mut rerun_id = None;
            if let Some(program) = program {
                let new_id =
                    new_run.ok_or_else(|| invalid_input("--new-run is required with --program"))?;
                let adapter = merl_compiler::ProcessCompiler::new(
                    program,
                    compiler_args,
                    compiler_version
                        .ok_or_else(|| invalid_input("--compiler-version is required"))?,
                    model.ok_or_else(|| invalid_input("--model is required"))?,
                    parse_sha256(
                        prompt_digest
                            .ok_or_else(|| invalid_input("--prompt-digest is required"))?,
                    )?,
                )?;
                let now = clock()?;
                if let Some(prepared) = merl_compiler::prepare_replay_compilation(
                    &mut store,
                    &project,
                    run_id,
                    rebuilt.clone(),
                    &adapter,
                    merl_compiler::RunRequest {
                        id: new_id,
                        limits,
                        mode: merl_compiler::RunMode::Replay,
                        now_millis: now,
                    },
                )? {
                    let response = merl_compiler::execute_compilation(&prepared, &adapter);
                    merl_compiler::record_compilation_result(
                        &mut store,
                        &project,
                        &prepared,
                        response,
                        clock()?,
                    )?;
                }
                rerun_id = Some(new_id);
            } else if new_run.is_some()
                || compiler_version.is_some()
                || model.is_some()
                || prompt_digest.is_some()
                || !compiler_args.is_empty()
            {
                return Err(invalid_input("compiler options require --program"));
            }
            let revision = store.project_revision(&project)?;
            if *json_output {
                render_json(&json!({
                    "schema": "merl.replay/v1", "action": "source.replay",
                    "project": project.as_str(), "run": run_id, "rerun": rerun_id,
                    "input_matches": true, "input_digest": digest_text(&original.context_digest),
                    "interpretation_basis_revision": rebuilt.interpretation_basis_revision.get(),
                    "source_observation_cutoff": rebuilt.source_observation_cutoff,
                    "accepted_revision": revision.get(), "accepted_history_changed": false
                }))
            } else {
                Ok(format!(
                    "Rebuilt {run_id} at source cutoff {}; input digest matches. Accepted revision remains {}.\n",
                    rebuilt.source_observation_cutoff,
                    revision.get()
                ))
            }
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
    commands::decorate(store, project, &object.id, &mut value)?;
    Ok(value)
}

fn render_project_view(
    store: &Store,
    project: &ProjectId,
    role: &str,
    focus: Option<&str>,
    json_output: bool,
) -> Result<String, CliError> {
    let revision = store.project_revision(project)?;
    let focus = focus
        .map(|value| {
            ObjectId::try_from(value).map_err(|error| CliError {
                code: "INVALID_ID",
                message: error.to_string(),
            })
        })
        .transpose()?;
    let mut neighbors = BTreeSet::new();
    let mut focus_truncated = false;
    if let Some(focus) = &focus {
        if store.object(project, focus)?.is_none() {
            return Err(invalid_input("focus object does not exist"));
        }
        let (related, truncated) = store.focus_neighbors(project, focus, 100)?;
        neighbors.extend(related.into_iter().map(|id| id.to_string()));
        focus_truncated = truncated;
    }
    let (objects, truncated) = store.current_objects_ranked(project, 20, |id, kind| {
        if focus.as_ref().is_some_and(|focus| focus.as_str() == id) {
            0
        } else if neighbors.contains(id) {
            1
        } else if focus.is_some() {
            2 + role_priority(kind, role)
        } else {
            role_priority(kind, role)
        }
    })?;
    let summaries = objects
        .iter()
        .map(|object| object_view_value(store, project, object))
        .collect::<Result<Vec<_>, _>>()?;
    let coverage = store.semantic_coverage(project)?;
    if json_output {
        render_json(&json!({
            "schema": "merl.project-view/v1", "project": project.as_str(), "role": role,
            "project_revision": revision.get(), "coverage_scope": "project",
            "coverage": coverage_json(coverage),
            "objects": summaries, "truncated": truncated,
            "focus": focus.as_ref().map(ObjectId::as_str), "focus_truncated": focus_truncated
        }))
    } else {
        let mut output = format!("{project}@{} ({role})\n", revision.get());
        if let Some(focus) = &focus {
            writeln!(output, "Focus: {focus}").expect("String write");
        }
        writeln!(
            output,
            "Project source head {}; required gaps {}",
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
            output.push_str(&commands::view_lines(object));
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

fn unavailable_source_json(
    store: &Store,
    project: &ProjectId,
    version: &SourceVersionId,
) -> Result<Value, CliError> {
    Ok(match store.purge_audit(project, version)? {
        Some(audit) => json!({
            "status": "unavailable", "reason": "source_content_unavailable",
            "tombstone": { "source": version.as_str(), "actor": audit.actor.as_str(),
                "requested_at_millis": audit.requested_at_millis, "completed": audit.completed }
        }),
        None => json!({"status": "unavailable"}),
    })
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
    let mut body = match &source.payload {
        Some(payload) => payload_json(store, project, payload)?,
        None => json!({"status": "unavailable"}),
    };
    if body["status"] == "unavailable" {
        body = unavailable_source_json(store, project, version)?;
    }
    let command = store.source_command(project, version)?;
    let lineage = command
        .as_ref()
        .map(|c| commands::lineage(store, project, c, false))
        .transpose()?
        .unwrap_or(Value::Null);
    if json_output {
        render_json(&json!({
            "semantic_origin":lineage["id"],"supplements":lineage["supplements"],"batch":lineage["batch"],
            "schema": "merl.source/v1", "project": project.as_str(), "version": version.as_str(),
            "source": source.source.as_str(), "kind": source.kind.as_str(),
            "observation": source.sequence,
            "source_author": source.source_author.as_ref().map(merl_core::ActorId::as_str),
            "version_actor": source.version_actor.as_ref().map(merl_core::ActorId::as_str),
            "supersedes": source.supersedes.as_ref().map(SourceVersionId::as_str), "body": body
        }))
    } else if body["status"] == "available" {
        Ok(format!(
            "{} observation {}\n{}{}\n",
            version,
            source.sequence,
            command.as_ref().map_or(String::new(), |command| format!(
                "Semantic origin: {}; supplements: {}; batch: {}\n",
                command.id, lineage["supplements"], lineage["batch"]
            )),
            body["text"].as_str().unwrap_or("")
        ))
    } else {
        Ok(format!("{version}: source bytes unavailable\n"))
    }
}

fn digest_text(digest: &[u8; 32]) -> String {
    let mut value = String::from("sha256:");
    for byte in digest {
        write!(value, "{byte:02x}").expect("writing a digest is infallible");
    }
    value
}

fn parse_sha256(value: &str) -> Result<[u8; 32], CliError> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or_else(|| invalid_input("digest must begin with sha256:"))?;
    if hex.len() != 64 {
        return Err(invalid_input("SHA-256 digest must have 64 hex digits"));
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| invalid_input("SHA-256 digest contains invalid hex"))?;
    }
    Ok(digest)
}

fn utc_now_millis() -> Result<i64, CliError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid_input("system clock is before Unix epoch"))?
        .as_millis();
    i64::try_from(now).map_err(|_| invalid_input("system clock is outside supported range"))
}

fn stable_id<T>(prefix: &str, meaning: &str) -> Result<T, CliError>
where
    for<'a> T: TryFrom<&'a str>,
    for<'a> <T as TryFrom<&'a str>>::Error: fmt::Display,
{
    let digest = Sha256::digest(meaning.as_bytes());
    let mut value = format!("{prefix}_");
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing a digest is infallible");
    }
    T::try_from(value.as_str()).map_err(|error| invalid_input(&error.to_string()))
}

const fn first_release_compiler_limits() -> merl_compiler::CompilerLimits {
    merl_compiler::CompilerLimits {
        input_bytes: 64 * 1024,
        output_bytes: 16 * 1024,
        output_tokens: 2_048,
        assertions: 32,
        context_requests: 8,
        expansion_rounds: 2,
        payload_bytes: 48 * 1024,
        source_window: 16,
        objects: 32,
    }
}

fn render_purge_preview(
    project: &ProjectId,
    preview: &PurgePreview,
    json_output: bool,
) -> Result<String, CliError> {
    let digest = digest_text(&preview.confirm_digest);
    if json_output {
        render_json(&json!({
            "schema": "merl.purge-preview/v1", "action": "source.purge.preview",
            "project": project.as_str(), "source": preview.source.as_str(),
            "confirm_digest": digest,
            "payloads": preview.payloads.iter().map(|item| json!({
                "id": item.id.as_str(), "digest": digest_text(&item.digest)
            })).collect::<Vec<_>>(),
            "runs": preview.runs.iter().map(merl_core::CompilationRunId::as_str).collect::<Vec<_>>(),
            "assertions": preview.assertions.iter().map(|item| json!({
                "run": item.run.as_str(), "index": item.index
            })).collect::<Vec<_>>(),
            "events": preview.events.iter().map(merl_core::EventId::as_str).collect::<Vec<_>>(),
            "objects": preview.objects.iter().map(ObjectId::as_str).collect::<Vec<_>>(),
            "relations": preview.relations.iter().map(merl_core::RelationId::as_str).collect::<Vec<_>>(),
            "covered_scopes": ["active_store"],
            "outside_scope": ["provider_systems", "unmanaged_backups", "prior_exports"]
        }))
    } else {
        Ok(format!(
            "Purge {}: {} payloads, {} compiler runs, {} accepted objects\nConfirm with --confirm-digest {digest}\n",
            preview.source,
            preview.payloads.len(),
            preview.runs.len(),
            preview.objects.len()
        ))
    }
}

fn render_purge_audit(
    project: &ProjectId,
    audit: &PurgeAudit,
    json_output: bool,
) -> Result<String, CliError> {
    if json_output {
        render_json(&json!({
            "schema": "merl.purge-audit/v1", "action": "source.purge",
            "project": project.as_str(), "source": audit.source.as_str(),
            "actor": audit.actor.as_str(), "requested_at_millis": audit.requested_at_millis,
            "reason_payload": audit.reason.as_str(),
            "confirm_digest": digest_text(&audit.preview_digest), "completed": audit.completed,
            "covered_scopes": ["active_store"],
            "outside_scope": ["provider_systems", "unmanaged_backups", "prior_exports"]
        }))
    } else {
        Ok(format!(
            "{}: protected bytes removed from active Merl store; provider systems, unmanaged backups, and prior exports are outside scope\n",
            audit.source
        ))
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
    let origin_json = object_origin_json(store, project, origin.as_ref(), expand_source)?;
    let history_entries = if history || expand_source {
        store.object_history(project, object)?
    } else {
        Vec::new()
    };
    let history_json = history.then(|| object_history_json(&history_entries));
    let evidence_history = if expand_source {
        candidates::evidence_history(store, project, &history_entries)?
    } else {
        Vec::new()
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
        commands::decorate(store, project, object, &mut result)?;
        result["content"] = state
            .payload
            .as_ref()
            .map(|p| payload_json(store, project, p))
            .transpose()?
            .unwrap_or(Value::Null);
        if history {
            result["history"] = history_json.expect("history requested");
        }
        if expand_source {
            result["evidence_history"] = json!(evidence_history);
            result["command_history"] = json!(commands::history(store, project, &history_entries)?);
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
        output.push_str(&commands::object_details(store, project, &state)?);
        if expand_source {
            for command in commands::history(store, project, &history_entries)? {
                if !command["note"].is_null() {
                    writeln!(
                        output,
                        "Supplemental note ({}): {}",
                        command["id"], command["note"]
                    )
                    .expect("String write");
                }
            }
        }
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
            for support in &evidence_history {
                let evidence = &support["assertion"]["evidence"];
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
        }
        if history {
            for entry in history_entries {
                writeln!(output, "Revision {}: {}", entry.revision.get(), entry.event)
                    .expect("String write");
            }
        }
        Ok(output)
    }
}

fn object_history_json(entries: &[ObjectHistoryEntry]) -> Value {
    Value::Array(entries.iter().map(|entry| {
        let input = entry.input.as_ref().map(|input| {
            let mut detail = json!({"kind": input.kind(), "id": input.id().as_str()});
            if let PolicyInput::ObservedAssertion { run, index, .. } = input {
                detail["run"] = json!(run.as_str());
                detail["index"] = json!(index);
            }
            detail
        });
        json!({
            "event": entry.event.as_str(), "batch": entry.batch.as_str(),
            "revision": entry.revision.get(), "lifecycle": entry.lifecycle.as_str(),
            "payload_ref": entry.payload.as_ref().map(merl_core::PayloadId::as_str),
            "evaluation": entry.evaluation.as_ref().map(merl_core::PolicyEvaluationId::as_str),
            "input": input
        })
    }).collect())
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
        "attribution_verified": assertion.attribution_verified,
        "act": assertion.act, "epistemic_basis": assertion.epistemic_basis,
        "polarity": assertion.polarity, "confidence_millis": assertion.confidence_millis
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
                PayloadRead::Unavailable => {
                    unavailable_source_json(store, project, &assertion.source)?
                }
            },
            None => unavailable_source_json(store, project, &assertion.source)?,
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

fn parse_offset(value: Option<&str>) -> Result<usize, CliError> {
    value
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| invalid_input("offset must be a nonnegative integer"))
}

fn render_batch_page(
    store: &Store,
    project: &ProjectId,
    batch: &merl_core::BatchId,
    offset: usize,
    json_output: bool,
) -> Result<String, CliError> {
    let (changes, has_more) = store.batch_changes_page(project, batch, offset, 20)?;
    if json_output {
        render_json(&json!({
            "schema": "merl.batch-page/v1", "project": project.as_str(),
            "batch": batch.as_str(), "offset": offset,
            "changes": changes_json(&changes),
            "next_offset": has_more.then_some(offset + changes.len())
        }))
    } else {
        let mut output = format!("{batch} changes from offset {offset}\n");
        for change in &changes {
            writeln!(output, "{} {}", change.kind, change.reference).expect("String write");
        }
        if has_more {
            writeln!(output, "More changes at offset {}", offset + changes.len())
                .expect("String write");
        }
        Ok(output)
    }
}

fn coverage_json(coverage: merl_store::SemanticCoverage) -> Value {
    json!({
        "observation_head": coverage.observation_head,
        "processed_through": coverage.processed_through,
        "required_gaps": coverage.required_gaps,
        "required_pending": coverage.required_pending,
        "required_failed": coverage.required_failed,
        "required_purged": coverage.required_purged,
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
                "changes_truncated": batch.changes_truncated,
                "next_offset": batch.changes_truncated.then_some(batch.changes.len())
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
            if batch.changes_truncated {
                writeln!(
                    output,
                    "  More changes: merl project batch --batch {} --offset {}",
                    batch.batch,
                    batch.changes.len()
                )
                .expect("String write");
            }
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
                "required_purged": coverage.required_purged,
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
            output.push_str(&commands::view_lines(object));
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
            "Groups: project, issue, source, candidate, decision, question, finding, hypothesis, claim, task, inbox, show. Run `merl help <group>` for commands.",
            vec![
                "project",
                "issue",
                "source",
                "candidate",
                "decision",
                "question",
                "finding",
                "hypothesis",
                "claim",
                "task",
                "inbox",
                "show",
            ],
        ),
        "project" => (
            "merl project <command>",
            "Commands: init, revision, rebuild, view, delta, batch, authority.",
            vec![
                "project authority",
                "project init",
                "project revision",
                "project rebuild",
                "project view",
                "project delta",
                "project batch",
            ],
        ),
        "project init" => (
            "merl project init --id <id> --database <path> [--administrator <actor>] [--format json]",
            "Create a local project at revision zero and optionally establish its first administrator.",
            vec!["project revision"],
        ),
        "project revision" => (
            "merl project revision --project <id> --database <path> [--format json]",
            "Read the accepted project revision.",
            vec!["project init"],
        ),
        "project rebuild" => (
            "merl project rebuild --project <id> --database <path> [--format json]",
            "Rebuild accepted object and relation projections from domain events. The compiler does not run.",
            vec!["project revision", "project view"],
        ),
        "project view" => (
            "merl project view --project <id> --database <path> [--role researcher|engineer|pm] [--focus <object-id>] [--format json]",
            "Read focused work first, then current accepted work in role order. Pull evidence by reference.",
            vec!["issue view", "show", "project delta"],
        ),
        "project delta" => (
            "merl project delta --project <id> --database <path> --since <revision> [--format json]",
            "Read accepted changes after a project revision. Source text is omitted.",
            vec!["project revision", "project batch", "inbox poll"],
        ),
        "project batch" => (
            "merl project batch --project <id> --database <path> --batch <id> [--offset <n>] [--format json]",
            "Read a bounded page of references in one accepted batch, including after acknowledgement.",
            vec!["project delta", "inbox poll"],
        ),
        "issue" => (
            "merl issue <command>",
            "Commands: capture, import-fixture, view.",
            vec!["issue capture", "issue import-fixture", "issue view"],
        ),
        "issue view" => (
            "merl issue view --project <id> --database <path> --issue <id> --scope <provider-id> [--role researcher|engineer|pm] [--format json]",
            "Read current Issue facts and accepted semantics. Source text stays cold.",
            vec!["issue import-fixture"],
        ),
        "inbox" => (
            "merl inbox <command>",
            "Commands: subscribe, poll, show, ack.",
            vec!["inbox subscribe", "inbox poll", "inbox show", "inbox ack"],
        ),
        "inbox subscribe" => (
            "merl inbox subscribe --project <id> --database <path> --agent <id> [--format json]",
            "Subscribe an agent to accepted project changes.",
            vec!["inbox poll"],
        ),
        "inbox poll" => (
            "merl inbox poll --project <id> --database <path> --agent <id> [--format json]",
            "Read pending accepted changes for a subscribed agent.",
            vec!["inbox show", "inbox ack", "project delta"],
        ),
        "inbox show" => (
            "merl inbox show --project <id> --database <path> --agent <id> --revision <n> [--offset <n>] [--format json]",
            "Read another bounded page of one inbox batch; older acknowledged entries remain readable.",
            vec!["inbox poll", "inbox ack"],
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
            "Commands: show, compile, assertions, apply, require, replay, purge, purge-audit.",
            vec![
                "source show",
                "source compile",
                "source assertions",
                "source apply",
                "source require",
                "source replay",
                "source purge",
                "source purge-audit",
            ],
        ),
        "source show" => (
            "merl source show --project <id> --database <path> --version <id> [--format json]",
            "Read one captured source version. Erased bytes report unavailable.",
            vec!["show"],
        ),
        "source assertions" => (
            "merl source assertions --project <id> --database <path> --run <id> [--json]",
            "Inspect recorded assertions, unresolved spans, and context requests. Reading does not accept state.",
            vec!["source apply", "source show"],
        ),
        "source apply" => (
            "merl source apply --project <id> --database <path> --run <id> --actor <id> --id <request-id> [--dry-run] [--json]",
            "Evaluate a completed live run with current grants. Reuse the request ID for retries; use a new ID to reevaluate candidates. Only direct explicit decisions qualify for decision-author acceptance.",
            vec!["source assertions", "project authority", "show"],
        ),
        "source compile" => (
            "merl source compile --project <id> --database <path> --version <id> --run <id> --actor <id> --reason <text> --program <path> --compiler-version <version> --model <model> --prompt-digest sha256:<hex> [--compiler-arg <arg>] [--format json]",
            "Authorize and compile one retained source through a configured process adapter.",
            vec!["source show", "issue view"],
        ),
        "source require" => (
            "merl source require --project <id> --database <path> --version <id> --scope <id> --actor <id> --reason <text> [--format json]",
            "Make retained optional evidence part of the completeness contract for one scope. Identical retries are safe.",
            vec!["source show", "issue view"],
        ),
        "source purge" => (
            "merl source purge --project <id> --database <path> --version <id> --reason <text> (--dry-run | --actor <id> --confirm-digest sha256:<hex>) [--format json]",
            "Preview affected bytes and provenance, then confirm the digest to erase them from active Merl storage.",
            vec!["source show"],
        ),
        "source purge-audit" => (
            "merl source purge-audit --project <id> --database <path> --version <id> [--format json]",
            "Read the completed purge receipt and retained payload digests without restoring erased bytes.",
            vec!["source purge", "source show"],
        ),
        "source replay" => (
            "merl source replay --project <id> --database <path> --run <id> [--program <path> --new-run <id> --compiler-version <version> --model <id> --prompt-digest sha256:<hex>] [--format json]",
            "Rebuild a causal compiler input and check its digest. A configured process compiler creates a new replay run without accepting state.",
            vec!["source show", "project rebuild"],
        ),
        "issue capture" => (
            "merl issue capture --project <id> --database <path> --repository <owner/name> --issue <number> [--mode eager|capture_only|on_demand] [--coverage required|optional] [--program <path> --compiler-version <version> --model <id> --prompt-digest sha256:<hex>] [--github-program <path>] [--format json]",
            "Capture or refresh a GitHub Issue. Install gh and authenticate with gh auth login. A new binding defaults to eager/required; refresh reuses its durable policy. Eager work needs a configured compiler. Outcomes: captured, unchanged, failed, incomplete. Partial provider responses never imply deletions.",
            vec!["issue view", "source show", "source replay", "inbox poll"],
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
        "project rebuild" => {
            Some("merl project rebuild --project P1 --database project.sqlite --json")
        }
        "project view" => {
            Some("merl project view --project P1 --database project.sqlite --role pm --json")
        }
        "project delta" => {
            Some("merl project delta --project P1 --database project.sqlite --since 42 --json")
        }
        "project batch" => Some(
            "merl project batch --project P1 --database project.sqlite --batch DB42 --offset 20 --json",
        ),
        "issue capture" => Some(
            "merl issue capture --project P1 --database project.sqlite --repository acme/project --issue 204 --mode capture_only --coverage optional --json",
        ),
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
        "inbox show" => Some(
            "merl inbox show --project P1 --database project.sqlite --agent dev --revision 43 --offset 20 --json",
        ),
        "inbox ack" => Some(
            "merl inbox ack --project P1 --database project.sqlite --agent dev --revision 43 --json",
        ),
        "show" => {
            Some("merl show D18 --project P1 --database project.sqlite --history --source --json")
        }
        "source show" => {
            Some("merl source show --project P1 --database project.sqlite --version SV1 --json")
        }
        "source assertions" => {
            Some("merl source assertions --project P1 --database project.sqlite --run CR42 --json")
        }
        "source apply" => Some(
            "merl source apply --project P1 --database project.sqlite --run CR42 --actor worker --id apply-CR42-1 --json",
        ),
        "source require" => Some(
            "merl source require --project P1 --database project.sqlite --version SV1 --scope task:T42 --actor pm --reason 'Required safety evidence' --json",
        ),
        "source purge" => Some(
            "merl source purge --project P1 --database project.sqlite --version SV1 --reason 'Sensitive text' --dry-run --json",
        ),
        "source purge-audit" => Some(
            "merl source purge-audit --project P1 --database project.sqlite --version SV1 --json",
        ),
        "source replay" => {
            Some("merl source replay --project P1 --database project.sqlite --run CR42 --json")
        }
        _ => None,
    };
    if json_output {
        let errors = if command == "issue capture" {
            vec![
                "INVALID_INPUT",
                "PROJECT_NOT_FOUND",
                "COMPILER_REQUIRED",
                "AUTHORITY_BUSY",
                "IO_ERROR",
                "COMPILER_ADAPTER_ERROR",
                "SOURCE_CONFLICT",
                "STALE_PROVIDER_OBSERVATION",
                "STORAGE_ERROR",
            ]
        } else if command == "issue import-fixture" {
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
        } else if command == "source purge" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "INVALID_SOURCE",
                "INVALID_PURGE",
                "STORAGE_ERROR",
            ]
        } else if command == "source purge-audit" {
            vec![
                "INVALID_INPUT",
                "INVALID_ID",
                "PURGE_AUDIT_NOT_FOUND",
                "STORAGE_ERROR",
            ]
        } else if command == "source replay" {
            vec![
                "INVALID_INPUT",
                "NON_CAUSAL_HISTORY",
                "MISSING_EVIDENCE",
                "COMPILER_INPUT_BUDGET",
                "COMPILER_OUTPUT_BUDGET",
                "INVALID_COMPILER_RESPONSE",
                "COMPILER_ADAPTER_ERROR",
                "STORAGE_ERROR",
            ]
        } else if command == "source apply" {
            vec![
                "INVALID_INPUT",
                "ASSERTION_RUN_INELIGIBLE",
                "POLICY_INPUT_CONFLICT",
                "POLICY_ERROR",
                "CORRUPT_HISTORY",
                "STORAGE_ERROR",
            ]
        } else if command == "source assertions" {
            vec!["INVALID_INPUT", "CORRUPT_HISTORY", "STORAGE_ERROR"]
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

fn object_origin_json(
    store: &Store,
    project: &ProjectId,
    origin: Option<&merl_store::ObjectPolicyOrigin>,
    expand_source: bool,
) -> Result<Value, CliError> {
    let mut origin_json = Value::Null;
    if let Some(origin) = origin {
        let input = &origin.input;
        let mut input_json = json!({"kind": input.kind(), "id": input.id().as_str()});
        if let PolicyInput::ObservedAssertion { run, index, .. } = input {
            input_json["assertion"] = assertion_json(store, project, run, *index, expand_source)?;
        }
        if let PolicyInput::Command(id) = input
            && let Some(review) = store.candidate_review(project, id)?
        {
            input_json["review"] = candidates::review_json(store, project, &review, expand_source)?;
        }
        if let PolicyInput::Command(id) = input
            && let Some(command) = store.semantic_command(project, id)?
        {
            input_json["command"] = commands::lineage(store, project, &command, expand_source)?;
        }
        origin_json = json!({
            "event": origin.event.as_str(), "evaluation": origin.evaluation.as_str(), "input": input_json
        });
    }
    Ok(origin_json)
}
