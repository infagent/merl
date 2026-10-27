use merl_cli::{CliResponse, run_with_clock};
use merl_mail::{
    Draft, Folder, Key, Mailbox,
    announcement::{AnnouncementSpec, Selection},
    team::{MemberSpec, TeamSpec},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf};

pub struct Announcements {
    home: PathBuf,
    results: BTreeMap<String, Value>,
}

impl Announcements {
    pub fn given_a_roster(name: &str, count: usize) -> Self {
        let home =
            std::env::temp_dir().join(format!("merl-announcements-{}-{name}", std::process::id()));
        assert!(!home.exists());
        let case = Self {
            home,
            results: BTreeMap::new(),
        };
        let mut store = Mailbox::open(&case.home).unwrap();
        store
            .create_team(
                &TeamSpec::new(key("epic"), None, "Synthetic scope".into(), vec![]).unwrap(),
            )
            .unwrap();
        store
            .join_team(
                &key("epic"),
                &MemberSpec::new(key("author"), None, "Synthetic guidance".into(), vec![]).unwrap(),
                &key("identity-author"),
            )
            .unwrap();
        for index in 0..count {
            join_index(&mut store, index);
        }
        case
    }

    fn call(&self, words: &[&str]) -> Value {
        let mut args: Vec<String> = words.iter().map(|s| (*s).into()).collect();
        args.extend([
            "--home".into(),
            self.home.to_str().unwrap().into(),
            "--json".into(),
        ]);
        match run_with_clock(&args, &|| Ok(1000)) {
            CliResponse::Success(text) => serde_json::from_str(&text).unwrap(),
            CliResponse::JsonError(error) => serde_json::from_str(&error.as_json()).unwrap(),
            CliResponse::HumanError(error) => panic!("unexpected human error {error}"),
        }
    }

    fn input(&self, text: &str) -> PathBuf {
        let path = self.home.with_extension("txt");
        fs::write(&path, text).unwrap();
        path
    }

    fn join(&self, team: &str, member: &str, role: Option<&str>) {
        let path = self.input("Synthetic saved guidance.");
        let mut args = vec![
            "team",
            "join",
            "--team",
            team,
            "--member",
            member,
            "--description-file",
            path.to_str().unwrap(),
        ];
        if let Some(role) = role {
            args.extend(["--role", role]);
        }
        assert_eq!(self.call(&args)["outcome"], "joined");
    }

    pub fn given_two_scopes_and_a_roleless_member() -> Self {
        let home = std::env::temp_dir().join(format!(
            "merl-announcements-{}-workflow",
            std::process::id()
        ));
        assert!(!home.exists());
        let case = Self {
            home,
            results: BTreeMap::new(),
        };
        let scope = case.input("Synthetic scope.");
        for team in ["project", "epic"] {
            assert_eq!(
                case.call(&[
                    "team",
                    "create",
                    "--name",
                    team,
                    "--scope-file",
                    scope.to_str().unwrap()
                ])["outcome"],
                "created"
            );
        }
        case.join("project", "author", None);
        case.join("epic", "dev", Some("research and docs"));
        case.join("epic", "other", Some("review"));
        case.join("epic", "roleless", None);
        case
    }

    fn announce(&self, id: &str, from: &str, flags: &[&str]) -> Value {
        let body = self.input("Synthetic notice.\n");
        let mut args = vec![
            "team",
            "announce",
            "--team",
            "epic",
            "--from",
            from,
            "--id",
            id,
            "--body-file",
            body.to_str().unwrap(),
        ];
        args.extend_from_slice(flags);
        self.call(&args)
    }

    pub fn when_notices_are_filtered_retried_and_receipted(mut self) -> Self {
        let work = self.input("Synthetic assignment.");
        self.results.insert(
            "work".into(),
            self.call(&[
                "mail",
                "send",
                "--from",
                "project/author",
                "--to",
                "epic/dev",
                "--id",
                "work",
                "--kind",
                "assign",
                "--body-file",
                work.to_str().unwrap(),
            ]),
        );
        self.results.insert(
            "filtered".into(),
            self.announce(
                "filtered",
                "project/author",
                &["--role", "review", "--role", "research and docs"],
            ),
        );
        self.results
            .insert("all".into(), self.announce("all", "dev", &["--all"]));
        self.results.insert(
            "self".into(),
            self.announce(
                "self",
                "dev",
                &["--role", "research and docs", "--include-self"],
            ),
        );
        self.results.insert(
            "self_filtered_out".into(),
            self.announce(
                "self-filtered",
                "dev",
                &["--role", "review", "--include-self"],
            ),
        );
        self.join("epic", "later", Some("research and docs"));
        self.results.insert(
            "retry".into(),
            self.announce(
                "filtered",
                "project/author",
                &[
                    "--role",
                    "research and docs",
                    "--role",
                    "review",
                    "--role",
                    "review",
                ],
            ),
        );
        self.results.insert(
            "conflict".into(),
            self.announce("filtered", "project/author", &["--all"]),
        );
        let id = self.results["filtered"]["recipients"][0]["message_id"]
            .as_str()
            .unwrap_or("missing")
            .to_owned();
        self.results.insert(
            "receipt".into(),
            self.call(&["mail", "receipt", "--agent", "epic/dev", "--id", &id]),
        );
        self.results.insert(
            "detail".into(),
            self.call(&["team", "announcement", "--team", "epic", "--id", "filtered"]),
        );
        fs::remove_file(self.home.with_extension("txt")).unwrap();
        self.results.insert(
            "detail_without_source".into(),
            self.call(&["team", "announcement", "--team", "epic", "--id", "filtered"]),
        );
        for member in ["dev", "other", "roleless", "later"] {
            self.results.insert(
                member.into(),
                self.call(&["mail", "list", "--agent", &format!("epic/{member}")]),
            );
        }
        self.results.insert(
            "outstanding".into(),
            self.call(&["mail", "list", "--agent", "epic/dev", "--outstanding"]),
        );
        self
    }

    pub fn then_retries_keep_the_snapshot_and_receipts_only_handle_one_note(self) {
        let members = |key: &str| {
            self.results[key]["recipients"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["member"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            self.results["filtered"]["outcome"], "published",
            "{}",
            self.results["filtered"]
        );
        assert_eq!(members("filtered"), vec![json!("dev"), json!("other")]);
        assert_eq!(members("all"), vec![json!("other"), json!("roleless")]);
        assert_eq!(members("self"), vec![json!("dev")]);
        assert_eq!(members("self_filtered_out"), vec![json!("other")]);
        assert_eq!(self.results["retry"]["outcome"], "unchanged");
        assert_eq!(
            self.results["retry"]["recipients"],
            self.results["filtered"]["recipients"]
        );
        assert_eq!(self.results["conflict"]["code"], "TEAM_CONFLICT");
        assert_eq!(self.results["receipt"]["outcome"], "received");
        assert_eq!(
            self.results["detail"],
            self.results["detail_without_source"]
        );
        assert!(self.results["detail"].get("body").is_none());
        assert_eq!(self.results["later"]["messages"], json!([]));
        let dev = self.results["dev"]["messages"].as_array().unwrap();
        assert_eq!(dev.len(), 3);
        assert_eq!(dev[1]["kind"], "note");
        assert_eq!(dev[1]["received_at"], 1000);
        assert!(self.results["other"]["messages"][0]["received_at"].is_null());
        assert_eq!(
            self.results["outstanding"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(self.results["outstanding"]["messages"][0]["id"], "work");
    }

    pub fn when_a_later_candidate_collides_and_the_notice_is_retried(mut self) -> Self {
        let mut store = Mailbox::open(&self.home).unwrap();
        // This foreign note exactly matches the proposed second child. Ordinary
        // send idempotency must never be mistaken for announcement ownership.
        let foreign = Draft::new(
            key("collision"),
            key("identity-author"),
            key("identity-001"),
            "note",
            "Synthetic notice".into(),
            None,
        )
        .unwrap();
        store.send(&foreign, 10).unwrap();
        self.results.insert(
            "before_first".into(),
            store.status(&key("identity-000")).unwrap(),
        );
        self.results.insert(
            "before_foreign".into(),
            store.status(&key("identity-001")).unwrap(),
        );
        let original = spec("atomic", Selection::all(), false, "Synthetic notice");
        self.results.insert(
            "failure".into(),
            json!(
                store
                    .announce(&original, &[key("first"), key("collision")], 20)
                    .unwrap_err()
                    .code
            ),
        );
        self.results.insert(
            "after_first".into(),
            store.status(&key("identity-000")).unwrap(),
        );
        self.results.insert(
            "after_foreign".into(),
            store.status(&key("identity-001")).unwrap(),
        );
        self.results.insert(
            "unaccepted".into(),
            json!(store.announcement(&key("atomic")).unwrap()),
        );
        self.results.insert(
            "foreign".into(),
            store
                .show(&key("identity-001"), &key("collision"), false, 30)
                .unwrap(),
        );
        // A failed transaction froze nothing; the next attempt sees this join.
        join_index(&mut store, 2);
        store
            .announce(&original, &[key("first"), key("second"), key("third")], 40)
            .unwrap();
        drop(store); // Discard the acknowledgment and reopen the public boundary.
        let mut store = Mailbox::open(&self.home).unwrap();
        join_index(&mut store, 3);
        self.results.insert(
            "lost_ack".into(),
            store.announce(&original, &[], 50).unwrap(),
        );
        self.results.insert(
            "detail".into(),
            store.announcement(&key("atomic")).unwrap().unwrap(),
        );
        self.record_changed_inputs(&mut store);
        store
            .receipt(&key("identity-000"), &key("first"), 70)
            .unwrap();
        self.results.insert(
            "before_retry".into(),
            store.status(&key("identity-000")).unwrap(),
        );
        store.announce(&original, &[], 80).unwrap();
        self.results.insert(
            "after_retry".into(),
            store.status(&key("identity-000")).unwrap(),
        );
        self.results.insert(
            "later".into(),
            store
                .list(&key("identity-003"), None, 100, Folder::All)
                .unwrap(),
        );
        self.results.insert(
            "first_notes".into(),
            store
                .list(&key("identity-000"), None, 100, Folder::All)
                .unwrap(),
        );
        self
    }

    fn record_changed_inputs(&mut self, store: &mut Mailbox) {
        let changes = [
            AnnouncementSpec::new(
                key("atomic"),
                key("elsewhere"),
                key("epic"),
                key("author"),
                "Synthetic notice".into(),
                Selection::all(),
                false,
            )
            .unwrap(),
            AnnouncementSpec::new(
                key("atomic"),
                key("epic"),
                key("elsewhere"),
                key("author"),
                "Synthetic notice".into(),
                Selection::all(),
                false,
            )
            .unwrap(),
            AnnouncementSpec::new(
                key("atomic"),
                key("epic"),
                key("epic"),
                key("missing"),
                "Synthetic notice".into(),
                Selection::all(),
                false,
            )
            .unwrap(),
            spec("atomic", Selection::all(), true, "Synthetic notice"),
            spec("atomic", Selection::all(), false, "Different body"),
        ];
        let conflicts = changes
            .iter()
            .map(|changed| store.announce(changed, &[], 60).unwrap_err().code)
            .collect::<Vec<_>>();
        self.results
            .insert("changed_inputs".into(), json!(conflicts));
    }

    pub fn then_rollback_preserves_foreign_mail_and_commit_preserves_the_snapshot(self) {
        assert_eq!(
            self.results["changed_inputs"],
            json!(vec!["TEAM_CONFLICT"; 5])
        );
        assert_eq!(self.results["before_retry"], self.results["after_retry"]);
        assert_eq!(self.results["failure"], "TEAM_MESSAGE_CONFLICT");
        assert_eq!(self.results["before_first"], self.results["after_first"]);
        assert_eq!(
            self.results["before_foreign"],
            self.results["after_foreign"]
        );
        assert!(self.results["unaccepted"].is_null());
        assert_eq!(
            self.results["foreign"]["message"]["body"],
            "Synthetic notice"
        );
        assert_eq!(self.results["foreign"]["message"]["created_at"], 10);
        assert_eq!(self.results["lost_ack"]["outcome"], "unchanged");
        assert_eq!(self.results["lost_ack"]["created_at"], 40);
        assert_eq!(self.results["lost_ack"]["recipient_count"], 3);
        assert_eq!(
            self.results["lost_ack"]["recipients"],
            self.results["detail"]["recipients"]
        );
        assert_eq!(self.results["later"]["messages"], json!([]));
        assert_eq!(
            self.results["first_notes"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    pub fn when_identical_callers_publish_concurrently(mut self) -> Self {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let calls = (0..2)
            .map(|caller| {
                let home = self.home.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut store = Mailbox::open(&home).unwrap();
                    let candidates = [
                        key(&format!("child-{caller}-0")),
                        key(&format!("child-{caller}-1")),
                    ];
                    barrier.wait();
                    store
                        .announce(
                            &spec("same", Selection::all(), false, "Concurrent notice"),
                            &candidates,
                            100,
                        )
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for (i, call) in calls.into_iter().enumerate() {
            self.results
                .insert(format!("caller{i}"), call.join().unwrap());
        }
        let store = Mailbox::open(&self.home).unwrap();
        for i in 0..2 {
            self.results.insert(
                format!("notes{i}"),
                store
                    .list(&key(&format!("identity-{i:03}")), None, 100, Folder::All)
                    .unwrap(),
            );
        }
        self
    }

    pub fn then_both_observe_one_committed_set_of_notes(self) {
        let mut outcomes = [
            self.results["caller0"]["outcome"].as_str().unwrap(),
            self.results["caller1"]["outcome"].as_str().unwrap(),
        ];
        outcomes.sort_unstable();
        assert_eq!(outcomes, ["published", "unchanged"]);
        assert_eq!(
            self.results["caller0"]["recipients"],
            self.results["caller1"]["recipients"]
        );
        for i in 0..2 {
            let notes = self.results[&format!("notes{i}")]["messages"]
                .as_array()
                .unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(
                notes[0]["id"],
                self.results["caller0"]["recipients"][i]["message_id"]
            );
        }
    }

    pub fn when_maximum_notices_and_invalid_selections_are_submitted(mut self) -> Self {
        let candidates = (0..100)
            .map(|i| key(&format!("maximum-{i}")))
            .collect::<Vec<_>>();
        let mut store = Mailbox::open(&self.home).unwrap();
        self.results.insert(
            "maximum".into(),
            store
                .announce(
                    &spec("maximum", Selection::all(), false, &"é".repeat(4096)),
                    &candidates,
                    100,
                )
                .unwrap(),
        );
        self.results.insert(
            "maximum_body".into(),
            store
                .show(&key("identity-099"), &key("maximum-99"), false, 100)
                .unwrap(),
        );
        // An author and >100 nonmatching members precede the sole role match.
        // Filtering after the cap would incorrectly reject or miss this member.
        join_index(&mut store, 100);
        self.results.insert(
            "overflow".into(),
            json!(
                store
                    .announce(
                        &spec("overflow", Selection::all(), false, "notice"),
                        &candidates,
                        110
                    )
                    .unwrap_err()
                    .code
            ),
        );
        let filtered = spec(
            "filtered-large-roster",
            Selection::roles(vec!["selected".into()]).unwrap(),
            true,
            "notice",
        );
        self.results.insert(
            "filtered".into(),
            store
                .announce(&filtered, &[key("only-match")], 120)
                .unwrap(),
        );
        self.results.insert(
            "empty".into(),
            json!(
                store
                    .announce(
                        &spec(
                            "empty",
                            Selection::roles(vec!["absent".into()]).unwrap(),
                            false,
                            "notice"
                        ),
                        &candidates,
                        130
                    )
                    .unwrap_err()
                    .code
            ),
        );
        for id in ["overflow", "empty"] {
            self.results.insert(
                format!("accepted-{id}"),
                json!(store.announcement(&key(id)).unwrap()),
            );
        }
        drop(store);
        self.record_invalid_cli_inputs();
        self
    }

    fn record_invalid_cli_inputs(&mut self) {
        let body = self.input("notice");
        let mut args = vec![
            "team",
            "announce",
            "--team",
            "epic",
            "--from",
            "author",
            "--id",
            "invalid",
            "--body-file",
            body.to_str().unwrap(),
        ];
        for _ in 0..17 {
            args.extend(["--role", "selected"]);
        }
        self.results
            .insert("too_many_roles".into(), self.call(&args));
        self.record_overflow_formats();
        for (label, text) in [("oversized", "é".repeat(4097)), ("blank", " \n".into())] {
            let body = self.input(&text);
            self.results.insert(
                label.into(),
                self.call(&[
                    "team",
                    "--json",
                    "announce",
                    "--team",
                    "epic",
                    "--from",
                    "author",
                    "--id",
                    "invalid",
                    "--all",
                    "--body-file",
                    body.to_str().unwrap(),
                ]),
            );
        }
        self.results
            .insert("help".into(), self.call(&["help", "team", "announce"]));
        let absent = self.home.join("never-created");
        let args = [
            "team",
            "announcement",
            "--team",
            "epic",
            "--id",
            "missing",
            "--home",
            absent.to_str().unwrap(),
            "--json",
        ]
        .map(str::to_owned);
        self.results.insert(
            "missing_detail".into(),
            match run_with_clock(&args, &|| panic!("read-only detail must not read clock")) {
                CliResponse::JsonError(e) => serde_json::from_str(&e.as_json()).unwrap(),
                other => panic!("expected missing detail: {other:?}"),
            },
        );
        self.results
            .insert("read_created_state".into(), json!(absent.exists()));
    }

    fn record_overflow_formats(&mut self) {
        let absent = self.home.join("overflow-must-not-create");
        let mut args = vec![
            "team",
            "--json",
            "announce",
            "--team",
            "epic",
            "--from",
            "author",
            "--id",
            "overflow-format",
            "--body-file",
            "/nonexistent-synthetic-notice",
        ];
        args.extend(["--home", absent.to_str().unwrap()]);
        let roles = (0..17)
            .flat_map(|_| ["--role", "selected"])
            .collect::<Vec<_>>();
        let mut results = Vec::new();
        for selectors in [
            roles,
            vec!["--all", "--all"],
            vec!["--include-self", "--include-self"],
        ] {
            for tail in [
                &["--format", "human"][..],
                &["--format", "human", "--json"],
                &["--format", "json"],
            ] {
                let words = args
                    .iter()
                    .chain(selectors.iter())
                    .chain(tail.iter())
                    .map(|s| (*s).to_owned())
                    .collect::<Vec<_>>();
                let output = std::process::Command::new(env!("CARGO_BIN_EXE_merl"))
                    .args(&words)
                    .output()
                    .unwrap();
                results.push(json!({"exit":output.status.code(),"stdout":String::from_utf8(output.stdout).unwrap(),"stderr":String::from_utf8(output.stderr).unwrap()}));
            }
        }
        self.results
            .insert("overflow_formats".into(), json!(results));
        self.results
            .insert("overflow_created_state".into(), json!(absent.exists()));
    }

    pub fn then_only_bounded_complete_snapshots_exist_and_detail_reads_are_pure(self) {
        assert_eq!(self.results["maximum"]["recipient_count"], 100);
        assert_eq!(
            self.results["maximum_body"]["message"]["body"]
                .as_str()
                .unwrap()
                .len(),
            8192
        );
        assert_eq!(self.results["overflow"], "TEAM_SELECTION");
        assert_eq!(self.results["empty"], "TEAM_SELECTION");
        assert!(self.results["accepted-overflow"].is_null());
        assert!(self.results["accepted-empty"].is_null());
        assert_eq!(self.results["filtered"]["recipient_count"], 1);
        assert_eq!(
            self.results["filtered"]["recipients"][0]["member"],
            "member-100"
        );
        assert_eq!(self.results["too_many_roles"]["code"], "TEAM_INPUT");
        for (index, output) in self.results["overflow_formats"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            assert_eq!(output["exit"], 2);
            if index % 3 == 0 {
                assert_eq!(output["stdout"], "");
                assert!(output["stderr"].as_str().unwrap().contains("TEAM_INPUT"));
            } else {
                assert_eq!(output["stderr"], "");
                let error: Value =
                    serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
                assert_eq!(error["schema"], "merl.error/v1");
                assert_eq!(error["code"], "TEAM_INPUT");
            }
        }
        assert_eq!(self.results["overflow_created_state"], false);
        for label in ["oversized", "blank"] {
            assert_eq!(self.results[label]["code"], "MAIL_INPUT");
        }
        assert_eq!(self.results["help"]["schema"], "merl.help/v1");
        assert_eq!(
            self.results["missing_detail"]["code"],
            "TEAM_ANNOUNCEMENT_NOT_FOUND"
        );
        assert_eq!(self.results["read_created_state"], false);
    }
}

fn key(text: &str) -> Key {
    text.parse().unwrap()
}
fn spec(id: &str, selection: Selection, include_self: bool, body: &str) -> AnnouncementSpec {
    AnnouncementSpec::new(
        key(id),
        key("epic"),
        key("epic"),
        key("author"),
        body.into(),
        selection,
        include_self,
    )
    .unwrap()
}
fn join_index(store: &mut Mailbox, index: usize) {
    store
        .join_team(
            &key("epic"),
            &MemberSpec::new(
                key(&format!("member-{index:03}")),
                Some(if index == 100 { "selected" } else { "ordinary" }.into()),
                "Synthetic guidance".into(),
                vec![],
            )
            .unwrap(),
            &key(&format!("identity-{index:03}")),
        )
        .unwrap();
}

impl Drop for Announcements {
    fn drop(&mut self) {
        if self.home.exists() {
            fs::remove_dir_all(&self.home).unwrap();
        }
        let input = self.home.with_extension("txt");
        if input.exists() {
            fs::remove_file(input).unwrap();
        }
    }
}
