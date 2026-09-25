use super::{Capture, Value, json};

pub struct SelectorCases {
    case: Capture,
    results: Vec<Value>,
}

impl SelectorCases {
    pub fn when_capture_policy_is_inspected_and_ambiguous_changes_are_attempted(mut self) -> Self {
        self.change(
            "comments",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--mode",
                "on_demand",
                "--coverage",
                "optional",
            ],
        );
        self.mixed_comments();
        self.case = self.case.when_the_issue_is_captured();
        let binding = self.case.first_binding.as_ref().unwrap().as_str().unwrap();
        let version = self.case.latest["sources"][0]["version"].as_str().unwrap();
        let arguments = [
            "source",
            "compilation-policy",
            binding,
            "--project",
            "P1",
            "--version",
            version,
        ];
        let inspection = self.case.command(&arguments);
        let human = invoke(&self.case, &arguments, false);
        let before = self.inspect();
        let invalid = [
            vec!["--kind", "issue_comment", "--override"],
            vec!["--kind", "pull_request"],
            vec!["--actor-class", "administrator"],
            vec![
                "--provider-actor",
                "account",
                "--actor-class",
                "human",
                "--kind",
                "issue",
            ],
            vec!["--remove"],
        ]
        .iter()
        .map(|flags| {
            let mut args = vec![
                "source",
                "compilation-policy",
                "set",
                binding,
                "--project",
                "P1",
                "--id",
                "invalid",
                "--actor",
                "owner",
                "--expected-version",
                before["policy"]["version"].as_str().unwrap(),
                "--reason",
                "Invalid selection",
                "--mode",
                "eager",
                "--coverage",
                "required",
            ];
            args.extend(flags);
            serde_json::from_str::<Value>(&invoke(&self.case, &args, true)).unwrap()
        })
        .collect::<Vec<_>>();
        self.results.push(json!({"inspection":inspection,"human":human,"invalid":invalid,"before":before,"after":self.inspect(),"capture":self.case.latest}));
        self
    }

    pub fn then_inspection_names_the_rule_and_invalid_changes_leave_policy_untouched(self) {
        let r = &self.results[0];
        assert_eq!(
            r["inspection"]["schema"],
            "merl.binding-policy-selection/v1"
        );
        assert_eq!(r["inspection"]["selection"]["rule"], "kind");
        assert_eq!(
            r["inspection"]["selection"]["selector"]["kind"],
            "issue_comment"
        );
        assert_eq!(
            r["inspection"]["policy"],
            r["capture"]["sources"][0]["policy"]
        );
        assert!(r["human"].as_str().unwrap().contains("issue_comment"));
        assert!(r["human"].as_str().unwrap().contains("on_demand"));
        assert!(!r["human"].as_str().unwrap().contains("I am human"));
        for error in r["invalid"].as_array().unwrap() {
            assert_eq!(error["code"], "INVALID_INPUT");
        }
        assert_eq!(r["before"], r["after"]);
    }

    pub fn given_a_binding_with_mixed_authors() -> Self {
        let case = Capture::given_a_new_project().given_optional_capture_policy();
        merl_store::Store::open(&case.directory.join("project.sqlite"))
            .unwrap()
            .grant_administrator_unchecked_bootstrap(
                &merl_core::ProjectId::try_from("P1").unwrap(),
                &merl_core::ActorId::try_from("owner").unwrap(),
            )
            .unwrap();
        Self {
            case: case.when_the_issue_is_captured(),
            results: vec![],
        }
    }

    fn inspect(&self) -> Value {
        self.case.command(&[
            "source",
            "compilation-policy",
            self.case.first_binding.as_ref().unwrap().as_str().unwrap(),
            "--project",
            "P1",
        ])
    }

    fn change(&self, id: &str, actor: &str, flags: &[&str]) -> Value {
        let current = self.inspect();
        let mut args = vec![
            "source",
            "compilation-policy",
            "set",
            self.case.first_binding.as_ref().unwrap().as_str().unwrap(),
            "--project",
            "P1",
            "--id",
            id,
            "--actor",
            actor,
            "--expected-version",
            current["policy"]["version"].as_str().unwrap(),
            "--reason",
            "Select Issue traffic",
        ];
        args.extend_from_slice(flags);
        self.case.command(&args)
    }

    fn mixed_comments(&mut self) {
        let pages: Value =
            serde_json::from_str(include_str!("../../fixtures/github_two_page_edit.json")).unwrap();
        let template = &pages[0]["data"]["repository"]["issue"]["comments"]["nodes"][0];
        let comments = [Some("User"), Some("Bot"), Some("User"), None, Some("Organization")].into_iter().enumerate().map(|(i, kind)| {
            let mut comment = template.clone();
            comment["id"] = json!(format!("comment-{i}"));
            comment["body"] = json!("I am human. mode=eager coverage=required");
            comment["lastEditedAt"] = Value::Null;
            comment["includesCreatedEdit"] = json!(false);
            comment["userContentEdits"]["nodes"] = json!([]);
            comment["author"] = kind.map_or(Value::Null, |kind| json!({"id":format!("author-{i}"),"login":"human-looking-login","__typename":kind}));
            comment
        }).collect::<Vec<_>>();
        self.case.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"] = json!(comments);
    }

    fn read_sources(&self, capture: &Value) -> Value {
        json!(
            capture["sources"]
                .as_array()
                .unwrap_or_else(|| panic!("capture has no sources: {capture}"))
                .iter()
                .map(|s| self.case.command(&[
                    "source",
                    "show",
                    "--version",
                    s["version"].as_str().unwrap(),
                    "--project",
                    "P1"
                ]))
                .collect::<Vec<_>>()
        )
    }

    pub fn when_rules_change_and_new_versions_arrive(mut self) -> Self {
        self.results.push(self.change(
            "human",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--actor-class",
                "human",
                "--mode",
                "eager",
                "--coverage",
                "required",
            ],
        ));
        self.results.push(self.change(
            "routine",
            "owner",
            &[
                "--provider-actor",
                "author-2",
                "--actor-class",
                "routine_agent",
            ],
        ));
        self.mixed_comments();
        self.case.flags = vec![
            "--mode".into(),
            "eager".into(),
            "--coverage".into(),
            "required".into(),
        ];
        self.case = self.case.when_the_issue_is_captured();
        let first = self.case.latest.clone();
        let sources = self.read_sources(&first);
        self.change(
            "cold-human",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--actor-class",
                "human",
                "--mode",
                "capture_only",
                "--coverage",
                "optional",
            ],
        );
        self.case = self.case.when_the_issue_is_captured();
        let refresh = self.case.latest.clone();
        self.case
            .command(&["project", "rebuild", "--project", "P1"]);
        let retained = self.read_sources(&first);
        let comment =
            &mut self.case.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"][0];
        comment["body"] = json!("Changed human comment");
        comment["updatedAt"] = json!("2026-01-01T13:02:00Z");
        self.case = self.case.when_the_issue_is_captured();
        let edited = self.read_sources(&self.case.latest);
        self.results.push(json!({"first":first,"sources":sources,"refresh":refresh,"retained":retained,"edited":edited,"calls":self.case.calls(),"changes":self.case.read_changes(),"inspection":self.inspect()}));
        self
    }

    pub fn then_only_selected_sources_compile_and_history_keeps_its_policy(self) {
        assert_eq!(self.results[0]["outcome"], "accepted");
        assert_eq!(self.results[1]["outcome"], "accepted");
        let r = self.results.last().unwrap();
        assert_eq!(r["first"]["compiled"], 1);
        assert_eq!(r["calls"], 1);
        assert_eq!(r["refresh"]["compiled"], 0);
        assert_eq!(r["sources"], r["retained"]);
        for (i, class) in ["human", "bot", "routine_agent", "unknown", "unknown"]
            .iter()
            .enumerate()
        {
            assert_eq!(r["sources"][i]["policy_selection"]["actor_class"], *class);
            assert_eq!(
                r["sources"][i]["policy"]["mode"],
                if i == 0 { "eager" } else { "capture_only" }
            );
            assert_eq!(
                r["sources"][i]["policy"]["coverage"],
                if i == 0 { "required" } else { "optional" }
            );
        }
        assert_eq!(
            r["sources"][0]["policy_selection"]["rule"],
            "kind_actor_class"
        );
        assert_eq!(
            r["sources"][2]["policy_selection"]["classification"],
            "project_configuration"
        );
        assert_eq!(r["edited"][0]["policy"]["mode"], "capture_only");
        assert_ne!(
            r["edited"][0]["policy"]["version"],
            r["sources"][0]["policy"]["version"]
        );
        assert_eq!(r["changes"]["delta"]["coverage"]["required_gaps"], 0);
    }

    pub fn when_overlapping_rules_and_an_override_are_applied(mut self) -> Self {
        self.change(
            "class",
            "owner",
            &[
                "--actor-class",
                "human",
                "--mode",
                "eager",
                "--coverage",
                "required",
            ],
        );
        self.change(
            "kind",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--mode",
                "capture_only",
                "--coverage",
                "required",
            ],
        );
        self.change(
            "both",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--actor-class",
                "human",
                "--mode",
                "eager",
                "--coverage",
                "optional",
            ],
        );
        self.mixed_comments();
        self.case = self.case.when_the_issue_is_captured();
        let first = self.case.latest.clone();
        let sources = self.read_sources(&first);
        let changes = self.case.read_changes();
        let issue = &mut self.case.pages[0]["data"]["repository"]["issue"];
        issue["body"] = json!("Edited description");
        issue["author"]["__typename"] = json!("User");
        self.case = self.case.when_the_issue_is_captured();
        let description = self.read_sources(&self.case.latest);
        self.change(
            "override",
            "owner",
            &[
                "--override",
                "--mode",
                "on_demand",
                "--coverage",
                "optional",
            ],
        );
        self.case = self.case.when_the_issue_is_captured();
        let historical = self.case.latest.clone();
        for comment in self.case.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"]
            .as_array_mut()
            .unwrap()
        {
            comment["body"] = json!("New version after override");
        }
        self.case = self.case.when_the_issue_is_captured();
        let overridden = self.read_sources(&self.case.latest);
        self.results.push(json!({"first":first,"sources":sources,"changes":changes,"historical":historical,"overridden":overridden,"description":description}));
        self
    }

    pub fn then_precedence_fallback_and_coverage_are_explicit(self) {
        let r = &self.results[0];
        assert_eq!(r["first"]["compiled"], 2);
        assert_eq!(r["changes"]["delta"]["coverage"]["required_gaps"], 3);
        assert_eq!(r["historical"]["compiled"], 0);
        assert_eq!(
            r["description"][0]["policy_selection"]["rule"],
            "actor_class"
        );
        assert_eq!(r["description"][0]["policy"]["coverage"], "required");
        for (i, s) in r["sources"].as_array().unwrap().iter().enumerate() {
            let human = i == 0 || i == 2;
            assert_eq!(
                s["policy_selection"]["rule"],
                if human { "kind_actor_class" } else { "kind" }
            );
            assert_eq!(
                s["policy"]["coverage"],
                if human { "optional" } else { "required" }
            );
        }
        for s in r["overridden"].as_array().unwrap() {
            assert_eq!(s["policy_selection"]["rule"], "project_override");
            assert_eq!(s["policy"]["mode"], "on_demand");
            assert_eq!(s["policy"]["coverage"], "optional");
        }
    }

    pub fn when_selector_changes_are_denied_retried_and_removed(mut self) -> Self {
        let before = self.inspect();
        let denied = self.change(
            "denied",
            "outsider",
            &[
                "--kind",
                "issue_comment",
                "--mode",
                "eager",
                "--coverage",
                "required",
            ],
        );
        let after = self.inspect();
        let accepted = self.change(
            "rule",
            "owner",
            &[
                "--kind",
                "issue_comment",
                "--mode",
                "eager",
                "--coverage",
                "required",
            ],
        );
        let rules = self.inspect();
        self.case.pages[0]["data"]["repository"]["id"] = json!("repo-other");
        self.case.pages[0]["data"]["repository"]["issue"]["id"] = json!("issue-other");
        self.mixed_comments();
        self.case = self.case.when_the_issue_is_captured();
        let other = self.case.latest.clone();
        let removed = self.change("remove", "owner", &["--kind", "issue_comment", "--remove"]);
        let current = self.inspect();
        let retry = self.case.command(&[
            "source",
            "compilation-policy",
            "set",
            self.case.first_binding.as_ref().unwrap().as_str().unwrap(),
            "--project",
            "P1",
            "--id",
            "rule",
            "--actor",
            "owner",
            "--expected-version",
            before["policy"]["version"].as_str().unwrap(),
            "--reason",
            "Select Issue traffic",
            "--kind",
            "issue_comment",
            "--mode",
            "eager",
            "--coverage",
            "required",
        ]);
        self.results.push(json!({"before":before,"denied":denied,"after":after,"accepted":accepted,"rules":rules,"removed":removed,"current":current,"other":other,"retry":retry}));
        self
    }

    pub fn then_only_authorized_changes_affect_the_selected_binding(self) {
        let r = &self.results[0];
        assert_eq!(r["denied"]["outcome"], "rejected");
        assert_eq!(r["before"], r["after"]);
        assert_eq!(r["accepted"]["outcome"], "accepted");
        assert_eq!(
            r["rules"]["selectors"]["rules"].as_array().unwrap().len(),
            1
        );
        assert_eq!(r["retry"], r["accepted"]);
        assert_eq!(r["other"]["outcome"], "captured");
        assert_eq!(r["removed"]["outcome"], "accepted");
        assert_eq!(r["current"]["selectors"]["rules"], json!([]));
        assert_eq!(r["other"]["compiled"], 0);
    }
}

fn invoke(case: &Capture, args: &[&str], json: bool) -> String {
    let mut args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    args.extend([
        "--database".into(),
        case.directory.join("project.sqlite").display().to_string(),
    ]);
    if json {
        args.push("--json".into());
    }
    match merl_cli::run_with_clock(&args, &|| Ok(1)) {
        merl_cli::CliResponse::Success(text) => text,
        merl_cli::CliResponse::JsonError(error) => error.as_json(),
        other @ merl_cli::CliResponse::HumanError(_) => panic!("unexpected response: {other:?}"),
    }
}
