use std::process::{Command, Output};

pub struct CliScenario {
    outputs: Vec<Output>,
}

impl CliScenario {
    pub fn given_the_merl_cli() -> Self {
        Self {
            outputs: Vec::new(),
        }
    }

    pub fn when_the_local_security_model_is_inspected(mut self) -> Self {
        self.outputs = vec![
            run(&["security", "explain"]),
            run(&["security", "explain", "--format", "json"]),
            run(&["security", "explain", "--json"]),
        ];
        self
    }

    pub fn then_both_formats_explain_the_same_trust_boundary(self) {
        for output in &self.outputs {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty());
        }
        let human = String::from_utf8_lossy(&self.outputs[0].stdout);
        let result: serde_json::Value = serde_json::from_slice(&self.outputs[1].stdout).unwrap();
        assert_eq!(result["schema"], "merl.security/v1");
        assert_eq!(result["action"], "security.explain");
        assert_eq!(result["deployment"], "local");
        assert_eq!(
            result,
            serde_json::from_slice::<serde_json::Value>(&self.outputs[2].stdout).unwrap()
        );
        for (field, expected) in [
            (
                "trust_boundary",
                "Unrestricted same-user processes are trusted",
            ),
            ("actor_claim", "--actor is a trusted local audit claim"),
            ("exposed_files", "SQLite"),
            (
                "provided_checks",
                "policy governance, validation, provenance, and audit",
            ),
            ("limitations", "actor impersonation"),
            ("host_controls", "does not inspect"),
            ("isolation_options", "separate daemon identity"),
        ] {
            let explanation = result[field].as_str().expect(field);
            assert!(explanation.contains(expected), "{field}: {explanation}");
            assert!(human.contains(explanation), "human output omits {field}");
        }
        let files = result["exposed_files"].as_str().unwrap();
        for file in [
            "payloads",
            "provider credentials",
            "configuration",
            "caches",
            "logs",
            "workspaces",
        ] {
            assert!(files.contains(file), "missing {file}");
        }
        let limits = result["limitations"].as_str().unwrap();
        for risk in [
            "file access",
            "state changes outside the API",
            "does not authenticate",
            "not a sandbox",
        ] {
            assert!(limits.contains(risk), "missing {risk}");
        }
        let isolation = result["isolation_options"].as_str().unwrap();
        for option in [
            "restricted IPC",
            "sandbox",
            "container",
            "shared authority",
            "client host",
        ] {
            assert!(isolation.contains(option), "missing {option}");
        }
    }

    pub fn when_security_and_actor_help_are_requested(mut self) -> Self {
        self.outputs = [
            "",
            "security",
            "security explain",
            "project authority grant",
            "project authority revoke",
            "source compilation-policy set",
            "source apply",
            "source compile",
            "source require",
            "source purge",
            "decision create",
            "question create",
            "question resolve",
            "finding create",
            "finding resolve",
            "hypothesis create",
            "claim create",
            "task request",
            "task accept",
            "task defer",
            "task start",
            "task complete",
            "candidate accept",
            "candidate reject",
            "candidate correct",
            "project revalidation run",
            "project revalidation resolve",
        ]
        .into_iter()
        .flat_map(|topic| {
            let mut args = vec!["help"];
            args.extend(topic.split_whitespace());
            let human = run(&args);
            args.push("--json");
            [human, run(&args)]
        })
        .collect();
        self
    }

    pub fn then_security_is_discoverable_and_actors_are_trusted_claims(self) {
        for pair in self.outputs.as_chunks::<2>().0 {
            assert!(pair[0].status.success());
            assert!(pair[1].status.success());
            let human = String::from_utf8_lossy(&pair[0].stdout);
            let help: serde_json::Value = serde_json::from_slice(&pair[1].stdout).unwrap();
            assert_eq!(help["schema"], "merl.help/v1");
            match help["command"].as_str().unwrap() {
                "" => {
                    assert!(human.contains("security"));
                    assert!(
                        help["related"]
                            .as_array()
                            .unwrap()
                            .contains(&serde_json::json!("security"))
                    );
                    assert!(!human.contains("--database"));
                    assert!(!human.contains("--actor"));
                }
                "security" => {
                    assert!(human.contains("explain"));
                    assert_eq!(help["related"], serde_json::json!(["security explain"]));
                }
                "security explain" => {
                    assert!(human.contains("merl security explain --json"));
                    assert_eq!(help["outcomes"], serde_json::json!(["explained"]));
                    assert_eq!(help["errors"], serde_json::json!(["INVALID_INPUT"]));
                }
                _ => {
                    let trust = help["trust_boundary"]
                        .as_str()
                        .expect("actor help explains trust");
                    assert!(trust.contains("--actor is a trusted local audit claim"));
                    assert!(trust.contains("not proof of identity"));
                    assert!(trust.contains("merl security explain"));
                    assert!(human.contains(trust));
                }
            }
        }
    }

    pub fn when_help_is_requested_at_each_depth(mut self) -> Self {
        self.outputs = vec![
            run(&["--help"]),
            run(&["help", "project"]),
            run(&["help", "project", "revision", "--format", "json"]),
        ];
        self
    }

    pub fn then_help_is_incremental_and_machine_readable(self) -> Self {
        let [top, group, command] = self.outputs.as_slice() else {
            panic!("help should be requested at all three depths");
        };
        assert!(top.status.success());
        let top_text = String::from_utf8(top.stdout.clone()).expect("UTF-8 help");
        assert!(top_text.contains("project"));
        assert!(!top_text.contains("--database"));

        assert!(group.status.success());
        assert!(
            String::from_utf8(group.stdout.clone())
                .expect("UTF-8 help")
                .contains("revision")
        );

        assert!(command.status.success());
        let help: serde_json::Value = serde_json::from_slice(&command.stdout).expect("JSON help");
        assert_eq!(help["schema"], "merl.help/v1");
        assert_eq!(help["command"], "project revision");
        self
    }

    pub fn when_a_project_is_initialized(mut self) -> Self {
        self.outputs = vec![run(&[
            "project",
            "init",
            "--id",
            "P1",
            "--database",
            ":memory:",
            "--format",
            "json",
        ])];
        self
    }

    pub fn then_initialization_has_a_versioned_result(self) -> Self {
        let initialized = self.only_output();
        assert!(initialized.status.success());
        let result: serde_json::Value =
            serde_json::from_slice(&initialized.stdout).expect("JSON result");
        assert_eq!(result["schema"], "merl.result/v1");
        assert_eq!(result["action"], "project.init");
        assert_eq!(result["revision"], 0);
        self
    }

    pub fn when_an_unknown_command_is_used(mut self) -> Self {
        self.outputs = vec![run(&["project", "unknown", "--format", "json"])];
        self
    }

    pub fn then_the_error_has_versioned_json(self) {
        let bad = self.only_output();
        assert!(!bad.status.success());
        let error: serde_json::Value = serde_json::from_slice(&bad.stdout).expect("JSON error");
        assert_eq!(error["schema"], "merl.error/v1");
        assert_eq!(error["code"], "INVALID_INPUT");
    }

    pub fn when_human_format_is_last(mut self) -> Self {
        self.outputs = vec![
            run(&["--json", "--format", "human", "project", "unknown"]),
            run(&[
                "--format", "json", "--format", "human", "project", "unknown",
            ]),
        ];
        self
    }

    pub fn then_errors_use_human_format(self) -> Self {
        for output in &self.outputs {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            assert!(String::from_utf8_lossy(&output.stderr).contains("INVALID_INPUT"));
        }
        self
    }

    pub fn when_json_format_is_last(mut self) -> Self {
        self.outputs = vec![run(&["--format", "human", "--json", "project", "unknown"])];
        self
    }

    pub fn then_errors_use_json_format(self) {
        let output = self.only_output();
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON error");
        assert_eq!(error["code"], "INVALID_INPUT");
    }

    fn only_output(&self) -> &Output {
        let [output] = self.outputs.as_slice() else {
            panic!("one command should have run");
        };
        output
    }
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_merl"))
        .args(arguments)
        .output()
        .expect("run Merl")
}
