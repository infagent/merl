//! Explain the local trust contract without inspecting files or host isolation.

use std::fmt::Write as _;

use serde_json::json;

use crate::{CliError, render_json};

/// The actor flag records a cooperating client's claim at the local boundary.
pub(super) const ACTOR_CLAIM: &str = "--actor is a trusted local audit claim from a cooperating client, not proof of identity. See `merl security explain`.";

/// Render the same local boundary in human and machine output.
pub(super) fn explain(json_output: bool) -> Result<String, CliError> {
    let sections = [
        (
            "trust_boundary",
            "Trust boundary",
            "Unrestricted same-user processes are trusted in local mode.",
        ),
        ("actor_claim", "Actor claim", ACTOR_CLAIM),
        (
            "exposed_files",
            "Exposed files",
            "Unrestricted processes running as the same OS user can read the SQLite database, captured payloads, provider credentials available to that user, and other local files: configuration, caches, pending commands, artifacts, logs, and workspaces.",
        ),
        (
            "provided_checks",
            "Merl checks",
            "Merl provides policy governance, validation, provenance, and audit for cooperating clients that use its API.",
        ),
        (
            "limitations",
            "Limits",
            "Local policy cannot prevent file access, state changes outside the API, or actor impersonation by unrestricted same-user processes. Merl does not authenticate a caller through --actor and is not a sandbox.",
        ),
        (
            "host_controls",
            "Host controls",
            "This command does not inspect host controls or verify isolation. The first-release local CLI supplies no process isolation.",
        ),
        (
            "isolation_options",
            "Isolation options",
            "To constrain local agents, operators must configure a separate daemon identity with restricted IPC and filesystem permissions, a host sandbox or container that restricts file and credential access, or a credential broker. A shared authority can authenticate remote clients and enforce its API boundary; it cannot protect an unrestricted client host. Shared authorities are outside the first release.",
        ),
    ];
    if json_output {
        let mut result = json!({
            "schema": "merl.security/v1",
            "action": "security.explain",
            "outcome": "explained",
            "deployment": "local"
        });
        for (key, _, explanation) in sections {
            result[key] = json!(explanation);
        }
        render_json(&result)
    } else {
        let mut output = String::from("Local security (explained)\n");
        for (_, label, explanation) in sections {
            writeln!(output, "\n{label}: {explanation}").expect("String write");
        }
        Ok(output)
    }
}
