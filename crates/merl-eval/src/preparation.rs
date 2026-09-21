//! Evaluator checks for a prepared Merl authority before any reader sees it.

use std::{collections::HashSet, fs, path::Path};

use merl_core::ProjectId;
use merl_corpus::fixture::{BodyAvailability, Fixture, body_availability};
use merl_ingest::fixture_version_id;
use merl_store::{CompilationRunStatus, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::TokenUsage;

const RECORD_SCHEMA: &str = "merl.eval-preparation/v1";

/// Frozen compiler files used when the authority was prepared.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerArtifacts {
    /// Executable that produced structured assertions.
    pub program: std::path::PathBuf,
    /// Compiler instruction text.
    pub prompt: std::path::PathBuf,
    /// Deterministic extraction and policy rules supplied to the compiler.
    pub rules: std::path::PathBuf,
    /// Model and adapter configuration.
    pub config: std::path::PathBuf,
}

/// One durable compiler attempt that helped prepare a candidate authority.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttestedRun {
    /// Run identity in the prepared authority.
    pub id: String,
    /// Live or causal replay; hindsight and eval runs cannot prepare this arm.
    pub mode: String,
    /// Source observation cutoff recorded on the compiler run.
    pub source_cutoff: u64,
    /// Selector that chose its historical object revisions.
    pub selector_version: String,
    /// Renderer that produced its retained input bytes.
    pub renderer_version: String,
}

/// Provider-reported cost of one preparation call, including retries and corrections.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PreparationCall {
    /// Stable evaluator call identity; repeated run IDs still need distinct calls.
    pub id: String,
    /// Recorded run whose work incurred this cost.
    pub run_id: String,
    /// Compile, retry, correction, or revalidation.
    pub phase: String,
    /// Provider-reported usage, including zero when a failed call returned none.
    pub usage: TokenUsage,
}

/// Files and metadata that tie a prepared database to one candidate.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompilerAttestation {
    /// Compiler implementation ID stored with every run.
    pub id: String,
    /// Compiler implementation version stored with every run.
    pub version: String,
    /// Model identity stored with every run.
    pub model: String,
    /// Artifact hashes checked against the files in the evaluation plan.
    pub program_sha256: String,
    /// Prompt hash.
    pub prompt_sha256: String,
    /// Rules hash.
    pub rules_sha256: String,
    /// Configuration hash.
    pub config_sha256: String,
    /// Domain-separated hash of the four hashes above, stored as each run's prompt digest.
    pub contract_sha256: String,
}

/// Preparation receipt checked against the database and candidate artifacts.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MerlPreparationRecord {
    /// Versioned evaluator record shape.
    pub schema: String,
    /// Paired trial this independent database serves.
    pub trial_id: String,
    /// Project contained in the prepared authority.
    pub project: String,
    /// Highest source observation allowed to contribute to this view.
    pub source_cutoff: u64,
    /// Candidate source revision that built the authority.
    pub candidate_commit: String,
    /// Candidate evaluator executable hash; the binary embeds the Merl crates.
    pub candidate_binary_sha256: String,
    /// Compiler files and run identity.
    pub compiler: CompilerAttestation,
    /// Every compiler run in the prepared authority, in durable attempt order.
    pub runs: Vec<AttestedRun>,
    /// Every measured preparation call, including failed retries.
    pub calls: Vec<PreparationCall>,
    /// Sum of all reported preparation calls.
    pub total_usage: TokenUsage,
}

/// Candidate and paired-trial identity expected by the evaluator.
#[derive(Debug)]
pub struct PreparationExpectation<'a> {
    /// Stable paired-trial ID.
    pub trial_id: &'a str,
    /// Project whose authority is under test.
    pub project: &'a str,
    /// Question cutoff shared by every method.
    pub source_cutoff: u64,
    /// Candidate source revision.
    pub candidate_commit: &'a str,
    /// Hash of the evaluator executable embedding this Merl candidate.
    pub candidate_binary_sha256: &'a str,
}

/// Verify one prepared authority without trusting a caller-supplied causal flag.
///
/// # Errors
/// Rejects changed candidate files, missing or extra runs, hindsight work,
/// mismatched compiler versions, and incomplete token accounting.
pub fn verify_preparation(
    record: &MerlPreparationRecord,
    database: &Path,
    artifacts: &CompilerArtifacts,
    fixture: &Fixture,
    expected: &PreparationExpectation<'_>,
) -> Result<TokenUsage, String> {
    if record.schema != RECORD_SCHEMA
        || record.trial_id != expected.trial_id
        || record.project != expected.project
        || record.source_cutoff != expected.source_cutoff
        || record.candidate_commit != expected.candidate_commit
        || record.candidate_binary_sha256 != expected.candidate_binary_sha256
    {
        return Err("preparation record does not match this candidate trial".to_owned());
    }
    let hashes = [
        file_digest(&artifacts.program)?,
        file_digest(&artifacts.prompt)?,
        file_digest(&artifacts.rules)?,
        file_digest(&artifacts.config)?,
    ];
    let claimed = [
        &record.compiler.program_sha256,
        &record.compiler.prompt_sha256,
        &record.compiler.rules_sha256,
        &record.compiler.config_sha256,
    ];
    if hashes
        .iter()
        .zip(claimed)
        .any(|(actual, claim)| actual != claim)
    {
        return Err("compiler artifact changed after preparation".to_owned());
    }
    let contract = contract_digest(&hashes);
    if record.compiler.contract_sha256 != digest_text(&contract) {
        return Err("compiler contract digest does not match its artifacts".to_owned());
    }

    let project_id = ProjectId::try_from(expected.project).map_err(|_| "invalid project ID")?;
    let store = Store::open(database).map_err(|error| error.to_string())?;
    verify_source_availability(&store, &project_id, fixture, expected.source_cutoff)?;
    let run_ids = store
        .compilation_run_ids(&project_id)
        .map_err(|error| error.to_string())?;
    if run_ids.is_empty()
        || run_ids
            != record
                .runs
                .iter()
                .map(|run| run.id.clone())
                .collect::<Vec<_>>()
    {
        return Err("preparation record omits or adds compiler runs".to_owned());
    }
    for run in &record.runs {
        let status = store
            .compilation_run_status(&project_id, &run.id)
            .map_err(|error| error.to_string())?
            .ok_or("preparation run is missing")?;
        validate_run(record, run, &status, &contract)?;
        if run.source_cutoff < fixture.observations.len() as u64 {
            let selected = store
                .compilation_context_sources(&project_id, &run.id)
                .map_err(|error| error.to_string())?;
            for source in &fixture.observations {
                if body_availability(fixture, source) == Some(BodyAvailability::AtCapture)
                    && selected.contains(
                        &fixture_version_id(&source.version_id)
                            .map_err(|error| error.to_string())?,
                    )
                {
                    return Err("early compiler context selected terminal-only body".to_owned());
                }
            }
        }
    }
    verify_usage(record, &run_ids)
}

/// Digest supplied as the compiler prompt/configuration identity when preparing
/// a candidate authority. It binds executable, prompt, rules, and configuration.
///
/// # Errors
/// Returns an error when any frozen compiler artifact cannot be read.
pub fn compiler_contract_sha256(artifacts: &CompilerArtifacts) -> Result<String, String> {
    let hashes = [
        file_digest(&artifacts.program)?,
        file_digest(&artifacts.prompt)?,
        file_digest(&artifacts.rules)?,
        file_digest(&artifacts.config)?,
    ];
    Ok(digest_text(&contract_digest(&hashes)))
}

fn verify_usage(record: &MerlPreparationRecord, run_ids: &[String]) -> Result<TokenUsage, String> {
    let mut call_ids = HashSet::new();
    let mut calls_by_run = HashSet::new();
    let mut total = TokenUsage::default();
    for call in &record.calls {
        if call.id.is_empty()
            || !call_ids.insert(call.id.as_str())
            || !run_ids.iter().any(|run| run == &call.run_id)
            || !matches!(
                call.phase.as_str(),
                "compile" | "retry" | "correction" | "revalidation"
            )
        {
            return Err("invalid preparation call lineage".to_owned());
        }
        calls_by_run.insert(call.run_id.as_str());
        total.input = total
            .input
            .checked_add(call.usage.input)
            .ok_or("usage overflow")?;
        total.output = total
            .output
            .checked_add(call.usage.output)
            .ok_or("usage overflow")?;
    }
    if total != record.total_usage
        || run_ids
            .iter()
            .any(|run| !calls_by_run.contains(run.as_str()))
    {
        return Err("preparation token usage is incomplete".to_owned());
    }
    Ok(total)
}

fn verify_source_availability(
    store: &Store,
    project: &ProjectId,
    fixture: &Fixture,
    cutoff: u64,
) -> Result<(), String> {
    if store
        .source_observation_head(project)
        .map_err(|error| error.to_string())?
        != cutoff
    {
        return Err("prepared authority source head differs from the question cutoff".to_owned());
    }
    for source in fixture
        .observations
        .iter()
        .filter(|source| source.sequence <= cutoff)
    {
        let stored = store
            .source_version_at(project, source.sequence)
            .map_err(|error| error.to_string())?
            .ok_or("prepared authority omitted a source observation")?;
        if stored.id != fixture_version_id(&source.version_id).map_err(|error| error.to_string())? {
            return Err("prepared source version differs from the fixture".to_owned());
        }
        let allowed = matches!(
            body_availability(fixture, source),
            Some(BodyAvailability::AtObservation)
        ) || (cutoff == fixture.observations.len() as u64
            && body_availability(fixture, source) == Some(BodyAvailability::AtCapture));
        if !allowed && stored.payload.is_some() {
            return Err(
                "prepared authority contains source bytes unavailable at cutoff".to_owned(),
            );
        }
        if let (Some(body), Some(_)) = (&source.body, &stored.payload)
            && stored.body_digest != Some(Sha256::digest(body.as_bytes()).into())
        {
            return Err("prepared source body differs from the fixture".to_owned());
        }
    }
    Ok(())
}

fn validate_run(
    record: &MerlPreparationRecord,
    run: &AttestedRun,
    status: &CompilationRunStatus,
    contract: &[u8; 32],
) -> Result<(), String> {
    if !matches!(status.mode.as_str(), "live" | "replay")
        || run.mode != status.mode
        || !status.completed
        || status.source_observation_cutoff != run.source_cutoff
        || run.source_cutoff > record.source_cutoff
        || status.compiler_id != record.compiler.id
        || status.compiler_version != record.compiler.version
        || status.model_id != record.compiler.model
        || &status.prompt_digest != contract
        || status.selector_version != run.selector_version
        || status.renderer_version != run.renderer_version
    {
        return Err("prepared authority has noncausal or mismatched compiler work".to_owned());
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read compiler artifact: {error}"))?;
    Ok(digest_text(&Sha256::digest(bytes)))
}

fn contract_digest(hashes: &[String; 4]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"merl.eval-compiler-contract/v1\0");
    for value in hashes {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hash.finalize().into()
}

fn digest_text(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::from("sha256:");
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use merl_core::{ProjectRevision, SourceVersionId};
    use merl_store::CompilationRunStatus;

    use super::{
        AttestedRun, CompilerAttestation, MerlPreparationRecord, TokenUsage, digest_text,
        validate_run,
    };

    #[test]
    fn causal_preparation_rejects_hindsight_and_changed_compiler_identity() {
        let contract = [7_u8; 32];
        let record = MerlPreparationRecord {
            schema: "merl.eval-preparation/v1".to_owned(),
            trial_id: "pair-a".to_owned(),
            project: "P1".to_owned(),
            source_cutoff: 4,
            candidate_commit: "a".repeat(40),
            candidate_binary_sha256: digest_text(&contract),
            compiler: CompilerAttestation {
                id: "process".to_owned(),
                version: "v1".to_owned(),
                model: "model-v1".to_owned(),
                program_sha256: digest_text(&contract),
                prompt_sha256: digest_text(&contract),
                rules_sha256: digest_text(&contract),
                config_sha256: digest_text(&contract),
                contract_sha256: digest_text(&contract),
            },
            runs: vec![],
            calls: vec![],
            total_usage: TokenUsage::default(),
        };
        let run = AttestedRun {
            id: "CR1".to_owned(),
            mode: "live".to_owned(),
            source_cutoff: 2,
            selector_version: "issue_context_v1".to_owned(),
            renderer_version: "json_v1".to_owned(),
        };
        let mut stored = CompilationRunStatus {
            attempt_order: 1,
            source: SourceVersionId::try_from("SV1").expect("source ID"),
            mode: "live".to_owned(),
            succeeded: true,
            needs_context: false,
            completed: true,
            started_at_millis: 1,
            failure_code: None,
            interpretation_basis_revision: ProjectRevision::from(0),
            source_observation_cutoff: 2,
            context_digest: [0; 32],
            renderer_version: "json_v1".to_owned(),
            selector_version: "issue_context_v1".to_owned(),
            compiler_id: "process".to_owned(),
            compiler_version: "v1".to_owned(),
            model_id: "model-v1".to_owned(),
            prompt_digest: contract,
            limits: [1; 9],
        };
        assert!(validate_run(&record, &run, &stored, &contract).is_ok());
        stored.mode = "hindsight".to_owned();
        assert!(validate_run(&record, &run, &stored, &contract).is_err());
        stored.mode = "live".to_owned();
        stored.prompt_digest = [8; 32];
        assert!(validate_run(&record, &run, &stored, &contract).is_err());
    }
}
