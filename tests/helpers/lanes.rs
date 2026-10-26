#![cfg(unix)]

use merl_cli::{CliResponse, run_with_clock};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

pub struct Lane {
    home: PathBuf,
    state: Value,
    messages: Vec<Vec<Value>>,
    status: Vec<Value>,
    results: Vec<Value>,
    requests: Vec<usize>,
    queues: Vec<Value>,
}

impl Lane {
    pub fn given_a_repository_with_existing_review_work(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!("merl-lane-{name}-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lane_github.py");
        fs::copy(source, home.join("gh")).unwrap();
        fs::set_permissions(home.join("gh"), fs::Permissions::from_mode(0o700)).unwrap();
        let state = json!({"page_size":1,"prs":[
            {"number":1,"labels":["review-ready","backend"],"events":["IssueComment","PullRequestReview"],
             "threads":[{"id":"thread-1"},{"id":"thread-2"}]},
            {"number":2,"labels":["review-ready","backend"],"events":["IssueComment"]},
            {"number":3,"labels":["review-ready","frontend"],"events":[]}
        ]});
        let case = Self {
            home,
            state,
            messages: Vec::new(),
            status: Vec::new(),
            results: Vec::new(),
            requests: Vec::new(),
            queues: Vec::new(),
        };
        case.write_state();
        case.success(&["mail", "register", "--agent", "reviewer"], 0);
        case
    }

    fn call(&self, words: &[&str], now: i64) -> Value {
        let mut args = words.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        args.extend([
            "--home".into(),
            self.home.to_str().unwrap().into(),
            "--json".into(),
        ]);
        let text = match run_with_clock(&args, &|| {
            Ok(if self.home.join("response-clock").exists() {
                now + 4000
            } else {
                now
            })
        }) {
            CliResponse::Success(text) => text,
            CliResponse::JsonError(error) => error.as_json(),
            CliResponse::HumanError(error) => panic!("lost JSON: {error}"),
        };
        serde_json::from_str(&text).unwrap()
    }

    fn success(&self, words: &[&str], now: i64) -> Value {
        let value = self.call(words, now);
        assert!(value.get("code").is_none(), "{value}");
        value
    }

    fn start(&self) {
        self.success(
            &[
                "lane",
                "start",
                "--name",
                "backend",
                "--repo",
                "example/widgets",
                "--agent",
                "reviewer",
                "--label",
                "review-ready",
                "--label",
                "backend",
            ],
            0,
        );
    }

    fn write_state(&self) {
        fs::write(self.home.join("provider.json"), self.state.to_string()).unwrap();
    }

    fn scan(&self, now: i64, limit: &str) -> Value {
        self.call(
            &[
                "lane",
                "scan",
                "--gh",
                self.home.join("gh").to_str().unwrap(),
                "--max-requests",
                limit,
            ],
            now,
        )
    }

    fn drain(&mut self, now: i64) {
        for _ in 0..16 {
            let result = self.scan(now, "16");
            assert!(result.get("code").is_none(), "{result}");
            let count = result["requests"].as_u64().unwrap();
            self.results.push(result);
            if count == 0 {
                break;
            }
        }
        self.record(now);
    }

    fn record(&mut self, now: i64) {
        let headers = self.success(&["mail", "list", "--agent", "reviewer"], now);
        let messages = headers["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| {
                self.success(
                    &[
                        "mail",
                        "show",
                        "--agent",
                        "reviewer",
                        "--id",
                        h["id"].as_str().unwrap(),
                    ],
                    now,
                )["message"]
                    .clone()
            })
            .collect();
        self.messages.push(messages);
        self.status
            .push(self.success(&["lane", "status", "--name", "backend"], now));
        self.requests.push(
            fs::read_to_string(self.home.join("requests.jsonl"))
                .unwrap_or_default()
                .lines()
                .count(),
        );
    }

    pub fn when_a_lane_observes_new_heads_comments_and_later_prs_across_restarts(mut self) -> Self {
        self.start();
        self.drain(1000);
        self.drain(1000);
        self.state["prs"][0]["head"] = json!("b".repeat(40));
        self.state["prs"][0]["events"]
            .as_array_mut()
            .unwrap()
            .extend([json!("IssueComment"), json!("PullRequestReview")]);
        self.state["prs"][0]["threads"][0]["count"] = json!(2);
        self.state["prs"][0]["threads"][0]["comment"] = json!("reply-from-the-shared-account");
        self.write_state();
        self.drain(62_000);
        self.state["prs"][0]["state"] = json!("MERGED");
        self.state["prs"].as_array_mut().unwrap().push(
            json!({"number":4,"labels":["review-ready","backend"],"events":["IssueComment"]}),
        );
        self.write_state();
        self.drain(123_000);
        self.drain(123_000);
        self
    }

    pub fn then_current_work_is_notified_without_replaying_old_discussions_or_unchanged_polls(
        self,
    ) {
        let bodies = |index: usize| {
            self.messages[index]
                .iter()
                .map(|m| m["body"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(bodies(0).contains("/pull/1") && bodies(0).contains("/pull/2"));
        assert!(!bodies(0).contains("/pull/3"));
        assert!(
            !bodies(0).contains("IssueComment"),
            "initial history is a baseline"
        );
        assert_eq!(self.messages[0], self.messages[1]);
        assert_eq!(
            self.requests[0], self.requests[1],
            "unchanged not-due polls make no provider calls"
        );
        assert!(bodies(2).contains(&"b".repeat(40)));
        assert!(bodies(2).contains("IssueComment") && bodies(2).contains("review-comments"));
        assert!(bodies(3).contains("/pull/4") && bodies(3).contains("MERGED"));
        assert_eq!(self.messages[3], self.messages[4]);
        assert!(
            self.messages
                .iter()
                .flatten()
                .all(|m| m["kind"] == "review")
        );
    }

    pub fn when_polling_is_paused_resumed_and_its_required_labels_change(mut self) -> Self {
        self.start();
        self.drain(1000);
        self.success(
            &[
                "lane",
                "stop",
                "--name",
                "backend",
                "--expected-generation",
                "1",
            ],
            1001,
        );
        self.drain(62_000);
        self.results.push(self.call(
            &[
                "lane",
                "update",
                "--name",
                "backend",
                "--expected-generation",
                "1",
                "--label",
                "frontend",
            ],
            62_001,
        ));
        self.success(
            &[
                "lane",
                "start",
                "--name",
                "backend",
                "--expected-generation",
                "2",
            ],
            62_002,
        );
        self.success(
            &[
                "lane",
                "update",
                "--name",
                "backend",
                "--expected-generation",
                "3",
                "--label",
                "review-ready",
                "--label",
                "frontend",
            ],
            62_003,
        );
        self.drain(62_004);
        let first = self.messages[0][0]["id"].as_str().unwrap();
        self.success(
            &["mail", "read", "--agent", "reviewer", "--id", first],
            62_005,
        );
        self.success(
            &["mail", "receipt", "--agent", "reviewer", "--id", first],
            62_006,
        );
        self.record(62_007);
        self
    }

    pub fn then_generations_scope_changes_and_published_review_work_remain_explicit(self) {
        assert_eq!(
            self.requests[0], self.requests[1],
            "paused lanes do not poll"
        );
        assert_eq!(
            self.messages[0], self.messages[1],
            "paused lanes do not discard published mail"
        );
        assert!(self.results.iter().any(|v| v["code"] == "LANE_CONFLICT"));
        let latest = self.messages.last().unwrap();
        let text = latest
            .iter()
            .map(|m| m["body"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("/pull/3") && text.contains("left-scope"));
        assert!(
            latest.iter().all(|m| m["completed_at"].is_null()),
            "receipt never completes reviews"
        );
        assert_eq!(self.status.last().unwrap()["lanes"][0]["agent"], "reviewer");
    }

    pub fn when_a_paged_scan_is_interrupted_by_a_long_server_rate_limit(mut self) -> Self {
        self.start();
        self.results.push(self.scan(1000, "1"));
        self.record(1000);
        self.state["error"] = json!("rate");
        self.write_state();
        self.results.push(self.scan(1001, "1"));
        self.record(1001);
        self.results.push(self.scan(62_000, "1"));
        self.record(62_000);
        self.state["error"] = Value::Null;
        self.write_state();
        self.drain(602_000);
        self
    }

    pub fn then_checkpoints_are_retained_and_retry_waits_for_the_server_deadline(self) {
        assert!(
            self.results
                .iter()
                .any(|v| v["code"] == "LANE_RATE_LIMITED")
        );
        assert_eq!(
            self.requests[1], self.requests[2],
            "server retry time exceeds local retry cap"
        );
        assert!(
            self.status[0]["lanes"][0]["last_scan_at"].is_null(),
            "first discovery page is incomplete"
        );
        assert_eq!(
            self.status[0]["lanes"][0]["discovery_after"],
            self.status[1]["lanes"][0]["discovery_after"]
        );
        assert!(self.status.last().unwrap()["lanes"][0]["last_scan_at"].is_i64());
        let latest = self.messages.last().unwrap();
        let text = latest
            .iter()
            .map(|m| m["body"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("/pull/1") && text.contains("/pull/2"));
    }

    pub fn when_a_cursor_is_rejected_and_later_scope_metadata_is_truncated(mut self) -> Self {
        self.start();
        self.drain(1000);
        self.results.push(self.scan(62_000, "1"));
        self.state["error"] = json!("cursor");
        self.write_state();
        self.results.push(self.scan(62_001, "1"));
        self.queues
            .push(self.success(&["lane", "queue", "--name", "backend"], 62_001));
        self.state["error"] = Value::Null;
        self.write_state();
        self.drain(68_000);
        self.queues
            .push(self.success(&["lane", "queue", "--name", "backend"], 68_000));
        self.state["prs"][0]["labels"] = json!(["review-ready"]);
        self.state["prs"][0]["truncated_labels"] = json!(true);
        self.write_state();
        self.results.push(self.scan(129_000, "16"));
        self.queues
            .push(self.success(&["lane", "queue", "--name", "backend"], 129_000));
        self.record(129_000);
        self
    }

    pub fn then_resynchronization_records_a_gap_and_truncation_cannot_retire_the_pr(self) {
        assert!(self.results.iter().any(|v| v["code"] == "LANE_CURSOR_GAP"));
        assert!(self.results.iter().any(|v| v["code"] == "LANE_INCOMPLETE"));
        let pr = |index: usize| {
            self.queues[index]["prs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["number"] == 1)
                .unwrap()
        };
        assert!(pr(0)["initializing"].as_bool().unwrap());
        assert_eq!(pr(1)["gaps"], 1);
        assert_eq!(pr(2)["in_scope"], true);
        assert_eq!(pr(1)["timeline_cursor"], pr(2)["timeline_cursor"]);
        assert_eq!(pr(2)["last_error"], "LANE_INCOMPLETE");
        let text = self
            .messages
            .last()
            .unwrap()
            .iter()
            .map(|m| m["body"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("cursor-gap"));
        assert!(!text.contains("left-scope"));
    }
}

impl Lane {
    pub fn when_an_unchanged_lane_is_polled_after_each_due_deadline(mut self) -> Self {
        self.start();
        for now in [1000, 62_000, 123_000] {
            self.drain(now);
            self.queues
                .push(self.success(&["lane", "queue", "--name", "backend"], now));
        }
        self
    }

    pub fn then_empty_pages_preserve_progress_and_publish_no_new_review_requests(self) {
        for queue in &self.queues {
            let pr = queue["prs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["number"] == 1)
                .unwrap();
            assert_eq!(pr["timeline_cursor"], "t:2");
        }
        assert_eq!(self.messages[0], self.messages[1]);
        assert_eq!(self.messages[1], self.messages[2]);
        assert!(
            self.requests[1] > self.requests[0] && self.requests[2] > self.requests[1],
            "eligible unchanged polls really fetch GitHub metadata"
        );
    }
}

impl Lane {
    pub fn when_a_rate_response_arrives_after_time_passes_during_io(mut self) -> Self {
        self.start();
        self.state["error"] = json!("rate");
        self.state["advance_clock"] = json!(true);
        self.write_state();
        self.results.push(self.scan(1000, "1"));
        self.record(1000);
        self
    }

    pub fn then_network_time_does_not_shorten_the_server_retry_delay(self) {
        assert!(
            self.results
                .iter()
                .any(|v| v["code"] == "LANE_RATE_LIMITED")
        );
        assert!(
            self.status[0]["provider_retry_at"].as_i64().unwrap() >= 605_000,
            "four seconds of I/O must precede the server-requested ten-minute wait"
        );
    }
}

impl Lane {
    pub fn when_github_fails_after_a_successful_scan_and_then_recovers(mut self) -> Self {
        self.start();
        self.drain(1000);
        self.state["error"] = json!("unavailable");
        self.write_state();
        self.results.push(self.scan(62_000, "16"));
        self.record(62_000);
        self.state["error"] = Value::Null;
        self.write_state();
        self.drain(73_000);
        self
    }

    pub fn then_the_outage_is_explicit_and_recovery_keeps_existing_progress_and_mail(self) {
        assert!(self.results.iter().any(|r| r["code"] == "LANE_GITHUB"));
        assert_eq!(
            self.status[1]["lanes"][0]["last_scan_at"],
            self.status[0]["lanes"][0]["last_scan_at"]
        );
        assert_eq!(self.status[1]["lanes"][0]["last_error"], "LANE_GITHUB");
        assert!(
            self.status[2]["lanes"][0]["last_scan_at"].as_i64().unwrap()
                > self.status[1]["lanes"][0]["last_scan_at"].as_i64().unwrap()
        );
        assert_eq!(self.messages[0], self.messages[1]);
        assert_eq!(self.messages[1], self.messages[2]);
    }
}

impl Drop for Lane {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.home).unwrap();
    }
}
