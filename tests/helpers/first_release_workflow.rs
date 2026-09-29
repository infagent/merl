//! One release workflow through the shipped binary. Only provider and compiler
//! processes are faked; setup, recovery, and inspection use public CLI commands.

use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

/// Owns one isolated CLI session and the JSON results inspected by `then_` steps.
pub struct Workflow {
    directory: PathBuf,
    pages: Value,
    recorded: BTreeMap<&'static str, Value>,
}

impl Workflow {
    pub fn given_an_issue_and_no_project_database() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "merl-release-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap()
        ));
        fs::create_dir(&directory).expect("isolated workflow directory");
        let mut pages: Value =
            serde_json::from_str(include_str!("../fixtures/github_two_page_edit.json")).unwrap();
        pages.as_array_mut().unwrap().truncate(1);
        let issue = &mut pages[0]["data"]["repository"]["issue"];
        issue["body"] = json!("Choose a gain policy.");
        issue["lastEditedAt"] = Value::Null;
        issue["includesCreatedEdit"] = json!(false);
        issue["userContentEdits"]["nodes"] = json!([]);
        issue["comments"]["pageInfo"] = json!({"hasNextPage": false, "endCursor": null});
        issue["comments"]["nodes"][0]["body"] = json!("Keep gain fixed.");
        let case = Self {
            directory,
            pages,
            recorded: BTreeMap::new(),
        };
        assert!(!case.directory.join("project.sqlite").exists());
        case.script(
            "github",
            include_str!("../fixtures/first_release_github.py"),
        );
        case.script(
            "compiler",
            include_str!("../fixtures/first_release_compiler.py"),
        );
        case
    }

    pub fn when_the_project_and_authorities_are_created(mut self) -> Self {
        self.record(
            "init",
            &["project", "init", "--id", "P1", "--administrator", "owner"],
        );
        self.record(
            "reviewer_grant",
            &[
                "project",
                "authority",
                "grant",
                "--actor",
                "owner",
                "--subject",
                "reviewer",
                "--permission",
                "command_actor",
                "--id",
                "grant-reviewer",
                "--reason",
                "Issue workflow responsibility",
            ],
        );
        self.record("subscription", &["inbox", "subscribe", "--agent", "reader"]);
        self
    }

    pub fn when_the_issue_is_captured_and_its_assertion_is_applied(mut self) -> Self {
        self.capture("initial_capture");
        let version = self.source("initial_capture", "issue-1", "version");
        let source = self.cli(&["source", "show", "--version", &version]);
        self.record(
            "author_grant",
            &[
                "project",
                "authority",
                "grant",
                "--actor",
                "owner",
                "--subject",
                source["source_author"].as_str().unwrap(),
                "--permission",
                "decision_author",
                "--id",
                "grant-author",
                "--reason",
                "Issue author may direct decisions",
            ],
        );
        let run = self.source("initial_capture", "comment-1", "run");
        self.record(
            "initial_assertions",
            &["source", "assertions", "--run", &run],
        );
        self.record(
            "initial_apply",
            &[
                "source",
                "apply",
                "--run",
                &run,
                "--actor",
                "reviewer",
                "--id",
                "apply-initial",
            ],
        );
        self.record("initial_candidates", &["candidate", "list"]);
        self.record("before_review", &["project", "view"]);
        self
    }

    pub fn then_the_interpretation_waits_for_review(self) -> Self {
        assert_eq!(self.get("reviewer_grant")["outcome"], "accepted");
        assert_eq!(self.get("author_grant")["outcome"], "accepted");
        let capture = self.get("initial_capture");
        assert_eq!(capture["captured"], 2);
        assert_eq!(capture["compiled"], 2, "{capture}");
        assert_eq!(
            self.get("initial_apply")["inputs"][0]["outcome"],
            "candidate"
        );
        assert!(self.get("initial_apply")["revision"].is_null());
        assert!(
            self.get("before_review")["objects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["id"] != "D1")
        );
        assert_eq!(
            self.get("initial_candidates")["candidates"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        self
    }

    pub fn when_the_candidate_is_accepted(mut self) -> Self {
        let candidate = self.get("initial_candidates")["candidates"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.record("candidate", &["candidate", "show", &candidate]);
        self.record(
            "review",
            &[
                "candidate",
                "accept",
                &candidate,
                "--actor",
                "reviewer",
                "--id",
                "review-initial",
            ],
        );
        self.record("decision", &["show", "D1", "--source", "--history"]);
        self
    }

    pub fn then_the_decision_has_exact_source_provenance(self) -> Self {
        assert_eq!(self.get("review")["outcome"], "accepted");
        assert_eq!(self.get("candidate")["status"], "pending");
        let object = self.get("decision");
        assert_eq!(object["support"], "current");
        assert_eq!(object["history"].as_array().unwrap().len(), 1);
        assert_eq!(
            object["policy_origin"]["evaluation"],
            self.get("review")["evaluation"]
        );
        assert!(
            object["policy_origin"]["input"]["id"].is_string(),
            "{object}"
        );
        assert!(
            object["policy_origin"]["evaluation"].is_string(),
            "{object}"
        );
        let assertion = &object["evidence_history"][0]["assertion"];
        assert_eq!(
            assertion["run"],
            self.source("initial_capture", "comment-1", "run")
        );
        assert_eq!(
            assertion["source_version"],
            self.source("initial_capture", "comment-1", "version")
        );
        assert_eq!(assertion["span"], json!({"start": 0, "end": 16}));
        assert_eq!(assertion["evidence"]["text"], "Keep gain fixed.");
        self
    }

    pub fn when_the_reader_catches_up(mut self) -> Self {
        let view = self.issue_view();
        self.ack(view["project_revision"].as_u64().unwrap());
        self.recorded.insert("caught_up", view);
        self.record("empty_inbox", &["inbox", "poll", "--agent", "reader"]);
        self
    }

    pub fn when_a_later_comment_is_captured_and_accepted(mut self) -> Self {
        let mut comment =
            self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"][0].clone();
        comment["id"] = json!("comment-2");
        comment["author"] = json!({"id": "user-1", "login": "author"});
        comment["body"] = json!("Use double precision.");
        comment["createdAt"] = json!("2026-01-01T12:00:00Z");
        comment["updatedAt"] = comment["createdAt"].clone();
        comment["userContentEdits"]["nodes"] = json!([]);
        comment["includesCreatedEdit"] = json!(false);
        // GitHub also advances the enclosing Issue timestamp when a comment arrives.
        self.pages[0]["data"]["repository"]["issue"]["updatedAt"] = comment["createdAt"].clone();
        self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(comment);
        self.capture("later_capture");
        let view = self.issue_view();
        self.recorded.insert("before_later_apply", view);
        self.record(
            "before_later_inbox",
            &["inbox", "poll", "--agent", "reader"],
        );
        let run = self.source("later_capture", "comment-2", "run");
        self.record(
            "later_apply",
            &[
                "source",
                "apply",
                "--run",
                &run,
                "--actor",
                "reviewer",
                "--id",
                "apply-later",
            ],
        );
        assert_eq!(
            self.get("later_apply")["inputs"][0]["outcome"],
            "accepted",
            "{}",
            self.get("later_apply")
        );
        self.record("later_decision", &["show", "D2", "--source", "--history"]);
        self.record("later_inbox", &["inbox", "poll", "--agent", "reader"]);
        let since = self.get("caught_up")["project_revision"].to_string();
        self.record("later_delta", &["project", "delta", "--since", &since]);
        self.capture("repeated_capture");
        self.record(
            "repeated_apply",
            &[
                "source",
                "apply",
                "--run",
                &run,
                "--actor",
                "reviewer",
                "--id",
                "apply-later",
            ],
        );
        self.record("repeated_inbox", &["inbox", "poll", "--agent", "reader"]);
        self
    }

    pub fn then_the_reader_receives_one_new_revision(self) -> Self {
        assert_caught_up(self.get("caught_up"));
        assert_eq!(self.get("empty_inbox")["entries"], json!([]));
        assert_eq!(self.get("later_capture")["captured"], 1);
        assert_eq!(self.get("later_capture")["compiled"], 1);
        assert_eq!(self.get("later_capture")["provider_changed"], false);
        assert_eq!(
            self.get("before_later_apply")["project_revision"],
            self.get("caught_up")["project_revision"]
        );
        assert_eq!(self.get("before_later_inbox")["entries"], json!([]));
        assert_eq!(
            self.get("before_later_apply")["provider"]["upstream_updated_at_millis"],
            self.get("caught_up")["provider"]["upstream_updated_at_millis"]
        );
        assert_eq!(
            self.get("before_later_apply")["provider"]["freshness"]["upstream_updated_at_millis"],
            1_767_268_800_000_i64
        );
        let expected = self.get("caught_up")["project_revision"].as_u64().unwrap() + 1;
        assert_eq!(self.get("later_apply")["revision"], expected);
        assert_eq!(self.get("later_apply")["inputs"][0]["outcome"], "accepted");
        let entries = self.get("later_inbox")["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["revision"], expected);
        assert_eq!(entries[0]["changes"].as_array().unwrap().len(), 1);
        assert_eq!(entries[0]["changes"][0]["ref"], "D2");
        assert_eq!(entries[0]["changes_truncated"], false);
        assert_eq!(
            self.get("later_delta")["batches"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            self.get("later_delta")["batches"][0]["changes"],
            entries[0]["changes"]
        );
        assert_eq!(self.get("later_delta")["head_revision"], expected);
        assert_eq!(self.get("repeated_capture")["outcome"], "unchanged");
        assert_eq!(self.get("repeated_apply"), self.get("later_apply"));
        assert_eq!(self.get("repeated_inbox"), self.get("later_inbox"));
        self
    }

    pub fn when_the_comment_is_edited_and_its_support_is_revalidated(mut self) -> Self {
        let comment = &mut self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"][1];
        comment["body"] = json!("Use double precision. Confirmed by measurements.");
        comment["updatedAt"] = json!("2026-01-01T13:00:00Z");
        comment["lastEditedAt"] = comment["updatedAt"].clone();
        self.capture("edited_capture");
        self.record("stale_decision", &["show", "D2", "--source", "--history"]);
        self.record("pending_work", &["project", "revalidation", "list"]);
        let impact = self.get("pending_work")["work"][0]["impact"]
            .as_str()
            .unwrap()
            .to_owned();
        let run = self.compile(&[
            "project",
            "revalidation",
            "run",
            "--id",
            &impact,
            "--run",
            "revalidate-D2",
            "--actor",
            "reviewer",
        ]);
        self.recorded.insert("revalidation_run", run);
        self.record(
            "revalidation_review",
            &[
                "project",
                "revalidation",
                "resolve",
                "--id",
                "confirm-D2",
                "--impact",
                &impact,
                "--actor",
                "reviewer",
                "--action",
                "confirm",
                "--run",
                "revalidate-D2",
                "--assertion-index",
                "0",
            ],
        );
        self.record(
            "confirmed_decision",
            &["show", "D2", "--source", "--history"],
        );
        self.record("resolved_work", &["project", "revalidation", "list"]);
        self
    }

    pub fn then_review_restores_support_without_replacing_the_decision(self) -> Self {
        assert_eq!(
            self.get("pending_work")["work"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            self.get("stale_decision")["support"],
            "revalidation_pending"
        );
        assert_eq!(self.get("revalidation_run")["mode"], "hindsight");
        assert_eq!(self.get("revalidation_run")["outcome"], "succeeded");
        assert_eq!(self.get("revalidation_review")["disposition"], "accepted");
        assert_eq!(self.get("confirmed_decision")["support"], "current");
        assert_eq!(
            self.get("confirmed_decision")["revision"],
            self.get("later_decision")["revision"]
        );
        assert_eq!(
            self.get("confirmed_decision")["history"],
            self.get("later_decision")["history"]
        );
        assert_eq!(self.get("resolved_work")["work"], json!([]));
        assert_eq!(
            self.get("pending_work")["work"][0]["replacement"],
            self.source("edited_capture", "comment-2", "version")
        );
        assert_eq!(
            self.get("confirmed_decision")["revalidation_history"][0]["run"],
            "revalidate-D2"
        );
        self
    }

    pub fn when_history_is_replayed_and_projections_are_rebuilt(mut self) -> Self {
        self.recorded.insert("before_recovery", self.issue_view());
        self.record(
            "before_recovery_delta",
            &["project", "delta", "--since", "0"],
        );
        let run = self.source("initial_capture", "comment-1", "run");
        let replay = self.compile(&[
            "source",
            "replay",
            "--run",
            &run,
            "--new-run",
            "replay-initial",
        ]);
        self.recorded.insert("replay", replay);
        self.record(
            "replayed_run",
            &["compilation", "show", "--run", "replay-initial"],
        );
        self.recorded.insert(
            "compiler_calls_before_rebuild",
            json!(self.compiler_calls()),
        );
        self.record("rebuild", &["project", "rebuild"]);
        self.recorded
            .insert("compiler_calls_after_rebuild", json!(self.compiler_calls()));
        self.recorded.insert("after_recovery", self.issue_view());
        self.record(
            "after_recovery_delta",
            &["project", "delta", "--since", "0"],
        );
        self.record(
            "after_recovery_decision",
            &["show", "D2", "--source", "--history"],
        );
        self
    }

    pub fn then_accepted_history_and_views_are_unchanged(self) -> Self {
        assert_eq!(self.get("replay")["input_matches"], true);
        assert_eq!(self.get("replayed_run")["mode"], "replay");
        assert_eq!(self.get("replayed_run")["outcome"], "succeeded");
        assert_eq!(
            self.get("compiler_calls_before_rebuild"),
            self.get("compiler_calls_after_rebuild")
        );
        assert_eq!(self.get("before_recovery"), self.get("after_recovery"));
        assert_eq!(
            self.get("before_recovery_delta"),
            self.get("after_recovery_delta")
        );
        assert_eq!(
            self.get("confirmed_decision"),
            self.get("after_recovery_decision")
        );
        self
    }

    pub fn when_a_structured_decision_includes_a_note(mut self) -> Self {
        self.recorded.insert(
            "compiler_calls_before_command",
            json!(self.compiler_calls()),
        );
        self.record(
            "direct_command",
            &[
                "decision",
                "create",
                "--subject",
                "D3",
                "--summary",
                "Retain calibration logs",
                "--note",
                "Retain calibration logs. Measurements explain drift.",
                "--issue",
                "issue-1",
                "--actor",
                "reviewer",
                "--id",
                "retain-logs",
            ],
        );
        self.record("direct_decision", &["show", "D3", "--source", "--history"]);
        let source = self.get("direct_command")["source"]
            .as_str()
            .unwrap()
            .to_owned();
        self.record("note_source", &["source", "show", "--version", &source]);
        self.recorded
            .insert("compiler_calls_after_command", json!(self.compiler_calls()));
        self
    }

    pub fn then_the_command_is_accepted_without_a_compiler(self) -> Self {
        assert_eq!(self.get("direct_command")["outcome"], "accepted");
        assert_eq!(
            self.get("compiler_calls_before_command"),
            self.get("compiler_calls_after_command")
        );
        assert_eq!(
            self.get("direct_decision")["policy_origin"]["input"]["kind"],
            "command"
        );
        assert_eq!(self.get("note_source")["semantic_origin"], "retain-logs");
        assert_eq!(self.get("note_source")["supplements"], "D3");
        self
    }

    pub fn when_the_note_is_compiled_and_its_added_finding_is_reviewed(mut self) -> Self {
        let source = self.get("direct_command")["source"]
            .as_str()
            .unwrap()
            .to_owned();
        let compiled = self.compile(&[
            "source",
            "compile",
            "--version",
            &source,
            "--run",
            "note-run",
            "--actor",
            "owner",
            "--reason",
            "Extract measurement evidence",
        ]);
        self.recorded.insert("note_compile", compiled);
        assert_eq!(
            self.get("note_compile")["outcome"],
            "compiled",
            "{}",
            self.get("note_compile")
        );
        self.record(
            "note_apply",
            &[
                "source",
                "apply",
                "--run",
                "note-run",
                "--actor",
                "reviewer",
                "--id",
                "apply-note",
            ],
        );
        let candidates = self.cli(&["candidate", "list"]);
        let finding = candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["run"] == "note-run" && c["index"] == 1)
            .expect("added finding candidate");
        self.record(
            "finding_review",
            &[
                "candidate",
                "accept",
                finding["id"].as_str().unwrap(),
                "--actor",
                "reviewer",
                "--id",
                "accept-finding",
            ],
        );
        self.record("finding", &["show", "F1", "--source", "--history"]);
        self.record(
            "decision_after_note",
            &["show", "D3", "--source", "--history"],
        );
        self
    }

    pub fn then_the_note_adds_evidence_without_recreating_the_decision(self) -> Self {
        let inputs = &self.get("note_apply")["inputs"];
        assert_eq!(inputs[0]["outcome"], "duplicate");
        assert_eq!(inputs[0]["reason"], "covered_by_command");
        assert_eq!(inputs[1]["outcome"], "candidate");
        assert_eq!(self.get("finding_review")["outcome"], "accepted");
        assert_eq!(
            self.get("finding")["evidence_history"][0]["assertion"]["evidence"]["text"],
            "Measurements explain drift."
        );
        assert_eq!(
            self.get("decision_after_note")["history"],
            self.get("direct_decision")["history"]
        );
        self
    }

    pub fn when_the_note_is_purged_and_the_reader_catches_up(mut self) -> Self {
        let source = self.get("direct_command")["source"]
            .as_str()
            .unwrap()
            .to_owned();
        self.record(
            "purge_preview",
            &[
                "source",
                "purge",
                "--version",
                &source,
                "--reason",
                "Remove private measurement details",
                "--dry-run",
            ],
        );
        let digest = self.get("purge_preview")["confirm_digest"]
            .as_str()
            .unwrap()
            .to_owned();
        self.record(
            "purge",
            &[
                "source",
                "purge",
                "--version",
                &source,
                "--actor",
                "owner",
                "--reason",
                "Remove private measurement details",
                "--confirm-digest",
                &digest,
            ],
        );
        self.record(
            "purge_audit",
            &["source", "purge-audit", "--version", &source],
        );
        self.record("erased_note", &["source", "show", "--version", &source]);
        self.record("erased_finding", &["show", "F1", "--source", "--history"]);
        let replay = self.invoke(&["source", "replay", "--run", "note-run"], false);
        self.recorded.insert("erased_replay", replay);
        self.recorded.insert("final_view", self.issue_view());
        let revision = self.get("final_view")["project_revision"].as_u64().unwrap();
        self.ack(revision);
        self.record("final_inbox", &["inbox", "poll", "--agent", "reader"]);
        self
    }

    pub fn then_protected_text_is_unavailable_with_an_audit_receipt(self) -> Self {
        assert_eq!(self.get("purge")["completed"], true);
        for field in [
            "actor",
            "source",
            "completed",
            "requested_at_millis",
            "covered_scopes",
            "outside_scope",
        ] {
            assert_eq!(self.get("purge_audit")[field], self.get("purge")[field]);
        }
        assert_eq!(self.get("purge_audit")["actor"], "owner");
        assert_eq!(
            self.get("purge_audit")["preview_digest"],
            self.get("purge_preview")["confirm_digest"]
        );
        assert_eq!(
            self.get("purge_audit")["payloads"],
            self.get("purge_preview")["payloads"]
        );
        assert_eq!(
            self.get("purge_audit")["reason"]["text"],
            "Remove private measurement details"
        );
        assert_eq!(
            self.get("purge_audit")["covered_scopes"],
            json!(["active_store"])
        );
        assert_eq!(self.get("erased_note")["body"]["status"], "unavailable");
        assert_eq!(
            self.get("erased_finding")["evidence_history"][0]["assertion"]["evidence"]["status"],
            "unavailable"
        );
        assert_eq!(self.get("erased_finding")["support"], "unsupported");
        assert_eq!(self.get("erased_replay")["code"], "MISSING_EVIDENCE");
        self
    }

    pub fn then_the_issue_and_reader_are_caught_up(self) {
        assert_caught_up(self.get("final_view"));
        assert_eq!(self.get("final_view")["provider"]["state"], "open");
        let ids: Vec<_> = self.get("final_view")["objects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["D1", "D2", "D3", "F1"]);
        assert_eq!(self.get("final_inbox")["entries"], json!([]));
        assert_eq!(
            self.get("final_inbox")["cursor"],
            self.get("final_view")["project_revision"]
        );
    }

    fn capture(&mut self, name: &'static str) {
        fs::write(
            self.directory.join("pages.json"),
            serde_json::to_vec(&self.pages).unwrap(),
        )
        .unwrap();
        let provider = self.directory.join("github");
        let result = self.compile(&[
            "issue",
            "capture",
            "--repository",
            "example/project",
            "--issue",
            "17",
            "--github-program",
            provider.to_str().unwrap(),
        ]);
        assert!(
            result["outcome"] == "captured" || result["outcome"] == "unchanged",
            "{result}"
        );
        self.recorded.insert(name, result);
    }

    fn compile(&self, args: &[&str]) -> Value {
        let program = self.directory.join("compiler");
        let digest = format!("sha256:{}", "0".repeat(64));
        let mut args = args.to_vec();
        args.extend([
            "--program",
            program.to_str().unwrap(),
            "--compiler-version",
            "workflow-v1",
            "--model",
            "deterministic",
            "--prompt-digest",
            &digest,
        ]);
        self.cli(&args)
    }

    fn source(&self, capture: &str, entity: &str, field: &str) -> String {
        self.get(capture)["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["entity"] == entity)
            .unwrap()[field]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn issue_view(&self) -> Value {
        // Live capture reports its accepted revision. Discover the Issue reference
        // from that public batch rather than reproducing Merl's ID hash in the test.
        let delta = self.cli(&["project", "delta", "--since", "0"]);
        let issue = delta["batches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["revision"] == self.get("initial_capture")["revision"])
            .unwrap_or_else(|| panic!("captured Issue must be discoverable: {delta}"))["changes"]
            [0]["ref"]
            .as_str()
            .unwrap();
        self.cli(&[
            "issue", "view", "--issue", issue, "--scope", "issue-1", "--role", "engineer",
        ])
    }

    fn ack(&self, revision: u64) {
        let inbox = self.cli(&["inbox", "poll", "--agent", "reader"]);
        assert_eq!(inbox["has_more"], false, "workflow fits one inbox page");
        for entry in inbox["entries"].as_array().unwrap() {
            assert!(entry["revision"].as_u64().unwrap() <= revision);
            self.cli(&[
                "inbox",
                "ack",
                "--agent",
                "reader",
                "--revision",
                &entry["revision"].to_string(),
            ]);
        }
    }

    fn record(&mut self, name: &'static str, args: &[&str]) {
        self.recorded.insert(name, self.cli(args));
    }

    fn get(&self, name: &str) -> &Value {
        self.recorded
            .get(name)
            .unwrap_or_else(|| panic!("missing workflow result: {name}"))
    }

    fn cli(&self, args: &[&str]) -> Value {
        self.invoke(args, true)
    }

    fn invoke(&self, args: &[&str], success: bool) -> Value {
        // Each action starts a fresh process. No user credentials or configuration
        // reach the fakes, and ordinary CLI defaults cannot read the user's home.
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(args)
            .args(["--project", "P1", "--database"])
            .arg(self.directory.join("project.sqlite"))
            .arg("--json")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.directory)
            .current_dir(&self.directory)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).expect("public JSON result");
        assert!(
            result["schema"]
                .as_str()
                .is_some_and(|s| s.starts_with("merl.") && s.ends_with("/v1")),
            "{result}"
        );
        result
    }

    fn compiler_calls(&self) -> usize {
        fs::read_to_string(self.directory.join("compiler-calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn script(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.directory.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn assert_caught_up(view: &Value) {
    assert_eq!(view["coverage"]["required_gaps"], 0, "{view}");
    assert_eq!(
        view["coverage"]["processed_through"], view["coverage"]["observation_head"],
        "{view}"
    );
}

impl Drop for Workflow {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
