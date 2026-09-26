//! Exercise help errors through the CLI with retained source and compiler work.

use merl_cli::{CliResponse, run_with_clock};
use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, PayloadId, ProjectId,
    SourceBindingId, SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{BindingCapturePolicy, SourceBinding, SourceCapture, Store};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::PathBuf};

const PROMPT: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

pub struct ErrorScenario {
    directory: PathBuf,
    failures: Vec<(Value, Value, &'static str)>,
    reads: Vec<Value>,
    unchanged: Option<Value>,
}

impl ErrorScenario {
    pub fn given_a_recorded_compilation() -> Self {
        let scenario = Self::given_a_retained_source();
        let result = scenario.call(&compile_arguments("CR1"));
        assert_eq!(result["outcome"], "compiled", "{result}");
        scenario
    }

    fn given_a_retained_source() -> Self {
        let directory = (0..1_000)
            .find_map(|index| {
                let path = std::env::temp_dir()
                    .join(format!("merl-help-errors-{}-{index}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => Some(path),
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("create test directory: {error}"),
                }
            })
            .expect("reserve test directory");
        let scenario = Self {
            directory,
            failures: Vec::new(),
            reads: Vec::new(),
            unchanged: None,
        };
        let created = scenario.call(&["project", "init", "--id", "P1", "--administrator", "alice"]);
        assert_eq!(created["outcome"], "accepted");
        let mut store = Store::open(&scenario.directory.join("project.sqlite")).unwrap();
        let project = ProjectId::try_from("P1").unwrap();
        let binding = SourceBinding {
            id: SourceBindingId::try_from("B1").unwrap(),
            provider: SourceProvider::try_from("controlled").unwrap(),
            provider_namespace_id: "fixture".into(),
            namespace_digest: Sha256::digest(b"fixture").into(),
        };
        store
            .resolve_capture_policy(
                &project,
                &binding,
                &BindingCapturePolicy {
                    mode: CompilationMode::CaptureOnly,
                    coverage: CoverageRequirement::Optional,
                    version: CapturePolicyVersion::try_from("fixture-v1").unwrap(),
                },
            )
            .unwrap();
        store
            .capture_source_version(
                &project,
                &SourceCapture {
                    binding,
                    source: SourceId::try_from("S1").unwrap(),
                    provider_entity_id: "S1",
                    context_scope_id: "issue-1",
                    version: SourceVersionId::try_from("SV1").unwrap(),
                    provider_version_id: "SV1",
                    kind: SourceKind::try_from("issue_comment").unwrap(),
                    supersedes: None,
                    ambiguous_order_with_previous: false,
                    author_time: None,
                    created_at_millis: 1,
                    occurred_at_millis: 1,
                    upstream_updated_at_millis: Some(1),
                    observed_at_millis: 1,
                    actor: Some(ActorId::try_from("alice").unwrap()),
                    provider_actor_id: Some("alice"),
                    source_author: Some(ActorId::try_from("alice").unwrap()),
                    provider_source_author_id: Some("alice"),
                    body: Some(b"Retained evidence."),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::CaptureOnly,
                    coverage_requirement: CoverageRequirement::Optional,
                    policy_version: CapturePolicyVersion::try_from("fixture-v1").unwrap(),
                },
            )
            .unwrap();
        std::fs::write(scenario.directory.join("compiler.sh"),
            "cat >/dev/null\nprintf '%s' '{\"schema\":\"merl.compiler-response/v1\",\"assertions\":[],\"relations\":[],\"context_required\":[],\"unresolved\":[{\"source\":\"SV1\",\"span_start\":0,\"span_end\":0}]}'\n").unwrap();
        scenario
    }

    pub fn when_the_same_request_retries_with_changed_compiler_configuration(mut self) -> Self {
        self.unchanged = Some(self.call(&compile_arguments("CR1")));
        for (option, replacement) in [
            ("--compiler-version", "v2"),
            ("--model", "another-model"),
            (
                "--prompt-digest",
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            ),
            ("--compiler-arg", "changed-adapter-argument"),
        ] {
            let mut arguments = compile_arguments("CR1");
            if option == "--compiler-arg" {
                arguments.extend([option, replacement]);
            } else {
                let index = arguments.iter().position(|value| *value == option).unwrap();
                arguments[index + 1] = replacement;
            }
            let result = self.call(&arguments);
            self.failures
                .push((help("source compile"), result, "POLICY_INPUT_CONFLICT"));
        }
        self
    }

    pub fn then_help_names_the_retry_conflict(self) {
        assert_eq!(self.unchanged.as_ref().unwrap()["outcome"], "unchanged");
        self.then_help_names_each_failure_at_its_public_boundary();
    }

    pub fn given_a_binding_with_an_erased_policy_reason() -> Self {
        let scenario = Self::given_a_retained_source();
        let result = scenario.call(&[
            "source",
            "compilation-policy",
            "set",
            "B1",
            "--id",
            "policy1",
            "--actor",
            "alice",
            "--expected-version",
            "fixture-v1",
            "--mode",
            "on_demand",
            "--coverage",
            "required",
            "--reason",
            "Review before compiling",
        ]);
        assert_eq!(result["outcome"], "accepted", "{result}");
        let policy = scenario.call(&["source", "compilation-policy", "B1"]);
        let reason = PayloadId::try_from(policy["change"]["reason"].as_str().unwrap()).unwrap();
        Store::open(&scenario.directory.join("project.sqlite"))
            .unwrap()
            .erase_payload(&ProjectId::try_from("P1").unwrap(), &reason)
            .unwrap();
        scenario
    }

    pub fn when_binding_metadata_and_help_are_read(mut self) -> Self {
        self.reads = vec![
            self.call(&["source", "compilation-policy", "B1"]),
            self.call(&["source", "compilation-policy", "B1", "--version", "SV1"]),
            help("source compilation-policy"),
        ];
        self
    }

    pub fn then_metadata_remains_readable_without_payload_errors(self) {
        assert_eq!(self.reads[0]["schema"], "merl.binding-policy/v1");
        assert!(self.reads[0]["change"]["reason"].is_string());
        assert_eq!(self.reads[1]["schema"], "merl.binding-policy-selection/v1");
        let errors = self.reads[2]["errors"].as_array().unwrap();
        assert!(!errors.contains(&Value::from("PAYLOAD_NOT_FOUND")));
        assert!(!errors.contains(&Value::from("PROJECT_NOT_FOUND")));
    }

    pub fn when_compilers_fail_and_recorded_input_is_erased(mut self) -> Self {
        std::fs::write(
            self.directory.join("compiler.sh"),
            "cat >/dev/null\nprintf 'invalid response'\n",
        )
        .unwrap();
        let result = self.call(&compile_arguments("bad-response"));
        self.failures
            .push((help("source compile"), result, "INVALID_COMPILER_RESPONSE"));
        let missing_program = self
            .directory
            .join("missing-compiler")
            .to_str()
            .unwrap()
            .to_owned();
        let mut arguments = compile_arguments("bad-adapter");
        let index = arguments
            .iter()
            .position(|arg| *arg == "--program")
            .unwrap();
        arguments[index + 1] = &missing_program;
        let result = self.call(&arguments);
        self.failures
            .push((help("source compile"), result, "COMPILER_ADAPTER_ERROR"));
        let preview = self.call(&[
            "source",
            "purge",
            "--version",
            "SV1",
            "--reason",
            "Erase evidence",
            "--dry-run",
        ]);
        let result = self.call(&[
            "source",
            "purge",
            "--version",
            "SV1",
            "--reason",
            "Erase evidence",
            "--actor",
            "alice",
            "--confirm-digest",
            preview["confirm_digest"].as_str().unwrap(),
        ]);
        assert!(result["completed"].as_bool().unwrap());
        for (topic, expected) in [
            ("source replay", "MISSING_EVIDENCE"),
            ("compilation context", "INVALID_COMPILATION"),
        ] {
            let mut arguments = topic.split_whitespace().collect::<Vec<_>>();
            arguments.extend(["--run", "CR1"]);
            let result = self.call(&arguments);
            self.failures.push((help(topic), result, expected));
        }
        self.reads
            .push(self.call(&["compilation", "show", "--run", "CR1"]));
        self
    }

    pub fn then_help_names_each_failure_at_its_public_boundary(self) {
        for (help, result, expected) in &self.failures {
            assert_eq!(result["code"], *expected, "{result}");
            assert!(
                help["errors"].as_array().unwrap().contains(&result["code"]),
                "{} omits {expected}",
                help["command"]
            );
        }
        for result in &self.reads {
            assert_eq!(
                result["outcome"], "succeeded",
                "metadata survives erasure: {result}"
            );
        }
    }

    fn call(&self, arguments: &[&str]) -> Value {
        let mut arguments = arguments
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect::<Vec<_>>();
        // Use an absolute script path so parallel tests never change the process directory.
        for argument in &mut arguments {
            if argument == "compiler.sh" {
                *argument = self.directory.join("compiler.sh").to_str().unwrap().into();
            }
        }
        arguments.extend([
            "--project".into(),
            "P1".into(),
            "--database".into(),
            self.directory
                .join("project.sqlite")
                .to_str()
                .unwrap()
                .into(),
            "--json".into(),
        ]);
        response(&arguments)
    }
}

impl Drop for ErrorScenario {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).unwrap();
    }
}

fn compile_arguments(run: &str) -> Vec<&str> {
    vec![
        "source",
        "compile",
        "--version",
        "SV1",
        "--run",
        run,
        "--actor",
        "alice",
        "--reason",
        "Review evidence",
        "--program",
        "/bin/sh",
        "--compiler-arg",
        "compiler.sh",
        "--compiler-version",
        "v1",
        "--model",
        "controlled",
        "--prompt-digest",
        PROMPT,
    ]
}

fn help(topic: &str) -> Value {
    response(
        &format!("help {topic} --json")
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
    )
}

fn response(arguments: &[String]) -> Value {
    match run_with_clock(arguments, &|| Ok(10)) {
        CliResponse::Success(text) => serde_json::from_str(&text).unwrap(),
        CliResponse::JsonError(error) => serde_json::from_str(&error.as_json()).unwrap(),
        other @ CliResponse::HumanError(_) => panic!("unexpected response: {other:?}"),
    }
}
