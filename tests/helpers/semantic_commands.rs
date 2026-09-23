use serde_json::Value;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
const NOW: i64 = 1_700_000_000_000;

pub struct Commands {
    path: PathBuf,
    results: Vec<Value>,
    human: String,
}
impl Drop for Commands {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
impl Commands {
    pub fn given_a_project_with_a_command_actor() -> Self {
        let case = Self {
            path: std::env::temp_dir().join(format!(
                "merl-commands-{}-{}.db",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            results: Vec::new(),
            human: String::new(),
        };
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&case.path)
            .unwrap();
        let mut store = merl_store::Store::open(&case.path).unwrap();
        store.create_project(&"P1".try_into().unwrap()).unwrap();
        store
            .grant_administrator_unchecked_bootstrap(
                &"P1".try_into().unwrap(),
                &"admin".try_into().unwrap(),
            )
            .unwrap();
        drop(store);
        let result = case.cli(&[
            "project",
            "authority",
            "grant",
            "--actor",
            "admin",
            "--id",
            "grant",
            "--subject",
            "writer",
            "--permission",
            "command_actor",
            "--reason",
            "Project writer",
        ]);
        assert_eq!(result["outcome"], "accepted", "{result}");
        case
    }
    fn run(&self, args: &[&str], json: bool) -> String {
        let mut args: Vec<String> = args.iter().map(|s| (*s).into()).collect();
        args.extend([
            "--project".into(),
            "P1".into(),
            "--database".into(),
            self.path.to_str().unwrap().into(),
        ]);
        if json {
            args.push("--json".into());
        }
        match merl_cli::run_with_clock(&args, &|| Ok(NOW)) {
            merl_cli::CliResponse::Success(s) => s,
            merl_cli::CliResponse::JsonError(e) => e.as_json(),
            merl_cli::CliResponse::HumanError(e) => panic!("{e}"),
        }
    }
    fn cli(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.run(args, true)).unwrap()
    }
    pub fn when_decisions_and_research_objects_are_submitted_and_retried(mut self) -> Self {
        let command = [
            "decision",
            "create",
            "--subject",
            "D1",
            "--summary",
            "Keep receive gain fixed",
            "--actor",
            "writer",
            "--id",
            "create-decision",
        ];
        self.results.push(self.cli(&command));
        self.results.push(self.cli(&command));
        let mut changed = command;
        changed[5] = "Change receive gain";
        self.results.push(self.cli(&changed));
        self.results.push(self.cli(&[
            "decision",
            "create",
            "--subject",
            "D2",
            "--summary",
            "Unauthorized",
            "--actor",
            "stranger",
            "--id",
            "unauthorized",
        ]));
        self.results.push(self.cli(&["show", "D1", "--history"]));
        self.human = self.run(&command, false);
        for (kind, object) in [
            ("question", "Q1"),
            ("finding", "F1"),
            ("hypothesis", "H1"),
            ("claim", "C1"),
        ] {
            self.results.push(self.cli(&[
                kind,
                "create",
                "--subject",
                object,
                "--summary",
                "Retained semantic content",
                "--actor",
                "writer",
                "--id",
                object,
            ]));
        }
        self.results.push(self.cli(&[
            "question",
            "resolve",
            "Q1",
            "--summary",
            "Answered by the gain decision",
            "--actor",
            "writer",
            "--id",
            "resolve-question",
        ]));
        self
    }
    pub fn then_only_authorized_commands_change_state_and_retries_keep_their_outcome(self) {
        assert_eq!(self.results[0]["outcome"], "accepted", "{:?}", self.results);
        assert_eq!(self.results[0], self.results[1]);
        assert_eq!(self.results[2]["code"], "POLICY_INPUT_CONFLICT");
        assert_eq!(self.results[3]["outcome"], "rejected");
        assert!(self.results[3]["revision"].is_null());
        assert_eq!(self.results[4]["history"].as_array().unwrap().len(), 1);
        assert_eq!(self.results[4]["policy_origin"]["input"]["kind"], "command");
        assert!(self.results[4]["policy_origin"]["input"]["assertion"].is_null());
        assert!(self.human.contains("accepted"));
        for result in &self.results[5..] {
            assert_eq!(result["outcome"], "accepted", "{result}");
        }
    }
    pub fn when_a_task_is_requested_accepted_and_deferred(mut self) -> Self {
        self.results.push(self.cli(&[
            "task",
            "request",
            "--subject",
            "T1",
            "--summary",
            "Add clipping metadata",
            "--actor",
            "writer",
            "--id",
            "request-task",
        ]));
        self.results.push(self.cli(&["show", "T1"]));
        self.results.push(self.cli(&[
            "task",
            "accept",
            "T1",
            "--actor",
            "writer",
            "--id",
            "accept-task",
        ]));
        self.human = self.run(
            &[
                "task",
                "defer",
                "T1",
                "--reason",
                "Migration has priority",
                "--review-at",
                "2026-10-01",
                "--actor",
                "writer",
                "--id",
                "defer-task",
            ],
            false,
        );
        self.cli(&["project", "rebuild"]);
        self.results.push(self.cli(&["show", "T1"]));
        self.results.push(self.cli(&["project", "view"]));
        self
    }
    pub fn then_planning_facets_and_deferral_details_survive_rebuild(self) {
        assert_eq!(self.results[0]["outcome"], "accepted", "{:?}", self.results);
        let initial = &self.results[1]["task"];
        assert_eq!(initial["commitment"], "pending");
        assert_eq!(initial["scheduling"], "unscheduled");
        assert_eq!(initial["execution"], "not_started");
        let task = &self.results[3]["task"];
        assert_eq!(task["commitment"], "accepted");
        assert_eq!(task["scheduling"], "deferred");
        assert_eq!(task["execution"], "not_started");
        assert_eq!(task["reason"]["text"], "Migration has priority");
        assert_eq!(task["review_at"], "2026-10-01");
        assert!(self.human.contains("accepted") && self.human.contains("deferred"));
        let viewed = self.results[4]["objects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "T1")
            .unwrap();
        assert_eq!(viewed["task"]["commitment"], "accepted");
        assert_eq!(viewed["task"]["scheduling"], "deferred");
    }
    pub fn when_a_decision_with_a_note_is_created_and_inspected(mut self) -> Self {
        self.results.push(self.cli(&[
            "decision",
            "create",
            "--subject",
            "D1",
            "--summary",
            "Keep gain fixed",
            "--note",
            "Supplemental measurement details",
            "--actor",
            "writer",
            "--id",
            "with-note",
        ]));
        self.results.push(self.cli(&["project", "view"]));
        self.results.push(self.cli(&["show", "D1", "--source"]));
        if let Some(source) = self.results[0]["source"].as_str() {
            self.results
                .push(self.cli(&["source", "show", "--version", source]));
        }
        self
    }
    pub fn then_the_note_is_optional_evidence_of_one_command(self) {
        assert_eq!(self.results[0]["outcome"], "accepted", "{:?}", self.results);
        assert!(
            !self.results[1]
                .to_string()
                .contains("Supplemental measurement details")
        );
        assert_eq!(self.results[1]["coverage"]["required_gaps"], 0);
        assert!(
            self.results[2]
                .to_string()
                .contains("Supplemental measurement details")
        );
        assert_eq!(self.results[3]["semantic_origin"], "with-note");
        assert_eq!(self.results[3]["supplements"], "D1");
        assert_eq!(
            self.results[3]["body"]["text"],
            "Supplemental measurement details"
        );
    }
}

fn id<T: for<'a> TryFrom<&'a str>>(value: &str) -> T
where
    for<'a> <T as TryFrom<&'a str>>::Error: std::fmt::Debug,
{
    T::try_from(value).unwrap()
}
fn request() -> merl_policy::SemanticCommand {
    merl_policy::SemanticCommand {
        id: id("interrupted"),
        actor: id("writer"),
        operation: merl_store::CommandOperation::Create,
        object: id("D1"),
        kind: id("decision"),
        issue_scope: None,
        summary: Some("Keep gain fixed".into()),
        reason: None,
        review_at: None,
        note: Some("Measurements explain the choice".into()),
    }
}

pub struct RecoveryCases(Vec<(Commands, &'static str, merl_policy::PreparedPolicy)>);
impl RecoveryCases {
    pub fn given_received_commands_without_outcomes() -> Self {
        Self(
            [
                "unchanged",
                "revoked",
                "erased",
                "changed_target",
                "unrelated",
            ]
            .into_iter()
            .map(|change| {
                let case = Commands::given_a_project_with_a_command_actor();
                let prepared = merl_policy::prepare_semantic_command(
                    &mut merl_store::Store::open(&case.path).unwrap(),
                    &id("P1"),
                    &request(),
                    NOW,
                )
                .unwrap();
                (case, change, prepared)
            })
            .collect(),
        )
    }
    pub fn when_the_commands_resume_after_restart_and_dependency_changes(mut self) -> Self {
        for (case, change, prepared) in &mut self.0 {
            match *change {
                "revoked" => {
                    case.cli(&[
                        "project",
                        "authority",
                        "revoke",
                        "--actor",
                        "admin",
                        "--id",
                        "revoke",
                        "--subject",
                        "writer",
                        "--permission",
                        "command_actor",
                        "--reason",
                        "Access ended",
                    ]);
                }
                "erased" => {
                    let mut store = merl_store::Store::open(&case.path).unwrap();
                    let receipt = store
                        .semantic_command(&id("P1"), &id("interrupted"))
                        .unwrap()
                        .unwrap();
                    store
                        .erase_payload(&id("P1"), receipt.payload.as_ref().unwrap())
                        .unwrap();
                }
                "changed_target" | "unrelated" => {
                    case.cli(&[
                        "decision",
                        "create",
                        "--subject",
                        if *change == "changed_target" {
                            "D1"
                        } else {
                            "D2"
                        },
                        "--summary",
                        "Another decision",
                        "--actor",
                        "writer",
                        "--id",
                        "other",
                    ]);
                }
                _ => {}
            }
            // Exercise the transaction guard with the originally prepared work,
            // then restart and retry through the CLI's durable receipt path.
            if *change != "unchanged" {
                let _ = prepared.commit(&mut merl_store::Store::open(&case.path).unwrap());
            }
            let args = [
                "decision",
                "create",
                "--subject",
                "D1",
                "--summary",
                "Keep gain fixed",
                "--note",
                "Measurements explain the choice",
                "--actor",
                "writer",
                "--id",
                "interrupted",
            ];
            case.results.push(case.cli(&args));
            case.results.push(case.cli(&args));
            let mut altered = args;
            altered[5] = "Different content";
            case.results.push(case.cli(&altered));
            case.results.push(case.cli(&["show", "D1", "--source"]));
        }
        self
    }
    pub fn then_recovery_preserves_identity_and_checks_current_dependencies(self) {
        for (case, change, _) in self.0 {
            let expected = if matches!(change, "unchanged" | "unrelated") {
                "accepted"
            } else {
                "conflict"
            };
            assert_eq!(
                case.results[0]["outcome"], expected,
                "{change}: {:?}",
                case.results
            );
            assert_eq!(case.results[0], case.results[1]);
            assert_eq!(case.results[2]["code"], "POLICY_INPUT_CONFLICT");
            let store = merl_store::Store::open(&case.path).unwrap();
            let receipt = store
                .semantic_command(&id("P1"), &id("interrupted"))
                .unwrap()
                .unwrap();
            assert_eq!(store.source_observation_head(&id("P1")).unwrap(), 1);
            if change == "erased" {
                assert_eq!(
                    store
                        .read_payload(&id("P1"), receipt.payload.as_ref().unwrap())
                        .unwrap(),
                    merl_store::PayloadRead::Unavailable
                );
            }
        }
    }
}

struct Compiler;
impl merl_compiler::CompilerAdapter for Compiler {
    fn id(&self) -> &'static str {
        "command-test"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn model(&self) -> &'static str {
        "fixture"
    }
    fn prompt_digest(&self) -> [u8; 32] {
        [1; 32]
    }
    fn compile(
        &self,
        _: &[u8],
        _: merl_compiler::CompilerLimits,
    ) -> Result<Vec<u8>, merl_compiler::CompileError> {
        unreachable!("controlled response")
    }
}
impl Commands {
    pub fn when_a_supplement_is_compiled_with_repeated_and_additional_semantics(mut self) -> Self {
        let note = "Keep gain fixed. Also use double precision. Measurements support both. Correction: sweep the gain instead.";
        let created = self.cli(&[
            "decision",
            "create",
            "--subject",
            "D1",
            "--summary",
            "Keep gain fixed",
            "--note",
            note,
            "--actor",
            "writer",
            "--id",
            "note-decision",
        ]);
        let source: merl_core::SourceVersionId = id(created["source"].as_str().unwrap());
        self.cli(&[
            "project",
            "authority",
            "grant",
            "--actor",
            "admin",
            "--id",
            "grant-decisions",
            "--subject",
            "writer",
            "--permission",
            "decision_author",
            "--reason",
            "Decision owner",
        ]);
        let mut store = merl_store::Store::open(&self.path).unwrap();
        let project = id("P1");
        let limits = merl_compiler::CompilerLimits {
            input_bytes: 32768,
            output_bytes: 16384,
            output_tokens: 2048,
            assertions: 8,
            context_requests: 2,
            expansion_rounds: 1,
            payload_bytes: 8192,
            source_window: 1,
            objects: 8,
        };
        authorize_note_compilation(&mut store, &project, &source, limits);
        let prepared = merl_compiler::prepare_authorized_compilation(
            &mut store,
            &project,
            &source,
            &Compiler,
            merl_compiler::RunRequest {
                id: "note-run",
                limits,
                mode: merl_compiler::RunMode::Live,
                now_millis: NOW + 2,
            },
        )
        .unwrap()
        .unwrap();
        self.results.push(
            serde_json::from_slice(
                &merl_compiler::rebuild_recorded_context(&store, &project, "note-run", limits)
                    .unwrap()
                    .rendered,
            )
            .unwrap(),
        );
        store
            .put_payload(&project, &id("gain-correction"), b"Sweep the gain instead")
            .unwrap();
        let original_value = self.results[0]["sources"][0]["semantic_origin"]["value"]
            .as_str()
            .unwrap();
        let response = supplemental_assertions(source.as_str(), note, original_value);
        merl_compiler::record_compilation_result(
            &mut store,
            &project,
            &prepared,
            Ok(serde_json::to_vec(&response).unwrap()),
            NOW + 3,
        )
        .unwrap();
        drop(store);
        self.results.push(self.cli(&[
            "source",
            "apply",
            "--run",
            "note-run",
            "--actor",
            "writer",
            "--id",
            "apply-note",
        ]));
        self.results.push(self.cli(&["show", "D1", "--history"]));
        self.results.push(self.cli(&["show", "D2"]));
        self
    }
    pub fn then_policy_covers_the_original_action_and_keeps_added_evidence(self) -> Self {
        assert_eq!(
            self.results[0]["sources"][0]["semantic_origin"]["command"],
            "note-decision"
        );
        assert_eq!(
            self.results[0]["sources"][0]["semantic_origin"]["subject"],
            "D1"
        );
        let inputs = self.results[1]["inputs"].as_array().unwrap();
        assert_eq!(inputs[0]["outcome"], "duplicate", "{:?}", self.results);
        assert_eq!(inputs[0]["reason"], "covered_by_command");
        assert_eq!(inputs[1]["outcome"], "candidate");
        assert_eq!(inputs[1]["reason"], "possible_supplemental_duplicate");
        assert_eq!(inputs[2]["outcome"], "candidate");
        assert_eq!(self.results[2]["history"].as_array().unwrap().len(), 1);
        assert_eq!(self.results[3]["code"], "OBJECT_NOT_FOUND");
        self
    }
    pub fn when_the_additional_decision_is_reviewed(mut self) -> Self {
        let candidates = self.cli(&["candidate", "list"]);
        let candidate = candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["index"] == 1)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        self.results
            .push(self.cli(&["candidate", "show", candidate]));
        self.results.push(self.cli(&[
            "candidate",
            "accept",
            candidate,
            "--actor",
            "writer",
            "--id",
            "accept-separate-decision",
        ]));
        self.results
            .push(self.cli(&["show", "D2", "--source", "--history"]));
        self
    }
    pub fn then_review_accepts_the_separate_decision_with_its_own_evidence(self) {
        assert_eq!(self.results[4]["status"], "pending");
        assert_eq!(self.results[4]["assertion"]["subject"], "D2");
        assert_eq!(self.results[5]["outcome"], "accepted");
        assert_eq!(self.results[6]["history"].as_array().unwrap().len(), 1);
        assert_eq!(
            self.results[6]["evidence_history"][0]["assertion"]["evidence"]["text"],
            "Also use double precision"
        );
    }
    pub fn then_corrective_requests_for_the_original_subject_remain_candidates(self) -> Self {
        let inputs = self.results[1]["inputs"].as_array().unwrap();
        for input in &inputs[3..5] {
            assert_eq!(input["outcome"], "candidate", "{input}");
            assert_eq!(input["reason"], "supplemental_correction");
        }
        assert_eq!(self.results[2]["history"].as_array().unwrap().len(), 1);
        self
    }
    pub fn when_the_original_decision_is_corrected(mut self) -> Self {
        let candidates = self.cli(&["candidate", "list"]);
        let candidate = candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["index"] == 3)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        self.results.push(self.cli(&[
            "candidate",
            "correct",
            candidate,
            "--actor",
            "writer",
            "--id",
            "correct-gain",
            "--subject",
            "D1",
            "--kind",
            "decision",
            "--value",
            "gain-correction",
            "--reason",
            "The note corrects the original gain setting",
        ]));
        self.results
            .push(self.cli(&["show", "D1", "--source", "--history"]));
        self.results
            .push(self.cli(&["candidate", "show", candidate]));
        self
    }
    pub fn then_review_preserves_the_original_and_accepts_the_correction(self) {
        assert_eq!(
            self.results[4]["outcome"], "accepted",
            "{:?}",
            self.results[4]
        );
        let decision = &self.results[5];
        assert_eq!(decision["content"]["text"], "Sweep the gain instead");
        let history = decision["history"].as_array().unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.contains(&self.results[2]["history"][0]));
        assert_eq!(
            decision["evidence_history"][0]["assertion"]["evidence"]["text"],
            "Correction: sweep the gain instead"
        );
        assert_eq!(self.results[6]["assertion"]["subject"], "D1");
        assert_eq!(self.results[6]["assertion"]["value"], "gain-correction");
    }
    pub fn when_invalid_planning_transitions_and_previews_are_submitted(mut self) -> Self {
        self.results.push(self.cli(&[
            "task",
            "request",
            "--subject",
            "preview-task",
            "--summary",
            "Preview",
            "--note",
            "Preview note",
            "--actor",
            "writer",
            "--id",
            "preview",
            "--dry-run",
        ]));
        self.results.push(self.cli(&["show", "preview-task"]));
        self.cli(&[
            "task",
            "request",
            "--subject",
            "T1",
            "--summary",
            "Do the work",
            "--actor",
            "writer",
            "--id",
            "task-request",
        ]);
        self.results.push(self.cli(&[
            "task",
            "start",
            "T1",
            "--actor",
            "writer",
            "--id",
            "premature-start",
        ]));
        self.cli(&[
            "task",
            "accept",
            "T1",
            "--actor",
            "writer",
            "--id",
            "task-accept",
        ]);
        self.cli(&[
            "task",
            "start",
            "T1",
            "--actor",
            "writer",
            "--id",
            "task-start",
        ]);
        self.results.push(self.cli(&[
            "task",
            "defer",
            "T1",
            "--reason",
            "Later",
            "--review-at",
            "2026-10-01",
            "--actor",
            "writer",
            "--id",
            "bad-defer",
        ]));
        self.results.push(self.cli(&["show", "T1"]));
        self.results.push(self.cli(&["help", "task", "defer"]));
        self
    }
    pub fn then_previews_are_read_only_and_invalid_transitions_leave_state_unchanged(self) {
        assert_eq!(self.results[0]["outcome"], "accepted");
        assert_eq!(self.results[1]["code"], "OBJECT_NOT_FOUND");
        assert_eq!(self.results[2]["outcome"], "rejected");
        assert_eq!(self.results[3]["outcome"], "rejected");
        assert_eq!(self.results[3]["reason"], "task_in_progress");
        assert_eq!(self.results[4]["task"]["execution"], "in_progress");
        assert_eq!(self.results[5]["schema"], "merl.help/v1");
        let store = merl_store::Store::open(&self.path).unwrap();
        assert!(
            store
                .semantic_command(&id("P1"), &id("preview"))
                .unwrap()
                .is_none()
        );
        assert_eq!(store.source_observation_head(&id("P1")).unwrap(), 0);
    }
}

impl Commands {
    pub fn when_a_noted_question_is_resolved_and_its_note_is_erased(mut self) -> Self {
        let args = [
            "question",
            "create",
            "--subject",
            "Q1",
            "--summary",
            "Which gain?",
            "--note",
            "Original measurements",
            "--actor",
            "writer",
            "--id",
            "original-question",
        ];
        let created = self.cli(&args);
        self.cli(&[
            "question",
            "resolve",
            "Q1",
            "--summary",
            "Use fixed gain",
            "--actor",
            "writer",
            "--id",
            "answer-question",
        ]);
        self.results
            .push(self.cli(&["show", "Q1", "--history", "--source"]));
        let mut store = merl_store::Store::open(&self.path).unwrap();
        let source = store
            .source_version(&id("P1"), &id(created["source"].as_str().unwrap()))
            .unwrap()
            .unwrap();
        store
            .erase_payload(&id("P1"), source.payload.as_ref().unwrap())
            .unwrap();
        drop(store);
        self.results.push(self.cli(&args));
        self.results
            .push(self.cli(&["show", "Q1", "--history", "--source"]));
        self
    }
    pub fn then_history_keeps_both_commands_and_retry_does_not_restore_the_note(self) {
        assert_eq!(self.results[0]["status"], "resolved");
        assert_eq!(
            self.results[0]["command_history"][0]["note"]["text"],
            "Original measurements"
        );
        assert!(self.results[0]["command_history"][0]["batch"].is_string());
        assert_eq!(self.results[0]["content"]["text"], "Use fixed gain");
        assert_eq!(self.results[1]["outcome"], "accepted");
        assert_eq!(
            self.results[2]["command_history"][0]["note"]["status"],
            "unavailable"
        );
        assert_eq!(self.results[2]["history"].as_array().unwrap().len(), 2);
    }
}

fn authorize_note_compilation(
    store: &mut merl_store::Store,
    project: &merl_core::ProjectId,
    source: &merl_core::SourceVersionId,
    limits: merl_compiler::CompilerLimits,
) {
    use merl_compiler::CompilerAdapter;
    let intent = store
        .prepare_compilation_authorization(
            project,
            source,
            "note-run",
            b"Inspect added evidence",
            merl_store::CompilationAuthorizationConfig {
                compiler_id: Compiler.id(),
                compiler_version: Compiler.version(),
                model_id: Compiler.model(),
                prompt_digest: Compiler.prompt_digest(),
                adapter_config_digest: Compiler.configuration_digest(),
                limits: limits.as_array(),
            },
        )
        .unwrap();
    merl_policy::apply_current(
        store,
        project,
        &id("admin"),
        id("authorize-note"),
        id("authorize-note-batch"),
        NOW + 1,
        &[merl_policy::Proposal::AdministrativeAction {
            id: id("authorize-note"),
            event: merl_core::DomainEvent::PutObject {
                id: id("authorize-note-event"),
                object: intent.object,
                kind: id("source_compilation_request"),
                payload: Some(intent.reason),
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            },
        }],
    )
    .unwrap();
}

impl Commands {
    pub fn when_a_project_note_yields_a_reviewed_finding(self) -> Self {
        let mut case = self.when_a_supplement_is_compiled_with_repeated_and_additional_semantics();
        let candidates = case.cli(&["candidate", "list"]);
        let candidate = candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["index"] == 2)
            .unwrap()["id"]
            .as_str()
            .unwrap();
        case.results.push(case.cli(&[
            "candidate",
            "accept",
            candidate,
            "--actor",
            "writer",
            "--id",
            "review-finding",
        ]));
        case
    }
    pub fn then_the_finding_has_no_invented_issue_scope(self) {
        assert_eq!(self.results.last().unwrap()["outcome"], "accepted");
        let store = merl_store::Store::open(&self.path).unwrap();
        assert_eq!(
            store
                .object(&id("P1"), &id("F1"))
                .unwrap()
                .unwrap()
                .issue_scope,
            None
        );
    }
}

fn supplemental_assertions(source: &str, note: &str, original_value: &str) -> Value {
    let assertion = |subject: &str, kind: &str, act: &str, text: &str, value: &str| {
        let start = note.find(text).unwrap();
        serde_json::json!({
            "source": source, "span_start": start, "span_end": start + text.len(),
            "subject": subject, "predicate": kind, "value": value, "act": act,
            "epistemic_basis": "reported", "polarity": "positive",
            "confidence_millis": 900, "attributed_to": null
        })
    };
    serde_json::json!({"schema": "merl.compiler-response/v1", "assertions": [
        assertion("D1", "decision", "request", "Keep gain fixed", original_value),
        assertion("D2", "decision", "request", "Also use double precision", "none"),
        assertion("F1", "finding", "report", "Measurements support both", "none"),
        assertion("D1", "decision", "request", "Correction: sweep the gain instead", "gain-correction"),
        assertion("D1", "decision", "request", "Correction: sweep the gain instead", "none")
    ]})
}
