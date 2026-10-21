use std::process::Output;

use serde_json::{Value, json};

use super::{CliIssueHistory, merl, path};

// The last fixture below was captured at 2026-09-19 02:00 UTC.
const LATEST_CAPTURE_MILLIS: i64 = 1_789_783_200_000;

pub struct FixtureRetry {
    history: CliIssueHistory,
    issue: String,
    before: Value,
    retries: Vec<(Output, Value)>,
    stale: Option<(Output, Value)>,
    rebuilt: Option<Value>,
}

impl FixtureRetry {
    pub fn given_an_issue_seen_at_three_capture_times() -> Self {
        Self::with_upstream_updated_at(None)
    }

    pub fn given_an_issue_with_legacy_sightings() -> Self {
        let case = Self::with_upstream_updated_at(Some("2026-09-18T23:00:00Z"));
        // Recreate schema 30's sightings from public imports. Reopening through
        // the CLI must migrate them without inventing stored upstream times.
        rusqlite::Connection::open(case.history.database())
            .expect("legacy database")
            .execute_batch(
                "DROP VIEW provider_issue_freshness;
                 DROP INDEX provider_sighting_watermarks;
                 ALTER TABLE provider_issue_sightings DROP COLUMN upstream_updated_at_millis;
                 PRAGMA user_version=30;",
            )
            .expect("restore schema 30");
        case
    }

    fn with_upstream_updated_at(updated_at: Option<&str>) -> Self {
        let mut history = CliIssueHistory::new();
        history.given_a_new_project();
        let mut fixture: Value =
            serde_json::from_str(include_str!("../../../corpus/development/DEV-C3.json"))
                .expect("fixture");
        if let Some(updated_at) = updated_at {
            fixture["provider_snapshot"]["updated_at"] = json!(updated_at);
            let parsed: merl_corpus::fixture::Fixture =
                serde_json::from_value(fixture.clone()).expect("versioned fixture");
            fixture["capture"]["source_sha256"] = json!(merl_corpus::fixture::source_digest(
                &parsed.provider_snapshot,
                &parsed.observations,
            ));
        }
        let mut issue = String::new();
        for (index, captured_at) in [
            "2026-09-19T00:00:00Z",
            "2026-09-19T01:00:00Z",
            "2026-09-19T02:00:00Z",
        ]
        .into_iter()
        .enumerate()
        {
            fixture["capture"]["captured_at"] = json!(captured_at);
            let file = history.directory.path.join(format!("fixture-{index}.json"));
            std::fs::write(&file, serde_json::to_vec(&fixture).expect("fixture JSON"))
                .expect("write fixture");
            let output = import(&history, index);
            assert!(output.status.success(), "{output:?}");
            let result: Value = serde_json::from_slice(&output.stdout).expect("import result");
            result["issue"]
                .as_str()
                .expect("issue ID")
                .clone_into(&mut issue);
        }
        let before = view(&history, &issue);
        Self {
            history,
            issue,
            before,
            retries: vec![],
            stale: None,
            rebuilt: None,
        }
    }

    pub fn when_the_earlier_fixtures_are_retried(mut self) -> Self {
        for index in 0..2 {
            let output = import(&self.history, index);
            self.retries
                .push((output, view(&self.history, &self.issue)));
        }
        self
    }

    pub fn then_the_retries_are_unchanged_and_keep_the_latest_freshness(self) -> Self {
        assert_eq!(
            self.before["provider"]["freshness"]["last_seen_at_millis"],
            LATEST_CAPTURE_MILLIS
        );
        for (output, after) in &self.retries {
            assert!(output.status.success(), "{output:?}");
            let result: Value = serde_json::from_slice(&output.stdout).expect("retry result");
            assert_eq!(result["outcome"], "unchanged");
            assert_eq!(result["captured"], 0);
            assert_eq!(result["observation_head"], 4);
            assert_eq!(result["revision"], 1);
            assert_eq!(after, &self.before);
        }
        self
    }

    pub fn when_the_project_is_rebuilt(mut self) -> Self {
        let output = merl(&[
            "project",
            "rebuild",
            "--project",
            "P1",
            "--database",
            path(&self.history.database()),
            "--json",
        ]);
        assert!(output.status.success(), "{output:?}");
        self.rebuilt = Some(view(&self.history, &self.issue));
        self
    }

    pub fn when_an_unseen_older_capture_is_imported(mut self) -> Self {
        let file = self.history.directory.path.join("fixture-0.json");
        let mut fixture: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        fixture["capture"]["captured_at"] = json!("2026-09-19T00:30:00Z");
        std::fs::write(
            self.history.directory.path.join("fixture-3.json"),
            serde_json::to_vec(&fixture).unwrap(),
        )
        .unwrap();
        self.stale = Some((import(&self.history, 3), view(&self.history, &self.issue)));
        self
    }

    pub fn then_the_stale_capture_is_rejected_without_changing_freshness(self) -> Self {
        let (output, after) = self.stale.as_ref().expect("stale capture");
        assert!(!output.status.success(), "{output:?}");
        let error: Value = serde_json::from_slice(&output.stdout).expect("stale error");
        assert_eq!(error["code"], "STALE_PROVIDER_OBSERVATION");
        assert_eq!(after, &self.before);
        self
    }

    pub fn then_the_latest_freshness_is_preserved(self) -> Self {
        assert_eq!(self.rebuilt.as_ref().expect("rebuilt view"), &self.before);
        self
    }
}

fn import(history: &CliIssueHistory, index: usize) -> Output {
    merl(&[
        "issue",
        "import-fixture",
        "--project",
        "P1",
        "--database",
        path(&history.database()),
        "--fixture",
        path(&history.directory.path.join(format!("fixture-{index}.json"))),
        "--json",
    ])
}

fn view(history: &CliIssueHistory, issue: &str) -> Value {
    let output = merl(&[
        "issue",
        "view",
        "--project",
        "P1",
        "--database",
        path(&history.database()),
        "--issue",
        issue,
        "--scope",
        "controlled:DEV-C3:issue",
        "--role",
        "researcher",
        "--json",
    ]);
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).expect("issue view")
}
