//! Public administration of project semantic grants.

use crate::{CliError, invalid_input, parse_project, render_json};
use merl_core::{ActorId, AuthorityPermission, PolicyInputId};
use merl_policy::{AuthorityChange, PolicyRules, change_authority};
use merl_store::Store;
use serde_json::json;
use std::fmt::Write as _;
use std::path::Path;

#[derive(Clone, Copy)]
pub(super) struct Options<'a> {
    pub database: Option<&'a str>,
    pub project: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub permission: Option<&'a str>,
    pub id: Option<&'a str>,
    pub reason: Option<&'a str>,
}

pub(super) fn execute(
    operation: &str,
    options: Options<'_>,
    json_output: bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    if !matches!(operation, "list" | "grant" | "revoke") {
        return Err(invalid_input(
            "authority command must be list, grant, or revoke",
        ));
    }
    let project = parse_project(required(options.project, "--project")?)?;
    let mut store = Store::open(Path::new(required(options.database, "--database")?))?;
    if operation == "list" {
        let rules = PolicyRules::from_store(&store, &project)?;
        let revision = store.project_revision(&project)?;
        let actors =
            |values: &[ActorId]| values.iter().map(ToString::to_string).collect::<Vec<_>>();
        return if json_output {
            render_json(&json!({
                "schema": "merl.authority/v1", "action": "project.authority.list",
                "project": project.as_str(), "revision": revision.get(),
                "policy_version": rules.version.as_str(),
                "configuration_digest": hex(&rules.configuration_digest()),
                "administrators": actors(&rules.administrators),
                "decision_authors": actors(&rules.decision_authors),
                "command_actors": actors(&rules.command_actors),
            }))
        } else {
            Ok(format!(
                "Authority for {project} at revision {} ({})\nAdministrators: {}\nDecision authors: {}\nCommand actors: {}\n",
                revision.get(),
                rules.version,
                actors(&rules.administrators).join(", "),
                actors(&rules.decision_authors).join(", "),
                actors(&rules.command_actors).join(", ")
            ))
        };
    }
    let change = AuthorityChange {
        id: PolicyInputId::try_from(required(options.id, "--id")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        actor: ActorId::try_from(required(options.actor, "--actor")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        subject: ActorId::try_from(required(options.subject, "--subject")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        permission: AuthorityPermission::try_from(required(options.permission, "--permission")?)
            .map_err(|e| invalid_input(&e.to_string()))?,
        grant: operation == "grant",
        reason: required(options.reason, "--reason")?.to_owned(),
    };
    let record = change_authority(&mut store, &project, &change, clock()?)?;
    let outcome = record.inputs[0].disposition.as_str();
    if json_output {
        render_json(&json!({
            "schema": "merl.authority-change/v1", "action": format!("project.authority.{operation}"),
            "project": project.as_str(), "actor": change.actor.as_str(), "subject": change.subject.as_str(),
            "permission": change.permission.as_str(), "request": change.id.as_str(),
            "outcome": outcome, "revision": record.committed_revision.map(merl_core::ProjectRevision::get),
            "evaluation": record.id.as_str(), "policy_version": record.version.as_str(),
            "configuration_digest": hex(&record.configuration_digest),
        }))
    } else {
        Ok(format!(
            "{outcome}: {operation} {} for {} by {}; accepted revision {}; policy {}.\n",
            change.permission.as_str(),
            change.subject,
            change.actor,
            record
                .committed_revision
                .map_or_else(|| "none".to_owned(), |r| r.get().to_string()),
            record.version
        ))
    }
}

fn required<'a>(value: Option<&'a str>, name: &str) -> Result<&'a str, CliError> {
    value.ok_or_else(|| invalid_input(&format!("{name} is required")))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

pub(super) fn help(operation: Option<&str>, json_output: bool) -> Result<String, CliError> {
    let (command, usage, summary, example) = match operation {
        None => (
            "project authority",
            "merl project authority <list|grant|revoke>",
            "Inspect project grants or change semantic authority as an administrator.",
            "merl help project authority grant",
        ),
        Some("list") => (
            "project authority list",
            "merl project authority list --project <id> --database <path> [--json]",
            "Read effective grants and their policy configuration digest.",
            "merl project authority list --project P1 --database project.sqlite --json",
        ),
        Some("grant") => (
            "project authority grant",
            "merl project authority grant --project <id> --database <path> --id <request> --actor <administrator> --subject <actor> --permission <decision_author|command_actor> --reason <text> [--json]",
            "Grant one semantic permission through administrative policy.",
            "merl project authority grant --project P1 --database project.sqlite --id grant_alice --actor owner --subject alice --permission decision_author --reason 'Project decision owner' --json",
        ),
        Some("revoke") => (
            "project authority revoke",
            "merl project authority revoke --project <id> --database <path> --id <request> --actor <administrator> --subject <actor> --permission <decision_author|command_actor> --reason <text> [--json]",
            "Revoke one semantic permission while preserving accepted history.",
            "merl project authority revoke --project P1 --database project.sqlite --id revoke_alice --actor owner --subject alice --permission decision_author --reason 'Role ended' --json",
        ),
        _ => return Err(invalid_input("unknown authority command")),
    };
    let trust = crate::security::ACTOR_CLAIM;
    if json_output {
        render_json(
            &json!({"schema": "merl.help/v1", "command": command, "usage": usage,
            "summary": summary, "example": example, "trust_boundary": trust,
            "outcomes": ["accepted", "rejected", "conflict"],
            "errors": ["INVALID_INPUT", "PROJECT_NOT_FOUND", "POLICY_INPUT_CONFLICT", "POLICY_CONFLICT", "POLICY_ERROR", "STORAGE_ERROR"],
            "related": ["project authority list", "project authority grant", "project authority revoke"]}),
        )
    } else {
        Ok(format!(
            "{usage}\n\n{summary}\n{trust}\n\nExample: {example}\n"
        ))
    }
}
