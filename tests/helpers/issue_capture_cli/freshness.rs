//! Provider freshness and accepted facts have separate public effects.

use super::{Capture, Value, fs, json};

// The provider fixture starts at 11:00 UTC; activity advances it to noon.
const ACTIVITY_TIME: i64 = 1_767_268_800_000;

pub struct Freshness {
    case: Capture,
    issue: String,
    before: Value,
    after: Value,
    rebuilt: Value,
    stale: Vec<Value>,
}

impl Freshness {
    pub fn given_a_captured_issue() -> Self {
        let case = Capture::given_a_new_project().when_the_issue_is_captured();
        let issue = case.changes_after["delta"]["batches"][0]["changes"][0]["ref"]
            .as_str()
            .unwrap()
            .to_owned();
        let before = view(&case, &issue);
        Self {
            case,
            issue,
            before,
            after: Value::Null,
            rebuilt: Value::Null,
            stale: vec![],
        }
    }

    pub fn when_only_the_activity_timestamp_advances(mut self) -> Self {
        self.case.pages[0]["data"]["repository"]["issue"]["updatedAt"] =
            json!("2026-01-01T12:00:00Z");
        self.case = self.case.when_the_issue_is_captured();
        self.after = view(&self.case, &self.issue);
        self
    }

    pub fn then_freshness_advances_without_accepted_work(self) -> Self {
        assert_eq!(self.case.latest["provider_changed"], false);
        assert_eq!(
            self.case.latest["revision"],
            self.before["project_revision"]
        );
        assert_eq!(self.case.changes_after, self.case.changes_before);
        assert_eq!(
            self.after["provider"]["revision"],
            self.before["provider"]["revision"]
        );
        assert_eq!(
            self.after["provider"]["upstream_updated_at_millis"],
            self.before["provider"]["upstream_updated_at_millis"]
        );
        assert_eq!(
            self.after["provider"]["freshness"]["upstream_updated_at_millis"],
            ACTIVITY_TIME
        );
        assert!(
            self.after["provider"]["freshness"]["last_seen_at_millis"]
                .as_i64()
                .unwrap()
                > self.before["provider"]["freshness"]["last_seen_at_millis"]
                    .as_i64()
                    .unwrap()
        );
        self
    }

    pub fn when_rebuilt_and_stale_responses_arrive(mut self) -> Self {
        self.case
            .command(&["project", "rebuild", "--project", "P1"]);
        self.rebuilt = view(&self.case, &self.issue);
        // Both an unchanged old snapshot and changed old facts must respect the watermark.
        for changed in [false, true] {
            let issue = &mut self.case.pages[0]["data"]["repository"]["issue"];
            issue["updatedAt"] = json!("2026-01-01T11:30:00Z");
            if changed {
                issue["labels"]["nodes"] = json!([{"id": "label-stale", "name": "stale"}]);
            }
            fs::write(
                self.case.directory.join("pages.json"),
                serde_json::to_vec(&self.case.pages).unwrap(),
            )
            .unwrap();
            self.case.tick += 1;
            let error = match self.case.capture_response(&self.case.last_args, true) {
                merl_cli::CliResponse::JsonError(error) => {
                    serde_json::from_str::<Value>(&error.as_json()).unwrap()
                }
                other => panic!("expected stale provider response, got {other:?}"),
            };
            self.stale.push(json!({"error": error, "view": view(&self.case, &self.issue), "changes": self.case.read_changes()}));
        }
        self
    }

    pub fn then_rebuild_preserves_the_watermark_and_stale_responses_change_nothing(self) -> Self {
        assert_eq!(self.rebuilt, self.after);
        for result in &self.stale {
            assert_eq!(result["error"]["code"], "STALE_PROVIDER_OBSERVATION");
            assert_eq!(result["view"], self.after);
            assert_eq!(result["changes"], self.case.changes_after);
        }
        self
    }

    pub fn when_newer_provider_facts_arrive(mut self) -> Self {
        self.case = self
            .case
            .when_the_provider_closes_the_issue()
            .when_the_issue_is_captured();
        self.after = view(&self.case, &self.issue);
        self.case
            .command(&["project", "rebuild", "--project", "P1"]);
        self.rebuilt = view(&self.case, &self.issue);
        self
    }

    pub fn then_the_provider_change_has_its_own_revision(self) {
        assert_eq!(self.case.latest["provider_changed"], true);
        assert_eq!(self.case.latest["revision"], 2);
        assert_eq!(self.after["provider"]["state"], "closed");
        assert_eq!(
            self.after["provider"]["freshness"]["upstream_updated_at_millis"],
            ACTIVITY_TIME + 7_200_000
        );
        assert_eq!(
            self.case.changes_after["inbox"]["entries"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(self.after, self.rebuilt);
    }
}

pub struct ProviderChanges {
    cases: Vec<(&'static str, Value, Capture)>,
}

impl ProviderChanges {
    pub fn given_each_kind_of_provider_change() -> Self {
        Self { cases: [
            ("state", json!("CLOSED")),
            ("labels", json!({"nodes": [{"id": "label-2", "name": "done"}], "pageInfo": {"hasNextPage": false}})),
            ("assignees", json!({"nodes": [], "pageInfo": {"hasNextPage": false}})),
            ("milestone", json!({"title": "First release"})),
            ("title", json!("Updated title")),
        ].into_iter().map(|(field, value)| (field, value, Capture::given_a_named_project(field))).collect() }
    }

    pub fn when_a_comment_and_the_provider_change_arrive_together(mut self) -> Self {
        self.cases = self
            .cases
            .into_iter()
            .map(|(field, value, case)| {
                let mut case = case.when_the_issue_is_captured().when_a_comment_arrives();
                let issue = &mut case.pages[0]["data"]["repository"]["issue"];
                issue["updatedAt"] = json!("2026-01-01T12:00:00Z");
                issue[field] = value.clone();
                if field == "state" {
                    issue["closedAt"] = issue["updatedAt"].clone();
                }
                (field, value, case.when_the_issue_is_captured())
            })
            .collect();
        self
    }

    pub fn then_each_provider_change_reaches_policy_delta_and_inbox(self) {
        for (field, _, case) in self.cases {
            assert_eq!(case.latest["captured"], 1, "{field}: {}", case.latest);
            assert_eq!(case.latest["compiled"], 1, "{field}");
            assert_eq!(case.latest["provider_changed"], true, "{field}");
            assert_eq!(case.latest["revision"], 2, "{field}");
            let batches = case.changes_after["delta"]["batches"].as_array().unwrap();
            let entries = case.changes_after["inbox"]["entries"].as_array().unwrap();
            assert_eq!(batches.len(), 2, "{field}");
            assert_eq!(entries.len(), 2, "{field}");
            assert_eq!(entries[1]["changes"], batches[1]["changes"], "{field}");
            assert_eq!(
                batches[1]["changes"][0]["ref"], batches[0]["changes"][0]["ref"],
                "{field}"
            );
        }
    }
}

fn view(case: &Capture, issue: &str) -> Value {
    case.command(&[
        "issue",
        "view",
        "--project",
        "P1",
        "--issue",
        issue,
        "--scope",
        "issue-1",
    ])
}
