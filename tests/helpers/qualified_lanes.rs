use merl_cli::{CliResponse, run_with_clock};
use merl_lane::{Gh, LaneError, MailSink, Store};
use merl_mail::{
    Key, Mailbox,
    team::{MemberSpec, TeamSpec},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
};

pub struct Subscriptions {
    home: PathBuf,
    results: BTreeMap<String, Value>,
    frozen: Vec<Value>,
}

impl Subscriptions {
    fn given_home(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!(
            "merl-qualified-lanes-{}-{name}",
            std::process::id()
        ));
        assert!(!home.exists());
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("gh"),
            include_bytes!("../fixtures/lane_github.py"),
        )
        .unwrap();
        fs::set_permissions(home.join("gh"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(home.join("provider.json"), json!({"page_size":50,"prs":[{"number":1,"labels":["activity"],"events":[],"threads":[]}]}).to_string()).unwrap();
        let case = Self {
            home,
            results: BTreeMap::new(),
            frozen: Vec::new(),
        };
        case.success(&["mail", "register", "--agent", "alex"]);
        case
    }

    pub fn given_identically_named_members_and_a_legacy_identity(name: &str) -> Self {
        let case = Self::given_home(name);
        case.join_members();
        case
    }

    fn join_members(&self) {
        let mut mail = Mailbox::open(&self.home).unwrap();
        for (team, role) in [("alpha", None), ("beta", Some("research and docs".into()))] {
            mail.create_team(
                &TeamSpec::new(key(team), None, "Synthetic scope".into(), vec![]).unwrap(),
            )
            .unwrap();
            mail.join_team(
                &key(team),
                &MemberSpec::new(
                    key("alex"),
                    role,
                    "Synthetic starting guidance".into(),
                    vec![],
                )
                .unwrap(),
                &key(&format!("{team}-key")),
            )
            .unwrap();
        }
    }

    fn call(&self, words: &[&str], human: bool) -> Value {
        let mut args = vec![words[0].to_owned(), "--json".into()];
        args.extend(words[1..].iter().map(|s| (*s).to_owned()));
        args.extend(["--home".into(), self.home.to_str().unwrap().into()]);
        if human {
            args.extend(["--format".into(), "human".into()]);
        }
        match run_with_clock(&args, &|| Ok(1000)) {
            CliResponse::Success(text) => serde_json::from_str(&text).unwrap(),
            CliResponse::JsonError(e) => serde_json::from_str(&e.as_json()).unwrap(),
            CliResponse::HumanError(e) => panic!("unexpected human error {e}"),
        }
    }
    fn success(&self, words: &[&str]) -> Value {
        let value = self.call(words, false);
        assert!(value.get("code").is_none(), "{value}");
        value
    }
    fn start(&self, name: &str, agent: &str, team: Option<&str>) -> Value {
        let mut args = vec![
            "lane",
            "start",
            "--name",
            name,
            "--repo",
            "example/widgets",
            "--agent",
            agent,
            "--label",
            "activity",
        ];
        if let Some(team) = team {
            args.extend(["--team", team]);
        }
        self.call(&args, false)
    }
    fn drain(&self) {
        for _ in 0..8 {
            let result = self.success(&[
                "lane",
                "scan",
                "--gh",
                self.home.join("gh").to_str().unwrap(),
                "--max-requests",
                "16",
            ]);
            if result["requests"] == 0 {
                return;
            }
        }
        panic!("synthetic source did not drain");
    }
    fn record_mail(&mut self) {
        for actor in ["alex", "alpha/alex", "beta/alex"] {
            self.results.insert(
                actor.into(),
                self.success(&["mail", "list", "--agent", actor]),
            );
        }
    }

    pub fn when_new_subscriptions_select_members_and_legacy_names(mut self) -> Self {
        self.results.insert(
            "alpha".into(),
            self.start("alpha-activity", "alpha/alex", Some("missing-default")),
        );
        self.results.insert(
            "beta".into(),
            self.start("beta-activity", "alex", Some("beta")),
        );
        self.results
            .insert("legacy".into(), self.start("legacy-activity", "alex", None));
        self.results.insert(
            "retry".into(),
            self.start("alpha-activity", "alex", Some("alpha")),
        );
        self.results.insert(
            "conflict".into(),
            self.start("alpha-activity", "beta/alex", None),
        );
        self.results.insert(
            "missing_member".into(),
            self.start("missing-member", "alex", Some("missing")),
        );
        self.results.insert(
            "not_joined".into(),
            self.start("not-joined", "absent", Some("alpha")),
        );
        self.drain();
        self.record_mail();
        self.results
            .insert("json_status".into(), self.success(&["lane", "status"]));
        self.results
            .insert("human_status".into(), self.call(&["lane", "status"], true));
        self.record_absent_creation_and_format();
        self
    }

    fn record_absent_creation_and_format(&mut self) {
        let absent = self.home.join("must-not-create");
        let cases = [
            vec![
                "lane",
                "start",
                "--name",
                "missing",
                "--repo",
                "example/widgets",
                "--agent",
                "missing/alex",
                "--label",
                "activity",
            ],
            vec![
                "lane",
                "start",
                "--name",
                "missing",
                "--team",
                "alpha",
                "--expected-generation",
                "1",
            ],
            vec![
                "lane",
                "start",
                "--name",
                "missing",
                "--team",
                "alpha",
                "--team",
                "beta",
                "--expected-generation",
                "1",
            ],
        ];
        let mut results = Vec::new();
        for words in cases {
            for format in ["json", "human"] {
                let mut args = vec!["lane".to_owned(), "--json".into()];
                args.extend(words[1..].iter().map(|s| (*s).into()));
                args.extend([
                    "--home".into(),
                    absent.to_str().unwrap().into(),
                    "--format".into(),
                    format.into(),
                ]);
                let (mode, code) = match run_with_clock(&args, &|| {
                    panic!("invalid actor form must reject before clock")
                }) {
                    CliResponse::JsonError(e) => ("json", e.code),
                    CliResponse::HumanError(e) => ("human", e.code),
                    CliResponse::Success(s) => panic!("invalid creation succeeded {s}"),
                };
                results.push(json!({"mode":mode,"code":code}));
            }
        }
        self.results.insert("absent_errors".into(), json!(results));
        self.results
            .insert("created_absent".into(), json!(absent.exists()));
    }

    pub fn then_each_subscription_reaches_only_its_explicit_recipient(self) {
        for (result, team) in [("alpha", "alpha"), ("beta", "beta")] {
            assert_eq!(
                self.results[result]["outcome"], "started",
                "{}",
                self.results[result]
            );
            assert_eq!(
                self.results[result]["resolved"]["--agent"]["qualified"],
                format!("{team}/alex")
            );
            assert_eq!(
                self.results[result]["resolved"]["--agent"]["mail_key"],
                format!("{team}-key")
            );
        }
        assert!(self.results["legacy"].get("resolved").is_none());
        assert_eq!(self.results["retry"]["outcome"], "unchanged");
        assert_eq!(self.results["conflict"]["code"], "LANE_CONFLICT");
        assert_eq!(self.results["missing_member"]["code"], "TEAM_NOT_FOUND");
        assert_eq!(self.results["not_joined"]["code"], "TEAM_MEMBER_MISSING");
        for (actor, lane, recipient) in [
            ("alex", "legacy-activity", "alex"),
            ("alpha/alex", "alpha-activity", "alpha-key"),
            ("beta/alex", "beta-activity", "beta-key"),
        ] {
            let notes = self.results[actor]["messages"].as_array().unwrap();
            assert_eq!(notes.len(), 1);
            assert!(
                notes[0]["id"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("lane.{lane}."))
            );
            assert_eq!(notes[0]["to"], recipient);
            assert_eq!(notes[0]["kind"], "review");
        }
        let raw = self.results["json_status"]["lanes"].as_array().unwrap();
        let human = self.results["human_status"]["lanes"].as_array().unwrap();
        assert_eq!(raw.len(), 3);
        for (json, human) in raw.iter().zip(human) {
            assert!(json.get("qualified_agent").is_none());
            assert_eq!(json["agent"], human["agent"]);
            if json["agent"] == "alex" {
                assert!(human.get("qualified_agent").is_none());
            } else {
                assert!(
                    human["qualified_agent"]
                        .as_str()
                        .unwrap()
                        .ends_with("/alex")
                );
            }
        }
        assert_eq!(
            self.results["absent_errors"],
            json!([
                {"mode":"json","code":"TEAM_NOT_FOUND"},{"mode":"human","code":"TEAM_NOT_FOUND"},
                {"mode":"json","code":"LANE_INPUT"},{"mode":"human","code":"LANE_INPUT"},
            {"mode":"json","code":"LANE_INPUT"},{"mode":"human","code":"LANE_INPUT"}
            ])
        );
        assert_eq!(self.results["created_absent"], false);
    }

    pub fn when_a_member_rebinds_and_optional_name_metadata_is_unavailable(mut self) -> Self {
        self.results.insert(
            "start".into(),
            self.start("member-activity", "alpha/alex", None),
        );
        self.drain();
        self.results.insert(
            "before_queue".into(),
            self.success(&["lane", "queue", "--name", "member-activity"]),
        );
        let socket = self.home.join("synthetic-host.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        for (generation, session) in [
            ("0", "00000000-0000-4000-8000-000000000001"),
            ("1", "00000000-0000-4000-8000-000000000002"),
        ] {
            self.success(&[
                "mail",
                "bind",
                "--agent",
                "alpha/alex",
                "--socket",
                socket.to_str().unwrap(),
                "--session",
                session,
                "--expected-generation",
                generation,
            ]);
        }
        self.results
            .insert("before_status".into(), self.success(&["lane", "status"]));
        self.results
            .insert("named_human".into(), self.call(&["lane", "status"], true));
        fs::rename(self.home.join("mail-v1"), self.home.join("hidden-mail")).unwrap();
        self.results
            .insert("missing_human".into(), self.call(&["lane", "status"], true));
        self.results
            .insert("missing_json".into(), self.success(&["lane", "status"]));
        self.results.insert(
            "display_created_mail".into(),
            json!(self.home.join("mail-v1").exists()),
        );
        fs::create_dir(self.home.join("mail-v1")).unwrap();
        let corrupt = self.home.join("mail-v1/mail.sqlite");
        fs::write(&corrupt, b"synthetic unreadable database metadata").unwrap();
        self.results.insert(
            "unavailable_human".into(),
            self.call(&["lane", "status"], true),
        );
        self.results
            .insert("unavailable_json".into(), self.success(&["lane", "status"]));
        self.results
            .insert("metadata_bytes".into(), json!(fs::read(&corrupt).unwrap()));
        self.success(&[
            "lane",
            "stop",
            "--name",
            "member-activity",
            "--expected-generation",
            "1",
        ]);
        self.results.insert(
            "resume".into(),
            self.success(&[
                "lane",
                "start",
                "--name",
                "member-activity",
                "--expected-generation",
                "2",
            ]),
        );
        fs::remove_dir_all(self.home.join("mail-v1")).unwrap();
        fs::rename(self.home.join("hidden-mail"), self.home.join("mail-v1")).unwrap();
        self.results
            .insert("after_status".into(), self.success(&["lane", "status"]));
        self.results.insert(
            "after_queue".into(),
            self.success(&["lane", "queue", "--name", "member-activity"]),
        );
        self.record_mail();
        self
    }

    pub fn then_saved_delivery_and_json_survive_name_lookup_failures(self) {
        assert_eq!(self.results["start"]["outcome"], "started");
        assert_eq!(
            self.results["named_human"]["lanes"][0]["qualified_agent"],
            "alpha/alex"
        );
        for name in [
            "missing_human",
            "unavailable_human",
            "missing_json",
            "unavailable_json",
        ] {
            assert_eq!(self.results[name], self.results["before_status"]);
        }
        assert_eq!(self.results["display_created_mail"], false);
        assert_eq!(
            self.results["metadata_bytes"],
            json!(b"synthetic unreadable database metadata".to_vec())
        );
        assert_eq!(self.results["resume"]["outcome"], "started");
        assert_eq!(
            self.results["after_status"]["lanes"][0]["agent"],
            "alpha-key"
        );
        assert_eq!(self.results["before_queue"], self.results["after_queue"]);
        assert_eq!(
            self.results["alpha/alex"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        for actor in ["alex", "beta/alex"] {
            assert_eq!(self.results[actor]["messages"], json!([]));
        }
    }

    pub fn given_a_legacy_subscription_with_frozen_activity() -> Self {
        let mut case = Self::given_home("frozen");
        assert_eq!(
            case.start("legacy-activity", "alex", None)["outcome"],
            "started"
        );
        let mut store = Store::open(&case.home).unwrap();
        let mut source = Gh::new(&case.home.join("gh"), &case.home).unwrap();
        let mut sink = Interrupted {
            attempts: &mut case.frozen,
        };
        let error = store.scan_at(&mut source, &mut sink, 1000, 16).unwrap_err();
        assert_eq!(error.code, "LANE_IO");
        case
    }

    pub fn when_members_join_and_a_different_recipient_is_requested(mut self) -> Self {
        self.join_members();
        self.results.insert(
            "retarget".into(),
            self.start("legacy-activity", "alpha/alex", None),
        );
        self.results.insert(
            "new".into(),
            self.start("new-member-activity", "alex", Some("alpha")),
        );
        self.drain();
        self.record_mail();
        let id = self.results["alex"]["messages"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.results.insert(
            "frozen_mail".into(),
            self.success(&["mail", "show", "--agent", "alex", "--id", &id]),
        );
        self.success(&["mail", "receipt", "--agent", "alex", "--id", &id]);
        for actor in ["alex", "alpha/alex"] {
            self.results.insert(
                format!("outstanding-{actor}"),
                self.success(&["mail", "list", "--agent", actor, "--outstanding"]),
            );
        }
        self
    }

    pub fn then_frozen_mail_and_individual_handling_keep_their_original_owner(self) {
        assert_eq!(self.results["retarget"]["code"], "LANE_CONFLICT");
        assert_eq!(self.results["new"]["outcome"], "started");
        assert_eq!(self.frozen.len(), 1);
        let message = &self.results["frozen_mail"]["message"];
        assert_eq!(message["id"], self.frozen[0]["id"]);
        assert_eq!(message["body"], self.frozen[0]["body"]);
        assert_eq!(message["to"], self.frozen[0]["recipient"]);
        assert_eq!(message["to"], "alex");
        assert_eq!(self.results["beta/alex"]["messages"], json!([]));
        assert_eq!(
            self.results["alpha/alex"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        for actor in ["alex", "alpha/alex"] {
            assert_eq!(
                self.results[&format!("outstanding-{actor}")]["messages"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

struct Interrupted<'a> {
    attempts: &'a mut Vec<Value>,
}
impl MailSink for Interrupted<'_> {
    fn publish(
        &mut self,
        id: &Key,
        recipient: &Key,
        body: &str,
        _now: i64,
    ) -> Result<(), LaneError> {
        self.attempts
            .push(json!({"id":id.as_str(),"recipient":recipient.as_str(),"body":body}));
        Err(io::Error::from(io::ErrorKind::Interrupted).into())
    }
}
fn key(text: &str) -> Key {
    text.parse().unwrap()
}
impl Drop for Subscriptions {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.home).unwrap();
    }
}
