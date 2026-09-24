use super::*;

pub struct TemporalScenario {
    scenario: AssertionScenario,
    result: Option<Result<(), CompileError>>,
    deferral: bool,
    expected_date: Option<&'static str>,
    replacement_temporal: Option<Value>,
}

impl TemporalScenario {
    pub fn given_a_relative_date_without_timezone_evidence() -> Self {
        let scenario = AssertionScenario::new();
        AssertionScenario::capture_body(
            &mut scenario.store(),
            "time",
            "time",
            None,
            Some("alice"),
            Some(b"Reconsider tomorrow."),
        );
        Self {
            scenario,
            result: None,
            deferral: false,
            expected_date: None,
            replacement_temporal: None,
        }
    }

    pub fn given_a_deferred_task_waiting_for_a_pull_request() -> Self {
        let scenario = AssertionScenario::new();
        AssertionScenario::capture_body_at(
            &mut scenario.store(),
            "time",
            "time",
            None,
            Some("alice"),
            Some(b"Reconsider tomorrow after PR 229 merges."),
            Some(
                merl_core::temporal::AuthorTime::new(NOW, Some(19800), Some("Asia/Kolkata".into()))
                    .unwrap(),
            ),
        );
        Self {
            scenario,
            result: None,
            deferral: true,
            expected_date: Some("2023-11-16"),
            replacement_temporal: None,
        }
    }

    pub fn when_the_source_is_compiled(mut self) -> Self {
        let mut store = self.scenario.store();
        let prepared = prepare_eager_compilation(
            &mut store,
            &id("P1"),
            &id("time"),
            &Compiler,
            RunRequest {
                id: "time-run",
                limits: CompilerLimits::from_array([32768, 16384, 2048, 32, 2, 1, 4096, 4, 4]),
                mode: RunMode::Live,
                now_millis: NOW + 1,
            },
        )
        .unwrap()
        .unwrap();
        let mut assertion = AssertionScenario::assertion("D1");
        assertion["source"] = json!("time");
        assertion["span_end"] = json!(if self.deferral { 40 } else { 20 });
        assertion["temporal"] = json!([{
            "role": "review_at", "span_start": 11, "span_end": 19,
            "expression": {"kind": "relative_date", "days": 1}
        }]);
        if self.deferral {
            assertion["predicate"] = json!("task");
            assertion["deferral"] = json!({"accepted":true,"reason":{"start":20,"end":39}});
            assertion["temporal"].as_array_mut().unwrap().push(json!({
                "role":"start_after","span_start":20,"span_end":39,
                "expression":{"kind":"after_event","predicate":{"kind":"provider_merged","subject":"github:example/parser/pull/229"}}
            }));
        }
        if let Some(temporal) = self.replacement_temporal.take() {
            assertion["temporal"] = temporal;
        }
        self.result = Some(record_compilation_result(
            &mut store,
            &id("P1"),
            &prepared,
            Ok(serde_json::to_vec(&AssertionScenario::response(&[assertion])).unwrap()),
            NOW + 2,
        ));
        self.scenario.inspected =
            Some(
                self.scenario
                    .cli(&["source", "assertions", "--run", "time-run"]),
            );
        self
    }

    pub fn when_the_task_is_reviewed_rebuilt_and_replayed(mut self) -> Self {
        let applied = self.scenario.cli(&[
            "source",
            "apply",
            "--run",
            "time-run",
            "--id",
            "apply-time",
            "--actor",
            "agent",
        ]);
        let candidate = applied["inputs"][0]["input"]
            .as_str()
            .expect("candidate identity");
        self.scenario.outputs.push(self.scenario.cli(&[
            "candidate",
            "accept",
            candidate,
            "--id",
            "review-time",
            "--actor",
            "agent",
        ]));
        self.scenario.detail = Some(self.scenario.cli(&["show", "D1", "--source"]));
        self.scenario.cli(&["project", "rebuild"]);
        self.scenario.retry = Some(self.scenario.cli(&["show", "D1", "--source"]));
        let mut store = self.scenario.store();
        let limits = CompilerLimits::from_array([32768, 16384, 2048, 32, 2, 1, 4096, 4, 4]);
        let rebuilt =
            merl_compiler::rebuild_recorded_context(&store, &id("P1"), "time-run", limits).unwrap();
        let prepared = prepare_replay_compilation(
            &mut store,
            &id("P1"),
            "time-run",
            rebuilt,
            &Compiler,
            RunRequest {
                id: "time-replay",
                limits,
                mode: RunMode::Replay,
                now_millis: NOW + 864_000_000,
            },
        )
        .unwrap()
        .unwrap();
        let response = store
            .compilation_response(&id("P1"), &id("time-run"))
            .unwrap();
        let Some(merl_store::PayloadRead::Available(bytes)) = response else {
            panic!("retained response")
        };
        record_compilation_result(
            &mut store,
            &id("P1"),
            &prepared,
            Ok(bytes),
            NOW + 864_000_001,
        )
        .unwrap();
        self.scenario.inbox =
            Some(
                self.scenario
                    .cli(&["source", "assertions", "--run", "time-replay"]),
            );
        self
    }

    pub fn then_the_task_retains_its_source_date_predicate_and_reason(self) {
        assert!(self.result.unwrap().is_ok());
        assert_eq!(self.scenario.outputs[0]["outcome"], "accepted");
        let detail = self.scenario.detail.as_ref().unwrap();
        assert_eq!(detail["task"]["commitment"], "accepted", "{detail}");
        assert_eq!(detail["task"]["scheduling"], "deferred");
        assert_eq!(detail["task"]["execution"], "not_started");
        assert_eq!(detail["task"]["review_at"], self.expected_date.unwrap());
        assert_eq!(detail["task"]["reason"]["text"], "after PR 229 merges");
        assert_eq!(
            detail["task"]["start_after"],
            json!({"kind":"provider_merged","subject":"github:example/parser/pull/229"})
        );
        assert_eq!(self.scenario.detail, self.scenario.retry);
        let original = &self.scenario.inspected.as_ref().unwrap()["assertions"][0]["temporal"];
        assert_eq!(
            original,
            &self.scenario.inbox.as_ref().unwrap()["assertions"][0]["temporal"]
        );
        assert_eq!(detail["temporal"], *original);
    }

    pub fn then_the_date_remains_unresolved(self) {
        assert!(
            self.result.unwrap().is_ok(),
            "a missing timezone is an unresolved interpretation, not a protocol failure"
        );
        let output = self.scenario.inspected.unwrap();
        assert_eq!(
            output["assertions"][0]["temporal"][0]["value"],
            json!({"kind":"unresolved", "reason":"missing_author_time"}),
            "{output}"
        );
    }

    pub fn when_acceptance_is_attempted(mut self) -> Self {
        let applied = self.scenario.cli(&[
            "source",
            "apply",
            "--run",
            "time-run",
            "--id",
            "apply-time",
            "--actor",
            "agent",
        ]);
        let candidate = applied["inputs"][0]["input"].as_str().unwrap();
        self.scenario.outputs.push(applied.clone());
        self.scenario.outputs.push(self.scenario.cli(&[
            "candidate",
            "accept",
            candidate,
            "--id",
            "review-time",
            "--actor",
            "agent",
        ]));
        self.scenario.detail = Some(self.scenario.cli(&["show", "D1"]));
        self
    }

    pub fn then_no_guessed_date_enters_accepted_state(self) {
        assert_eq!(
            self.scenario.outputs[0]["inputs"][0]["outcome"],
            "candidate"
        );
        assert_eq!(
            self.scenario.outputs[0]["inputs"][0]["reason"],
            "temporal_unresolved"
        );
        assert_eq!(self.scenario.outputs[1]["outcome"], "rejected");
        assert_eq!(self.scenario.outputs[1]["reason"], "temporal_unresolved");
        assert!(self.scenario.detail.as_ref().unwrap()["code"].is_string());
    }

    pub fn when_a_later_source_needs_the_accepted_schedule(mut self) -> Self {
        let mut store = self.scenario.store();
        AssertionScenario::capture_body(
            &mut store,
            "followup",
            "followup",
            None,
            Some("alice"),
            Some(b"What is the review date?"),
        );
        let context = merl_compiler::build_context(
            &store,
            &id("P1"),
            &id("followup"),
            CompilerLimits::from_array([32768, 16384, 2048, 32, 2, 1, 4096, 4, 4]),
        )
        .unwrap();
        self.scenario.inbox = Some(serde_json::from_slice(&context.rendered).unwrap());
        self
    }

    pub fn then_the_compiler_receives_the_accepted_temporal_values(self) {
        let context = self.scenario.inbox.unwrap();
        assert_eq!(
            context["objects"][0]["temporal"],
            self.scenario.detail.unwrap()["temporal"]
        );
    }
}

pub struct CalendarCases(Vec<(TemporalScenario, Value)>);
impl CalendarCases {
    pub fn given_sources_at_calendar_boundaries() -> Self {
        let cases = [
            (
                1_700_000_000_000,
                Some(0),
                json!({"kind":"date","date":"2023-11-15"}),
            ),
            (
                1_700_000_000_000,
                Some(-28800),
                json!({"kind":"date","date":"2023-11-15"}),
            ),
            (
                1_704_065_400_000,
                Some(19800),
                json!({"kind":"date","date":"2024-01-02"}),
            ),
            (
                1_709_163_000_000,
                Some(0),
                json!({"kind":"date","date":"2024-02-29"}),
            ),
            (
                1_700_000_000_000,
                None,
                json!({"kind":"unresolved","reason":"missing_timezone"}),
            ),
        ];
        Self(
            cases
                .into_iter()
                .map(|(instant, offset, expected)| {
                    let scenario = AssertionScenario::new();
                    AssertionScenario::capture_body_at(
                        &mut scenario.store(),
                        "time",
                        "time",
                        None,
                        Some("alice"),
                        Some(b"Reconsider tomorrow."),
                        Some(merl_core::temporal::AuthorTime::new(instant, offset, None).unwrap()),
                    );
                    (
                        TemporalScenario {
                            scenario,
                            result: None,
                            deferral: false,
                            expected_date: None,
                            replacement_temporal: None,
                        },
                        expected,
                    )
                })
                .collect(),
        )
    }
    pub fn when_each_source_is_compiled(self) -> Self {
        Self(
            self.0
                .into_iter()
                .map(|(s, e)| (s.when_the_source_is_compiled(), e))
                .collect(),
        )
    }
    pub fn then_dates_follow_the_authors_local_calendar(self) {
        for (scenario, expected) in self.0 {
            assert!(scenario.result.unwrap().is_ok());
            assert_eq!(
                scenario.scenario.inspected.unwrap()["assertions"][0]["temporal"][0]["value"],
                expected
            );
        }
    }
}

pub struct InvalidCases(Vec<TemporalScenario>);
impl InvalidCases {
    pub fn given_invalid_temporal_interpretations() -> Self {
        let base = json!({"role":"review_at","span_start":11,"span_end":19,"expression":{"kind":"relative_date","days":1}});
        let mut forged = base.clone();
        forged["basis"] = json!({"utc_millis":0,"offset_seconds":0});
        let mut empty_span = base.clone();
        empty_span["span_end"] = json!(11);
        let mut date_as_event = base.clone();
        date_as_event["role"] = json!("start_after");
        let mut missing_pr_identity = base.clone();
        missing_pr_identity["expression"] =
            json!({"kind":"after_event","predicate":{"kind":"provider_merged","subject":"229"}});
        Self(
            [
                json!([forged]),
                json!([empty_span]),
                json!([base.clone(), base]),
                json!([date_as_event]),
                json!([missing_pr_identity]),
            ]
            .into_iter()
            .map(|temporal| {
                let mut scenario =
                    TemporalScenario::given_a_relative_date_without_timezone_evidence();
                scenario.replacement_temporal = Some(temporal);
                scenario
            })
            .collect(),
        )
    }
    pub fn when_each_interpretation_is_submitted(self) -> Self {
        Self(
            self.0
                .into_iter()
                .map(TemporalScenario::when_the_source_is_compiled)
                .collect(),
        )
    }
    pub fn then_no_invalid_temporal_assertions_are_retained(self) {
        for scenario in self.0 {
            assert!(matches!(
                scenario.result.unwrap(),
                Err(CompileError::InvalidResponse)
            ));
            assert_eq!(
                scenario.scenario.inspected.unwrap()["assertions"],
                json!([])
            );
        }
    }
}

pub struct CaptureCases {
    cases: Vec<(merl_corpus::fixture::Fixture, Option<i32>)>,
    captured: Vec<Value>,
}
impl CaptureCases {
    pub fn given_sources_with_explicit_and_unknown_author_offsets() -> Self {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../corpus/adversarial/ADV-C3.json")).unwrap();
        let mut explicit = fixture.clone();
        explicit["observations"][1]["kind"] = json!("issue_comment");
        explicit["observations"][1]["authored_at"] = json!("2026-01-02T14:30:00+05:30");
        explicit["observations"][1]["author_timezone"] = json!("Asia/Kolkata");
        let mut unknown = fixture.clone();
        unknown["observations"][1]["kind"] = json!("issue_comment");
        Self {
            cases: [(fixture, Some(0)), (explicit, Some(19800)), (unknown, None)]
                .into_iter()
                .map(|(f, o)| {
                    let mut fixture: merl_corpus::fixture::Fixture =
                        serde_json::from_value(f).unwrap();
                    fixture.capture.source_sha256 = merl_corpus::fixture::source_digest(
                        &fixture.provider_snapshot,
                        &fixture.observations,
                    );
                    (fixture, o)
                })
                .collect(),
            captured: Vec::new(),
        }
    }
    pub fn when_the_sources_are_imported_and_retried(mut self) -> Self {
        for (fixture, _) in &self.cases {
            let mut store = Store::open_in_memory().unwrap();
            store.create_project(&id("P1")).unwrap();
            merl_ingest::import_fixture(&mut store, &id("P1"), fixture).unwrap();
            merl_ingest::import_fixture(&mut store, &id("P1"), fixture).unwrap();
            let source = store
                .source_version(
                    &id("P1"),
                    &merl_ingest::fixture_version_id(&fixture.observations[1].version_id).unwrap(),
                )
                .unwrap()
                .unwrap();
            self.captured.push(json!(source.author_time));
        }
        self
    }
    pub fn then_only_recorded_local_offsets_are_available(self) {
        for ((_, expected), actual) in self.cases.into_iter().zip(self.captured) {
            assert_eq!(actual["offset_seconds"], json!(expected));
        }
    }
}
