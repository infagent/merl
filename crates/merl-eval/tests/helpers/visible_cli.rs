use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

pub struct Scenario {
    directory: PathBuf,
    output: Option<Output>,
    provider: Option<crate::mock_provider::MockProvider>,
}
impl Scenario {
    pub fn given_a_synthetic_package() -> Self {
        let directory = (0..1_000)
            .find_map(|i| {
                let p = std::env::temp_dir()
                    .join(format!("merl-visible-cli-{}-{i}", std::process::id()));
                std::fs::create_dir(&p).ok().map(|()| p)
            })
            .unwrap();
        let root = directory.join("package");
        let generator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/visible_package/generate.py");
        let result = Command::new("/usr/bin/python3")
            .arg(generator)
            .arg(&root)
            .output()
            .unwrap();
        assert!(result.status.success());
        Self {
            directory,
            output: None,
            provider: None,
        }
    }
    pub fn when_the_package_is_verified(mut self) -> Self {
        let root = self.directory.join("package");
        self.output = Some(
            Command::new(env!("CARGO_BIN_EXE_merl-eval"))
                .args(["package", "verify", "--package"])
                .arg(&root)
                .args([
                    "--manifest-sha256",
                    &hash(&root.join("package.sha256")),
                    "--inventory-sha256",
                    &hash(&root.join("evaluator-only/approved-artifacts/question-inventory.json")),
                    "--rules-sha256",
                    &hash(&root.join("evaluator-only/approved-artifacts/scoring-rules.json")),
                    "--expected-units",
                    "3",
                ])
                .output()
                .unwrap(),
        );
        self
    }
    pub fn then_all_units_are_verified_without_scoring(self) {
        let output = self.output.as_ref().unwrap();
        assert!(output.status.success(), "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            report["unit_ids"],
            json!(["SYN-Q1@1", "SYN-Q2@1", "SYN-Q1@2"])
        );
        assert_eq!(report["source_prefix_count"], 2);
        assert_eq!(report["scored_units"], 0);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("required_meanings"));
    }
    pub fn when_synthetic_readers_execute_a_prepared_prefix(self) -> Self {
        self.execute_prefix(false)
    }
    pub fn when_remote_readers_execute_a_prepared_prefix(self) -> Self {
        self.execute_prefix(true)
    }

    fn execute_prefix(mut self, remote: bool) -> Self {
        let root = self.directory.join("package");
        let model = self.directory.join("model.py");
        let grader = self.directory.join("grader.py");
        std::fs::write(&model, r"#!/usr/bin/python3
import json,os,sys
r=json.load(sys.stdin)
assert os.listdir('/input') == []
if r['kind']=='summarize':
    out={'schema':'merl.eval-adapter/v1','kind':'summary','text':r['source']['body'],'usage':{'input':5,'output':2}}
else:
    assert r['question_id'] in ['SYN-Q1@1','SYN-Q2@1']
    assert 'violet' not in r['context']
    assert '[reader instructions]' in r['context']
    assert '[approved reader method]' in r['context']
    out={'schema':'merl.eval-adapter/v1','kind':'final','answer':'An amber heading and square icon.','citations':['O1'],'usage':{'input':10,'output':5}}
json.dump(out,sys.stdout)
").unwrap();
        std::fs::write(&grader, r"#!/usr/bin/python3
import json,sys
r=json.load(sys.stdin)
assert json.load(open('/input/oracle.json'))['unit_id']==r['unit_id']
json.dump({'unit_id':r['unit_id'],'verdict':'not_scored','reason':'Synthetic exclusion outcome, never success credit.'},sys.stdout)
").unwrap();
        let prefix = self.prepare_prefix();
        let mut plan = json!({"schema":"merl.eval-visible-plan/v1", "package":{
            "root":root,"manifest_sha256":hash(&root.join("package.sha256")),
            "inventory_sha256":hash(&root.join("evaluator-only/approved-artifacts/question-inventory.json")),
            "rules_sha256":hash(&root.join("evaluator-only/approved-artifacts/scoring-rules.json")),"expected_units":3},
            "config":{"model":"synthetic","model_version":"v1","effort":"none","system_prompt":"Use supplied context.","task_prompt":"Answer the question.",
                "trials":[{"id":"synthetic-pair-a","randomness":{"seed":7}}],"recent_window":1,"max_tool_rounds":1,"search_results":1,"temperature":0},
            "model_program":model,"model_program_sha256":hash(&model),"grader_program":grader,"grader_program_sha256":hash(&grader),
            "limits":{"request_bytes":1_048_576,"response_bytes":262_144,"program_bytes":67_108_864,"seconds":30},"prefixes":[prefix]});
        if remote {
            let provider = crate::mock_provider::MockProvider::start("package");
            let adapter = self.directory.join("provider.py");
            std::fs::write(&adapter, include_str!("../fixtures/provider_adapter.py")).unwrap();
            for (role, path, env) in [
                ("reader", &model, "READER_KEY"),
                ("grader", &grader, "GRADER_KEY"),
            ] {
                let config = json!({"schema":"merl.remote-transport/v1","role":role,"endpoint":provider.endpoint(&format!("/{role}")),
                    "credential_headers":[{"name":"Authorization","env":env,"prefix":"Bearer "}],
                    "adapter_program":adapter,"adapter_program_sha256":hash(&adapter),
                    "settings":{"model":"synthetic","model_version":"v1","effort":"none","temperature":0,"max_output_tokens":50},
                    "limits":{"request_bytes":1_048_576,"response_bytes":262_144,"program_bytes":67_108_864,"seconds":10},"max_calls":1});
                let config_path = self.directory.join(format!("{role}-transport.json"));
                std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
                let key = if role == "reader" {
                    "model_transport"
                } else {
                    "grader_transport"
                };
                plan[key] = json!({"path":config_path,"sha256":hash(&config_path)});
                let mut code = std::fs::read_to_string(path)
                    .unwrap()
                    .replace("json.load(sys.stdin)", "json.loads(sys.stdin.readline())");
                if role == "reader" {
                    code=code.replace("if r['kind']=='summarize':", r"print(json.dumps({'schema':'merl.model-call/v1','messages':[{'role':'user','text':json.dumps(r)}]}),flush=True)
reply=json.loads(sys.stdin.readline())
assert reply['usage']=={'input':10,'output':5}
if r['kind']=='summarize':")
                        .replace("{'input':5,'output':2}","reply['usage']")
                        .replace("{'input':10,'output':5}}", "reply['usage']}")
                        .replace("json.dump(out,sys.stdout)","print(json.dumps({'schema':'merl.transport-final/v1','response':out}),flush=True)");
                } else {
                    code=r"#!/usr/bin/python3
import json,os,sys
r=json.loads(sys.stdin.readline())
assert 'READER_KEY' not in os.environ and 'GRADER_KEY' not in os.environ
oracle=json.load(open('/input/oracle.json'))
assert oracle['unit_id']==r['unit_id']
print(json.dumps({'schema':'merl.model-call/v1','messages':[{'role':'user','text':json.dumps(oracle)}]}),flush=True)
reply=json.loads(sys.stdin.readline())
assert reply['usage']=={'input':10,'output':5}
out={'unit_id':r['unit_id'],'verdict':'not_scored','reason':'Synthetic exclusion outcome, never success credit.'}
print(json.dumps({'schema':'merl.transport-final/v1','response':out}),flush=True)
".into();
                }
                std::fs::write(path, code).unwrap();
                plan[if role == "reader" {
                    "model_program_sha256"
                } else {
                    "grader_program_sha256"
                }] = json!(hash(path));
            }
            self.provider = Some(provider);
        }
        let path = self.directory.join("plan.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&plan).unwrap()).unwrap();
        self.output = Some(
            Command::new(env!("CARGO_BIN_EXE_merl-eval"))
                .env("READER_KEY", "reader-secret")
                .env("GRADER_KEY", "grader-secret")
                .args(["package", "run", "--plan"])
                .arg(&path)
                .args(["--plan-sha256", &hash(&path)])
                .output()
                .unwrap(),
        );
        self
    }
    pub fn then_exclusions_failures_missing_units_and_shared_costs_remain_explicit(self) {
        let output = self.output.as_ref().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "a partial inventory must not report a clean run: {output:?}"
        );
        let wire: Value = serde_json::from_slice(&output.stdout).expect("retained partial report");
        assert_eq!(wire["complete"], false);
        let report = &wire["report"];
        assert_eq!(report["missing"].as_array().unwrap().len(), 5);
        let prefix = &report["prefixes"][0];
        assert_eq!(prefix["trials"].as_array().unwrap().len(), 10);
        assert!(prefix["failures"].as_array().unwrap().is_empty());
        assert_eq!(prefix["preparation"].as_array().unwrap().len(), 3);
        assert_eq!(prefix["summary_calls"].as_array().unwrap().len(), 1);
        for counts in report["counts"].as_array().unwrap() {
            assert_eq!(counts["correct"], 0);
            assert_eq!(counts["not_scored"], 2);
            assert_eq!(counts["missing"], 1);
            assert_eq!(counts["expected"], 3);
        }
        assert_eq!(wire["model_calls"].as_array().unwrap().len(), 11);
        assert_eq!(wire["grader_calls"].as_array().unwrap().len(), 10);
        assert!(wire["grader_calls"][0]["input_sha256"]["response.json"].is_string());
        assert!(
            wire["model_calls"][0]["input_sha256"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }

    pub fn then_remote_calls_keep_role_separation_and_explicit_costs(self) {
        let output = self.output.as_ref().unwrap();
        let wire: Value =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{e}: {output:?}"));
        assert_eq!(wire["model_transport"]["role"], "reader");
        assert_eq!(wire["grader_transport"]["role"], "grader");
        assert_eq!(
            wire["model_transport"]["calls"].as_array().unwrap().len(),
            11,
            "{wire}"
        );
        assert_eq!(
            wire["grader_transport"]["calls"].as_array().unwrap().len(),
            10,
            "{wire}"
        );
        for key in ["model_transport", "grader_transport"] {
            for call in wire[key]["calls"].as_array().unwrap() {
                assert_eq!(call["usage"], json!({"input":10,"output":5}));
                assert!(call["error"].is_null());
                for adapter_call in call["adapter_calls"].as_array().unwrap() {
                    assert_eq!(adapter_call["input_sha256"], json!({}));
                }
            }
        }
        let requests = self.provider.as_ref().unwrap().requests();
        assert_eq!(requests.len(), 21);
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /grader "))
                .count(),
            10
        );
        self.then_exclusions_failures_missing_units_and_shared_costs_remain_explicit();
    }

    fn prepare_prefix(&self) -> Value {
        use merl_corpus::fixture::{Fixture, source_digest};
        let root = self.directory.join("package");
        let source: Value = serde_json::from_slice(
            &std::fs::read(root.join("reader-inputs/source-prefixes/SYN-C1__through-2.json"))
                .unwrap(),
        )
        .unwrap();
        let mut value: Value =
            serde_json::from_str(include_str!("../../../../corpus/development/DEV-C1.json"))
                .unwrap();
        value["id"] = json!("SYN-C1");
        value["source"] = source["source_identity"].clone();
        value["observations"] = source["observations"].clone();
        value["gold_states"] = json!([]);
        value["capture"]["captured_at"] = json!("2030-01-02T00:00:00Z");
        value["capture"]["history_fidelity"] = source["history_fidelity"].clone();
        let mut fixture: Fixture = serde_json::from_value(value).unwrap();
        fixture.capture.provider_observation_count = fixture.observations.len();
        fixture.capture.source_sha256 =
            source_digest(&fixture.provider_snapshot, &fixture.observations);
        let fixture_path = self.directory.join("fixture.json");
        std::fs::write(&fixture_path, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
        let database = self.directory.join("authority.sqlite");
        {
            let mut store = merl_store::Store::open(&database).unwrap();
            let project = merl_core::ProjectId::try_from("P1").unwrap();
            store.create_project(&project).unwrap();
            merl_ingest::import_fixture_for_causal_replay_through(
                &mut store, &project, &fixture, 1,
            )
            .unwrap();
        }
        let examples = root.join("interface-examples");
        let compiler = examples.join("compiler");
        let record_path = self.directory.join("preparation.json");
        let mut record: Value = serde_json::from_slice(
            &std::fs::read(examples.join("preparation-through-1.json")).unwrap(),
        )
        .unwrap();
        record["authority_sha256"] = json!(format!("sha256:{}", hash(&database)));
        record["runs"] = json!([]);
        record["calls"] = json!([]);
        record["total_usage"] = json!({"input":0,"output":0});
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        json!({"case":"SYN-C1","cutoff":1,"fixture":fixture_path,"fixture_sha256":hash(&fixture_path),
            "candidate_preparer_program":examples.join("preparer-placeholder.txt"),"candidate_commit":"a".repeat(40),
            "compiler_artifacts":{"program":compiler.join("program.txt"),"prompt":compiler.join("prompt.txt"),"rules":compiler.join("rules.txt"),"config":compiler.join("config.txt")},
            "merl":{"project":"P1","issue":merl_ingest::fixture_issue_id(&fixture).unwrap().to_string(),"scope":"controlled:SYN-C1","role":"researcher",
                "trials":[{"trial_id":"synthetic-pair-a","source_cutoff":1,"capture_phase":false,"database":database,"preparation_record":record_path}]}})
    }
}
impl Drop for Scenario {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn hash(path: &Path) -> String {
    use std::fmt::Write as _;
    Sha256::digest(std::fs::read(path).unwrap())
        .iter()
        .fold(String::new(), |mut s, b| {
            write!(s, "{b:02x}").unwrap();
            s
        })
}
