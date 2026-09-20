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
