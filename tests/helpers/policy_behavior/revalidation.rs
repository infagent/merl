//! Public revalidation scenarios share the existing captured-source fixtures.

use super::{NOW, PolicyScenario, StatementCompiler, event, id, limits};
use merl_compiler::{
    RunMode, RunRequest, execute_compilation, prepare_hindsight_compilation,
    record_compilation_result,
};
use merl_core::PolicyDisposition;
use merl_policy::Proposal;
use merl_store::{StoreError, SupportStatus};

impl PolicyScenario {
    pub fn given_a_resolved_question_with_source_support() -> Self {
        let mut scenario = Self::file_backed();
        scenario
            .store
            .grant_administrator_unchecked_bootstrap(&scenario.project, &id("admin"))
            .unwrap();
        scenario.cli(&[
            "project",
            "authority",
            "grant",
            "--actor",
            "admin",
            "--subject",
            "reviewer",
            "--permission",
            "command_actor",
            "--id",
            "grant-reviewer",
            "--reason",
            "Review sources",
        ]);
        scenario.capture("direct-v1", "Use fixed gain", "alice");
        scenario.compile("direct-v1", 14, None, "question-run");
        let result = merl_policy::apply_assertions(
            &mut scenario.store,
            &scenario.project,
            &id("question-run"),
            &id("reviewer"),
            &id("question-application"),
            NOW + 5,
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.inputs[0].disposition, PolicyDisposition::Candidate);
        scenario
            .store
            .put_review_reason(
                &scenario.project,
                &id("question-reason"),
                b"This is an open question",
            )
            .unwrap();
        let review = merl_store::CandidateReview {
            id: id("question-review"),
            candidate: result.inputs[0].input.id().clone(),
            actor: id("reviewer"),
            action: merl_store::ReviewAction::Correct {
                object: id("Q1"),
                kind: id("question"),
                payload: None,
            },
            reason: Some(id("question-reason")),
        };
        merl_policy::review_candidate(&mut scenario.store, &scenario.project, &review, NOW + 6)
            .unwrap();
        let result = scenario.cli(&[
            "question",
            "resolve",
            "Q1",
            "--id",
            "answer-question",
            "--actor",
            "reviewer",
            "--statement",
            "Use automatic gain",
        ]);
        assert_eq!(result["outcome"], "accepted");
        scenario.capture_edit(
            "direct-v1",
            "direct-v2",
            "Correction: sweep the gain.",
            "direct-v1",
        );
        scenario
    }

    pub fn when_its_changed_support_is_withdrawn(&mut self) -> &mut Self {
        let impact = self
            .store
            .pending_evidence_impacts(&self.project)
            .unwrap()
            .remove(0);
        self.cli(&["project","revalidation","run","--id",&impact.id,"--actor","reviewer","--program","/bin/sh","--compiler-arg","-c","--compiler-arg", r#"cat >/dev/null; printf '%s' '{"schema":"merl.compiler-response/v1","assertions":[]}'"#, "--compiler-version","v1","--model","test","--prompt-digest","sha256:0000000000000000000000000000000000000000000000000000000000000000"]);
        self.cli(&[
            "project",
            "revalidation",
            "resolve",
            "--id",
            "withdraw-question-support",
            "--actor",
            "reviewer",
            "--impact",
            &impact.id,
            "--run",
            &impact.id,
            "--action",
            "weaken",
        ]);
        self.when_the_authority_restarts();
        self.cli(&["project", "rebuild"]);
        self.revalidation_output = self.cli(&["show", "Q1", "--source"]);
        self
    }

    pub fn then_the_question_remains_resolved(&mut self) {
        assert_eq!(self.revalidation_output["status"], "resolved");
        assert_eq!(self.revalidation_output["support"], "unsupported");
        assert_eq!(
            self.revalidation_output["policy_origin"]["input"]["revalidation"]["action"],
            "weaken"
        );
    }

    pub fn given_a_prepared_revalidation_review() -> Self {
        let mut scenario = Self::given_an_authorized_evidence_reviewer("confirm");
        let impact = scenario
            .store
            .pending_evidence_impacts(&scenario.project)
            .unwrap()
            .remove(0);
        let compiler = StatementCompiler {
            version: "direct-v1",
            body_len: 14,
            attributed_to: None,
            value: "none",
        };
        let request = RunRequest {
            id: &impact.id,
            limits: limits(),
            mode: RunMode::Hindsight,
            now_millis: NOW + 10,
        };
        prepare_hindsight_compilation(
            &mut scenario.store,
            &scenario.project,
            &id("direct-v1"),
            &compiler,
            request,
        )
        .unwrap()
        .unwrap();
        let saved = scenario
            .store
            .load_compilation_context(&scenario.project, &impact.id)
            .unwrap()
            .rendered;
        scenario.when_the_authority_restarts();
        let prepared = prepare_hindsight_compilation(
            &mut scenario.store,
            &scenario.project,
            &id("direct-v1"),
            &compiler,
            RunRequest {
                id: &impact.id,
                limits: limits(),
                mode: RunMode::Hindsight,
                now_millis: NOW + 11,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            saved,
            scenario
                .store
                .load_compilation_context(&scenario.project, &impact.id)
                .unwrap()
                .rendered
        );
        let response = execute_compilation(&prepared, &compiler);
        record_compilation_result(
            &mut scenario.store,
            &scenario.project,
            &prepared,
            response,
            NOW + 12,
        )
        .unwrap();
        let review = merl_store::RevalidationReview {
            id: id("prepared-review"),
            impact: impact.id,
            actor: id("reviewer"),
            action: merl_store::RevalidationAction::Confirm,
            run: Some(id(prepared.id())),
            assertion_index: Some(0),
        };
        scenario.prepared = Some(
            merl_policy::prepare_revalidation_review(
                &mut scenario.store,
                &scenario.project,
                &review,
                NOW + 13,
            )
            .unwrap(),
        );
        scenario
    }

    pub fn when_the_project_changes_before_revalidation_commits(
        &mut self,
        change: &str,
    ) -> &mut Self {
        match change {
            "authority" => {
                self.cli(&[
                    "project",
                    "authority",
                    "revoke",
                    "--actor",
                    "admin",
                    "--subject",
                    "reviewer",
                    "--permission",
                    "command_actor",
                    "--id",
                    "revoke-reviewer",
                    "--reason",
                    "Review finished",
                ]);
            }
            "evidence" => {
                self.when_the_context_is_edited_again();
            }
            "target" | "unrelated" => {
                merl_policy::apply_current(
                    &mut self.store,
                    &self.project,
                    &id("reviewer"),
                    id("other-evaluation"),
                    id("other-batch"),
                    NOW + 14,
                    &[Proposal::Command {
                        id: id("other-command"),
                        event: event(
                            "other-event",
                            if change == "target" { "D1" } else { "D9" },
                            "decision",
                            None,
                        ),
                    }],
                )
                .unwrap();
            }
            "purge" => {
                let preview = self
                    .store
                    .preview_source_purge(&self.project, &id("context-v2"))
                    .unwrap();
                self.store
                    .purge_source(
                        &self.project,
                        &id("context-v2"),
                        &id("admin"),
                        b"remove protected evidence",
                        NOW + 14,
                        preview.confirm_digest,
                    )
                    .unwrap();
            }
            "competing_review" => {
                let mut review = self
                    .store
                    .revalidation_review(&self.project, &id("prepared-review"))
                    .unwrap()
                    .unwrap();
                review.id = id("competing-review");
                merl_policy::resolve_revalidation(
                    &mut self.store,
                    &self.project,
                    &review,
                    NOW + 14,
                )
                .unwrap();
            }
            _ => panic!("unknown change"),
        }
        self.result = Some(self.prepared.as_ref().unwrap().commit(&mut self.store));
        let review = self
            .store
            .revalidation_review(&self.project, &id("prepared-review"))
            .unwrap()
            .unwrap();
        self.retry_disposition = Some(
            merl_policy::resolve_revalidation(&mut self.store, &self.project, &review, NOW + 20)
                .unwrap()
                .inputs[0]
                .disposition,
        );

        self
    }

    pub fn then_revalidation_respects_the_changed_dependency(&mut self, change: &str) {
        if change == "unrelated" {
            assert!(self.result.as_ref().unwrap().is_ok());
        } else {
            assert!(
                matches!(
                    self.result.as_ref().unwrap(),
                    Err(StoreError::PolicyConflict)
                ),
                "{change}: {:?}",
                self.result
            );
        }
        let review = self
            .store
            .revalidation_review(&self.project, &id("prepared-review"))
            .unwrap()
            .unwrap();
        let recorded = self
            .store
            .policy_evaluation(
                &self.project,
                &merl_policy::revalidation_evaluation(&self.project, &review.id).unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(Some(recorded.inputs[0].disposition), self.retry_disposition);
    }

    pub fn when_the_context_is_edited_again(&mut self) -> &mut Self {
        self.capture_edit(
            "context-v2",
            "context-v3",
            "Use a different baseline.",
            "issue-204",
        );
        self
    }

    pub fn then_the_latest_replacement_supports_the_decision(&mut self) {
        assert_eq!(self.revalidation_output["disposition"], "accepted");
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .unwrap(),
            0
        );
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .unwrap(),
            SupportStatus::Current
        );
        let impact = &self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .unwrap()[0];
        assert!(
            self.store
                .compilation_context_sources(&self.project, &impact.id)
                .unwrap()
                .contains(&id("context-v3"))
        );
    }

    pub fn then_all_remaining_support_needs_revalidation(&mut self) {
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .unwrap(),
            SupportStatus::RevalidationPending
        );
        assert_eq!(
            self.store
                .pending_evidence_impacts(&self.project)
                .unwrap()
                .len(),
            1
        );
    }

    pub fn given_changed_direct_and_context_evidence(action: &str) -> Self {
        let mut scenario = Self::given_an_authorized_evidence_reviewer("confirm");
        scenario.capture_change(
            "direct-v1",
            "direct-v2",
            if action == "unavailable" {
                None
            } else {
                Some("Use fixed gain.")
            },
            "issue-204",
        );
        scenario
    }

    pub fn given_an_authorized_evidence_reviewer(action: &str) -> Self {
        let mut scenario = Self::given_a_decision_compiled_with_an_earlier_comment();
        scenario
            .store
            .grant_administrator_unchecked_bootstrap(&scenario.project, &id("admin"))
            .unwrap();
        scenario.cli(&[
            "project",
            "authority",
            "grant",
            "--actor",
            "admin",
            "--subject",
            "reviewer",
            "--permission",
            "command_actor",
            "--id",
            "grant-reviewer",
            "--reason",
            "Review changed evidence",
        ]);
        if action == "supersede" {
            scenario
                .store
                .put_payload(
                    &scenario.project,
                    &id("sweep-body"),
                    b"Sweep the receive gain.",
                )
                .unwrap();
        }
        if action == "unavailable" {
            scenario.when_the_earlier_comment_is_purged();
        } else {
            scenario.when_the_earlier_comment_is_edited();
        }
        scenario
    }

    fn cli(&self, args: &[&str]) -> serde_json::Value {
        let mut args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        args.extend([
            "--project".to_owned(),
            self.project.to_string(),
            "--database".to_owned(),
            self.persisted_file
                .as_ref()
                .unwrap()
                .0
                .to_str()
                .unwrap()
                .to_owned(),
            "--json".to_owned(),
        ]);
        match merl_cli::run_with_clock(&args, &|| Ok(NOW + 10)) {
            merl_cli::CliResponse::Success(output) => serde_json::from_str(&output).unwrap(),
            error => panic!("command {args:?}: {error:?}"),
        }
    }

    pub fn when_revalidation_is_run_and_resolved_after_restart(
        &mut self,
        action: &str,
    ) -> &mut Self {
        self.revalidate_after_restart(action, None)
    }

    pub fn when_changed_evidence_is_revalidated_again(&mut self) -> &mut Self {
        self.revalidate_after_restart("confirm", Some("second-attempt"))
    }

    pub fn then_both_interpretations_remain_inspectable(&mut self) {
        assert_eq!(self.revalidation_output["disposition"], "accepted");
        let impact = &self
            .store
            .evidence_impacts_for_object(&self.project, &id("D1"))
            .unwrap()[0];
        assert_eq!(
            self.store
                .latest_revalidation_attempt(&self.project, &impact.id)
                .unwrap(),
            Some(id("second-attempt"))
        );
        assert!(
            self.store
                .compilation_context_sources(&self.project, &impact.id)
                .unwrap()
                .contains(&id("context-v2"))
        );
        assert!(
            self.store
                .compilation_context_sources(&self.project, "second-attempt")
                .unwrap()
                .contains(&id("context-v3"))
        );
    }

    fn revalidate_after_restart(&mut self, action: &str, attempt: Option<&str>) -> &mut Self {
        self.when_the_authority_restarts();
        let work = self.cli(&["project", "revalidation", "list"]);
        let impact = work["work"][0]["impact"].as_str().unwrap().to_owned();
        let run = attempt.unwrap_or(&impact);
        if action != "unavailable" {
            let subject = if action == "supersede" { "D2" } else { "D1" };
            let value = if action == "supersede" {
                "sweep-body"
            } else {
                "none"
            };
            let trigger = self
                .store
                .latest_source_version(&self.project, &id("direct-v1"))
                .unwrap();
            let response = format!(
                r#"{{"schema":"merl.compiler-response/v1","assertions":[{{"source":"{trigger}","span_start":0,"span_end":14,"subject":"{subject}","predicate":"decision","value":"{value}","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900}}]}}"#
            );
            let script = format!("cat >/dev/null; printf '%s' '{response}'");
            let args = [
                "project",
                "revalidation",
                "run",
                "--id",
                &impact,
                "--run",
                run,
                "--actor",
                "reviewer",
                "--program",
                "/bin/sh",
                "--compiler-arg",
                "-c",
                "--compiler-arg",
                &script,
                "--compiler-version",
                "test-v1",
                "--model",
                "fixture",
                "--prompt-digest",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            ];
            self.cli(&args);
            self.when_the_authority_restarts();
            self.cli(&args);
        }
        let mut args = vec![
            "project",
            "revalidation",
            "resolve",
            "--id",
            "resolve-evidence",
            "--impact",
            &impact,
            "--action",
            action,
            "--actor",
            "reviewer",
        ];
        if action != "unavailable" {
            args.extend(["--run", run]);
        }
        if matches!(action, "confirm" | "supersede") {
            args.extend(["--assertion-index", "0"]);
        }
        self.revalidation_output = self.cli(&args);
        let revision = self.store.project_revision(&self.project).unwrap();
        self.when_the_authority_restarts();
        assert_eq!(self.cli(&args), self.revalidation_output);
        assert_eq!(
            self.store.project_revision(&self.project).unwrap(),
            revision
        );
        self.cli(&["project", "rebuild"]);
        self.revalidation_details = self.cli(&[
            "show",
            if action == "supersede" { "D2" } else { "D1" },
            "--source",
            "--history",
        ]);

        self
    }

    pub fn then_the_reviewed_support_outcome_is_durable(&mut self, action: &str) {
        assert_eq!(
            self.revalidation_details["policy_origin"]["input"]["revalidation"]["action"],
            action
        );
        if action != "unavailable" {
            assert!(
                self.store
                    .semantic_coverage(&self.project)
                    .unwrap()
                    .required_gaps
                    > 0,
                "hindsight cannot close live gaps"
            );
        }
        assert_eq!(self.revalidation_output["disposition"], "accepted");
        assert_eq!(
            self.store
                .pending_revalidation_count(&self.project)
                .unwrap(),
            0
        );
        let object = self
            .store
            .object(&self.project, &id("D1"))
            .unwrap()
            .unwrap();
        assert_eq!(
            object.lifecycle.as_str(),
            match action {
                "supersede" => "superseded",
                "invalidate" => "invalidated",
                _ => "active",
            }
        );
        assert_eq!(
            self.store
                .object_support_status(&self.project, &id("D1"))
                .unwrap(),
            if action == "confirm" {
                SupportStatus::Current
            } else {
                SupportStatus::Unsupported
            }
        );
        if action == "supersede" {
            assert_eq!(
                self.store
                    .object(&self.project, &id("D2"))
                    .unwrap()
                    .unwrap()
                    .payload,
                Some(id("sweep-body"))
            );
            assert_eq!(
                self.store
                    .object(&self.project, &id("D2"))
                    .unwrap()
                    .unwrap()
                    .lifecycle
                    .as_str(),
                "active"
            );
        }
        if action != "unavailable" {
            let impact = &self
                .store
                .evidence_impacts_for_object(&self.project, &id("D1"))
                .unwrap()[0];
            let status = self
                .store
                .compilation_run_status(&self.project, &impact.id)
                .unwrap()
                .unwrap();
            assert_eq!(status.mode, "hindsight");
            assert_eq!(
                self.store
                    .compilation_context_sources(&self.project, &impact.id)
                    .unwrap(),
                {
                    let mut sources = vec![
                        self.store
                            .latest_source_version(&self.project, &id("direct-v1"))
                            .unwrap(),
                        self.store
                            .latest_source_version(&self.project, &id("context-v1"))
                            .unwrap(),
                    ];
                    sources.sort_by_key(|v| {
                        self.store
                            .source_version(&self.project, v)
                            .unwrap()
                            .unwrap()
                            .sequence
                    });
                    sources
                }
            );
        }
    }

    pub fn when_pending_evidence_work_is_listed_through_the_cli(&mut self) -> &mut Self {
        let path = self
            .persisted_file
            .as_ref()
            .expect("file-backed authority")
            .0
            .to_str()
            .unwrap();
        let args: Vec<String> = [
            "project",
            "revalidation",
            "list",
            "--project",
            self.project.as_str(),
            "--database",
            path,
            "--json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let result = merl_cli::run_with_clock(&args, &|| Ok(NOW + 10));
        let merl_cli::CliResponse::Success(output) = result else {
            panic!("list pending revalidation: {result:?}")
        };
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["work"].as_array().unwrap().len(), 1);
        self.pending_impacts = self.store.pending_evidence_impacts(&self.project).unwrap();
        assert_eq!(value["work"][0]["impact"], self.pending_impacts[0].id);
        self
    }
}
