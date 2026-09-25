use super::{Capture, Value, json};
use merl_core::{ProjectId, SourceVersionId};
use merl_store::Store;

pub struct PolicyCases {
    cases: Vec<(Capture, &'static str, &'static str)>,
    results: Vec<Value>,
}

impl PolicyCases {
    pub fn given_cold_bindings_for_each_policy() -> Self {
        let mut cases = Vec::new();
        for mode in ["capture_only", "on_demand", "eager"] {
            for coverage in ["optional", "required"] {
                cases.push((binding(&format!("{mode}-{coverage}")), mode, coverage));
            }
        }
        Self {
            cases,
            results: vec![],
        }
    }

    pub fn given_a_binding_with_an_administrator() -> Self {
        Self {
            cases: vec![(binding("default"), "eager", "optional")],
            results: vec![],
        }
    }

    pub fn when_an_administrator_changes_policy_and_captures_new_activity(mut self) -> Self {
        for (case, mode, coverage) in self.cases.drain(..) {
            let initial = inspect(&case);
            let old = case.latest["sources"][0]["version"]
                .as_str()
                .unwrap()
                .to_owned();
            let change = set(
                &case,
                "change",
                "owner",
                mode,
                coverage,
                "github_capture_v1",
            );
            let after_set_calls = case.calls();
            let case = case.when_the_issue_is_captured();
            let after_refresh_calls = case.calls();
            let case = case.when_a_comment_arrives().when_the_issue_is_captured();
            let new_capture = case.latest.clone();
            let case = case
                .when_the_comment_is_edited()
                .when_the_issue_is_captured();
            let edited = case.latest.clone();
            case.command(&["project", "rebuild", "--project", "P1"]);
            let current = inspect(&case);
            let store = Store::open(&case.directory.join("project.sqlite")).unwrap();
            let old = store
                .source_version(
                    &ProjectId::try_from("P1").unwrap(),
                    &SourceVersionId::try_from(old.as_str()).unwrap(),
                )
                .unwrap()
                .unwrap();
            self.results.push(json!({"mode":mode,"coverage":coverage,"initial":initial,"change":change,"current":current,"after_set_calls":after_set_calls,"after_refresh_calls":after_refresh_calls,"calls":case.calls(),"new":new_capture,"edited":edited,"old_mode":old.compilation_mode.as_str(),"old_coverage":old.coverage_requirement.as_str(),"old_version":old.policy_version.as_str(),"delta":case.read_changes()}));
        }
        self
    }

    pub fn then_each_version_keeps_its_policy_and_only_eager_work_runs(self) {
        for row in self.results {
            let eager = row["mode"] == "eager";
            assert_eq!(row["change"]["outcome"], "accepted", "{row}");
            assert_eq!(row["after_set_calls"], 0);
            assert_eq!(row["after_refresh_calls"], 0);
            assert_eq!(row["calls"], if eager { 2 } else { 0 });
            assert_eq!(row["new"]["compiled"], u64::from(eager));
            assert_eq!(row["edited"]["compiled"], u64::from(eager));
            assert_eq!(row["new"]["policy"]["mode"], row["mode"]);
            assert_eq!(row["new"]["policy"]["coverage"], row["coverage"]);
            assert_eq!(row["edited"]["policy"], row["new"]["policy"]);
            assert_eq!(row["current"]["policy"], row["new"]["policy"]);
            assert_eq!(row["old_mode"], "capture_only");
            assert_eq!(row["old_coverage"], "optional");
            assert_eq!(row["old_version"], "github_capture_v1");
            assert_ne!(
                row["current"]["policy"]["version"],
                row["initial"]["policy"]["version"]
            );
            assert_eq!(row["current"]["change"]["actor"], "owner");
            let gaps = row["delta"]["delta"]["coverage"]["required_gaps"]
                .as_u64()
                .unwrap();
            assert_eq!(gaps > 0, !eager && row["coverage"] == "required", "{row}");
        }
    }

    pub fn when_policy_changes_are_rejected_accepted_retried_and_contested(mut self) -> Self {
        let (case, _, _) = self.cases.pop().unwrap();
        let initial = inspect(&case);
        let before = case.read_changes();
        let rejected = set(
            &case,
            "denied",
            "outsider",
            "eager",
            "required",
            "github_capture_v1",
        );
        let after_rejection = case.read_changes();
        let policy_after_rejection = inspect(&case);
        let accepted = set(
            &case,
            "change",
            "owner",
            "eager",
            "optional",
            "github_capture_v1",
        );
        let current = inspect(&case);
        let stale = set(
            &case,
            "stale",
            "owner",
            "capture_only",
            "required",
            "github_capture_v1",
        );
        let retry = set(
            &case,
            "change",
            "owner",
            "eager",
            "optional",
            "github_capture_v1",
        );
        let after = case.read_changes();
        self.results.push(json!({"initial":initial,"before":before,"rejected":rejected,"after_rejection":after_rejection,"policy_after_rejection":policy_after_rejection,"accepted":accepted,"current":current,"stale":stale,"retry":retry,"after":after,"calls":case.calls()}));
        self
    }

    pub fn then_only_authorized_current_changes_reach_the_audit_stream(self) {
        let row = &self.results[0];
        assert_eq!(row["rejected"]["outcome"], "rejected");
        assert_eq!(row["before"], row["after_rejection"]);
        assert_eq!(row["initial"], row["policy_after_rejection"]);
        assert_eq!(row["accepted"]["outcome"], "accepted");
        assert_eq!(row["stale"]["outcome"], "conflict");
        assert_eq!(row["retry"], row["accepted"]);
        assert_eq!(row["calls"], 0);
        assert_eq!(
            row["after"]["delta"]["head_revision"].as_u64().unwrap(),
            row["before"]["delta"]["head_revision"].as_u64().unwrap() + 1
        );
        assert_eq!(
            row["after"]["inbox"]["entries"].as_array().unwrap().len(),
            row["before"]["inbox"]["entries"].as_array().unwrap().len() + 1
        );
        assert!(!row["after"].to_string().contains("Policy audit reason"));
        assert!(row["current"]["change"]["reason"].is_string());
    }
}

fn binding(name: &str) -> Capture {
    let case = Capture::given_a_named_project(name).given_optional_capture_policy();
    let store = Store::open(&case.directory.join("project.sqlite")).unwrap();
    store
        .grant_administrator_unchecked_bootstrap(
            &ProjectId::try_from("P1").unwrap(),
            &merl_core::ActorId::try_from("owner").unwrap(),
        )
        .unwrap();
    drop(store);
    case.when_the_issue_is_captured()
}

fn inspect(case: &Capture) -> Value {
    case.command(&[
        "source",
        "compilation-policy",
        case.first_binding.as_ref().unwrap().as_str().unwrap(),
        "--project",
        "P1",
    ])
}

fn set(case: &Capture, id: &str, actor: &str, mode: &str, coverage: &str, expected: &str) -> Value {
    serde_json::from_str(&response(
        case,
        &[
            "source",
            "compilation-policy",
            "set",
            case.first_binding.as_ref().unwrap().as_str().unwrap(),
            "--project",
            "P1",
            "--id",
            id,
            "--actor",
            actor,
            "--mode",
            mode,
            "--coverage",
            coverage,
            "--expected-version",
            expected,
            "--reason",
            "Policy audit reason",
        ],
        true,
    ))
    .unwrap()
}

impl PolicyCases {
    pub fn given_bindings_with_competing_administrative_work() -> Self {
        let cases = ["same_binding", "other_binding", "authority"]
            .into_iter()
            .map(|competitor| {
                let mut case = binding(competitor);
                case.pages[0]["data"]["repository"]["id"] = json!("repo-2");
                case.pages[0]["data"]["repository"]["issue"]["id"] = json!("issue-2");
                (case.when_the_issue_is_captured(), competitor, "")
            })
            .collect();
        Self {
            cases,
            results: vec![],
        }
    }

    pub fn when_prepared_changes_commit_after_other_work(mut self) -> Self {
        use merl_core::{
            ActorId, AuthorityPermission, CapturePolicyVersion, CompilationMode,
            CoverageRequirement, PolicyInputId, SourceBindingId,
        };
        use merl_policy::{
            AuthorityChange, BindingPolicyChange, change_authority, change_binding_policy,
            prepare_binding_policy_change,
        };
        for (case, competitor, _) in self.cases.drain(..) {
            let first = case
                .first_binding
                .as_ref()
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            let second = case.latest["binding"].as_str().unwrap();
            let project = ProjectId::try_from("P1").unwrap();
            let mut store = Store::open(&case.directory.join("project.sqlite")).unwrap();
            let change = BindingPolicyChange {
                id: PolicyInputId::try_from("prepared").unwrap(),
                actor: ActorId::try_from("owner").unwrap(),
                binding: SourceBindingId::try_from(first.as_str()).unwrap(),
                expected_version: CapturePolicyVersion::try_from("github_capture_v1").unwrap(),
                mode: CompilationMode::Eager,
                coverage: CoverageRequirement::Optional,
                reason: "Prepared change".into(),
            };
            let prepared = prepare_binding_policy_change(&mut store, &project, &change, 1).unwrap();
            if competitor == "authority" {
                change_authority(
                    &mut store,
                    &project,
                    &AuthorityChange {
                        id: PolicyInputId::try_from("grant").unwrap(),
                        actor: change.actor.clone(),
                        subject: ActorId::try_from("reviewer").unwrap(),
                        permission: AuthorityPermission::CommandActor,
                        grant: true,
                        reason: "Grant changed after preparation".into(),
                    },
                    2,
                )
                .unwrap();
            } else {
                let competing = BindingPolicyChange {
                    id: PolicyInputId::try_from("competing").unwrap(),
                    binding: SourceBindingId::try_from(if competitor == "same_binding" {
                        &first
                    } else {
                        second
                    })
                    .unwrap(),
                    mode: CompilationMode::OnDemand,
                    ..change.clone()
                };
                change_binding_policy(&mut store, &project, &competing, 2).unwrap();
            }
            let committed = prepared.commit(&mut store);
            let evaluation = store
                .policy_evaluation(&project, &prepared.evaluation.id)
                .unwrap()
                .unwrap();
            let first_policy = store.binding_policy(&project, &change.binding).unwrap();
            let second_policy = store
                .binding_policy(&project, &SourceBindingId::try_from(second).unwrap())
                .unwrap();
            self.results.push(json!({"competitor":competitor,"committed":committed.is_ok(),"outcome":evaluation.inputs[0].disposition.as_str(),"first":first_policy.policy.mode.as_str(),"second":second_policy.policy.mode.as_str(),"calls":case.calls()}));
        }
        self
    }

    pub fn then_only_unrelated_binding_changes_can_commit(self) {
        for row in self.results {
            let unrelated = row["competitor"] == "other_binding";
            assert_eq!(row["committed"], unrelated, "{row}");
            assert_eq!(
                row["outcome"],
                if unrelated { "accepted" } else { "conflict" }
            );
            assert_eq!(
                row["second"],
                if unrelated {
                    "on_demand"
                } else {
                    "capture_only"
                }
            );
            assert_eq!(
                row["first"],
                if unrelated {
                    "eager"
                } else if row["competitor"] == "same_binding" {
                    "on_demand"
                } else {
                    "capture_only"
                }
            );
            assert_eq!(row["calls"], 0);
        }
    }

    pub fn when_help_human_results_and_retries_are_read(mut self) -> Self {
        let (case, _, _) = self.cases.pop().unwrap();
        let binding = case.first_binding.as_ref().unwrap().as_str().unwrap();
        let help = case.command(&["help", "source", "compilation-policy", "set"]);
        let group = case.command(&["help", "source"]);
        let accepted = set(
            &case,
            "accepted",
            "owner",
            "eager",
            "optional",
            "github_capture_v1",
        );
        let current = inspect(&case);
        let version = current["policy"]["version"].as_str().unwrap();
        let human = response(
            &case,
            &["source", "compilation-policy", binding, "--project", "P1"],
            false,
        );
        let human_set = response(
            &case,
            &[
                "source",
                "compilation-policy",
                "set",
                binding,
                "--project",
                "P1",
                "--id",
                "human",
                "--actor",
                "owner",
                "--mode",
                "on_demand",
                "--coverage",
                "required",
                "--expected-version",
                version,
                "--reason",
                "Human request",
            ],
            false,
        );
        let mut store = Store::open(&case.directory.join("project.sqlite")).unwrap();
        store
            .erase_payload(
                &ProjectId::try_from("P1").unwrap(),
                &merl_core::PayloadId::try_from(current["change"]["reason"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap();
        drop(store);
        let before_retry = inspect(&case);
        let retry = set(
            &case,
            "accepted",
            "owner",
            "eager",
            "optional",
            "github_capture_v1",
        );
        let after_retry = inspect(&case);
        let changed = response(
            &case,
            &[
                "source",
                "compilation-policy",
                "set",
                binding,
                "--project",
                "P1",
                "--id",
                "accepted",
                "--actor",
                "owner",
                "--mode",
                "eager",
                "--coverage",
                "optional",
                "--expected-version",
                "github_capture_v1",
                "--reason",
                "Different reason",
            ],
            true,
        );
        let view = case.command(&["project", "view", "--project", "P1"]);
        self.results.push(json!({"help":help,"group":group,"accepted":accepted,"retry":retry,"changed":changed,"before":before_retry,"after":after_retry,"human":human,"human_set":human_set,"view":view,"calls":case.calls()}));
        self
    }

    pub fn then_help_and_results_preserve_policy_and_retry_identity(self) {
        let row = &self.results[0];
        assert_eq!(row["help"]["schema"], "merl.help/v1");
        assert!(
            row["help"]["usage"]
                .as_str()
                .unwrap()
                .contains("--expected-version")
        );
        assert!(
            row["group"]
                .to_string()
                .contains("source compilation-policy")
        );
        assert_eq!(row["accepted"], row["retry"]);
        assert_eq!(row["before"], row["after"]);
        assert!(
            row["changed"]
                .as_str()
                .unwrap()
                .contains("POLICY_INPUT_CONFLICT")
        );
        assert!(row["human"].as_str().unwrap().contains("Changed by owner"));
        assert!(row["human_set"].as_str().unwrap().contains("accepted:"));
        assert!(
            !row["view"]
                .to_string()
                .contains("binding_compilation_policy")
        );
        assert_eq!(row["calls"], 0);
    }
}

fn response(case: &Capture, args: &[&str], json: bool) -> String {
    let mut args = args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
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
        other @ merl_cli::CliResponse::HumanError(_) => {
            panic!("unexpected command response: {other:?}")
        }
    }
}
