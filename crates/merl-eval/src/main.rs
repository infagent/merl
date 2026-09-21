//! Evaluator-side runner for a frozen five-method Issue comparison.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use merl_corpus::fixture::{Fixture, Partition, validate};
use merl_eval::{
    BenchmarkConfig, BenchmarkRunner, CliMerlSurface, CompilerArtifacts, EvaluationQuestion,
    MerlPreparationRecord, MerlTrialArtifact, PreparationExpectation, ProcessModelAdapter,
    ProcessScorer, compiler_contract_sha256, verify_preparation,
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

const PLAN_SCHEMA: &str = "merl.eval-plan/v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: String,
    fixture: PathBuf,
    questions: Vec<EvaluationQuestion>,
    config: BenchmarkConfig,
    model_program: PathBuf,
    scorer_program: PathBuf,
    candidate_commit: String,
    compiler_artifacts: CompilerArtifacts,
    merl: MerlPlan,
    limits: ProcessLimits,
    freeze: Option<FreezeRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MerlPlan {
    project: String,
    issue: String,
    scope: String,
    role: String,
    trials: Vec<MerlTrialPlan>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MerlTrialPlan {
    trial_id: String,
    source_cutoff: u64,
    capture_phase: bool,
    database: PathBuf,
    preparation_record: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessLimits {
    max_request_bytes: usize,
    max_response_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreezeRecord {
    candidate_commit: String,
    candidate_binary_sha256: String,
    approved_manifest: PathBuf,
    approved_manifest_sha256: String,
    fixture_sha256: String,
    question_sha256: String,
    config_sha256: String,
    model_program_sha256: String,
    scorer_program_sha256: String,
    scoring_spec: PathBuf,
    scoring_spec_sha256: String,
    preparation_record_sha256: Vec<String>,
    merl_database_sha256: Vec<String>,
}

struct VerifiedTrials {
    artifacts: Vec<MerlTrialArtifact>,
    record_digests: Vec<String>,
    database_digests: Vec<String>,
    databases: Vec<PathBuf>,
}

fn main() {
    let arguments = env::args().collect::<Vec<_>>();
    if arguments.len() == 2 && arguments[1] == "--help" {
        println!(
            "merl-eval <inspect|run> --plan <path>\nInspect freeze digests or run five paired Issue readers. Held-out runs require a freeze record."
        );
        return;
    }
    if arguments.len() != 4
        || arguments[2] != "--plan"
        || !matches!(arguments[1].as_str(), "inspect" | "run")
    {
        eprintln!("usage: merl-eval <inspect|run> --plan <path>");
        process::exit(2);
    }
    let path = PathBuf::from(&arguments[3]);
    let result = if arguments[1] == "inspect" {
        inspect(&path)
    } else {
        run(&path)
    };
    match result {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("{error}");
            process::exit(1);
        }
    }
}

fn inspect(path: &PathBuf) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("could not read plan: {error}"))?;
    let plan: Plan =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid plan: {error}"))?;
    if plan.schema != PLAN_SCHEMA {
        return Err("unsupported evaluation plan schema".to_owned());
    }
    let binary = env::current_exe().map_err(|error| error.to_string())?;
    let output = json!({
        "schema": "merl.eval-freeze-inputs/v1",
        "evaluator_binary_sha256": digest(&fs::read(binary).map_err(|error| error.to_string())?),
        "fixture_sha256": digest(&fs::read(&plan.fixture).map_err(|error| error.to_string())?),
        "question_sha256": digest(&serde_json::to_vec(&plan.questions).map_err(|error| error.to_string())?),
        "config_sha256": digest(&serde_json::to_vec(&plan.config).map_err(|error| error.to_string())?),
        "model_program_sha256": digest(&fs::read(&plan.model_program).map_err(|error| error.to_string())?),
        "scorer_program_sha256": digest(&fs::read(&plan.scorer_program).map_err(|error| error.to_string())?),
        "compiler_program_sha256": digest(&fs::read(&plan.compiler_artifacts.program).map_err(|error| error.to_string())?),
        "compiler_prompt_sha256": digest(&fs::read(&plan.compiler_artifacts.prompt).map_err(|error| error.to_string())?),
        "compiler_rules_sha256": digest(&fs::read(&plan.compiler_artifacts.rules).map_err(|error| error.to_string())?),
        "compiler_config_sha256": digest(&fs::read(&plan.compiler_artifacts.config).map_err(|error| error.to_string())?),
        "compiler_contract_sha256": compiler_contract_sha256(&plan.compiler_artifacts)?,
        "merl_preparation_record_sha256": plan.merl.trials.iter().map(|trial| fs::read(&trial.preparation_record).map(|bytes| digest(&bytes))).collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?,
        "merl_database_sha256": plan.merl.trials.iter().map(|trial| fs::read(&trial.database).map(|bytes| digest(&bytes))).collect::<Result<Vec<_>,_>>().map_err(|error| error.to_string())?,
        "approved_manifest_sha256": plan.freeze.as_ref().map(|record| fs::read(&record.approved_manifest).map(|bytes| digest(&bytes))).transpose().map_err(|error| error.to_string())?,
        "scoring_spec_sha256": plan.freeze.as_ref().map(|record| fs::read(&record.scoring_spec).map(|bytes| digest(&bytes))).transpose().map_err(|error| error.to_string())?
    });
    serde_json::to_string_pretty(&output).map_err(|error| error.to_string())
}

fn run(path: &PathBuf) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("could not read plan: {error}"))?;
    let plan: Plan =
        serde_json::from_slice(&bytes).map_err(|error| format!("invalid plan: {error}"))?;
    if plan.schema != PLAN_SCHEMA {
        return Err("unsupported evaluation plan schema".to_owned());
    }
    let fixture_bytes =
        fs::read(&plan.fixture).map_err(|error| format!("could not read fixture: {error}"))?;
    let fixture: Fixture = serde_json::from_slice(&fixture_bytes)
        .map_err(|error| format!("invalid fixture JSON: {error}"))?;
    validate(&fixture).map_err(|error| format!("invalid fixture: {error}"))?;
    if matches!(fixture.partition, Partition::HeldOut) {
        verify_freeze(&plan, &fixture_bytes)?;
    }
    let prepared = verify_trial_artifacts(&plan, &fixture)?;

    let mut model = ProcessModelAdapter::new(
        plan.model_program,
        plan.limits.max_request_bytes,
        plan.limits.max_response_bytes,
    );
    let mut scorer = ProcessScorer::new(plan.scorer_program, plan.limits.max_response_bytes);
    if let Some(freeze) = &plan.freeze {
        scorer.scoring_spec = Some(freeze.scoring_spec.clone());
    }
    let mut merl = CliMerlSurface::new(
        plan.merl.project,
        plan.merl.issue,
        plan.merl.scope,
        plan.merl.role,
        prepared.artifacts,
    );
    let report = BenchmarkRunner {
        model: &mut model,
        merl: &mut merl,
        scorer: &mut scorer,
    }
    .run_suite(&fixture, plan.questions, plan.config)
    .map_err(|error| error.to_string())?;
    for (database, expected_digest) in prepared.databases.iter().zip(&prepared.database_digests) {
        require_closed_database(database)?;
        let current = fs::read(database)
            .map_err(|error| format!("could not reread prepared Merl authority: {error}"))?;
        if &digest(&current) != expected_digest {
            return Err("a prepared Merl authority changed during the benchmark".to_owned());
        }
    }
    serde_json::to_string_pretty(&json!({
        "schema": "merl.eval-report/v1",
        "fixture_sha256": digest(&fixture_bytes),
        "merl_preparation_record_sha256": prepared.record_digests,
        "merl_database_sha256": prepared.database_digests,
        "freeze": plan.freeze.as_ref().map(|freeze| json!({
            "candidate_commit": freeze.candidate_commit,
            "candidate_binary_sha256": freeze.candidate_binary_sha256,
            "approved_manifest_sha256": freeze.approved_manifest_sha256,
            "fixture_sha256": freeze.fixture_sha256,
            "question_sha256": freeze.question_sha256,
            "config_sha256": freeze.config_sha256,
            "model_program_sha256": freeze.model_program_sha256,
            "scorer_program_sha256": freeze.scorer_program_sha256,
            "scoring_spec_sha256": freeze.scoring_spec_sha256
            ,"preparation_record_sha256": freeze.preparation_record_sha256
            ,"merl_database_sha256": freeze.merl_database_sha256
        })),
        "report": report
    }))
    .map_err(|error| error.to_string())
}

fn verify_trial_artifacts(plan: &Plan, fixture: &Fixture) -> Result<VerifiedTrials, String> {
    let mut cutoffs = Vec::new();
    for question in &plan.questions {
        if !cutoffs.contains(&(question.cutoff, question.capture_phase)) {
            cutoffs.push((question.cutoff, question.capture_phase));
        }
    }
    if cutoffs.is_empty() || plan.merl.trials.len() != plan.config.trials.len() * cutoffs.len() {
        return Err("one prepared Merl authority is required per trial and cutoff".to_owned());
    }
    let candidate_binary_sha256 = digest(
        &fs::read(env::current_exe().map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?,
    );
    let mut result = VerifiedTrials {
        artifacts: Vec::new(),
        record_digests: Vec::new(),
        database_digests: Vec::new(),
        databases: Vec::new(),
    };
    for ((identity, (cutoff, capture_phase)), trial) in plan
        .config
        .trials
        .iter()
        .flat_map(|identity| cutoffs.iter().map(move |cutoff| (identity, cutoff)))
        .zip(&plan.merl.trials)
    {
        if trial.trial_id != identity.id
            || trial.source_cutoff != *cutoff
            || trial.capture_phase != *capture_phase
        {
            return Err(
                "Merl preparation order or identity differs from the paired trial".to_owned(),
            );
        }
        require_closed_database(&trial.database)?;
        let bytes = fs::read(&trial.preparation_record)
            .map_err(|error| format!("could not read Merl preparation record: {error}"))?;
        let record: MerlPreparationRecord = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid Merl preparation record: {error}"))?;
        let usage = verify_preparation(
            &record,
            &trial.database,
            &plan.compiler_artifacts,
            fixture,
            &PreparationExpectation {
                trial_id: &identity.id,
                project: &plan.merl.project,
                source_cutoff: *cutoff,
                capture_phase: *capture_phase,
                candidate_commit: &plan.candidate_commit,
                candidate_binary_sha256: &candidate_binary_sha256,
            },
        )?;
        result.record_digests.push(digest(&bytes));
        require_closed_database(&trial.database)?;
        let database = fs::read(&trial.database)
            .map_err(|error| format!("could not read prepared Merl authority: {error}"))?;
        result.database_digests.push(digest(&database));
        result.databases.push(trial.database.clone());
        result.artifacts.push(MerlTrialArtifact {
            database: trial.database.clone(),
            preparation_usage: usage,
        });
    }
    if let Some(freeze) = &plan.freeze
        && (freeze.preparation_record_sha256 != result.record_digests
            || freeze.merl_database_sha256 != result.database_digests)
    {
        return Err("prepared Merl artifacts changed after the held-out freeze".to_owned());
    }
    Ok(result)
}

fn require_closed_database(path: &Path) -> Result<(), String> {
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    let wal = PathBuf::from(wal_name);
    if wal.metadata().is_ok_and(|metadata| metadata.len() > 0) {
        return Err("prepared Merl authority has an uncheckpointed WAL".to_owned());
    }
    Ok(())
}

fn verify_freeze(plan: &Plan, fixture_bytes: &[u8]) -> Result<(), String> {
    let freeze = plan
        .freeze
        .as_ref()
        .ok_or("held-out evaluation needs a freeze record")?;
    if freeze.candidate_commit != plan.candidate_commit
        || freeze.preparation_record_sha256.len() != plan.config.trials.len()
        || freeze.merl_database_sha256.len() != plan.config.trials.len()
        || freeze
            .preparation_record_sha256
            .iter()
            .chain(&freeze.merl_database_sha256)
            .any(|value| !valid_digest(value))
    {
        return Err("held-out freeze does not bind every prepared trial".to_owned());
    }
    if freeze.candidate_commit.len() != 40
        || !freeze
            .candidate_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !valid_digest(&freeze.approved_manifest_sha256)
        || !valid_digest(&freeze.candidate_binary_sha256)
        || !valid_digest(&freeze.fixture_sha256)
        || !valid_digest(&freeze.question_sha256)
        || !valid_digest(&freeze.config_sha256)
        || !valid_digest(&freeze.model_program_sha256)
        || !valid_digest(&freeze.scorer_program_sha256)
        || !valid_digest(&freeze.scoring_spec_sha256)
    {
        return Err("held-out freeze record has invalid identities".to_owned());
    }
    let manifest = fs::read(&freeze.approved_manifest)
        .map_err(|error| format!("could not read approved corpus manifest: {error}"))?;
    if digest(&manifest) != freeze.approved_manifest_sha256 {
        return Err("approved corpus manifest digest does not match the freeze record".to_owned());
    }
    let executable = env::current_exe()
        .map_err(|error| format!("could not locate evaluator binary: {error}"))?;
    let executable_bytes = fs::read(executable)
        .map_err(|error| format!("could not read evaluator binary: {error}"))?;
    if digest(&executable_bytes) != freeze.candidate_binary_sha256 {
        return Err("evaluator binary changed after freeze".to_owned());
    }
    if digest(fixture_bytes) != freeze.fixture_sha256 {
        return Err("fixture changed after freeze".to_owned());
    }
    let question = serde_json::to_vec(&plan.questions).map_err(|error| error.to_string())?;
    if digest(&question) != freeze.question_sha256 {
        return Err("question changed after freeze".to_owned());
    }
    let config = serde_json::to_vec(&plan.config).map_err(|error| error.to_string())?;
    if digest(&config) != freeze.config_sha256 {
        return Err("model configuration changed after freeze".to_owned());
    }
    let model = fs::read(&plan.model_program)
        .map_err(|error| format!("could not read model program: {error}"))?;
    if digest(&model) != freeze.model_program_sha256 {
        return Err("model program changed after freeze".to_owned());
    }
    let scorer = fs::read(&plan.scorer_program)
        .map_err(|error| format!("could not read scorer program: {error}"))?;
    if digest(&scorer) != freeze.scorer_program_sha256 {
        return Err("scorer program changed after freeze".to_owned());
    }
    let scoring_spec = fs::read(&freeze.scoring_spec)
        .map_err(|error| format!("could not read scoring specification: {error}"))?;
    if digest(&scoring_spec) != freeze.scoring_spec_sha256 {
        return Err("scoring specification changed after freeze".to_owned());
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut digest = String::from("sha256:");
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(digest, "{byte:02x}");
    }
    digest
}

#[cfg(test)]
mod tests {
    use super::{
        BenchmarkConfig, EvaluationQuestion, FreezeRecord, MerlPlan, MerlTrialPlan, PathBuf, Plan,
        ProcessLimits, digest, verify_freeze,
    };
    use merl_eval::{CompilerArtifacts, RandomnessControl, TrialIdentity};

    #[test]
    fn held_out_execution_requires_unchanged_frozen_artifacts() {
        let location =
            std::env::temp_dir().join(format!("merl-eval-freeze-{}", std::process::id()));
        std::fs::create_dir_all(&location).expect("temporary location");
        let manifest = location.join("approved.json");
        let scorer = location.join("scorer");
        let model = location.join("model");
        let scoring_spec = location.join("scoring.json");
        std::fs::write(&manifest, b"approved manifest").expect("manifest");
        std::fs::write(&scorer, b"scorer program").expect("scorer");
        std::fs::write(&model, b"model program").expect("model");
        std::fs::write(&scoring_spec, b"frozen scoring rubric").expect("scoring rubric");
        let fixture = b"frozen fixture";
        let question = EvaluationQuestion {
            id: "Q1".to_owned(),
            cutoff: 3,
            capture_phase: false,
            text: "What changed?".to_owned(),
        };
        let config = BenchmarkConfig {
            model: "model".to_owned(),
            model_version: "v1".to_owned(),
            effort: "medium".to_owned(),
            system_prompt: "Read the evidence.".to_owned(),
            task_prompt: "Answer the question.".to_owned(),
            trials: vec![
                TrialIdentity {
                    id: "pair-a".to_owned(),
                    randomness: RandomnessControl::Seed(1),
                },
                TrialIdentity {
                    id: "pair-b".to_owned(),
                    randomness: RandomnessControl::Seed(2),
                },
            ],
            recent_window: 2,
            max_tool_rounds: 2,
            search_results: 2,
            temperature: Some(0.0),
        };
        let mut plan = Plan {
            schema: "merl.eval-plan/v1".to_owned(),
            fixture: PathBuf::new(),
            questions: vec![question],
            config,
            model_program: model.clone(),
            scorer_program: scorer.clone(),
            candidate_commit: "a".repeat(40),
            compiler_artifacts: CompilerArtifacts {
                program: model.clone(),
                prompt: scoring_spec.clone(),
                rules: scoring_spec.clone(),
                config: scoring_spec.clone(),
            },
            merl: MerlPlan {
                project: "P1".to_owned(),
                issue: "I1".to_owned(),
                scope: "issue-1".to_owned(),
                role: "engineer".to_owned(),
                trials: vec![MerlTrialPlan {
                    trial_id: "pair-a".to_owned(),
                    source_cutoff: 3,
                    capture_phase: false,
                    database: PathBuf::new(),
                    preparation_record: PathBuf::new(),
                }],
            },
            limits: ProcessLimits {
                max_request_bytes: 4096,
                max_response_bytes: 4096,
            },
            freeze: None,
        };
        assert!(verify_freeze(&plan, fixture).is_err());
        plan.freeze = Some(FreezeRecord {
            candidate_commit: "a".repeat(40),
            candidate_binary_sha256: digest(
                &std::fs::read(std::env::current_exe().expect("test binary path"))
                    .expect("test binary"),
            ),
            approved_manifest: manifest.clone(),
            approved_manifest_sha256: digest(b"approved manifest"),
            fixture_sha256: digest(fixture),
            question_sha256: digest(&serde_json::to_vec(&plan.questions).expect("question JSON")),
            config_sha256: digest(&serde_json::to_vec(&plan.config).expect("config JSON")),
            model_program_sha256: digest(b"model program"),
            scorer_program_sha256: digest(b"scorer program"),
            scoring_spec: scoring_spec.clone(),
            scoring_spec_sha256: digest(b"frozen scoring rubric"),
            preparation_record_sha256: vec![digest(b"prep-a"), digest(b"prep-b")],
            merl_database_sha256: vec![digest(b"db-a"), digest(b"db-b")],
        });
        assert!(verify_freeze(&plan, fixture).is_ok());
        std::fs::write(&scoring_spec, b"changed scoring rubric").expect("change rubric");
        assert!(verify_freeze(&plan, fixture).is_err());
        std::fs::remove_file(&manifest).expect("remove manifest");
        std::fs::remove_file(&scorer).expect("remove scorer");
        std::fs::remove_file(&model).expect("remove model");
        std::fs::remove_file(&scoring_spec).expect("remove rubric");
        std::fs::remove_dir(&location).expect("remove temporary location");
    }
}
