//! Confirm support through the CLI while keeping the task's accepted reason intact.

use super::*;

const ORIGINAL_REASON: &str = "vendor approval is missing";
const EVENT: &str = "after PR 229 merges";

pub struct DeferralReviewCases(Vec<Case>);

struct Case {
    scenario: AssertionScenario,
    prefix: &'static str,
    reason: &'static str,
    erase: Option<&'static str>,
    confirms: bool,
}

impl DeferralReviewCases {
    pub fn given_deferred_tasks_with_edited_reasons() -> Self {
        Self(
            [
                ("Wait because ", "budget approval is granted", None, false),
                ("Wait because ", ORIGINAL_REASON, None, true),
                ("Please wait because ", ORIGINAL_REASON, None, true),
                ("Wait because ", ORIGINAL_REASON, Some("original"), false),
                ("Wait because ", ORIGINAL_REASON, Some("edited"), false),
            ]
            .into_iter()
            .map(|(prefix, reason, erase, confirms)| {
                assert_eq!(
                    reason.len(),
                    ORIGINAL_REASON.len(),
                    "reason edits must preserve byte offsets"
                );
                let mut scenario = AssertionScenario::new();
                let body = format!("Wait because {ORIGINAL_REASON}. Start {EVENT}.");
                AssertionScenario::capture_body(
                    &mut scenario.store(),
                    "planning",
                    "original",
                    None,
                    Some("alice"),
                    Some(body.as_bytes()),
                );
                compile(
                    &scenario,
                    "original",
                    "initial",
                    &body,
                    ORIGINAL_REASON,
                    false,
                );
                let applied = scenario.cli(&[
                    "source", "apply", "--run", "initial", "--id", "apply", "--actor", "agent",
                ]);
                let accepted = scenario.cli(&[
                    "candidate",
                    "accept",
                    applied["inputs"][0]["input"].as_str().unwrap(),
                    "--id",
                    "accept",
                    "--actor",
                    "agent",
                ]);
                assert_eq!(accepted["outcome"], "accepted", "{accepted}");
                scenario.detail = Some(scenario.cli(&["show", "T1", "--source"]));
                Case {
                    scenario,
                    prefix,
                    reason,
                    erase,
                    confirms,
                }
            })
            .collect(),
        )
    }

    pub fn when_each_edit_is_recompiled_and_confirmation_is_requested(mut self) -> Self {
        for case in &mut self.0 {
            let body = format!(
                "{}{}. Start {EVENT}. Updated note.",
                case.prefix, case.reason
            );
            let scenario = &mut case.scenario;
            AssertionScenario::capture_body(
                &mut scenario.store(),
                "planning",
                "edited",
                Some("original"),
                Some("alice"),
                Some(body.as_bytes()),
            );
            let impact = scenario
                .store()
                .pending_evidence_impacts(&id("P1"))
                .unwrap()
                .remove(0);
            compile(scenario, "edited", &impact.id, &body, case.reason, true);
            if let Some(version) = case.erase {
                let mut store = scenario.store();
                let source = store
                    .source_version(&id("P1"), &id(version))
                    .unwrap()
                    .unwrap();
                store
                    .erase_payload(&id("P1"), &source.payload.unwrap())
                    .unwrap();
            }
            scenario.basis = scenario.store().project_revision(&id("P1")).unwrap().get();
            scenario.outputs.push(scenario.cli(&[
                "project",
                "revalidation",
                "resolve",
                "--id",
                "confirm",
                "--impact",
                &impact.id,
                "--actor",
                "agent",
                "--action",
                "confirm",
                "--run",
                &impact.id,
                "--assertion-index",
                "0",
            ]));
            scenario.cli(&["project", "rebuild"]);
            scenario.retry = Some(scenario.cli(&["show", "T1", "--source"]));
        }
        self
    }

    pub fn then_only_unchanged_available_reasons_restore_support(self) {
        for case in self.0 {
            let scenario = case.scenario;
            let review = &scenario.outputs[0];
            assert_eq!(
                review["disposition"],
                if case.confirms {
                    "accepted"
                } else {
                    "conflict"
                },
                "reason={:?}, prefix={:?}, erased={:?}: {review}",
                case.reason,
                case.prefix,
                case.erase
            );
            if !case.confirms {
                assert_eq!(review["reason"], "revalidation_temporal_changed");
            }
            let after = scenario.retry.as_ref().unwrap();
            assert_eq!(
                after["revision"],
                scenario.detail.as_ref().unwrap()["revision"]
            );
            assert_eq!(
                after["task"]["start_after"],
                scenario.detail.as_ref().unwrap()["task"]["start_after"]
            );
            if case.erase != Some("original") {
                assert_eq!(after["task"]["reason"]["text"], ORIGINAL_REASON);
            }
            assert_eq!(after["support"] == "current", case.confirms, "{after}");
            assert_eq!(
                scenario.store().project_revision(&id("P1")).unwrap().get(),
                scenario.basis + u64::from(case.confirms)
            );
        }
    }
}

fn compile(
    scenario: &AssertionScenario,
    source: &str,
    run: &str,
    body: &str,
    reason: &str,
    hindsight: bool,
) {
    let mut store = scenario.store();
    let request = RunRequest {
        id: run,
        limits: CompilerLimits::from_array([32768, 16384, 2048, 32, 2, 1, 4096, 4, 4]),
        mode: if hindsight {
            RunMode::Hindsight
        } else {
            RunMode::Live
        },
        now_millis: NOW + 1,
    };
    let prepared = if hindsight {
        prepare_hindsight_compilation(&mut store, &id("P1"), &id(source), &Compiler, request)
    } else {
        prepare_eager_compilation(&mut store, &id("P1"), &id(source), &Compiler, request)
    }
    .unwrap()
    .unwrap();
    let mut assertion = AssertionScenario::assertion("T1");
    assertion["source"] = json!(source);
    assertion["span_end"] = json!(body.len());
    assertion["predicate"] = json!("task");
    let start = body.find(reason).unwrap();
    assertion["deferral"] =
        json!({"accepted":true,"reason":{"start":start,"end":start + reason.len()}});
    let start = body.find(EVENT).unwrap();
    assertion["temporal"] = json!([{
        "role":"start_after", "span_start":start, "span_end":start + EVENT.len(),
        "expression":{"kind":"after_event","predicate":{"kind":"provider_merged","subject":"github:example/parser/pull/229"}}
    }]);
    record_compilation_result(
        &mut store,
        &id("P1"),
        &prepared,
        Ok(serde_json::to_vec(&AssertionScenario::response(&[assertion])).unwrap()),
        NOW + 2,
    )
    .unwrap();
}
