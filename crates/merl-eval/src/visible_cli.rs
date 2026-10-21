//! Package integration commands; approved artifact schemas remain unchanged.

use merl_corpus::fixture::{Fixture, Partition};
use merl_eval::grading::CategoricalProcessGrader;
use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
use merl_eval::package::{PackagePin, VerifiedPackage};
use merl_eval::transport::{RemoteTransport, TransportPin, TransportRole};
use merl_eval::visible::{PackageReport, run_prefix};
use merl_eval::{BenchmarkConfig, CliMerlSurface, EvaluationQuestion, ProcessModelAdapter};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackagePlan {
    root: PathBuf,
    manifest_sha256: String,
    inventory_sha256: String,
    rules_sha256: String,
    expected_units: usize,
}
impl PackagePlan {
    fn open(&self) -> Result<VerifiedPackage, String> {
        let pin = PackagePin::new(
            &self.manifest_sha256,
            &self.inventory_sha256,
            &self.rules_sha256,
            self.expected_units,
        )
        .map_err(|e| e.to_string())?;
        VerifiedPackage::open(&self.root, &pin).map_err(|e| e.to_string())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisiblePlan {
    schema: String,
    package: PackagePlan,
    config: BenchmarkConfig,
    model_program: PathBuf,
    model_program_sha256: String,
    grader_program: PathBuf,
    grader_program_sha256: String,
    model_transport: Option<TransportPin>,
    grader_transport: Option<TransportPin>,
    limits: ProcessLimits,
    prefixes: Vec<PrefixPlan>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrefixPlan {
    case: String,
    cutoff: u64,
    fixture: PathBuf,
    fixture_sha256: String,
    candidate_preparer_program: PathBuf,
    candidate_commit: String,
    compiler_artifacts: merl_eval::CompilerArtifacts,
    merl: super::MerlPlan,
}

pub(super) fn command(args: &[String]) -> Result<(String, bool), String> {
    if args == ["--help"] {
        return Ok(("merl-eval package verify --package ROOT --manifest-sha256 HEX --inventory-sha256 HEX --rules-sha256 HEX --expected-units N\nmerl-eval package <inspect|run> --plan PATH --plan-sha256 HEX".into(), true));
    }
    match args.first().map(String::as_str) {
        Some("verify") => verify(&args[1..]),
        Some(action @ ("inspect" | "run")) if args.len() == 5 && args[1] == "--plan" && args[3] == "--plan-sha256" => {
            execute(Path::new(&args[2]), &args[4], action == "run")
        }
        _ => Err("usage: merl-eval package verify --package ROOT --manifest-sha256 HEX --inventory-sha256 HEX --rules-sha256 HEX --expected-units N; or package <inspect|run> --plan PATH --plan-sha256 HEX".into()),
    }
}

fn verify(args: &[String]) -> Result<(String, bool), String> {
    let names = [
        "--package",
        "--manifest-sha256",
        "--inventory-sha256",
        "--rules-sha256",
        "--expected-units",
    ];
    if args.len() != names.len() * 2
        || args
            .as_chunks::<2>()
            .0
            .iter()
            .zip(names)
            .any(|(pair, name)| pair[0] != name)
    {
        return Err("package verify requires package, manifest, inventory, rules and expected-units pins in that order".into());
    }
    let plan = PackagePlan {
        root: PathBuf::from(&args[1]),
        manifest_sha256: args[3].clone(),
        inventory_sha256: args[5].clone(),
        rules_sha256: args[7].clone(),
        expected_units: args[9].parse().map_err(|_| "invalid expected unit count")?,
    };
    let package = plan.open()?;
    output(
        &json!({ "schema": "merl.eval-visible-verification/v1", "verified": true,
        "package": package.identity(), "unit_ids": package.units().iter().map(merl_eval::package::VerifiedUnit::id).collect::<Vec<_>>(),
        "source_prefix_count": package.prefix_count(),
        "question_count":package.units().iter().map(merl_eval::package::VerifiedUnit::question_id).collect::<BTreeSet<_>>().len(),
        "case_count":package.units().iter().map(merl_eval::package::VerifiedUnit::case).collect::<BTreeSet<_>>().len(), "exclusion_ids": package.exclusion_ids(), "scored_units": 0 }),
        true,
    )
}

struct PreparedPrefix {
    fixture: Fixture,
    fixture_bytes: Vec<u8>,
    cutoff: u64,
    fixture_sha256: String,
    plan: super::Plan,
    trials: super::VerifiedTrials,
}

#[expect(
    clippy::too_many_lines,
    reason = "pin all inputs and verify all authorities before allowing a model or grader call"
)]
fn execute(path: &Path, pin: &str, run: bool) -> Result<(String, bool), String> {
    let bytes = read(path)?;
    check_digest(&bytes, pin)?;
    let plan: VisiblePlan =
        serde_json::from_slice(&bytes).map_err(|_| "invalid visible integration plan")?;
    if plan.schema != "merl.eval-visible-plan/v1" {
        return Err("unsupported visible integration plan".into());
    }
    let package = plan.package.open()?;
    let model_program =
        IsolatedProgram::new(&plan.model_program, plan.limits).map_err(|e| e.to_string())?;
    let grader_program =
        IsolatedProgram::new(&plan.grader_program, plan.limits).map_err(|e| e.to_string())?;
    if !same_digest(model_program.sha256(), &plan.model_program_sha256)
        || !same_digest(grader_program.sha256(), &plan.grader_program_sha256)
    {
        return Err("worker executable differs from integration plan".into());
    }
    let mut seen = BTreeSet::new();
    let mut prepared = Vec::new();
    for prefix in &plan.prefixes {
        if !seen.insert((prefix.case.as_str(), prefix.cutoff)) {
            return Err("duplicate planned prefix".into());
        }
        let questions = package
            .units()
            .iter()
            .filter(|u| u.case() == prefix.case && u.cutoff() == prefix.cutoff)
            .map(|u| EvaluationQuestion {
                id: u.id().to_owned(),
                cutoff: u.cutoff(),
                capture_phase: false,
                text: u.question().to_owned(),
            })
            .collect::<Vec<_>>();
        if questions.is_empty() {
            return Err("unapproved planned prefix".into());
        }
        let fixture_bytes = read(&prefix.fixture)?;
        check_digest(&fixture_bytes, &prefix.fixture_sha256)?;
        let fixture: Fixture =
            serde_json::from_slice(&fixture_bytes).map_err(|_| "invalid planned fixture")?;
        if fixture.id != prefix.case || matches!(fixture.partition, Partition::HeldOut) {
            return Err(
                "visible integration does not accept another case or a held-out fixture".into(),
            );
        }
        // Validate original JSON before serde can erase unknown fields/default spelling.
        merl_eval::visible::verify_prefix(&package, &fixture_bytes, prefix.cutoff)
            .map_err(|e| e.to_string())?;
        let legacy = super::Plan {
            schema: super::PLAN_SCHEMA.into(),
            fixture: prefix.fixture.clone(),
            questions,
            config: plan.config.clone(),
            model_program: plan.model_program.clone(),
            scorer_program: plan.grader_program.clone(),
            candidate_preparer_program: prefix.candidate_preparer_program.clone(),
            candidate_commit: prefix.candidate_commit.clone(),
            compiler_artifacts: merl_eval::CompilerArtifacts {
                program: prefix.compiler_artifacts.program.clone(),
                prompt: prefix.compiler_artifacts.prompt.clone(),
                rules: prefix.compiler_artifacts.rules.clone(),
                config: prefix.compiler_artifacts.config.clone(),
            },
            merl: super::MerlPlan {
                project: prefix.merl.project.clone(),
                issue: prefix.merl.issue.clone(),
                scope: prefix.merl.scope.clone(),
                role: prefix.merl.role.clone(),
                trials: prefix
                    .merl
                    .trials
                    .iter()
                    .map(|t| super::MerlTrialPlan {
                        trial_id: t.trial_id.clone(),
                        source_cutoff: t.source_cutoff,
                        capture_phase: t.capture_phase,
                        database: t.database.clone(),
                        preparation_record: t.preparation_record.clone(),
                    })
                    .collect(),
            },
            limits: super::ProcessLimits {
                max_request_bytes: plan.limits.request_bytes,
                max_response_bytes: plan.limits.response_bytes,
            },
            freeze: None,
        };
        let trials = super::verify_trial_artifacts(&legacy, &fixture)
            .map_err(|_| "preparation verification failed")?;
        prepared.push(PreparedPrefix {
            fixture,
            fixture_bytes,
            cutoff: prefix.cutoff,
            fixture_sha256: prefix.fixture_sha256.clone(),
            plan: legacy,
            trials,
        });
    }
    let manifest = prepared.iter().map(|p|json!({"case":p.fixture.id,"cutoff":p.cutoff,
        "fixture_sha256":p.fixture_sha256,
        "preparation_record_sha256":p.trials.record_digests,"database_sha256":p.trials.database_digests})).collect::<Vec<_>>();
    let mut reports = Vec::new();
    let mut prefix_failures = Vec::new();
    let mut model = if let Some(pin) = &plan.model_transport {
        let transport =
            RemoteTransport::open(pin, TransportRole::Reader).map_err(|e| e.to_string())?;
        transport
            .check_reader(&plan.config)
            .map_err(|e| e.to_string())?;
        ProcessModelAdapter::with_transport(model_program, transport).map_err(|e| e.to_string())?
    } else {
        ProcessModelAdapter::isolated(model_program)
    };
    let mut grader = if let Some(pin) = &plan.grader_transport {
        let transport =
            RemoteTransport::open(pin, TransportRole::Grader).map_err(|e| e.to_string())?;
        CategoricalProcessGrader::with_transport(&package, grader_program, transport)
            .map_err(|e| e.to_string())?
    } else {
        CategoricalProcessGrader::new(&package, grader_program)
    };
    if run {
        for prefix in prepared {
            let mut merl = CliMerlSurface::new(
                prefix.plan.merl.project,
                prefix.plan.merl.issue,
                prefix.plan.merl.scope,
                prefix.plan.merl.role,
                prefix.trials.artifacts,
            );
            match run_prefix(&package, &prefix.fixture_bytes, prefix.cutoff, plan.config.clone(), &mut model, &mut merl, &mut grader) {
                Ok(mut report) => {
                    let unchanged = prefix.trials.databases.iter().zip(&prefix.trials.database_digests).all(|(path,hash)|
                        super::require_closed_database(path).is_ok() && fs::read(path).is_ok_and(|bytes| super::digest(&bytes) == *hash));
                    if !unchanged { report.invalidate_authority(); }
                    reports.push(report);
                }
                Err(_) => prefix_failures.push(json!({"case":prefix.fixture.id,"cutoff":prefix.cutoff,"code":"prefix_execution_failed"})),
            }
        }
    }
    let report =
        PackageReport::assemble(&package, plan.config, reports).map_err(|e| e.to_string())?;
    let complete = run && report.missing.is_empty() && report.counts.iter().all(|c| c.failed == 0);
    output(
        &json!({"schema":"merl.eval-visible-execution/v1","executed":run,"complete":complete,
        "plan_sha256":super::digest(&bytes),"evaluator_binary_sha256":binary_digest()?,
        "model_program_sha256":plan.model_program_sha256,"grader_program_sha256":plan.grader_program_sha256,
        "prepared_prefixes":manifest,"prefix_failures":prefix_failures,"report":report,
        "model_calls":model.calls(),"grader_calls":grader.calls(),
        "model_transport":model.transport_report(),"grader_transport":grader.transport_report()}),
        !run || complete,
    )
}

fn binary_digest() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    use std::io::Read as _;
    let path = std::env::current_exe().map_err(|_| "evaluator path")?;
    let mut file = fs::File::open(path).map_err(|_| "evaluator binary unreadable")?;
    // Debug builds can exceed the package input ceiling. Stream trusted binary
    // bytes so adding transport code does not weaken bounds on external inputs.
    let mut hash = Sha256::new();
    let mut buffer = [0; 16_384];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "evaluator binary unreadable")?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    let mut text = String::from("sha256:");
    for byte in hash.finalize() {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

fn output(value: &Value, success: bool) -> Result<(String, bool), String> {
    serde_json::to_string_pretty(value)
        .map(|s| (s, success))
        .map_err(|_| "report encoding failed".into())
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    const MAX_BYTES: u64 = 67_108_864;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "integration input unreadable")?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "integration input unreadable")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("integration input exceeds byte limit".into());
    }
    Ok(bytes)
}
fn check_digest(bytes: &[u8], pin: &str) -> Result<(), String> {
    if same_digest(&super::digest(bytes), pin) {
        Ok(())
    } else {
        Err("integration input digest mismatch".into())
    }
}
fn same_digest(left: &str, right: &str) -> bool {
    left.strip_prefix("sha256:").unwrap_or(left) == right.strip_prefix("sha256:").unwrap_or(right)
}
