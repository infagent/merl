use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

pub struct Teams {
    home: PathBuf,
    results: BTreeMap<String, Value>,
}

impl Teams {
    pub fn given_a_synthetic_home(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!("merl-team-{}-{name}", std::process::id()));
        assert!(!home.exists(), "synthetic home must be unique");
        Self {
            home,
            results: BTreeMap::new(),
        }
    }

    fn call(&self, words: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(words)
            .arg("--home")
            .arg(&self.home)
            .arg("--json")
            .output()
            .unwrap();
        let result: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("non-JSON result: {output:?}"));
        assert_eq!(
            output.status.success(),
            result.get("code").is_none(),
            "{result}"
        );
        result
    }

    fn save(&mut self, name: &str, words: &[&str]) {
        self.results.insert(name.into(), self.call(words));
    }

    fn input(&self, name: &str, body: &str) -> PathBuf {
        let directory = self.home.with_extension("inputs");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn create(&self, team: &str) -> Value {
        let scope = self.input("scope.txt", "Synthetic shared scope.\n");
        self.call(&[
            "team",
            "create",
            "--name",
            team,
            "--coordinator",
            "planner",
            "--scope-file",
            scope.to_str().unwrap(),
            "--guidance-ref",
            "/never/open/scope.md",
        ])
    }

    fn join(&self, team: &str, member: &str, role: &str) -> Value {
        let description = self.input("description.txt", "Keep this exact guidance.\n");
        self.call(&[
            "team",
            "join",
            "--team",
            team,
            "--member",
            member,
            "--role",
            role,
            "--description-file",
            description.to_str().unwrap(),
            "--guidance-ref",
            "/never/open/member.md",
        ])
    }

    pub fn given_a_team_with_four_members(self) -> Self {
        assert_eq!(self.create("alpha")["outcome"], "created");
        for (member, role) in [
            ("planner", "planning"),
            ("dev-a", "implementation"),
            ("dev-b", "research and docs"),
            ("reviewer", "review"),
        ] {
            assert_eq!(self.join("alpha", member, role)["outcome"], "joined");
        }
        self
    }

    fn send(
        &self,
        team: Option<&str>,
        id: &str,
        from: &str,
        to: &str,
        kind: &str,
        reply: Option<&str>,
    ) -> Value {
        let body = self.input("body.txt", "Synthetic handoff.\n");
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
            kind,
            "--body-file",
            body.to_str().unwrap(),
        ];
        if let Some(team) = team {
            words.extend(["--team", team]);
        }
        if let Some(reply) = reply {
            words.extend(["--in-reply-to", reply]);
        }
        self.call(&words)
    }

    fn handle(
        &mut self,
        label: &str,
        operation: &str,
        member: &str,
        id: &str,
        reply: Option<&str>,
    ) {
        let mut words = vec![
            "mail", operation, "--team", "alpha", "--agent", member, "--id", id,
        ];
        if let Some(reply) = reply {
            words.extend(["--reply", reply]);
        }
        self.save(label, &words);
    }

    fn save_human(&mut self, label: &str, words: &[&str]) {
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(words)
            .arg("--home")
            .arg(&self.home)
            .output()
            .unwrap();
        assert!(output.status.success());
        self.results.insert(
            label.into(),
            json!(String::from_utf8(output.stdout).unwrap()),
        );
    }

    fn save_mail_state(&mut self, label: &str, operation: &str, agent: &str) {
        self.save(
            label,
            &["mail", operation, "--team", "alpha", "--agent", agent],
        );
    }

    pub fn when_members_discover_and_exchange_work(mut self) -> Self {
        self.save("scope", &["team", "show", "--team", "alpha"]);
        self.save(
            "roster",
            &["team", "members", "--team", "alpha", "--limit", "2"],
        );
        let after = self.results["roster"]["next_after"]
            .as_str()
            .unwrap()
            .to_owned();
        self.save(
            "roster_tail",
            &["team", "members", "--team", "alpha", "--after", &after],
        );
        self.save(
            "guidance",
            &["team", "member", "--team", "alpha", "--member", "dev-b"],
        );
        self.results.insert(
            "build".into(),
            self.send(
                Some("alpha"),
                "alpha-build-1",
                "planner",
                "dev-a",
                "assign",
                None,
            ),
        );
        self.handle("read", "read", "dev-a", "alpha-build-1", None);
        self.save_human(
            "human_delivery",
            &[
                "mail",
                "show",
                "--team",
                "alpha",
                "--agent",
                "dev-a",
                "--id",
                "alpha-build-1",
            ],
        );
        self.handle("receipt", "receipt", "dev-a", "alpha-build-1", None);
        self.results.insert(
            "review".into(),
            self.send(
                Some("alpha"),
                "alpha-review-1",
                "dev-a",
                "reviewer",
                "review",
                None,
            ),
        );
        self.results.insert(
            "outcome".into(),
            self.send(
                Some("alpha"),
                "alpha-outcome-1",
                "reviewer",
                "planner",
                "note",
                None,
            ),
        );
        self.results.insert(
            "reply".into(),
            self.send(
                Some("alpha"),
                "alpha-reply-1",
                "dev-a",
                "planner",
                "reply",
                Some("alpha-build-1"),
            ),
        );
        self.save_mail_state("build_before", "status", "dev-a");
        self.save_mail_state("other_worker", "list", "dev-b");
        self.handle("wrong_owner", "complete", "dev-b", "alpha-build-1", None);
        self.handle(
            "review_done",
            "complete",
            "reviewer",
            "alpha-review-1",
            None,
        );
        self.save_mail_state("build_after_review", "status", "dev-a");
        self.handle(
            "build_done",
            "complete",
            "dev-a",
            "alpha-build-1",
            Some("alpha-reply-1"),
        );
        self.save_mail_state("build_final", "status", "dev-a");
        self.save_mail_state("review_final", "status", "reviewer");
        self
    }

    pub fn then_guidance_is_durable_and_each_request_has_one_owner(self) {
        let human = self.results["human_delivery"].as_str().unwrap();
        assert!(
            human.contains("alpha/planner"),
            "human delivery should identify the sender: {human}"
        );
        assert!(human.contains("alpha/dev-a"));
        assert!(
            !human.contains("mail_key"),
            "opaque transport metadata belongs in JSON"
        );
        assert_eq!(self.results["scope"]["scope"], "Synthetic shared scope.\n");
        assert_eq!(
            self.results["scope"]["guidance_refs"],
            json!(["/never/open/scope.md"])
        );
        assert_eq!(
            self.results["roster"]["members"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            self.results["roster_tail"]["members"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(
            self.results["roster"]["members"][0]
                .get("description")
                .is_none()
        );
        assert_eq!(
            self.results["guidance"]["description"],
            "Keep this exact guidance.\n"
        );
        assert_eq!(self.results["guidance"]["role"], "research and docs");
        assert_eq!(self.results["guidance"]["qualified"], "alpha/dev-b");
        for name in ["build", "review", "outcome", "reply"] {
            assert_eq!(self.results[name]["outcome"], "sent");
        }
        assert_eq!(self.results["build_before"]["outstanding"], 1);
        assert_eq!(self.results["build_after_review"]["outstanding"], 1);
        assert_eq!(self.results["other_worker"]["messages"], json!([]));
        assert_eq!(self.results["wrong_owner"]["code"], "MAIL_NOT_RECIPIENT");
        for name in ["review_done", "build_done"] {
            assert_eq!(self.results[name]["outcome"], "completed");
        }
        for name in ["build_final", "review_final"] {
            assert_eq!(self.results[name]["outstanding"], 0);
        }
    }

    pub fn given_identical_names_in_two_teams_and_legacy_mail(self) -> Self {
        for member in ["planner", "dev-a"] {
            assert_eq!(
                self.call(&["mail", "register", "--agent", member])["outcome"],
                "registered"
            );
        }
        for team in ["alpha", "beta"] {
            assert_eq!(self.create(team)["outcome"], "created");
            for member in ["planner", "dev-a"] {
                assert_eq!(self.join(team, member, "ordinary")["outcome"], "joined");
            }
        }
        self
    }

    pub fn when_names_and_conversations_are_crossed(mut self) -> Self {
        for (team, id) in [
            (Some("alpha"), "alpha-1"),
            (Some("beta"), "beta-1"),
            (None, "legacy-1"),
        ] {
            assert_eq!(
                self.send(team, id, "planner", "dev-a", "assign", None)["outcome"],
                "sent"
            );
        }
        for team in ["alpha", "beta"] {
            self.save(team, &["mail", "list", "--team", team, "--agent", "dev-a"]);
            self.save(
                &format!("{team}_member"),
                &["team", "member", "--team", team, "--member", "dev-a"],
            );
        }
        self.save("legacy", &["mail", "list", "--agent", "dev-a"]);
        self.save(
            "wrong_read",
            &[
                "mail", "show", "--team", "beta", "--agent", "dev-a", "--id", "alpha-1",
            ],
        );
        self.save(
            "wrong_cursor",
            &[
                "mail", "list", "--team", "beta", "--agent", "dev-a", "--after", "alpha-1",
            ],
        );
        self.results.insert(
            "wrong_reply".into(),
            self.send(
                Some("beta"),
                "beta-reply",
                "dev-a",
                "planner",
                "reply",
                Some("alpha-1"),
            ),
        );
        self.results.insert(
            "id_conflict".into(),
            self.send(Some("beta"), "alpha-1", "planner", "dev-a", "assign", None),
        );
        self.results.insert(
            "unknown".into(),
            self.send(
                Some("alpha"),
                "missing-1",
                "planner",
                "not-joined",
                "note",
                None,
            ),
        );
        self.save("not_created", &["mail", "status", "--agent", "not-joined"]);
        self.save(
            "missing_team",
            &["mail", "status", "--team", "absent", "--agent", "dev-a"],
        );
        self.save(
            "register_qualified",
            &["mail", "register", "--team", "alpha", "--agent", "dev-a"],
        );
        self
    }

    pub fn then_scopes_stay_distinct_and_message_ids_stay_global(self) {
        for (name, id) in [
            ("alpha", "alpha-1"),
            ("beta", "beta-1"),
            ("legacy", "legacy-1"),
        ] {
            assert_eq!(self.results[name]["messages"].as_array().unwrap().len(), 1);
            assert_eq!(self.results[name]["messages"][0]["id"], id);
        }
        assert_ne!(
            self.results["alpha_member"]["mail_key"],
            self.results["beta_member"]["mail_key"]
        );
        assert_ne!(self.results["alpha_member"]["mail_key"], "dev-a");
        assert_eq!(
            self.results["alpha"]["resolved"]["--agent"]["qualified"],
            "alpha/dev-a"
        );
        assert!(self.results["legacy"].get("resolved").is_none());
        assert_eq!(self.results["wrong_read"]["code"], "MAIL_NOT_RECIPIENT");
        assert!(self.results["wrong_cursor"].get("code").is_some());
        assert_eq!(self.results["wrong_reply"]["code"], "MAIL_REPLY_MISMATCH");
        assert_eq!(self.results["id_conflict"]["code"], "MAIL_ID_CONFLICT");
        assert_eq!(self.results["unknown"]["code"], "TEAM_MEMBER_MISSING");
        assert_eq!(self.results["not_created"]["code"], "MAIL_IDENTITY_MISSING");
        assert_eq!(self.results["missing_team"]["code"], "TEAM_NOT_FOUND");
        assert_eq!(self.results["register_qualified"]["code"], "MAIL_INPUT");
    }

    #[cfg(unix)]
    pub fn when_a_member_restarts_and_explicitly_rebinds(mut self) -> Self {
        let socket = self.home.join("host.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        self.save(
            "before",
            &["team", "member", "--team", "alpha", "--member", "dev-a"],
        );
        self.save(
            "bind",
            &[
                "mail",
                "bind",
                "--team",
                "alpha",
                "--agent",
                "dev-a",
                "--session",
                "00000000-0000-4000-8000-000000000001",
                "--socket",
                socket.to_str().unwrap(),
                "--expected-generation",
                "0",
            ],
        );
        self.results.insert(
            "send".into(),
            self.send(
                Some("alpha"),
                "alpha-restart",
                "planner",
                "dev-a",
                "assign",
                None,
            ),
        );
        self.results.insert(
            "retry".into(),
            self.join("alpha", "dev-a", "implementation"),
        );
        self.save(
            "unchanged_binding",
            &["mail", "status", "--team", "alpha", "--agent", "dev-a"],
        );
        for (name, generation) in [("stale", "0"), ("rebind", "1")] {
            self.save(
                name,
                &[
                    "mail",
                    "bind",
                    "--team",
                    "alpha",
                    "--agent",
                    "dev-a",
                    "--session",
                    "00000000-0000-4000-8000-000000000002",
                    "--socket",
                    socket.to_str().unwrap(),
                    "--expected-generation",
                    generation,
                ],
            );
        }
        self.save(
            "after",
            &["team", "member", "--team", "alpha", "--member", "dev-a"],
        );
        self.save(
            "status_after",
            &["mail", "status", "--team", "alpha", "--agent", "dev-a"],
        );
        self
    }

    #[cfg(unix)]
    pub fn then_identity_and_guidance_survive_without_implicit_rebinding(self) {
        assert_eq!(self.results["bind"]["outcome"], "bound");
        assert_eq!(self.results["retry"]["outcome"], "unchanged");
        assert_eq!(
            self.results["before"]["mail_key"],
            self.results["after"]["mail_key"]
        );
        assert_eq!(
            self.results["after"]["description"],
            "Keep this exact guidance.\n"
        );
        assert_eq!(
            self.results["unchanged_binding"]["binding"]["session"],
            "00000000-0000-4000-8000-000000000001"
        );
        assert_eq!(self.results["stale"]["code"], "MAIL_BINDING_CONFLICT");
        assert_eq!(self.results["rebind"]["outcome"], "bound");
        assert_eq!(self.results["status_after"]["outstanding"], 1);
        assert_eq!(self.results["status_after"]["binding"]["generation"], 2);
    }

    pub fn when_help_discovery_and_conflicting_inputs_are_requested(mut self) -> Self {
        self.save("help", &["help", "team", "join"]);
        self.save("absent", &["team", "members", "--team", "alpha"]);
        self.results
            .insert("read_created_home".into(), json!(self.home.exists()));
        self.results.insert("create".into(), self.create("alpha"));
        self.results
            .insert("create_retry".into(), self.create("alpha"));
        self.results
            .insert("join".into(), self.join("alpha", "dev-a", "implementation"));
        self.results.insert(
            "join_retry".into(),
            self.join("alpha", "dev-a", "implementation"),
        );
        self.results.insert(
            "join_conflict".into(),
            self.join("alpha", "dev-a", "different role"),
        );
        let huge = self.input("huge.txt", &"x".repeat(8193));
        self.save(
            "huge",
            &[
                "team",
                "join",
                "--team",
                "alpha",
                "--member",
                "too-big",
                "--description-file",
                huge.to_str().unwrap(),
            ],
        );
        self.save(
            "not_joined",
            &["team", "member", "--team", "alpha", "--member", "too-big"],
        );
        self.save(
            "guidance",
            &["team", "member", "--team", "alpha", "--member", "dev-a"],
        );
        self.save(
            "bad_limit",
            &["team", "members", "--team", "alpha", "--limit", "101"],
        );
        self
    }

    pub fn then_reads_are_pure_and_changed_inputs_never_overwrite_guidance(self) {
        assert_eq!(self.results["help"]["schema"], "merl.help/v1");
        assert_eq!(self.results["absent"]["code"], "TEAM_NOT_FOUND");
        assert_eq!(self.results["read_created_home"], false);
        assert_eq!(self.results["create_retry"]["outcome"], "unchanged");
        assert_eq!(self.results["join_retry"]["outcome"], "unchanged");
        assert_eq!(self.results["join_conflict"]["code"], "TEAM_CONFLICT");
        assert_eq!(self.results["huge"]["code"], "TEAM_INPUT");
        assert_eq!(self.results["not_joined"]["code"], "TEAM_MEMBER_MISSING");
        assert_eq!(self.results["guidance"]["role"], "implementation");
        assert_eq!(self.results["guidance"]["schema"], "merl.team/v1");
        assert_eq!(self.results["bad_limit"]["code"], "TEAM_INPUT");
    }
}

impl Drop for Teams {
    fn drop(&mut self) {
        if self.home.exists() {
            fs::remove_dir_all(&self.home).unwrap();
        }
        let inputs = self.home.with_extension("inputs");
        if inputs.exists() {
            fs::remove_dir_all(inputs).unwrap();
        }
    }
}
