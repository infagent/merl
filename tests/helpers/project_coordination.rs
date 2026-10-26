use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

pub struct Project {
    home: PathBuf,
    input: PathBuf,
    results: BTreeMap<String, Value>,
}
impl Project {
    pub fn given_a_synthetic_project() -> Self {
        let input = std::env::temp_dir().join(format!("merl-project-{}", std::process::id()));
        assert!(!input.exists());
        fs::create_dir(&input).unwrap();
        fs::write(input.join("brief.txt"), "Synthetic saved guidance.\n").unwrap();
        Self {
            home: input.join("mail"),
            input,
            results: BTreeMap::new(),
        }
    }
    fn call(&self, words: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .arg(words[0])
            .arg("--json")
            .args(&words[1..])
            .arg("--home")
            .arg(&self.home)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.success(), value.get("code").is_none());
        value
    }
    fn record(&mut self, label: &str, words: &[&str]) -> Value {
        let value = self.call(words);
        self.results.insert(label.into(), value.clone());
        value
    }
    fn scope(&self, name: &str, project: Option<&str>) -> Value {
        let brief = self.input.join("brief.txt");
        let mut words = vec![
            "team",
            "create",
            "--name",
            name,
            "--scope-file",
            brief.to_str().unwrap(),
        ];
        if let Some(project) = project {
            words.extend(["--project", project]);
        }
        self.call(&words)
    }
    fn join(&self, scope: &str, member: &str) -> Value {
        let brief = self.input.join("brief.txt");
        self.call(&[
            "team",
            "join",
            "--team",
            scope,
            "--member",
            member,
            "--description-file",
            brief.to_str().unwrap(),
        ])
    }
    fn send(
        &self,
        from: &str,
        to: &str,
        id: &str,
        reply: Option<&str>,
        team: Option<&str>,
    ) -> Value {
        let brief = self.input.join("brief.txt");
        let mut words = vec![
            "mail",
            "send",
            "--from",
            from,
            "--to",
            to,
            "--id",
            id,
            "--kind",
            if reply.is_some() { "reply" } else { "assign" },
            "--body-file",
            brief.to_str().unwrap(),
        ];
        if let Some(reply) = reply {
            words.extend(["--in-reply-to", reply]);
        }
        if let Some(team) = team {
            words.extend(["--team", team]);
        }
        self.call(&words)
    }
    fn handle(&self, operation: &str, actor: &str, id: &str, reply: Option<&str>) -> Value {
        let mut words = vec!["mail", operation, "--agent", actor, "--id", id];
        if let Some(reply) = reply {
            words.extend(["--reply", reply]);
        }
        self.call(&words)
    }
    pub fn when_a_manager_discovers_and_coordinates_two_epic_planners(mut self) -> Self {
        self.record("empty", &["team", "list"]);
        self.results
            .insert("discovery_created_home".into(), json!(self.home.exists()));
        assert_eq!(self.scope("project", None)["outcome"], "created");
        self.results
            .insert("manager_before".into(), self.join("project", "manager"));
        for epic in ["epic-a", "epic-b"] {
            let created = self.scope(epic, Some("project"));
            assert_eq!(created["outcome"], "created", "{created}");
            assert_eq!(self.join(epic, "planner")["outcome"], "joined");
        }
        self.record(
            "page1",
            &["team", "list", "--project", "project", "--limit", "1"],
        );
        let after = self.results["page1"]["next_after"]
            .as_str()
            .unwrap()
            .to_owned();
        self.record(
            "page2",
            &[
                "team",
                "list",
                "--project",
                "project",
                "--after",
                &after,
                "--limit",
                "1",
            ],
        );
        self.record("all", &["team", "list"]);
        self.record("scope", &["team", "show", "--team", "epic-a"]);
        self.results.insert(
            "missing_project".into(),
            self.scope("bad-epic", Some("missing")),
        );
        self.results.insert(
            "changed_project".into(),
            self.scope("epic-a", Some("epic-b")),
        );
        self.exchange("epic-a", None, None);
        self.exchange("epic-b", Some("epic-b"), Some("project"));
        self.results.insert(
            "wrong_reply".into(),
            self.send(
                "epic-b/planner",
                "project/manager",
                "wrong-reply",
                Some("epic-a-work"),
                None,
            ),
        );
        self.results.insert(
            "wrong_recipient".into(),
            self.handle("show", "project/manager", "epic-a-work", None),
        );
        self.record(
            "manager_after",
            &["team", "member", "--team", "project", "--member", "manager"],
        );
        self.record(
            "manager_inbox",
            &["mail", "list", "--agent", "project/manager"],
        );
        self.record(
            "not_in_epic",
            &["team", "member", "--team", "epic-a", "--member", "manager"],
        );
        for actor in [
            "epic-a//planner",
            "/planner",
            "epic-a/",
            "epic-a/planner/extra",
        ] {
            let result = self.call(&["mail", "status", "--agent", actor]);
            assert_eq!(result["code"], "MAIL_INPUT");
        }
        self.record(
            "missing_actor",
            &["mail", "status", "--agent", "epic-a/missing"],
        );
        self.record(
            "missing_scope",
            &["mail", "status", "--agent", "missing/planner"],
        );
        self
    }
    fn exchange(&mut self, epic: &str, send_team: Option<&str>, reply_team: Option<&str>) {
        let planner = format!("{epic}/planner");
        let work = format!("{epic}-work");
        let reply = format!("{epic}-reply");
        let to = if send_team.is_some() {
            "planner"
        } else {
            &planner
        };
        let sent = self.send("project/manager", to, &work, None, send_team);
        assert_eq!(sent["outcome"], "sent");
        self.results.insert(format!("{epic}_send"), sent);
        assert_eq!(
            self.handle("read", &planner, &work, None)["message"]["id"],
            work
        );
        assert_eq!(
            self.handle("receipt", &planner, &work, None)["outcome"],
            "received"
        );
        let manager = if reply_team.is_some() {
            "manager"
        } else {
            "project/manager"
        };
        assert_eq!(
            self.send(&planner, manager, &reply, Some(&work), reply_team)["outcome"],
            "sent"
        );
        assert_eq!(
            self.handle("read", "project/manager", &reply, None)["message"]["id"],
            reply
        );
        assert_eq!(
            self.handle("receipt", "project/manager", &reply, None)["outcome"],
            "received"
        );
        self.record(
            &format!("{epic}_outstanding"),
            &["mail", "status", "--agent", &planner],
        );
        assert_eq!(
            self.handle("complete", &planner, &work, Some(&reply))["outcome"],
            "completed"
        );
        self.record(
            &format!("{epic}_done"),
            &["mail", "status", "--agent", &planner],
        );
    }
    pub fn then_one_saved_manager_identity_receives_both_outcomes(self) {
        assert_eq!(self.results["empty"]["teams"], json!([]));
        assert_eq!(self.results["discovery_created_home"], false);
        assert_eq!(self.results["page1"]["teams"][0]["team"], "epic-a");
        assert_eq!(self.results["page2"]["teams"][0]["team"], "epic-b");
        assert_eq!(self.results["page2"]["next_after"], Value::Null);
        assert_eq!(self.results["all"]["teams"].as_array().unwrap().len(), 3);
        assert!(self.results["all"]["teams"][0].get("scope").is_none());
        assert_eq!(self.results["scope"]["project"], "project");
        assert_eq!(self.results["scope"]["coordinator"], Value::Null);
        assert_eq!(self.results["manager_after"]["role"], Value::Null);
        assert_eq!(self.results["missing_project"]["code"], "TEAM_NOT_FOUND");
        assert_eq!(self.results["changed_project"]["code"], "TEAM_CONFLICT");
        assert_eq!(
            self.results["manager_before"]["mail_key"],
            self.results["manager_after"]["mail_key"]
        );
        assert_eq!(
            self.results["manager_after"]["description"],
            "Synthetic saved guidance.\n"
        );
        assert_eq!(
            self.results["manager_inbox"]["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(self.results["not_in_epic"]["code"], "TEAM_MEMBER_MISSING");
        for epic in ["epic-a", "epic-b"] {
            assert_eq!(
                self.results[&format!("{epic}_send")]["resolved"]["--from"]["team"],
                "project"
            );
            assert_eq!(
                self.results[&format!("{epic}_send")]["resolved"]["--to"]["team"],
                epic
            );
            assert_eq!(
                self.results[&format!("{epic}_outstanding")]["outstanding"],
                1
            );
            assert_eq!(self.results[&format!("{epic}_done")]["outstanding"], 0);
        }
        assert_eq!(self.results["wrong_reply"]["code"], "MAIL_REPLY_MISMATCH");
        assert_eq!(
            self.results["wrong_recipient"]["code"],
            "MAIL_NOT_RECIPIENT"
        );
        assert_eq!(self.results["missing_actor"]["code"], "TEAM_MEMBER_MISSING");
        assert_eq!(self.results["missing_scope"]["code"], "TEAM_NOT_FOUND");
        assert_eq!(self.results["manager_after"]["guidance_refs"], json!([]));
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.input).unwrap();
    }
}
