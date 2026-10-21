use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

pub struct Scenario {
    directory: PathBuf,
    provider: crate::mock_provider::MockProvider,
    output: Option<Output>,
    mode: String,
    benchmark: Option<Value>,
}

impl Scenario {
    pub fn given_a_mock_provider_and_isolated_worker(mode: &str) -> Self {
        let directory = (0..1_000)
            .find_map(|i| {
                let path =
                    std::env::temp_dir().join(format!("merl-transport-{}-{i}", std::process::id()));
                std::fs::create_dir(&path).ok().map(|()| path)
            })
            .unwrap();
        let private = directory.join("private-answer.txt");
        std::fs::write(&private, "SYNTHETIC_PRIVATE_CANARY").unwrap();
        let provider = crate::mock_provider::MockProvider::start(mode);
        let adapter = directory.join("provider.py");
        std::fs::write(&adapter, include_str!("../fixtures/provider_adapter.py")).unwrap();
        let worker = directory.join("worker.py");
        let source = r"#!/usr/bin/python3
import json,os,socket,sys
r=json.loads(sys.stdin.readline())
assert r.get('question_id','SYN-Q1@1') in ['SYN-Q1@1','SYN-Q2@1']
assert os.listdir('/input')==[]
assert not os.path.exists(PRIVATE)
assert 'MOCK_PROVIDER_KEY' not in os.environ
try:
    socket.create_connection(('127.0.0.1',PORT),timeout=0.1)
    raise AssertionError('worker reached host network')
except OSError:
    pass
assert not os.path.exists('/tmp/previous-worker')
open('/tmp/previous-worker','w').write('synthetic state')
frame={'schema':'merl.model-call/v1','messages':[{'role':'user','text':r.get('question','Synthetic question?')}]}
if MODE=='override_url':
    frame['endpoint']='http://127.0.0.1:1/arbitrary'
print(json.dumps(frame),flush=True)
reply=json.loads(sys.stdin.readline())
assert reply['kind']=='success'
assert reply['text']=='Synthetic amber answer.'
assert reply['usage']=={'input':10,'output':5}
out={'schema':'merl.eval-adapter/v1','kind':'final','answer':reply['text'],'citations':[],'usage':reply['usage']}
if MODE=='preparer':
    assert r['schema']=='merl.compiler-request/v1'
    out={'schema':'merl.compiler-response/v1','assertions':[],'relations':[],'context_required':[],'unresolved':[]}
if MODE=='empty_summary' and r['kind']=='summarize':
    out={'schema':'merl.eval-adapter/v1','kind':'summary','text':'','usage':reply['usage']}
if MODE=='bad_final' or MODE=='empty_summary' and r['kind']=='answer':
    sys.exit(7)
if MODE=='bad_usage':
    out['usage']={'input':0,'output':0}
if MODE=='call_limit':
    print(json.dumps(frame),flush=True)
    sys.stdin.readline()
print(json.dumps({'schema':'merl.transport-final/v1','response':out}),flush=True)
".replace("PRIVATE", &serde_json::to_string(&private).unwrap())
            .replace("PORT", &provider.port().to_string())
            .replace("MODE", &format!("{mode:?}"));
        std::fs::write(&worker, source).unwrap();
        let role = if mode == "preparer" {
            "preparer"
        } else {
            "reader"
        };
        let config = json!({"schema":"merl.remote-transport/v1","role":role,
            "endpoint":provider.endpoint("/generate"),
            "credential_headers":[{"name":"Authorization","env":"MOCK_PROVIDER_KEY","prefix":"Bearer "}],
            "adapter_program":adapter,"adapter_program_sha256":hash(&adapter),
            "settings":{"model":"synthetic-model","model_version":"synthetic-v1","effort":"none","temperature":0,"max_output_tokens":50},
            "limits":{"request_bytes":1_048_576,"response_bytes":262_144,"program_bytes":67_108_864,"seconds":10},
            "max_calls":1});
        let config_path = directory.join("transport.json");
        std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        if mode == "adapter_tamper" {
            std::fs::write(&adapter, "changed after pinning").unwrap();
        }
        if mode == "role_swap" {
            let mut config = config.clone();
            config["role"] = json!("grader");
            std::fs::write(&config_path, serde_json::to_vec(&config).unwrap()).unwrap();
        }
        let plan = json!({"schema":"merl.transport-plan/v1","role":role,
            "worker_program":worker,"worker_program_sha256":hash(&worker),
            "transport":{"path":config_path,"sha256":hash(&config_path)},
            "limits":{"request_bytes":1_048_576,"response_bytes":262_144,"program_bytes":67_108_864,"seconds":10}});
        std::fs::write(
            directory.join("plan.json"),
            serde_json::to_vec(&plan).unwrap(),
        )
        .unwrap();
        Self {
            directory,
            provider,
            output: None,
            mode: mode.into(),
            benchmark: None,
        }
    }

    pub fn when_the_transport_runs(mut self) -> Self {
        let plan = self.directory.join("plan.json");
        let mut child = Command::new(env!("CARGO_BIN_EXE_merl-eval"))
            .args(["transport", "call", "--plan"])
            .arg(&plan)
            .args(["--plan-sha256", &hash(&plan)])
            .env(
                "MOCK_PROVIDER_KEY",
                if self.mode == "missing_credentials" {
                    ""
                } else {
                    "synthetic-secret-value"
                },
            )
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("http_proxy", "http://127.0.0.1:1")
            .env("https_proxy", "http://127.0.0.1:1")
            .env("all_proxy", "http://127.0.0.1:1")
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let request = if self.mode == "preparer" {
            json!({"schema":"merl.compiler-request/v1","limits":{"output_tokens":20},"context":{"source_observation_cutoff":1}})
        } else {
            json!({"schema":"merl.eval-adapter/v1","kind":"answer","question_id":"SYN-Q1@1","question":"Synthetic question?","trial":{"id":"pair-a","randomness":{"seed":7}}})
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&request).unwrap())
            .unwrap();
        self.output = Some(child.wait_with_output().unwrap());
        self
    }

    pub fn then_the_provider_receives_only_the_pinned_request(self) -> Self {
        let requests = self.provider.requests();
        assert_eq!(
            requests.len(),
            1,
            "provider calls: {requests:?}; {:?}",
            self.output
        );
        let request = &requests[0];
        assert!(
            request.starts_with("POST /generate HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer synthetic-secret-value\r\n")
        );
        let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(
            body,
            json!({"model":"synthetic-model","max_tokens":if self.mode=="preparer" {20} else {50},"messages":[{"role":"user","text":"Synthetic question?"}]})
        );
        self
    }

    pub fn then_the_response_and_reported_cost_have_provenance(self) {
        let output = self.output.as_ref().unwrap();
        assert!(output.status.success(), "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        if self.mode == "preparer" {
            assert_eq!(report["response"]["schema"], "merl.compiler-response/v1");
            assert!(report["response"].get("usage").is_none());
        } else {
            assert_eq!(report["response"]["answer"], "Synthetic amber answer.");
        }
        self.check_receipt(&report);
    }

    pub fn then_failure_keeps_reported_usage(self) {
        let output = self.output.as_ref().unwrap();
        assert!(!output.status.success(), "{output:?}");
        let report: Value =
            serde_json::from_slice(&output.stdout).expect("failure must retain receipts");
        assert!(report["response"].is_null());
        assert!(report["error"].is_string());
        self.check_receipt(&report);
    }

    pub fn then_the_boundary_fails_closed(self) {
        let output = self.output.as_ref().unwrap();
        assert!(
            !output.status.success(),
            "mode {} accepted: {output:?}",
            self.mode
        );
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("synthetic-secret-value"));
        if ["role_swap", "adapter_tamper", "missing_credentials"].contains(&self.mode.as_str()) {
            assert!(
                output.stdout.is_empty(),
                "preflight must reject before invoking workers"
            );
            assert!(self.provider.requests().is_empty());
            return;
        }
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(report["response"].is_null());
        let calls = report["transport"]["calls"].as_array().unwrap();
        assert_eq!(
            calls.len(),
            if self.mode == "call_limit" { 2 } else { 1 },
            "{report}"
        );
        if self.mode == "call_limit" {
            assert_eq!(calls[1]["attempted"], false);
            assert_eq!(calls[1]["error"], "model_call_limit");
        }
        let call = &calls[0];
        if self.mode == "override_url" {
            assert_eq!(call["attempted"], false);
            assert_eq!(call["error"], "transport_protocol");
            assert!(self.provider.requests().is_empty());
        } else {
            assert_eq!(call["attempted"], true);
            assert_eq!(self.provider.requests().len(), 1);
        }
        match self.mode.as_str() {
            "provider_failure" => {
                assert_eq!(call["error"], "rate_limited");
                assert_eq!(call["usage"], json!({"input":10,"output":5}));
            }
            "call_limit" | "bad_usage" => assert_eq!(call["usage"], json!({"input":10,"output":5})),
            "redirect" => {
                assert_eq!(call["http_status"], 307);
                assert_eq!(call["error"], "provider_redirect");
            }
            mode if mode.starts_with("credential_echo") => {
                assert_eq!(call["error"], "credential_echo");
                assert!(call["reported_model"].is_null());
                assert!(call["request_id"].is_null());
                assert!(call["usage"].is_null());
                let adapter_calls = call["adapter_calls"].as_array().unwrap();
                assert_eq!(
                    adapter_calls.len(),
                    1,
                    "provider JSON must not reach the decoder"
                );
                assert_eq!(adapter_calls[0]["kind"], "encode");
            }
            "unavailable" => {
                assert_eq!(call["error"], "provider_http");
                assert!(call["usage"].is_null());
            }
            "missing_usage" => {
                assert!(call["usage"].is_null());
                assert!(call["error"].is_string());
            }
            _ => {}
        }
    }

    pub fn when_reader_trials_fail_after_provider_usage(mut self) -> Self {
        use merl_eval::{BenchmarkRunner, EvaluationQuestion};
        let mut model = self.reader();
        let fixture =
            serde_json::from_str(include_str!("../../../../corpus/development/DEV-C1.json"))
                .unwrap();
        let config = reader_config();
        let questions = ["SYN-Q1@1", "SYN-Q2@1"]
            .into_iter()
            .map(|id| EvaluationQuestion {
                id: id.into(),
                cutoff: 1,
                capture_phase: false,
                text: "Synthetic question?".into(),
            })
            .collect();
        let report = BenchmarkRunner {
            model: &mut model,
            merl: &mut SyntheticMerl,
            scorer: &mut UnusedScorer,
        }
        .run_suite_recording(&fixture, questions, config)
        .unwrap();
        self.benchmark = Some(serde_json::to_value(report).unwrap());
        self
    }

    fn reader(&self) -> merl_eval::ProcessModelAdapter {
        use merl_eval::ProcessModelAdapter;
        use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
        use merl_eval::transport::{RemoteTransport, TransportPin, TransportRole};
        let path = self.directory.join("transport.json");
        let transport = RemoteTransport::open_with_credentials(
            &TransportPin {
                sha256: hash(&path),
                path,
            },
            TransportRole::Reader,
            |_| Some("synthetic-secret-value".into()),
        )
        .unwrap();
        assert!(!format!("{transport:?}").contains("synthetic-secret-value"));
        let worker =
            IsolatedProgram::new(&self.directory.join("worker.py"), ProcessLimits::default())
                .unwrap();
        ProcessModelAdapter::with_transport(worker, transport).unwrap()
    }

    pub fn when_the_reader_requests_another_model(mut self) -> Self {
        use merl_eval::{AnswerRequest, BenchmarkMethod, ModelAdapter};
        let mut model = self.reader();
        let mut config = reader_config();
        config.model = "another-model".into();
        let response = model.answer(AnswerRequest {
            config: &config,
            trial: &config.trials[0],
            method: BenchmarkMethod::RawHistory,
            question_id: "SYN-Q1@1",
            question: "Synthetic question?",
            context: "Synthetic prefix.",
            can_search: false,
            can_expand: false,
        });
        self.benchmark =
            Some(json!({"failed":response.is_err(),"known_usage":model.failed_usage()}));
        self
    }

    pub fn then_no_provider_call_or_cost_is_attributed(self) {
        let report = self.benchmark.as_ref().unwrap();
        assert_eq!(report["failed"], true);
        assert_eq!(report["known_usage"], json!({"input":0,"output":0}));
        assert!(self.provider.requests().is_empty());
    }

    pub fn then_each_failed_read_and_shared_preparation_is_charged(self) {
        let report = self.benchmark.as_ref().unwrap();
        assert_eq!(report["failures"].as_array().unwrap().len(), 10);
        assert_eq!(
            self.provider.requests().len(),
            7,
            "one shared preparation and six independent reads"
        );
        for method in report["methods"].as_array().unwrap() {
            let summary = ["rolling_summary", "summary_retrieval"]
                .contains(&method["method"].as_str().unwrap());
            assert_eq!(
                method["total_read_tokens"],
                if summary { 0 } else { 30 },
                "{report}"
            );
            assert_eq!(
                method["total_preparation_tokens"],
                if summary { 15 } else { 0 },
                "{report}"
            );
            assert_eq!(method["usage_incomplete"], true);
        }
    }

    fn check_receipt(&self, report: &Value) {
        assert_eq!(
            report["transport"]["calls"][0]["usage"],
            json!({"input":10,"output":5})
        );
        assert_eq!(
            report["transport"]["calls"][0]["reported_model"],
            "synthetic-resolved-v1"
        );
        assert_eq!(report["transport"]["calls"][0]["request_id"], "mock-1");
        assert_eq!(
            report["transport"]["role"],
            if self.mode == "preparer" {
                "preparer"
            } else {
                "reader"
            }
        );
        assert_eq!(
            report["transport"]["config_sha256"],
            hash(&self.directory.join("transport.json"))
        );
        assert_eq!(
            report["transport"]["adapter_sha256"],
            hash(&self.directory.join("provider.py"))
        );
        let text = serde_json::to_string(report).unwrap();
        assert!(!text.contains("synthetic-secret-value"));
        assert!(!text.contains("SYNTHETIC_PRIVATE_CANARY"));
        assert_eq!(
            ["success", "preparer"].contains(&self.mode.as_str()),
            report["error"].is_null()
        );
    }
}

impl Drop for Scenario {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub fn hash(path: &Path) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for byte in Sha256::digest(std::fs::read(path).unwrap()) {
        write!(text, "{byte:02x}").unwrap();
    }
    text
}

fn reader_config() -> merl_eval::BenchmarkConfig {
    serde_json::from_value(json!({"model":"synthetic-model","model_version":"synthetic-v1","effort":"none","system_prompt":"Synthetic system.","task_prompt":"Synthetic task.",
        "trials":[{"id":"pair-a","randomness":{"seed":7}}],"recent_window":1,"max_tool_rounds":1,"search_results":1,"temperature":0})).unwrap()
}

struct SyntheticMerl;
impl merl_eval::MerlSurface for SyntheticMerl {
    fn prepare(
        &mut self,
        question: &merl_eval::EvaluationQuestion,
    ) -> Result<merl_eval::MerlPrepared, String> {
        Ok(merl_eval::MerlPrepared {
            source_cutoff: question.cutoff,
            view: "Synthetic prepared view.".into(),
            preparation_usage: merl_eval::TokenUsage::default(),
            required_coverage_complete: true,
            causal: true,
        })
    }
    fn expand(&mut self, _: &str) -> Result<String, String> {
        Err("no expansion requested".into())
    }
}
struct UnusedScorer;
impl merl_eval::Scorer for UnusedScorer {
    fn score(&mut self, _: &str, _: &str, _: &[String]) -> Result<merl_eval::Score, String> {
        panic!("failed reads cannot be scored")
    }
}
