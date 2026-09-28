use merl_cli::{CliResponse, run_with_clock};
use serde_json::Value;
use std::collections::BTreeSet;

pub struct HelpScenario {
    records: Vec<(String, Value, String)>,
    failures: Vec<(Value, String)>,
    outcome: Option<(Value, Value)>,
}

impl HelpScenario {
    pub fn then_errors_stay_with_the_commands_that_expose_them(self) {
        for (topic, record, _) in &self.records {
            let errors = record["errors"].as_array().unwrap();
            let has = |code: &str| errors.contains(&Value::from(code));
            // These operations return conflict receipts after policy commit races.
            if matches!(
                topic.as_str(),
                "decision create"
                    | "question create"
                    | "question resolve"
                    | "finding create"
                    | "finding resolve"
                    | "hypothesis create"
                    | "claim create"
                    | "task request"
                    | "task accept"
                    | "task defer"
                    | "task start"
                    | "task complete"
                    | "source apply"
                    | "candidate accept"
                    | "candidate reject"
                    | "candidate correct"
                    | "project revalidation resolve"
            ) {
                assert!(
                    !has("POLICY_CONFLICT"),
                    "{topic}: conflict is a receipt outcome"
                );
                assert!(
                    has("POLICY_INPUT_CONFLICT"),
                    "{topic}: changed retries are errors"
                );
            }
            if matches!(
                topic.as_str(),
                "source compilation-policy"
                    | "compilation list"
                    | "compilation show"
                    | "project authority list"
                    | "project revalidation list"
                    | "candidate list"
                    | "project batch"
                    | "project delta"
                    | "inbox poll"
                    | "inbox show"
                    | "decision create"
                    | "question create"
                    | "question resolve"
                    | "finding create"
                    | "finding resolve"
                    | "hypothesis create"
                    | "claim create"
                    | "task request"
                    | "source purge"
            ) {
                assert!(
                    !has("PAYLOAD_NOT_FOUND"),
                    "{topic}: no payload lookup error escapes"
                );
            }
            if matches!(
                topic.as_str(),
                "project view"
                    | "issue view"
                    | "compilation context"
                    | "task defer"
                    | "project revalidation resolve"
            ) {
                assert!(
                    has("PAYLOAD_NOT_FOUND"),
                    "{topic}: retained bytes are resolved"
                );
            }
        }
    }

    pub fn given_the_merl_cli() -> Self {
        Self {
            records: Vec::new(),
            failures: Vec::new(),
            outcome: None,
        }
    }

    pub fn when_a_project_is_created_with_its_help(mut self) -> Self {
        let result =
            serde_json::from_str(&success("project init --id P1 --database :memory: --json"))
                .unwrap();
        self.outcome = Some((help("project init"), result));
        self
    }

    pub fn then_help_names_the_emitted_outcome(self) {
        let (help, result) = self.outcome.unwrap();
        assert!(
            help["outcomes"]
                .as_array()
                .unwrap()
                .contains(&result["outcome"])
        );
    }

    pub fn when_the_help_tree_is_read(mut self) -> Self {
        let mut pending = vec![String::new()];
        let mut visited = BTreeSet::new();
        while let Some(topic) = pending.pop() {
            if !visited.insert(topic.clone()) {
                continue;
            }
            let record = help(&topic);
            if let Some(children) = record["children"].as_array() {
                pending.extend(children.iter().map(|v| v.as_str().unwrap().to_owned()));
            }
            let human = success(&format!("help {topic}"));
            self.records.push((topic, record, human));
        }
        self
    }

    pub fn then_each_command_has_complete_matching_help(self) {
        let topics: BTreeSet<_> = self
            .records
            .iter()
            .map(|(topic, _, _)| topic.as_str())
            .collect();
        assert!(
            topics.contains("source compile"),
            "root help must discover shipped commands"
        );
        assert!(topics.contains("source compilation-policy set"));
        assert!(topics.contains("task complete"));
        assert!(topics.contains("project revalidation resolve"));
        for (topic, record, human) in &self.records {
            assert_eq!(record["schema"], "merl.help/v1", "{topic}");
            assert_eq!(record["command"], *topic);
            for field in ["usage", "summary"] {
                let text = record[field]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| panic!("{topic}: missing {field}"));
                assert!(human.contains(text), "{topic}: human help omits {field}");
            }
            let children = record["children"].as_array().expect("child command names");
            for child in children {
                assert!(topics.contains(child.as_str().unwrap()));
                assert!(child.is_string(), "help must not embed child schemas");
            }
            if record["kind"] == "group" {
                continue;
            }
            assert_eq!(record["kind"], "command");
            let example = record["example"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| panic!("{topic}: missing example"));
            assert!(
                example.starts_with(&format!("merl {topic} ")),
                "{topic}: example belongs to another command"
            );
            assert!(human.contains(example));
            for field in ["outcomes", "errors", "related"] {
                let values = record[field]
                    .as_array()
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| panic!("{topic}: missing {field}"));
                for value in values {
                    let text = value
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .expect("nonempty help field");
                    assert!(
                        human.contains(text),
                        "{topic}: human help omits {field}: {text}"
                    );
                    if field == "related" {
                        assert!(
                            topics.contains(text),
                            "{topic}: unknown related command {text}"
                        );
                    }
                }
            }
            assert!(
                record["commands"].as_array().is_none_or(Vec::is_empty),
                "leaf help includes unrelated operations"
            );
            assert!(!human.contains("merl project init --id") || topic == "project init");
            assert!(!human.contains("merl source compile --project") || topic == "source compile");
        }
    }

    pub fn when_public_failures_and_their_help_are_read(mut self) -> Self {
        for (topic, args) in [
            ("project revision", "--project missing --database :memory:"),
            (
                "source show",
                "--project missing --database :memory: --version SV1",
            ),
            (
                "compilation context",
                "--project missing --database :memory: --run CR1",
            ),
            ("source compile", "--project bad/id"),
            ("candidate show", "C1 --project P1 --database :memory:"),
            ("candidate list", "--project missing --database :memory:"),
            (
                "task request",
                "--project P1 --database :memory: --actor A1 --id R1 --subject T1 --summary work",
            ),
        ] {
            let args = format!("--json {topic} {args}")
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let code = match run_with_clock(&args, &|| Ok(0)) {
                CliResponse::JsonError(error) => error.code.to_owned(),
                other => panic!("expected public failure for {topic}: {other:?}"),
            };
            self.failures.push((help(topic), code));
        }
        for topic in [
            "source compile",
            "source replay",
            "compilation expand",
            "compilation list",
            "project authority list",
            "source compilation-policy",
        ] {
            self.records
                .push((topic.into(), help(topic), success(&format!("help {topic}"))));
        }
        self
    }

    pub fn then_help_matches_the_command_error_boundary(self) {
        for (record, code) in &self.failures {
            assert!(
                record["errors"]
                    .as_array()
                    .unwrap()
                    .contains(&Value::from(code.as_str())),
                "{} omits reachable {code}",
                record["command"]
            );
        }
        for (topic, record, _) in &self.records {
            let errors = record["errors"].as_array().unwrap();
            if matches!(
                topic.as_str(),
                "source compile" | "source replay" | "compilation expand"
            ) {
                for code in [
                    "COMPILER_ADAPTER_ERROR",
                    "COMPILER_INPUT_BUDGET",
                    "COMPILER_OUTPUT_BUDGET",
                    "INVALID_COMPILER_RESPONSE",
                ] {
                    assert!(errors.contains(&Value::from(code)), "{topic} omits {code}");
                }
            } else {
                assert!(
                    !errors
                        .iter()
                        .any(|v| v.as_str().unwrap().starts_with("COMPILER_")),
                    "read help advertises compiler execution errors"
                );
                assert!(
                    !record["outcomes"]
                        .as_array()
                        .unwrap()
                        .contains(&Value::from("accepted")),
                    "read help advertises mutation outcomes"
                );
            }
        }
    }
}

fn help(topic: &str) -> Value {
    serde_json::from_str(&success(&format!("help {topic} --json"))).unwrap()
}

fn success(command: &str) -> String {
    let arguments = command
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    match run_with_clock(&arguments, &|| Ok(0)) {
        CliResponse::Success(text) => text,
        other => panic!("{command}: {other:?}"),
    }
}
