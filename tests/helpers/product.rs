use merl_cli::{CliResponse, run_with_clock};
use serde_json::Value;
use std::{fs, path::PathBuf};

pub struct Product {
    help: Vec<(String, Value, String)>,
    home: Option<PathBuf>,
    results: Vec<Value>,
}
impl Product {
    pub fn given_the_messaging_product() -> Self {
        Self {
            help: Vec::new(),
            home: None,
            results: Vec::new(),
        }
    }
    pub fn given_an_opaque_old_runtime_file() -> Self {
        let mut case = Self::given_the_messaging_product();
        let home =
            std::env::temp_dir().join(format!("merl-retired-runtime-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("old.sqlite"), b"opaque historical runtime bytes").unwrap();
        case.home = Some(home);
        case
    }
    pub fn when_the_public_help_tree_is_read(mut self) -> Self {
        let mut pending = vec![String::new()];
        while let Some(topic) = pending.pop() {
            let mut args = vec!["help".to_owned()];
            args.extend(topic.split_whitespace().map(str::to_owned));
            let human = success(&args);
            args.push("--json".into());
            let value: Value = serde_json::from_str(&success(&args)).unwrap();
            pending.extend(
                value["children"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned()),
            );
            self.help.push((topic, value, human));
        }
        self
    }
    pub fn then_messaging_and_lane_commands_are_discoverable_with_matching_help(self) {
        let root = &self.help.iter().find(|(t, _, _)| t.is_empty()).unwrap().1;
        assert_eq!(
            root["children"],
            serde_json::json!(["mail", "lane", "init", "status", "team", "usage"])
        );
        assert_eq!(
            self.help.len(),
            32,
            "root, three groups, twenty-five operations, init, status, and usage"
        );
        for (topic, value, human) in &self.help {
            assert_eq!(value["schema"], "merl.help/v1");
            assert_eq!(value["command"], *topic);
            assert!(
                topic.is_empty()
                    || topic == "usage"
                    || topic == "status"
                    || topic == "init"
                    || topic == "mail"
                    || topic.starts_with("mail ")
                    || topic == "lane"
                    || topic.starts_with("lane ")
                    || topic == "team"
                    || topic.starts_with("team ")
            );
            for field in ["usage", "summary"] {
                assert!(human.contains(value[field].as_str().unwrap()));
            }
            if value["kind"] == "command" {
                let example = value["example"].as_str().unwrap();
                assert!(example.starts_with(&format!("merl {topic} ")));
                assert!(human.contains(example));
                for field in ["errors", "outcomes", "related"] {
                    let values = value[field].as_array().unwrap();
                    assert!(!values.is_empty());
                    for item in values {
                        assert!(human.contains(item.as_str().unwrap()));
                    }
                }
            }
        }
    }
    pub fn when_retired_product_commands_target_that_file(mut self) -> Self {
        let file = self.home.as_ref().unwrap().join("old.sqlite");
        for words in [
            ["project", "init"],
            ["issue", "view"],
            ["source", "compile"],
            ["compilation", "list"],
            ["candidate", "list"],
            ["task", "request"],
            ["inbox", "poll"],
            ["security", "explain"],
        ] {
            let mut args: Vec<String> = words.into_iter().map(str::to_owned).collect();
            args.extend([
                "--database".to_owned(),
                file.to_str().unwrap().to_owned(),
                "--json".to_owned(),
            ]);
            self.results
                .push(match run_with_clock(&args, &|| Ok(1000)) {
                    CliResponse::JsonError(e) => serde_json::from_str(&e.as_json()).unwrap(),
                    CliResponse::HumanError(e) => panic!("lost JSON mode: {e}"),
                    CliResponse::Success(s) => panic!("retired command succeeded: {s}"),
                });
        }
        self
    }
    pub fn then_commands_are_unavailable_and_the_old_bytes_are_unchanged(self) {
        for result in &self.results {
            assert_eq!(result["code"], "UNKNOWN_COMMAND");
        }
        let home = self.home.as_ref().unwrap();
        assert_eq!(
            fs::read(home.join("old.sqlite")).unwrap(),
            b"opaque historical runtime bytes"
        );
        assert_eq!(
            fs::read_dir(home).unwrap().count(),
            1,
            "no migration or new store was created"
        );
    }
}
fn success(args: &[String]) -> String {
    match run_with_clock(args, &|| Ok(0)) {
        CliResponse::Success(s) => s,
        CliResponse::JsonError(e) | CliResponse::HumanError(e) => panic!("help failed: {e}"),
    }
}
impl Drop for Product {
    fn drop(&mut self) {
        if let Some(home) = &self.home {
            fs::remove_dir_all(home).unwrap();
        }
    }
}
