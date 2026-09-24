use super::{AssertionScenario, NOW, RunMode, id};
use merl_core::{RelationId, SourceVersionId};
use serde_json::{Value, json};

pub struct RelationHealthScenario {
    cases: Vec<Case>,
}
struct Case {
    scenario: AssertionScenario,
    change: &'static str,
    original: RelationId,
    stale: Value,
    work: Value,
    neighbors: Vec<String>,
    final_state: Value,
    final_work: Value,
    final_edges: usize,
}
impl RelationHealthScenario {
    pub fn given_accepted_relations() -> Self {
        let cases = [
            "trigger",
            "context",
            "deletion",
            "purge",
            "response",
            "upgrade-edit",
            "upgrade-purge",
        ]
        .into_iter()
        .map(|change| {
            let mut scenario = AssertionScenario::given_a_decision_and_a_grounded_relation()
                .when_the_run_is_inspected_and_applied();
            compile_edge(&mut scenario, "accepted-edge", 2);
            apply_and_review(&scenario, "accepted-edge");
            let original = scenario
                .store()
                .issue_state(&id("P1"), &id("D1"), "issue-1")
                .unwrap()
                .relations[0]
                .relation
                .id
                .clone();
            Case {
                scenario,
                change,
                original,
                stale: Value::Null,
                work: Value::Null,
                neighbors: Vec::new(),
                final_state: Value::Null,
                final_work: Value::Null,
                final_edges: 0,
            }
        })
        .collect();
        Self { cases }
    }
    pub fn when_evidence_changes_and_the_project_restarts(mut self) -> Self {
        for case in &mut self.cases {
            let s = &case.scenario;
            let mut store = s.store();
            match case.change {
                "trigger" | "context" | "upgrade-edit" => {
                    let source = if case.change == "context" {
                        "source-compound"
                    } else {
                        "source-accepted-edge"
                    };
                    AssertionScenario::capture(
                        &mut store,
                        source,
                        "edited-source",
                        Some(source),
                        Some("alice"),
                    );
                }
                "deletion" => AssertionScenario::capture_body(
                    &mut store,
                    "source-accepted-edge",
                    "deleted-source",
                    Some("source-accepted-edge"),
                    Some("alice"),
                    None,
                ),
                "purge" | "upgrade-purge" => {
                    let source: SourceVersionId = id("source-accepted-edge");
                    let preview = store.preview_source_purge(&id("P1"), &source).unwrap();
                    store
                        .purge_source(
                            &id("P1"),
                            &source,
                            &id("admin"),
                            b"Remove captured text",
                            NOW + 10,
                            preview.confirm_digest,
                        )
                        .unwrap();
                }
                "response" => store
                    .erase_payload(&id("P1"), &id("rsp_accepted-edge"))
                    .unwrap(),
                _ => unreachable!(),
            }
            drop(store);
            if case.change.starts_with("upgrade-") {
                remove_relation_health_schema(&s.database);
            }
            s.cli(&["project", "rebuild"]);
            case.stale = s.cli(&["show", case.original.as_str(), "--source"]);
            case.work = s.cli(&["project", "revalidation", "list"]);
            case.neighbors = s
                .store()
                .focus_neighbors(&id("P1"), &id("D1"), 100)
                .unwrap()
                .0
                .iter()
                .map(|o| o.as_str().to_owned())
                .collect();
        }
        self
    }
    pub fn then_stale_edges_are_qualified_and_pending_work_is_durable(self) -> Self {
        for case in &self.cases {
            assert!(
                case.neighbors.is_empty(),
                "{}: stale edge still selects context",
                case.change
            );
            assert_eq!(
                case.stale["support"],
                if matches!(case.change, "purge" | "response" | "upgrade-purge") {
                    "unsupported"
                } else {
                    "revalidation_pending"
                },
                "{}: {}",
                case.change,
                case.stale
            );
            let work = case.work["work"].as_array().expect("work page");
            assert!(
                work.iter().any(|w| w["relation"] == case.original.as_str()),
                "{}: missing relation work: {}",
                case.change,
                case.work
            );
        }
        self
    }
    pub fn when_the_work_is_reviewed(mut self) -> Self {
        for case in &mut self.cases {
            let s = &mut case.scenario;
            if matches!(case.change, "trigger" | "context") {
                compile_edge(s, "replacement-edge", 1);
                apply_and_review(s, "replacement-edge");
            } else {
                let impact = case.work["work"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|w| w["relation"] == case.original.as_str())
                    .unwrap()["impact"]
                    .as_str()
                    .unwrap();
                let result = s.cli(&[
                    "project",
                    "revalidation",
                    "resolve",
                    "--id",
                    "withdraw-edge",
                    "--impact",
                    impact,
                    "--actor",
                    "agent",
                    "--action",
                    "withdraw",
                ]);
                assert_eq!(result["disposition"], "accepted", "{result}");
                let retry = s.cli(&[
                    "project",
                    "revalidation",
                    "resolve",
                    "--id",
                    "withdraw-edge",
                    "--impact",
                    impact,
                    "--actor",
                    "agent",
                    "--action",
                    "withdraw",
                ]);
                assert_eq!(retry, result);
            }
            s.cli(&["project", "rebuild"]);
            case.final_state = s.cli(&["show", case.original.as_str(), "--source"]);
            case.final_work = s.cli(&["project", "revalidation", "list"]);
            case.final_edges = s
                .store()
                .issue_state(&id("P1"), &id("D1"), "issue-1")
                .unwrap()
                .relations
                .len();
        }
        self
    }
    pub fn then_review_restores_or_withdraws_support_without_rewriting_history(self) {
        for case in self.cases {
            assert_eq!(
                case.final_state["support"], "unsupported",
                "{}",
                case.final_state
            );
            assert!(
                case.final_state["support_resolution"]["evaluation"].is_string(),
                "{}",
                case.final_state
            );
            assert_eq!(
                case.final_state["support_resolution"]["replacement_relation"].is_string(),
                matches!(case.change, "trigger" | "context")
            );
            assert_eq!(case.final_state["revision"], case.stale["revision"]);
            assert_eq!(
                case.final_state["policy_origin"],
                case.stale["policy_origin"]
            );
            assert!(
                !case.final_work["work"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|w| w["relation"] == case.original.as_str()),
                "{}",
                case.final_work
            );
            assert_eq!(
                case.final_edges,
                usize::from(matches!(case.change, "trigger" | "context")),
                "{}",
                case.change
            );
        }
    }
}
fn compile_edge(s: &mut AssertionScenario, run: &str, window: usize) {
    s.compile_with_window(run,Some("alice"),RunMode::Live,Some(json!({"schema":"merl.compiler-response/v1","assertions":[],"relations":[{"subject":"D1","predicate":"supports","object":"D0"}]})),window);
}
fn apply_and_review(s: &AssertionScenario, run: &str) {
    let result = s.cli(&[
        "source",
        "apply",
        "--run",
        run,
        "--actor",
        "worker",
        "--id",
        &format!("apply-{run}"),
    ]);
    let candidate = result["inputs"][0]["input"]
        .as_str()
        .expect("relation candidate");
    let result = s.cli(&[
        "candidate",
        "accept",
        candidate,
        "--actor",
        "agent",
        "--id",
        &format!("accept-{run}"),
    ]);
    assert_eq!(result["outcome"], "accepted", "{result}");
}

// Schema 26 retained accepted relation provenance but had no support-health tables.
// Remove only the new state so reopening exercises migration from that history.
fn remove_relation_health_schema(database: &super::Database) {
    let connection = rusqlite::Connection::open(&database.0).unwrap();
    connection
        .execute_batch(
            "DROP VIEW current_relations;
        DROP VIEW relation_support_availability;
        DROP TABLE relation_evidence_resolutions;
        DROP TABLE relation_withdrawals;
        DROP TABLE relation_evidence_impacts;
        DROP TABLE relation_evidence_supports;
        ALTER TABLE source_versions DROP COLUMN author_time;
        ALTER TABLE observed_assertions DROP COLUMN temporal;
        ALTER TABLE observed_assertions DROP COLUMN deferral;
        ALTER TABLE semantic_commands DROP COLUMN planning_evidence;
        PRAGMA user_version=26;",
        )
        .unwrap();
}

pub struct WithdrawalRaceScenario {
    cases: Vec<(&'static str, AssertionScenario, bool, Value)>,
}
impl WithdrawalRaceScenario {
    pub fn given_prepared_withdrawals() -> Self {
        Self {
            cases: ["grant", "replacement", "competing-withdrawal"]
                .into_iter()
                .map(|case| {
                    let mut s = AssertionScenario::given_a_decision_and_a_grounded_relation()
                        .when_the_run_is_inspected_and_applied();
                    compile_edge(&mut s, "accepted-edge", 1);
                    apply_and_review(&s, "accepted-edge");
                    AssertionScenario::capture(
                        &mut s.store(),
                        "source-accepted-edge",
                        "edited",
                        Some("source-accepted-edge"),
                        Some("alice"),
                    );
                    (case, s, false, Value::Null)
                })
                .collect(),
        }
    }
    pub fn when_authority_or_support_changes_before_commit(mut self) -> Self {
        for (case, s, conflict, receipt) in &mut self.cases {
            let impact = s.store().relation_impact_page(&id("P1"), None).unwrap().0[0]
                .id
                .clone();
            let review = merl_store::RelationWithdrawal {
                id: id("prepared-withdrawal"),
                impact,
                actor: id("agent"),
            };
            let prepared = merl_policy::prepare_relation_withdrawal(
                &mut s.store(),
                &id("P1"),
                &review,
                NOW + 10,
            )
            .unwrap();
            match *case {
                "grant" => {
                    s.cli(&[
                        "project",
                        "authority",
                        "revoke",
                        "--actor",
                        "admin",
                        "--subject",
                        "agent",
                        "--permission",
                        "command_actor",
                        "--id",
                        "revoke",
                        "--reason",
                        "Responsibility changed",
                    ]);
                }
                "replacement" => {
                    compile_edge(s, "replacement-edge", 1);
                    apply_and_review(s, "replacement-edge");
                }
                "competing-withdrawal" => {
                    s.cli(&[
                        "project",
                        "revalidation",
                        "resolve",
                        "--id",
                        "first-withdrawal",
                        "--impact",
                        &review.impact,
                        "--actor",
                        "agent",
                        "--action",
                        "withdraw",
                    ]);
                }
                _ => unreachable!(),
            }
            *conflict = matches!(
                prepared.commit(&mut s.store()),
                Err(merl_store::StoreError::PolicyConflict)
            );
            *receipt = s.cli(&[
                "project",
                "revalidation",
                "resolve",
                "--id",
                review.id.as_str(),
                "--impact",
                &review.impact,
                "--actor",
                "agent",
                "--action",
                "withdraw",
            ]);
        }
        self
    }
    pub fn then_stale_withdrawals_leave_durable_conflicts(self) {
        for (case, _, conflict, receipt) in self.cases {
            assert!(conflict, "{case}: stale withdrawal committed");
            assert_eq!(receipt["disposition"], "conflict", "{case}: {receipt}");
        }
    }
}

pub struct IndependentRelationScenario {
    scenario: AssertionScenario,
    before: (Vec<merl_core::ObjectId>, bool),
    after: (Vec<merl_core::ObjectId>, bool),
    work: Value,
    current_edges: usize,
}
impl IndependentRelationScenario {
    pub fn given_independent_support_for_the_same_edge() -> Self {
        let mut scenario = AssertionScenario::given_a_decision_and_a_grounded_relation()
            .when_the_run_is_inspected_and_applied();
        for run in ["first-edge", "second-edge"] {
            compile_edge(&mut scenario, run, 1);
            apply_and_review(&scenario, run);
        }
        Self {
            scenario,
            before: (Vec::new(), false),
            after: (Vec::new(), false),
            work: Value::Null,
            current_edges: 0,
        }
    }
    pub fn when_one_derivations_source_changes(mut self) -> Self {
        let s = &self.scenario;
        self.before = s.store().focus_neighbors(&id("P1"), &id("D1"), 1).unwrap();
        AssertionScenario::capture(
            &mut s.store(),
            "source-first-edge",
            "edited",
            Some("source-first-edge"),
            Some("alice"),
        );
        self.after = s.store().focus_neighbors(&id("P1"), &id("D1"), 1).unwrap();
        self.work = s.cli(&["project", "revalidation", "list"]);
        self.current_edges = s
            .store()
            .issue_state(&id("P1"), &id("D1"), "issue-1")
            .unwrap()
            .relations
            .len();
        self
    }
    pub fn then_only_stale_support_needs_review_and_the_neighbor_stays_current(self) {
        assert_eq!(
            self.before,
            (vec![id("D0")], false),
            "independent derivations must not consume extra neighbor slots"
        );
        assert_eq!(self.after, (vec![id("D0")], false));
        assert_eq!(self.current_edges, 1);
        let relations = self.work["work"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["relation"].is_string())
            .collect::<Vec<_>>();
        assert_eq!(relations.len(), 1);
        assert_eq!(relations[0]["affected_run"], "first-edge");
    }
}

pub struct RelationUpgradeScenario {
    scenario: AssertionScenario,
    work: Value,
    current_edges: usize,
}
impl RelationUpgradeScenario {
    pub fn given_reviewed_edges_before_and_after_a_source_edit() -> Self {
        let mut scenario = AssertionScenario::given_a_decision_and_a_grounded_relation()
            .when_the_run_is_inspected_and_applied();
        compile_edge(&mut scenario, "first-edge", 1);
        apply_and_review(&scenario, "first-edge");
        AssertionScenario::capture(
            &mut scenario.store(),
            "source-first-edge",
            "edited",
            Some("source-first-edge"),
            Some("alice"),
        );
        // The next context deliberately includes both versions. Seeing an earlier
        // source alongside its correction is valid compiler input.
        compile_edge(&mut scenario, "second-edge", 3);
        apply_and_review(&scenario, "second-edge");
        Self {
            scenario,
            work: Value::Null,
            current_edges: 0,
        }
    }
    pub fn when_relation_health_is_migrated_from_accepted_history(mut self) -> Self {
        remove_relation_health_schema(&self.scenario.database);
        self.work = self.scenario.cli(&["project", "revalidation", "list"]);
        self.current_edges = self
            .scenario
            .store()
            .issue_state(&id("P1"), &id("D1"), "issue-1")
            .unwrap()
            .relations
            .len();
        self
    }
    pub fn then_only_the_derivation_before_the_edit_needs_review(self) {
        assert_eq!(
            self.current_edges, 1,
            "migration must preserve the edge whose compiler saw the edit"
        );
        let work = self.work["work"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["relation"].is_string())
            .collect::<Vec<_>>();
        assert_eq!(work.len(), 1);
        assert_eq!(work[0]["affected_run"], "first-edge");
    }
}
