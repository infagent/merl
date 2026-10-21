use std::{path::PathBuf, process::Command};

use merl_eval::package::{PackagePin, VerifiedPackage};
use sha2::{Digest, Sha256};

pub struct PackageCase {
    directory: PathBuf,
    package: Option<VerifiedPackage>,
    report: Option<merl_eval::visible::PackageReport>,
    rejected_grades: Vec<bool>,
}

impl PackageCase {
    pub fn given_a_synthetic_approved_package() -> Self {
        let directory = (0..1_000)
            .find_map(|index| {
                let parent = std::env::temp_dir()
                    .join(format!("merl-visible-{}-{index}", std::process::id()));
                match std::fs::create_dir(&parent) {
                    Ok(()) => Some(parent),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("create synthetic package parent: {error}"),
                }
            })
            .expect("reserve package parent");
        let generator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/visible_package/generate.py");
        let output = Command::new("/usr/bin/python3")
            .arg(generator)
            .arg(directory.join("package"))
            .output()
            .expect("generate synthetic package");
        assert!(output.status.success(), "{output:?}");
        Self {
            directory,
            package: None,
            report: None,
            rejected_grades: Vec::new(),
        }
    }

    pub fn when_the_package_is_verified(mut self) -> Self {
        self.package = Some(self.verify().expect("approved package integration"));
        self
    }

    fn verify(&self) -> Result<VerifiedPackage, merl_eval::package::PackageError> {
        let root = self.directory.join("package");
        let hash = |path: &str| {
            Sha256::digest(std::fs::read(root.join(path)).unwrap())
                .iter()
                .fold(String::new(), |mut text, byte| {
                    use std::fmt::Write as _;
                    write!(text, "{byte:02x}").unwrap();
                    text
                })
        };
        let pin = PackagePin::new(
            &hash("package.sha256"),
            &hash("evaluator-only/approved-artifacts/question-inventory.json"),
            &hash("evaluator-only/approved-artifacts/scoring-rules.json"),
            3,
        )
        .unwrap();
        VerifiedPackage::open(&root, &pin)
    }

    pub fn then_three_units_share_two_prefixes_and_keep_their_prompts(self) -> Self {
        let package = self.package.as_ref().expect("verified package");
        assert_eq!(
            package
                .units()
                .iter()
                .map(merl_eval::package::VerifiedUnit::id)
                .collect::<Vec<_>>(),
            ["SYN-Q1@1", "SYN-Q2@1", "SYN-Q1@2"]
        );
        assert_eq!(package.prefix_count(), 2);
        assert_eq!(
            package.units()[0].question(),
            "What heading color is instructed at this cutoff?"
        );
        assert_eq!(
            package.units()[1].question(),
            "What icon shape must be retained at this cutoff?"
        );
        assert_eq!(package.units()[2].question(), package.units()[0].question());
        self
    }

    pub fn then_reader_stages_exclude_oracles_and_audit_records(self) {
        let package = self.package.as_ref().expect("verified package");
        for unit in package.units() {
            let files = unit.reader_files();
            assert_eq!(
                files.keys().map(String::as_str).collect::<Vec<_>>(),
                [
                    "instructions.txt",
                    "method.json",
                    "prompt.json",
                    "source.json"
                ]
            );
            let source: serde_json::Value = serde_json::from_slice(&files["source.json"]).unwrap();
            assert_eq!(
                source["observations"].as_array().unwrap().len() as u64,
                unit.cutoff()
            );
            assert!(!format!("{package:?}").contains("required_meanings"));
        }
    }
}

impl PackageCase {
    pub fn when_isolated_semantic_workers_read_and_grade(mut self) -> Self {
        use merl_eval::grading::{CategoricalProcessGrader, Grader, Verdict};
        use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
        use merl_eval::{AnswerRequest, BenchmarkMethod, ModelAdapter, ProcessModelAdapter};
        use std::os::unix::fs::PermissionsExt;
        let package = self.verify().unwrap();
        let model_file = self.directory.join("model.py");
        let grader_file = self.directory.join("grader.py");
        std::fs::write(&model_file, r"#!/usr/bin/python3
import json, os, sys
r = json.load(sys.stdin)
assert os.listdir('/input') == [], 'source pool or oracle mounted in model'
assert not os.path.exists('/tmp/grader-state'), 'grader output reached reader state'
assert r['question'] == 'What heading color is instructed at this cutoff?'
assert 'private-oracle' not in r['context']
json.dump({'schema':'merl.eval-adapter/v1', 'kind':'final', 'answer':'Violet headings; the square icon remains.', 'citations':['O2'], 'usage':{'input':10,'output':5}}, sys.stdout)
").unwrap();
        std::fs::write(&grader_file, r"#!/usr/bin/python3
import json, os, sys
r = json.load(sys.stdin)
assert sorted(os.listdir('/input')) == ['grading-instructions.txt','method.json','oracle.json','prompt.json','response.json','source.json']
oracle = json.load(open('/input/oracle.json'))
assert oracle['unit_id'] == r['unit_id']
assert len(json.load(open('/input/source.json'))['observations']) == int(r['unit_id'].split('@')[1])
assert json.load(open('/input/response.json')) == r
assert not os.path.exists('/input/unit-index.json')
open('/tmp/grader-state','w').write('private-oracle')
json.dump({'unit_id':r['unit_id'], 'verdict':'incomplete', 'reason':'Synthetic meaning-based review; no object equality.'},sys.stdout)
").unwrap();
        for file in [&model_file, &grader_file] {
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut model = ProcessModelAdapter::isolated(
            IsolatedProgram::new(&model_file, ProcessLimits::default()).unwrap(),
        );
        let mut grader = CategoricalProcessGrader::new(
            &package,
            IsolatedProgram::new(&grader_file, ProcessLimits::default()).unwrap(),
        );
        let debug = format!("{model:?} {grader:?}");
        assert!(
            !debug.contains("private-oracle"),
            "executable contents leaked through Debug"
        );
        // Replace both the executable path and a package file after verification.
        // Calls must use their frozen bytes, not mutable paths.
        std::fs::write(&grader_file, "corrupted executable after pinning").unwrap();
        std::fs::write(
            self.directory
                .join("package/evaluator-only/oracle-units/SYN-Q1__cutoff-1.json"),
            "corrupted oracle after verification",
        )
        .unwrap();
        // Intentionally grade the future unit first. No worker state may flow back.
        let config = semantic_config();
        for index in [2, 0] {
            let unit = &package.units()[index];
            let result = model
                .answer(AnswerRequest {
                    config: &config,
                    trial: &config.trials[0],
                    method: BenchmarkMethod::RollingSummary,
                    question_id: unit.id(),
                    question: unit.question(),
                    context: "Only the method-disclosed synthetic summary.",
                    can_search: false,
                    can_expand: false,
                })
                .expect("isolated semantic answer");
            let merl_eval::AnswerAction::Final { answer, citations } = result.action else {
                panic!("final answer")
            };
            let grade = grader
                .grade(unit.id(), &answer, &citations)
                .expect("isolated unit grade");
            assert_eq!(grade.verdict, Verdict::Incomplete);
            assert_eq!(grade.unit_id, unit.id());
        }
        self.package = Some(package);
        self
    }
}

impl PackageCase {
    pub fn when_incompatible_grades_are_submitted(mut self) -> Self {
        use merl_eval::grading::{CategoricalProcessGrader, Grader};
        use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
        let package = self.verify().unwrap();
        for response in [
            serde_json::json!({"schema":"merl.eval-score/v1","score":{"correctness":1.0,"provenance":1.0,"stale_state_error":false,"missed_blocker":false}}),
            serde_json::json!({"unit_id":"SYN-Q1@2","verdict":"correct","reason":"Wrong cutoff."}),
            serde_json::json!({"unit_id":"SYN-Q1@1","verdict":"correct","reason":" "}),
        ] {
            let program = self.directory.join("incompatible.py");
            let body = format!(
                "#!/usr/bin/python3\nimport sys\nsys.stdout.write({:?})\n",
                response.to_string()
            );
            std::fs::write(&program, body).unwrap();
            let mut grader = CategoricalProcessGrader::new(
                &package,
                IsolatedProgram::new(&program, ProcessLimits::default()).unwrap(),
            );
            self.rejected_grades
                .push(grader.grade("SYN-Q1@1", "An amber heading.", &[]).is_err());
        }
        self
    }
    pub fn then_no_incompatible_grade_receives_credit(self) {
        assert_eq!(self.rejected_grades, [true, true, true]);
    }
}

pub fn semantic_config() -> merl_eval::BenchmarkConfig {
    merl_eval::BenchmarkConfig {
        model: "synthetic".into(),
        model_version: "v1".into(),
        effort: "none".into(),
        system_prompt: "Use supplied evidence.".into(),
        task_prompt: "Answer the question.".into(),
        trials: vec![merl_eval::TrialIdentity {
            id: "pair-a".into(),
            randomness: merl_eval::RandomnessControl::Seed(7),
        }],
        recent_window: 1,
        max_tool_rounds: 1,
        search_results: 1,
        temperature: Some(0.0),
    }
}

impl Drop for PackageCase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).expect("remove synthetic package");
    }
}

pub struct IntegrityCases {
    cases: Vec<(&'static str, PackageCase)>,
    results: Vec<(&'static str, bool)>,
}
impl IntegrityCases {
    pub fn given_independently_tampered_synthetic_packages() -> Self {
        let mut cases = vec![];
        for mutation in [
            "symlink",
            "unlisted",
            "missing_file",
            "wrong_hash",
            "prompt",
            "oracle",
            "future",
            "overlay",
            "destination",
            "traversal",
            "missing",
        ] {
            let case = PackageCase::given_a_synthetic_approved_package();
            let root = case.directory.join("package");
            tamper_filesystem(mutation, &case.directory, &root);
            if ["symlink", "unlisted", "missing_file", "wrong_hash"].contains(&mutation) {
                cases.push((mutation, case));
                continue;
            }
            let (file, edit): (&str, fn(&mut serde_json::Value)) = match mutation {
                "prompt" => ("reader-inputs/units/SYN-Q1__cutoff-1.json", |value| {
                    value["question"] =
                        serde_json::json!("Use the broad descriptive title instead.");
                }),
                "oracle" => (
                    "evaluator-only/oracle-units/SYN-Q1__cutoff-1.json",
                    |value| {
                        value["approved_answer"]["required_meanings"] =
                            serde_json::json!(["An altered approved meaning."]);
                    },
                ),
                "future" => (
                    "reader-inputs/source-prefixes/SYN-C1__through-1.json",
                    |value| {
                        let mut later = value["observations"][0].clone();
                        later["sequence"] = serde_json::json!(2);
                        value["observations"].as_array_mut().unwrap().push(later);
                    },
                ),
                "overlay" => ("approval/approved-scope.json", |value| {
                    value["historical_metadata_overlay"]["effective_approved_inventory_denominator"] =
                        serde_json::json!(0);
                }),
                "destination" => ("evaluator-only/unit-index.json", |value| {
                    value["units"][0]["reader_allowlist"][1]["reader_destination"] =
                        serde_json::json!("prompt.json");
                }),
                "traversal" => ("evaluator-only/unit-index.json", |value| {
                    value["units"][0]["reader_allowlist"][0]["package_path"] =
                        serde_json::json!("../private-answer.json");
                }),
                "missing" => ("evaluator-only/unit-index.json", |value| {
                    value["units"].as_array_mut().unwrap().pop();
                }),
                _ => unreachable!(),
            };
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join(file)).unwrap()).unwrap();
            edit(&mut value);
            std::fs::write(root.join(file), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
            let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/visible_package/rebind.py");
            let output = Command::new("/usr/bin/python3")
                .arg(script)
                .arg(&root)
                .output()
                .unwrap();
            assert!(output.status.success(), "{mutation}: {output:?}");
            cases.push((mutation, case));
        }
        Self {
            cases,
            results: vec![],
        }
    }
    pub fn when_the_packages_are_verified(mut self) -> Self {
        for (mutation, case) in &self.cases {
            self.results.push((mutation, case.verify().is_err()));
        }
        self
    }
    pub fn then_every_tampered_contract_is_rejected(self) {
        for (mutation, rejected) in self.results {
            assert!(rejected, "accepted tampered {mutation}");
        }
    }
}

impl PackageCase {
    pub fn when_one_prefix_has_a_grading_failure(mut self) -> Self {
        use merl_eval::visible::{PackageReport, run_prefix};
        let package = self.verify().unwrap();
        let fixture = Self::fixture(&package);
        let config = semantic_config();
        let prefix = run_prefix(
            &package,
            &serde_json::to_vec(&fixture).unwrap(),
            1,
            config.clone(),
            &mut ToyModel,
            &mut ToyMerl,
            &mut FaultGrader { calls: 0 },
        )
        .expect("execute a verified prefix");
        self.report = Some(
            PackageReport::assemble(&package, config, vec![prefix])
                .expect("explicit package accounting"),
        );
        self
    }
    fn fixture(package: &VerifiedPackage) -> merl_corpus::fixture::Fixture {
        let source = &package.units()[2].source();
        let mut fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../corpus/development/DEV-C1.json")).unwrap();
        fixture["id"] = serde_json::json!("SYN-C1");
        fixture["source"] = source["source_identity"].clone();
        fixture["observations"] = source["observations"].clone();
        fixture["gold_states"] = serde_json::json!([]);
        fixture["capture"]["history_fidelity"] = source["history_fidelity"].clone();
        fixture["capture"]["captured_at"] = serde_json::json!("2030-01-02T00:00:00Z");
        let mut fixture: merl_corpus::fixture::Fixture = serde_json::from_value(fixture).unwrap();
        fixture.capture.provider_observation_count = fixture.observations.len();
        fixture.capture.source_sha256 =
            merl_corpus::fixture::source_digest(&fixture.provider_snapshot, &fixture.observations);
        fixture
    }

    pub fn when_a_shared_summary_fails(mut self) -> Self {
        let package = self.verify().unwrap();
        let fixture = Self::fixture(&package);
        let config = semantic_config();
        let prefix = merl_eval::visible::run_prefix(
            &package,
            &serde_json::to_vec(&fixture).unwrap(),
            1,
            config.clone(),
            &mut EmptySummary,
            &mut ToyMerl,
            &mut FaultGrader { calls: 1 },
        )
        .unwrap();
        self.report = Some(
            merl_eval::visible::PackageReport::assemble(&package, config, vec![prefix]).unwrap(),
        );
        self
    }
    pub fn then_only_dependent_units_fail_and_known_preparation_cost_survives(self) {
        let report = self.report.as_ref().unwrap();
        let prefix = &report.prefixes[0];
        assert_eq!(prefix.trials.len(), 6);
        assert_eq!(prefix.failures.len(), 4);
        assert_eq!(prefix.summary_calls.len(), 1);
        for record in &prefix.preparation[..2] {
            assert_eq!(record.known_usage.total(), 7);
            assert!(!record.complete);
        }
        for failure in &prefix.failures {
            assert!(matches!(
                failure.method,
                merl_eval::BenchmarkMethod::RollingSummary
                    | merl_eval::BenchmarkMethod::SummaryRetrieval
            ));
            assert!(failure.answer_calls.is_empty());
            assert_eq!(failure.code, "model_failure");
            assert!(failure.usage_incomplete);
        }
        for index in [1, 3] {
            assert_eq!(report.counts[index].correct, 0);
            assert_eq!(report.counts[index].failed, 2);
            assert_eq!(report.counts[index].missing, 1);
        }
    }

    pub fn then_all_scoring_slots_and_shared_costs_remain_accounted_for(self) {
        let report = self.report.as_ref().unwrap();
        assert_eq!(report.unit_ids.len(), 3);
        assert_eq!(report.missing.len(), 5);
        let prefix = &report.prefixes[0];
        assert_eq!(prefix.trials.len(), 9);
        assert_eq!(prefix.failures.len(), 1);
        assert_eq!(prefix.failures[0].unit_id, "SYN-Q1@1");
        assert_eq!(prefix.failures[0].code, "grading_failure");
        assert_eq!(
            prefix.failures[0].answer.as_deref(),
            Some("An amber heading and a square icon.")
        );
        assert_eq!(prefix.failures[0].answer_calls[0].usage.total(), 15);
        assert!(!prefix.failures[0].usage_incomplete);
        assert_eq!(prefix.summary_calls.len(), 1);
        assert_eq!(prefix.preparation.len(), 3);
        assert_eq!(prefix.preparation[0].known_usage.total(), 7);
        assert_eq!(prefix.preparation[1].known_usage.total(), 7);
        assert_eq!(prefix.preparation[2].known_usage.total(), 20);
        assert_eq!(report.counts[0].expected, 3);
        assert_eq!(report.counts[0].correct, 1);
        assert_eq!(report.counts[0].failed, 1);
        assert_eq!(report.counts[0].missing, 1);
        for counts in &report.counts {
            assert_eq!(
                counts.expected,
                counts.correct
                    + counts.incomplete
                    + counts.incorrect
                    + counts.not_scored
                    + counts.failed
                    + counts.missing
            );
        }
    }
}

struct ToyModel;
impl merl_eval::ModelAdapter for ToyModel {
    fn summarize(
        &mut self,
        request: merl_eval::SummaryRequest<'_>,
    ) -> Result<merl_eval::SummaryResponse, String> {
        assert!(request.source.body.unwrap().contains("amber"));
        assert!(!request.source.body.unwrap().contains("violet"));
        Ok(merl_eval::SummaryResponse {
            text: "Amber heading and square icon.".into(),
            usage: merl_eval::TokenUsage {
                input: 5,
                output: 2,
            },
        })
    }
    fn answer(
        &mut self,
        request: merl_eval::AnswerRequest<'_>,
    ) -> Result<merl_eval::AnswerResponse, String> {
        assert!(!request.context.contains("violet"));
        assert!(request.context.contains("[reader instructions]"));
        assert!(request.context.contains("[approved reader method]"));
        Ok(merl_eval::AnswerResponse {
            action: merl_eval::AnswerAction::Final {
                answer: "An amber heading and a square icon.".into(),
                citations: vec!["O1".into()],
            },
            usage: merl_eval::TokenUsage {
                input: 10,
                output: 5,
            },
        })
    }
}
struct ToyMerl;
impl merl_eval::MerlSurface for ToyMerl {
    fn prepare(
        &mut self,
        question: &merl_eval::EvaluationQuestion,
    ) -> Result<merl_eval::MerlPrepared, String> {
        Ok(merl_eval::MerlPrepared {
            source_cutoff: question.cutoff,
            view: "Synthetic accepted view.".into(),
            preparation_usage: merl_eval::TokenUsage {
                input: 15,
                output: 5,
            },
            required_coverage_complete: true,
            causal: true,
        })
    }
    fn expand(&mut self, _: &str) -> Result<String, String> {
        Err("not requested".into())
    }
}
struct FaultGrader {
    calls: usize,
}
impl merl_eval::grading::Grader for FaultGrader {
    type Grade = merl_eval::grading::CategoricalGrade;
    fn grade(&mut self, id: &str, _: &str, _: &[String]) -> Result<Self::Grade, String> {
        self.calls += 1;
        if self.calls == 1 {
            return Err("synthetic grading failure".into());
        }
        Ok(Self::Grade {
            unit_id: id.into(),
            verdict: merl_eval::grading::Verdict::Correct,
            reason: "Synthetic semantic agreement.".into(),
        })
    }
}

struct EmptySummary;
impl merl_eval::ModelAdapter for EmptySummary {
    fn summarize(
        &mut self,
        _: merl_eval::SummaryRequest<'_>,
    ) -> Result<merl_eval::SummaryResponse, String> {
        Ok(merl_eval::SummaryResponse {
            text: String::new(),
            usage: merl_eval::TokenUsage {
                input: 5,
                output: 2,
            },
        })
    }
    fn answer(
        &mut self,
        request: merl_eval::AnswerRequest<'_>,
    ) -> Result<merl_eval::AnswerResponse, String> {
        merl_eval::ModelAdapter::answer(&mut ToyModel, request)
    }
}

fn tamper_filesystem(mutation: &str, directory: &std::path::Path, root: &std::path::Path) {
    let prompt_path = root.join("reader-inputs/units/SYN-Q1__cutoff-1.json");
    match mutation {
        "symlink" => {
            #[cfg(unix)]
            {
                let outside = directory.join("outside.json");
                std::fs::copy(&prompt_path, &outside).unwrap();
                std::fs::remove_file(&prompt_path).unwrap();
                std::os::unix::fs::symlink(outside, &prompt_path).unwrap();
            }
            #[cfg(not(unix))]
            {
                std::fs::write(&prompt_path, "corrupted").unwrap();
            }
        }
        "unlisted" => {
            std::fs::write(root.join("unexpected.txt"), "synthetic extra file").unwrap();
        }
        "missing_file" => {
            std::fs::remove_file(&prompt_path).unwrap();
        }
        "wrong_hash" => {
            std::fs::write(&prompt_path, "corrupted prompt bytes").unwrap();
        }
        _ => {}
    }
}
