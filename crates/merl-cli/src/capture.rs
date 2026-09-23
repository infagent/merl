//! CLI configuration and outcome rendering for live Issue capture.

use super::{CliError, first_release_compiler_limits, invalid_input, render_json};
use merl_core::ProjectId;
use merl_core::{CapturePolicyVersion, CompilationMode, CoverageRequirement};
use merl_ingest::live::{capture_issue, fetch_issue};
use merl_store::BindingCapturePolicy;
use merl_store::Store;
use serde_json::{Value, json};
use std::{fmt::Write as _, path::Path};

#[derive(Default)]
pub(super) struct Options<'a> {
    repository: Option<&'a str>,
    github_program: Option<&'a str>,
    observed_at: Option<&'a str>,
    mode: Option<&'a str>,
    coverage: Option<&'a str>,
}

impl<'a> Options<'a> {
    fn policy(&self) -> Result<BindingCapturePolicy, CliError> {
        let mode = match self.mode.unwrap_or("eager") {
            "eager" => CompilationMode::Eager,
            "capture_only" => CompilationMode::CaptureOnly,
            "on_demand" => CompilationMode::OnDemand,
            _ => {
                return Err(invalid_input(
                    "--mode must be eager, capture_only, or on_demand",
                ));
            }
        };
        let coverage = match self.coverage.unwrap_or("required") {
            "required" => CoverageRequirement::Required,
            "optional" => CoverageRequirement::Optional,
            _ => return Err(invalid_input("--coverage must be required or optional")),
        };
        Ok(BindingCapturePolicy {
            mode,
            coverage,
            version: CapturePolicyVersion::try_from("github_capture_v1")
                .map_err(|error| invalid_input(&error.to_string()))?,
        })
    }

    pub(super) fn set(&mut self, option: &str, value: &'a str) {
        match option {
            "--repository" => self.repository = Some(value),
            "--github-program" => self.github_program = Some(value),
            "--observed-at" => self.observed_at = Some(value),
            "--mode" => self.mode = Some(value),
            "--coverage" => self.coverage = Some(value),
            _ => unreachable!("capture option came from the CLI parser"),
        }
    }
}

pub(super) fn execute(
    database: &Path,
    project: &ProjectId,
    number: u64,
    options: &Options<'_>,
    adapter: Option<&merl_compiler::ProcessCompiler>,
    json_output: bool,
) -> Result<String, CliError> {
    let repository = options
        .repository
        .ok_or_else(|| invalid_input("--repository is required"))?;
    if repository
        .split_once('/')
        .is_none_or(|(owner, name)| owner.is_empty() || name.is_empty() || name.contains('/'))
    {
        return Err(invalid_input("--repository must be owner/name"));
    }
    let default = options.policy()?;
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| invalid_input(&error.to_string()))?;
    let observed_at = options.observed_at.unwrap_or(&now);
    time::OffsetDateTime::parse(observed_at, &time::format_description::well_known::Rfc3339)
        .map_err(|_| invalid_input("--observed-at must be RFC3339"))?;
    let mut store = Store::open(database)?;

    let _capture_lock = lock_capture(database)?;
    let revision = store.project_revision(project)?.get();
    let fixture = match fetch_issue(
        Path::new(options.github_program.unwrap_or("gh")),
        repository,
        number,
        observed_at,
    ) {
        Ok(fixture) => fixture,
        Err(error) => {
            let outcome = if matches!(error, merl_ingest::live::FetchError::Incomplete(_)) {
                "incomplete"
            } else {
                "failed"
            };
            let message = error.to_string();
            return if json_output {
                render_json(
                    &json!({"schema":"merl.issue-capture/v1", "action":"issue.capture", "project":project.as_str(), "outcome":outcome, "captured":0, "deleted":0, "compiled":0, "revision":revision, "warnings":[message]}),
                )
            } else {
                Ok(format!(
                    "Issue capture {outcome}: {message}. No observations captured.\n"
                ))
            };
        }
    };
    let report = capture_issue(
        &mut store,
        project,
        &fixture,
        &default,
        adapter,
        first_release_compiler_limits(),
    )?;
    let outcome = if !report.failures.is_empty() {
        "failed"
    } else if report.sources.is_empty() && !report.provider_changed {
        "unchanged"
    } else {
        "captured"
    };
    if json_output {
        render_json(&json!({
            "schema":"merl.issue-capture/v1", "action":"issue.capture", "project":project.as_str(), "outcome":outcome,
            "binding":report.binding.as_str(), "policy":{"mode":report.policy.mode.as_str(), "coverage":report.policy.coverage.as_str(), "version":report.policy.version.as_str()},
            "captured":report.sources.len(), "unchanged":report.unchanged, "deleted":report.deleted, "compiled":report.compiled,
            "failed":report.failures.len(), "warnings":report.failures, "observation_head":report.observation_head, "revision":report.revision, "provider_changed":report.provider_changed,
            "sources":report.sources.iter().map(|source| json!({"entity":source.entity, "version":source.version.as_str(), "run":source.run, "deleted":source.deleted, "upstream_deleted_at":Value::Null})).collect::<Vec<_>>()
        }))
    } else {
        let mut output = format!(
            "Issue capture {outcome}: {} captured, {} unchanged, {} deleted, {} compiled, {} failed; head {}, revision {}.\nBinding {}: {}, {}, policy {}.\n",
            report.sources.len(),
            report.unchanged,
            report.deleted,
            report.compiled,
            report.failures.len(),
            report.observation_head,
            report.revision,
            report.binding,
            report.policy.mode.as_str(),
            report.policy.coverage.as_str(),
            report.policy.version
        );
        for failure in report.failures {
            writeln!(output, "{failure}").expect("writing a string is infallible");
        }
        Ok(output)
    }
}

// The lock covers provider fetch and compiler dispatch, including time outside
// SQLite transactions. A second capture must not execute the same pending intent.
fn lock_capture(database: &Path) -> Result<std::fs::File, CliError> {
    let database = std::fs::canonicalize(database).map_err(|error| CliError {
        code: "IO_ERROR",
        message: error.to_string(),
    })?;
    let mut path = database.into_os_string();
    path.push(".capture.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| CliError {
            code: "IO_ERROR",
            message: error.to_string(),
        })?;
    file.try_lock().map_err(|error| CliError {
        code: "AUTHORITY_BUSY",
        message: format!("another Issue capture owns this authority: {error}"),
    })?;
    Ok(file)
}
