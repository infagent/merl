//! Render one help topic from the same contract in either output format.
//!
//! Child entries contain names only. A caller loads each child's contract on demand.

mod errors;

use crate::{CliError, commands, invalid_input, render_json};
use serde_json::json;
use std::fmt::Write as _;

/// Renders a command's metadata without loading its relatives' help records.
pub(super) fn render(
    command: &str,
    usage: &str,
    summary: &str,
    example: Option<&str>,
    related: &[&str],
    json_output: bool,
) -> Result<String, CliError> {
    let children = children(command);
    let group = !children.is_empty() && command != "source compilation-policy";
    let (outcomes, errors) = if group {
        (Vec::new(), Vec::new())
    } else {
        (outcomes(command)?, errors::for_command(command)?)
    };
    let trust = (usage.contains("--actor ") || command.starts_with("project authority"))
        .then_some(crate::security::ACTOR_CLAIM);
    if json_output {
        // Keep the older aliases for callers of the v1 records from individual modules.
        let arguments = usage
            .strip_prefix(&format!("merl {command} "))
            .unwrap_or(usage);
        let operations: Vec<_> = children
            .iter()
            .filter_map(|child| child.strip_prefix(&format!("{command} ")))
            .collect();
        render_json(&json!({
            "schema": "merl.help/v1", "command": command,
            "kind": if group { "group" } else { "command" },
            "usage": usage, "summary": summary, "description": summary,
            "example": example, "examples": example.into_iter().collect::<Vec<_>>(),
            "arguments": [arguments], "outcomes": outcomes, "errors": errors,
            "related": related, "children": children,
            "commands": if group { operations } else { Vec::new() },
            "trust_boundary": trust,
        }))
    } else {
        let mut output = format!("{usage}\n{summary}\n");
        if !children.is_empty() {
            writeln!(output, "Commands: {}", children.join(", ")).expect("String write");
        }
        if let Some(trust) = trust {
            writeln!(output, "{trust}").expect("String write");
        }
        if let Some(example) = example {
            writeln!(
                output,
                "Outcomes: {}\nErrors: {}\nExample: {example}",
                outcomes.join(", "),
                errors.join(", ")
            )
            .expect("String write");
        }
        if !related.is_empty() {
            writeln!(output, "Related: {}", related.join(", ")).expect("String write");
        }
        Ok(output)
    }
}

/// Lists direct children, including `set` beneath the executable policy inspector.
fn children(command: &str) -> Vec<String> {
    let names: &[&str] = match command {
        "" => &[
            "project",
            "issue",
            "source",
            "compilation",
            "candidate",
            "decision",
            "question",
            "finding",
            "hypothesis",
            "claim",
            "task",
            "inbox",
            "show",
            "security",
        ],
        "project" => &[
            "init",
            "revision",
            "rebuild",
            "view",
            "delta",
            "batch",
            "authority",
            "revalidation",
        ],
        "project authority" => &["list", "grant", "revoke"],
        "project revalidation" => &["list", "run", "resolve"],
        "issue" => &["capture", "import-fixture", "view"],
        "source" => &[
            "show",
            "compilation-policy",
            "compile",
            "assertions",
            "apply",
            "require",
            "replay",
            "purge",
            "purge-audit",
        ],
        "source compilation-policy" => &["set"],
        "compilation" => &["list", "show", "context", "expand"],
        "candidate" => &["list", "show", "accept", "reject", "correct"],
        "decision" | "hypothesis" | "claim" => &["create"],
        "question" | "finding" => &["create", "resolve"],
        "task" => &["request", "accept", "defer", "start", "complete"],
        "inbox" => &["subscribe", "poll", "show", "ack"],
        "security" => &["explain"],
        _ => &[],
    };
    names
        .iter()
        .map(|name| {
            if command.is_empty() {
                (*name).into()
            } else {
                format!("{command} {name}")
            }
        })
        .collect()
}

/// Names emitted outcomes, or describes data when a result has no outcome field.
fn outcomes(command: &str) -> Result<Vec<&'static str>, CliError> {
    let outcomes: &[&str] = match command {
        "security explain" => &["explained"],
        "project init" | "project revision" => &["accepted"],
        "project rebuild" => &["rebuilt projections and provenance availability"],
        "project view" | "issue view" => &["accepted state with semantic coverage"],
        "project delta" => &["accepted batches after the supplied revision"],
        "project batch" | "inbox show" => &["bounded page of batch references"],
        "project authority list" => &["effective grants and configuration digest"],
        "project authority grant"
        | "project authority revoke"
        | "source compilation-policy set"
        | "project revalidation resolve" => &["accepted", "rejected", "conflict"],
        "project revalidation list" => &["pending evidence impacts and continuation cursor"],
        "project revalidation run" => &["succeeded", "needs_context", "failed"],
        "issue capture" => &["captured", "unchanged", "failed", "incomplete"],
        "issue import-fixture" => &["captured", "unchanged"],
        "inbox subscribe" => &["subscription and current cursor"],
        "inbox poll" => &["pending entries and semantic coverage"],
        "inbox ack" => &["acknowledged cursor; repeated acknowledgement unchanged"],
        "show" => &["accepted object with requested history and evidence availability"],
        "source show" => &["available", "unavailable"],
        "source compilation-policy" => &["binding policy or recorded source policy selection"],
        "source assertions" => &["recorded assertions, relations, and context requests"],
        "source apply" => &[
            "accepted",
            "candidate",
            "rejected",
            "duplicate",
            "conflict",
            "dry-run preview",
        ],
        "source compile" => &["compiled", "unchanged", "rejected"],
        "source require" => &["promoted", "unchanged", "rejected", "conflict"],
        "source purge" => &["purge preview", "completed purge receipt"],
        "source purge-audit" => &["purge receipt and retained digests"],
        "source replay" => &["matching input digest; optional replay run without accepted changes"],
        "compilation list" => &["expansion requests and continuation offset"],
        "compilation show" | "compilation context" | "compilation expand" => {
            &["pending", "needs_context", "succeeded", "failed"]
        }
        "candidate list" | "candidate show" => &["pending", "accepted", "rejected", "corrected"],
        "candidate accept" | "candidate reject" | "candidate correct" => {
            &["accepted", "rejected", "conflict", "dry-run preview"]
        }
        _ if command
            .split_once(' ')
            .is_some_and(|(group, _)| commands::is_group(group)) =>
        {
            &["accepted", "rejected", "conflict", "dry-run preview"]
        }
        _ => return Err(invalid_input("command has no help outcome contract")),
    };
    Ok(outcomes.to_vec())
}
