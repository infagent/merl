use merl_compiler::{
    CompileError, CompilerAdapter, CompilerLimits, RunMode, RunRequest, execute_compilation,
    prepare_compilation, record_compilation_result,
};
use merl_core::{
    ActorId, AgentId, BatchId, CompilationMode, CoverageRequirement, DomainEvent, EventId,
    ObjectId, ObjectKind, PolicyEvaluationId, PolicyInputId, PolicyVersion, ProjectId, Relation,
    RelationId, RelationKind, SourceVersionId,
};
use merl_policy::{PolicyRules, Proposal, evaluate};
use merl_store::{SourceBinding, SourceCapture, Store};
use sha2::{Digest, Sha256};
use std::{io::ErrorKind, path::PathBuf, process::Command};

pub struct ReadScenario {
    directory: TestDirectory,
    issue: String,
    source_version: String,
    result: Option<serde_json::Value>,
}

impl ReadScenario {
    pub fn given_an_imported_issue() -> Self {
        let directory = TestDirectory::new();
        let database = directory.path.join("project.sqlite");
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");
        let fixture_data: merl_corpus::fixture::Fixture =
            serde_json::from_slice(&std::fs::read(&fixture).expect("fixture bytes"))
                .expect("fixture");
        let issue = merl_ingest::fixture_issue_id(&fixture_data)
            .expect("Issue identity")
            .to_string();
        let source_version =
            merl_ingest::fixture_version_id(&fixture_data.observations[0].version_id)
                .expect("source version")
                .to_string();
        for arguments in [
            vec![
                "project",
                "init",
                "--id",
                "P1",
                "--database",
                database.to_str().unwrap(),
            ],
            vec![
                "issue",
                "import-fixture",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--fixture",
                fixture.to_str().unwrap(),
            ],
        ] {
            let output = Command::new(env!("CARGO_BIN_EXE_merl"))
                .args(arguments)
                .output()
                .expect("run Merl");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Self {
            directory,
            issue,
            source_version,
            result: None,
        }
    }

    pub fn when_the_issue_view_is_requested(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "issue",
                "view",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--issue",
                &self.issue,
                "--scope",
                "controlled:DEV-C3:issue",
                "--json",
            ])
            .output()
            .expect("view Issue");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.result = Some(serde_json::from_slice(&output.stdout).expect("JSON view"));
        self
    }

    pub fn then_the_view_separates_revision_from_coverage_and_keeps_source_text_cold(self) {
        let view = self.result.expect("view");
        assert_eq!(view["schema"], "merl.issue-view/v1");
        assert_eq!(view["project_revision"], 1);
        assert_eq!(view["coverage"]["observation_head"], 4);
        assert_eq!(view["coverage"]["required_gaps"], 4);
        assert_eq!(view["coverage"]["processed_through"], 0);
        assert!(!view.to_string().contains("report A must use Parquet"));
    }

    pub fn when_a_source_is_expanded(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "source",
                "show",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--version",
                &self.source_version,
                "--json",
            ])
            .output()
            .expect("expand source");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.result = Some(serde_json::from_slice(&output.stdout).expect("source JSON"));
        self
    }

    pub fn then_the_captured_body_is_returned(self) -> Self {
        let result = self.result.as_ref().expect("source result");
        assert_eq!(result["schema"], "merl.source/v1");
        assert_eq!(result["body"]["status"], "available");
        assert!(
            result["body"]["text"]
                .as_str()
                .expect("text")
                .contains("export CSV")
        );
        self
    }

    pub fn when_its_bytes_are_erased_and_the_source_is_expanded_again(self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let version =
            merl_core::SourceVersionId::try_from(self.source_version.as_str()).expect("version");
        let source = store
            .source_version(&project, &version)
            .expect("source")
            .expect("source exists");
        store
            .erase_payload(&project, source.payload.as_ref().expect("body payload"))
            .expect("erase");
        self.when_a_source_is_expanded()
    }

    pub fn then_the_source_is_unavailable(self) {
        let result = self.result.expect("source result");
        assert_eq!(result["body"]["status"], "unavailable");
        assert!(result["body"].get("text").is_none());
    }

    pub fn when_an_agent_subscribes_and_polls(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        for command in ["subscribe", "poll"] {
            let output = Command::new(env!("CARGO_BIN_EXE_merl"))
                .args([
                    "inbox",
                    command,
                    "--project",
                    "P1",
                    "--database",
                    database.to_str().unwrap(),
                    "--agent",
                    "dev",
                    "--json",
                ])
                .output()
                .expect("inbox command");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            self.result = Some(serde_json::from_slice(&output.stdout).expect("inbox JSON"));
        }
        self
    }

    pub fn then_the_agent_starts_after_existing_history(self) {
        let poll = self.result.expect("poll");
        assert_eq!(poll["schema"], "merl.inbox/v1");
        assert_eq!(poll["cursor"], 1);
        assert!(poll["entries"].as_array().unwrap().is_empty());
    }
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        for index in 0..1_000 {
            let path =
                std::env::temp_dir().join(format!("merl-read-{}-{index}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create test directory: {error}"),
            }
        }
        panic!("could not reserve a test directory")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).expect("remove test directory");
    }
}

pub struct InboxScenario {
    directory: TestDirectory,
    poll: Option<serde_json::Value>,
    acknowledgements: Vec<serde_json::Value>,
    expansion: Option<serde_json::Value>,
    role_views: Vec<serde_json::Value>,
    delta: Option<serde_json::Value>,
    batch_page: Option<serde_json::Value>,
    post_ack_page: Option<serde_json::Value>,
}

impl InboxScenario {
    pub fn given_an_agent_subscribed_before_a_decision() -> Self {
        let directory = TestDirectory::new();
        let database = directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let agent = AgentId::try_from("dev").expect("agent");
        let actor = ActorId::try_from("owner").expect("owner");
        store.create_project(&project).expect("project");
        store.subscribe_all(&project, &agent).expect("subscription");
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("version"),
            decision_authors: vec![],
            command_actors: vec![actor.clone()],
            administrators: vec![],
        };
        let prepared = evaluate(
            &store,
            &project,
            &actor,
            PolicyEvaluationId::try_from("decision-eval").expect("evaluation"),
            BatchId::try_from("decision-batch").expect("batch"),
            1,
            &rules,
            &[
                Proposal::Command {
                    id: PolicyInputId::try_from("decision-command").expect("command"),
                    event: DomainEvent::PutObject {
                        id: EventId::try_from("decision-event").expect("event"),
                        object: ObjectId::try_from("D18").expect("object"),
                        kind: ObjectKind::try_from("decision").expect("kind"),
                        payload: None,
                        issue_scope: None,
                        lifecycle: merl_core::ObjectLifecycle::Active,
                    },
                },
                Proposal::Command {
                    id: PolicyInputId::try_from("hypothesis-command").expect("command"),
                    event: DomainEvent::PutObject {
                        id: EventId::try_from("hypothesis-event").expect("event"),
                        object: ObjectId::try_from("H4").expect("object"),
                        kind: ObjectKind::try_from("hypothesis").expect("kind"),
                        payload: None,
                        issue_scope: None,
                        lifecycle: merl_core::ObjectLifecycle::Active,
                    },
                },
            ],
        )
        .expect("evaluate");
        prepared.commit(&mut store).expect("accept decision");
        Self {
            directory,
            poll: None,
            acknowledgements: Vec::new(),
            expansion: None,
            role_views: Vec::new(),
            delta: None,
            batch_page: None,
            post_ack_page: None,
        }
    }

    pub fn when_the_agent_polls(mut self) -> Self {
        self.poll = Some(self.command(&["inbox", "poll", "--agent", "dev"]));
        self
    }

    pub fn then_one_entry_names_the_changed_decision_without_source_prose(self) -> Self {
        let poll = self.poll.as_ref().expect("poll");
        assert_eq!(poll["schema"], "merl.inbox/v1");
        assert_eq!(poll["cursor"], 0);
        assert_eq!(poll["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(poll["entries"][0]["revision"], 1);
        assert_eq!(poll["entries"][0]["changes"][0]["ref"], "D18");
        assert!(!poll.to_string().contains("source_body"));
        self
    }

    pub fn when_the_agent_acknowledges_it_twice(mut self) -> Self {
        self.acknowledgements = vec![
            self.command(&["inbox", "ack", "--agent", "dev", "--revision", "1"]),
            self.command(&["inbox", "ack", "--agent", "dev", "--revision", "1"]),
            self.command(&["inbox", "poll", "--agent", "dev"]),
        ];
        self
    }

    pub fn then_the_cursor_stays_at_the_decision_revision(self) {
        assert_eq!(self.acknowledgements[0]["cursor"], 1);
        assert_eq!(self.acknowledgements[1]["cursor"], 1);
        assert_eq!(self.acknowledgements[2]["cursor"], 1);
        assert!(
            self.acknowledgements[2]["entries"]
                .as_array()
                .expect("entries")
                .is_empty()
        );
    }

    pub fn when_the_decision_is_expanded(mut self) -> Self {
        self.expansion = Some(self.command(&["show", "D18", "--history"]));
        self
    }

    pub fn then_its_command_and_history_are_visible(self) {
        let expansion = self.expansion.expect("expansion");
        assert_eq!(expansion["schema"], "merl.object/v1");
        assert_eq!(expansion["policy_origin"]["input"]["kind"], "command");
        assert_eq!(
            expansion["policy_origin"]["input"]["id"],
            "decision-command"
        );
        assert_eq!(expansion["history"][0]["revision"], 1);
        assert_eq!(expansion["history"][0]["event"], "decision-event");
        assert_eq!(expansion["history"][0]["input"]["id"], "decision-command");
    }

    pub fn when_researcher_engineer_and_pm_views_are_requested(mut self) -> Self {
        self.role_views = vec![
            self.command(&["project", "view", "--role", "researcher"]),
            self.command(&["project", "view", "--role", "engineer"]),
            self.command(&["project", "view", "--role", "pm"]),
        ];
        self
    }

    pub fn then_each_view_has_its_own_order_and_the_same_revision(self) {
        let [researcher, engineer, pm] = self.role_views.as_slice() else {
            panic!("three role views")
        };
        assert_eq!(researcher["schema"], "merl.project-view/v1");
        assert_eq!(researcher["project_revision"], 1);
        assert_eq!(engineer["project_revision"], 1);
        assert_eq!(pm["project_revision"], 1);
        assert_eq!(researcher["objects"][0]["id"], "H4");
        assert_eq!(engineer["objects"][0]["id"], "D18");
        assert_eq!(pm["objects"][0]["id"], "D18");
        assert_eq!(
            researcher["objects"].as_array().unwrap().len(),
            engineer["objects"].as_array().unwrap().len()
        );
    }

    pub fn when_the_project_delta_is_requested_since_zero(mut self) -> Self {
        self.delta = Some(self.command(&["project", "delta", "--since", "0"]));
        self
    }

    pub fn then_the_delta_names_the_accepted_batch_and_refs(self) {
        let delta = self.delta.expect("delta");
        assert_eq!(delta["schema"], "merl.delta/v1");
        assert_eq!(delta["since"], 0);
        assert_eq!(delta["batches"].as_array().unwrap().len(), 1);
        assert_eq!(delta["batches"][0]["batch"], "decision-batch");
        assert_eq!(delta["batches"][0]["changes"][0]["ref"], "D18");
    }

    pub fn when_a_large_batch_is_accepted_and_the_delta_is_requested(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let actor = ActorId::try_from("owner").expect("actor");
        let proposals = (0..25)
            .map(|index| Proposal::Command {
                id: PolicyInputId::try_from(format!("large-command-{index}").as_str())
                    .expect("command"),
                event: DomainEvent::PutObject {
                    id: EventId::try_from(format!("event-{index}").as_str()).expect("event"),
                    object: ObjectId::try_from(format!("T{index}").as_str()).expect("object"),
                    kind: ObjectKind::try_from("task").expect("kind"),
                    payload: None,
                    issue_scope: None,
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            })
            .collect::<Vec<_>>();
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("version"),
            decision_authors: vec![],
            command_actors: vec![actor.clone()],
            administrators: vec![],
        };
        let prepared = evaluate(
            &store,
            &project,
            &actor,
            PolicyEvaluationId::try_from("large-eval").expect("evaluation"),
            BatchId::try_from("large-batch").expect("batch"),
            2,
            &rules,
            &proposals,
        )
        .expect("evaluate");
        prepared.commit(&mut store).expect("large accepted batch");
        self.delta = Some(self.command(&["project", "delta", "--since", "1"]));
        self
    }

    pub fn then_the_delta_is_bounded_and_marked_truncated(self) {
        let delta = self.delta.expect("delta");
        assert_eq!(delta["batches"].as_array().unwrap().len(), 1);
        assert_eq!(delta["batches"][0]["changes"].as_array().unwrap().len(), 20);
        assert_eq!(delta["batches"][0]["changes_truncated"], true);
    }

    pub fn when_a_large_batch_is_accepted_and_every_page_is_read(self) -> Self {
        let mut scenario = self.when_a_large_batch_is_accepted_and_the_delta_is_requested();
        scenario.poll = Some(scenario.command(&["inbox", "poll", "--agent", "dev"]));
        scenario.batch_page = Some(scenario.command(&[
            "inbox",
            "show",
            "--agent",
            "dev",
            "--revision",
            "2",
            "--offset",
            "20",
        ]));
        scenario.acknowledgements = vec![
            scenario.command(&["inbox", "ack", "--agent", "dev", "--revision", "1"]),
            scenario.command(&["inbox", "ack", "--agent", "dev", "--revision", "2"]),
        ];
        scenario.post_ack_page = Some(scenario.command(&[
            "inbox",
            "show",
            "--agent",
            "dev",
            "--revision",
            "2",
            "--offset",
            "20",
        ]));
        scenario
    }

    pub fn then_all_references_are_observable_and_the_batch_can_be_acknowledged(self) {
        let first = &self.poll.as_ref().expect("poll")["entries"][1];
        let second = self.batch_page.as_ref().expect("second page");
        assert_eq!(first["changes"].as_array().unwrap().len(), 20);
        assert_eq!(first["changes_truncated"], true);
        assert_eq!(second["changes"].as_array().unwrap().len(), 5);
        assert_eq!(second["next_offset"], serde_json::Value::Null);
        let mut refs: Vec<_> = first["changes"]
            .as_array()
            .unwrap()
            .iter()
            .chain(second["changes"].as_array().unwrap())
            .map(|change| change["ref"].as_str().unwrap().to_owned())
            .collect();
        refs.sort();
        refs.dedup();
        assert_eq!(refs.len(), 25);
        assert_eq!(self.acknowledgements[1]["cursor"], 2);
        assert_eq!(self.post_ack_page.as_ref().expect("page after ack"), second);
    }

    pub fn when_many_low_priority_objects_and_one_task_are_viewed_as_pm(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let mut events: Vec<_> = (0..101)
            .map(|index| DomainEvent::PutObject {
                id: EventId::try_from(format!("low-event-{index}").as_str()).expect("event"),
                object: ObjectId::try_from(format!("A{index:03}").as_str()).expect("object"),
                kind: ObjectKind::try_from("hypothesis").expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            })
            .collect();
        events.push(DomainEvent::PutObject {
            id: EventId::try_from("task-event").expect("event"),
            object: ObjectId::try_from("T999").expect("object"),
            kind: ObjectKind::try_from("task").expect("kind"),
            payload: None,
            issue_scope: None,
            lifecycle: merl_core::ObjectLifecycle::Active,
        });
        store
            .commit_unchecked_bootstrap(&merl_core::DomainEventBatch {
                id: BatchId::try_from("priority-batch").expect("batch"),
                project: ProjectId::try_from("P1").expect("project"),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 2,
                events,
            })
            .expect("accepted batch");
        self.role_views = vec![self.command(&["project", "view", "--role", "pm"])];
        self
    }

    pub fn then_the_task_is_in_the_bounded_pm_view(self) {
        let view = &self.role_views[0];
        assert!(
            view["objects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|object| object["id"] == "T999")
        );
        assert_eq!(view["truncated"], true);
    }

    pub fn when_two_engineers_focus_on_different_tasks(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let mut events = (0..30)
            .map(|index| DomainEvent::PutObject {
                id: EventId::try_from(format!("task-event-{index}").as_str()).expect("event"),
                object: ObjectId::try_from(format!("T{index}").as_str()).expect("object"),
                kind: ObjectKind::try_from("task").expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            })
            .collect::<Vec<_>>();
        for (task, blocker) in [("T17", "B9"), ("T24", "B10")] {
            events.push(DomainEvent::PutObject {
                id: EventId::try_from(format!("{blocker}-event").as_str()).expect("event"),
                object: ObjectId::try_from(blocker).expect("blocker"),
                kind: ObjectKind::try_from("blocker").expect("kind"),
                payload: None,
                issue_scope: None,
                lifecycle: merl_core::ObjectLifecycle::Active,
            });
            events.push(DomainEvent::PutRelation {
                id: EventId::try_from(format!("{task}-blocked-event").as_str()).expect("event"),
                relation: Relation {
                    id: RelationId::try_from(format!("{task}-blocked").as_str()).expect("relation"),
                    project: ProjectId::try_from("P1").expect("project"),
                    subject: ObjectId::try_from(task).expect("task"),
                    kind: RelationKind::try_from("blocked_by").expect("kind"),
                    object: ObjectId::try_from(blocker).expect("blocker"),
                },
            });
        }
        store
            .commit_unchecked_bootstrap(&merl_core::DomainEventBatch {
                id: BatchId::try_from("engineer-tasks").expect("batch"),
                project: ProjectId::try_from("P1").expect("project"),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 2,
                events,
            })
            .expect("accepted batch");
        self.role_views = vec![
            self.command(&["project", "view", "--role", "engineer", "--focus", "T17"]),
            self.command(&["project", "view", "--role", "engineer", "--focus", "T24"]),
        ];
        self
    }

    pub fn then_each_engineer_sees_their_task_first(self) {
        assert_eq!(self.role_views[0]["objects"][0]["id"], "T17");
        assert_eq!(self.role_views[0]["objects"][1]["id"], "B9");
        assert_eq!(self.role_views[1]["objects"][0]["id"], "T24");
        assert_eq!(self.role_views[1]["objects"][1]["id"], "B10");
        assert_eq!(self.role_views[0]["role"], self.role_views[1]["role"]);
        assert_eq!(
            self.role_views[0]["project_revision"],
            self.role_views[1]["project_revision"]
        );
        assert_eq!(self.role_views[0]["coverage_scope"], "project");
        assert_eq!(self.role_views[1]["coverage_scope"], "project");
    }

    fn command(&self, command: &[&str]) -> serde_json::Value {
        let database = self.directory.path.join("project.sqlite");
        let mut args = command.to_vec();
        args.extend([
            "--project",
            "P1",
            "--database",
            database.to_str().unwrap(),
            "--json",
        ]);
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(&args)
            .output()
            .expect("run Merl");
        assert!(
            output.status.success(),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice(&output.stdout).expect("JSON result")
    }
}

struct DecisionCompiler;

impl CompilerAdapter for DecisionCompiler {
    fn id(&self) -> &'static str {
        "read-test"
    }
    fn version(&self) -> &'static str {
        "v1"
    }
    fn model(&self) -> &'static str {
        "deterministic"
    }
    fn prompt_digest(&self) -> [u8; 32] {
        Sha256::digest(b"read-test").into()
    }
    fn compile(&self, _context: &[u8], _limits: CompilerLimits) -> Result<Vec<u8>, CompileError> {
        Ok(br#"{"schema":"merl.compiler-response/v1","assertions":[{"source":"comment-v1","span_start":0,"span_end":14,"subject":"D1","predicate":"decision","value":"none","act":"request","epistemic_basis":"reported","polarity":"positive","confidence_millis":900,"attributed_to":null}]}"#.to_vec())
    }
}

pub struct AssertionScenario {
    directory: TestDirectory,
    expansion: Option<serde_json::Value>,
    human_expansion: Option<String>,
    poll: Option<serde_json::Value>,
}

impl AssertionScenario {
    #[expect(
        clippy::too_many_lines,
        reason = "the Given step builds one complete provenance chain through public APIs"
    )]
    pub fn given_a_decision_from_a_captured_comment() -> Self {
        let directory = TestDirectory::new();
        let database = directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let alice = ActorId::try_from("alice").expect("actor");
        let version = SourceVersionId::try_from("comment-v1").expect("version");
        store.create_project(&project).expect("project");
        store
            .subscribe_all(&project, &AgentId::try_from("dev").expect("agent"))
            .expect("subscribe");
        store
            .capture_source_version(
                &project,
                &SourceCapture {
                    binding: SourceBinding {
                        id: merl_core::SourceBindingId::try_from("binding").expect("binding"),
                        provider: merl_core::SourceProvider::try_from("github").expect("provider"),
                        provider_namespace_id: "repo-1".into(),
                        namespace_digest: Sha256::digest(b"repo-1").into(),
                    },
                    source: merl_core::SourceId::try_from("comment").expect("source"),
                    provider_entity_id: "comment-1",
                    context_scope_id: "issue-1",
                    version: version.clone(),
                    provider_version_id: "comment-v1",
                    kind: merl_core::SourceKind::try_from("issue_comment").expect("kind"),
                    supersedes: None,
                    ambiguous_order_with_previous: false,
                    created_at_millis: 1,
                    occurred_at_millis: 1,
                    upstream_updated_at_millis: Some(1),
                    observed_at_millis: 1,
                    actor: Some(alice.clone()),
                    provider_actor_id: Some("alice"),
                    source_author: Some(alice.clone()),
                    provider_source_author_id: Some("alice"),
                    body: Some(b"Use fixed gain. A second sentence."),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::Eager,
                    coverage_requirement: CoverageRequirement::Required,
                    policy_version: merl_core::CapturePolicyVersion::try_from("v1")
                        .expect("policy version"),
                },
            )
            .expect("capture source");
        let limits = CompilerLimits {
            input_bytes: 4096,
            output_bytes: 4096,
            output_tokens: 512,
            assertions: 2,
            context_requests: 1,
            expansion_rounds: 1,
            payload_bytes: 2048,
            source_window: 2,
            objects: 4,
        };
        let compiler = DecisionCompiler;
        let prepared = prepare_compilation(
            &mut store,
            &project,
            &version,
            &compiler,
            RunRequest {
                id: "decision-run",
                limits,
                mode: RunMode::Live,
                now_millis: 2,
            },
        )
        .expect("prepare")
        .expect("new run");
        let response = execute_compilation(&prepared, &compiler);
        record_compilation_result(&mut store, &project, &prepared, response, 3)
            .expect("record assertion");
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("version"),
            decision_authors: vec![alice.clone()],
            command_actors: vec![],
            administrators: vec![],
        };
        let accepted = evaluate(
            &store,
            &project,
            &alice,
            PolicyEvaluationId::try_from("decision-eval").expect("evaluation"),
            BatchId::try_from("decision-batch").expect("batch"),
            4,
            &rules,
            &[Proposal::ObservedAssertion {
                id: PolicyInputId::try_from("decision-assertion").expect("input"),
                run: merl_core::CompilationRunId::try_from("decision-run").expect("run"),
                index: 0,
                event: DomainEvent::PutObject {
                    id: EventId::try_from("decision-event").expect("event"),
                    object: ObjectId::try_from("D1").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: None,
                    issue_scope: Some("issue-1".into()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            }],
        )
        .expect("evaluate");
        accepted.commit(&mut store).expect("accept");
        Self {
            directory,
            expansion: None,
            human_expansion: None,
            poll: None,
        }
    }

    pub fn when_the_decision_source_is_requested(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "show",
                "D1",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--source",
                "--history",
                "--json",
            ])
            .output()
            .expect("expand decision");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.expansion = Some(serde_json::from_slice(&output.stdout).expect("JSON expansion"));
        let human = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "show",
                "D1",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--source",
            ])
            .output()
            .expect("human expansion");
        assert!(
            human.status.success(),
            "{}",
            String::from_utf8_lossy(&human.stderr)
        );
        self.human_expansion = Some(String::from_utf8(human.stdout).expect("human UTF-8"));
        self
    }

    pub fn then_the_assertion_and_its_exact_source_span_are_shown(self) {
        let value = self.expansion.expect("expansion");
        let assertion = &value["policy_origin"]["input"]["assertion"];
        assert_eq!(assertion["source_version"], "comment-v1");
        assert_eq!(assertion["span"]["start"], 0);
        assert_eq!(assertion["span"]["end"], 14);
        assert_eq!(assertion["evidence"]["text"], "Use fixed gain");
        assert_eq!(assertion["asserted_by"], "alice");
        assert!(!value.to_string().contains("A second sentence"));
        assert!(
            self.human_expansion
                .expect("human expansion")
                .contains("Evidence: Use fixed gain")
        );
    }

    pub fn when_a_command_changes_the_decision_and_its_source_is_requested(self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let mut store = Store::open(&database).expect("store");
        let project = ProjectId::try_from("P1").expect("project");
        let actor = ActorId::try_from("alice").expect("actor");
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("version"),
            decision_authors: vec![],
            command_actors: vec![actor.clone()],
            administrators: vec![],
        };
        let prepared = evaluate(
            &store,
            &project,
            &actor,
            PolicyEvaluationId::try_from("later-eval").expect("evaluation"),
            BatchId::try_from("later-batch").expect("batch"),
            5,
            &rules,
            &[Proposal::Command {
                id: PolicyInputId::try_from("later-command").expect("command"),
                event: DomainEvent::PutObject {
                    id: EventId::try_from("later-event").expect("event"),
                    object: ObjectId::try_from("D1").expect("object"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: None,
                    issue_scope: Some("issue-1".into()),
                    lifecycle: merl_core::ObjectLifecycle::Active,
                },
            }],
        )
        .expect("evaluate");
        prepared.commit(&mut store).expect("accept later command");
        self.when_the_decision_source_is_requested()
    }

    pub fn then_the_original_assertion_span_is_still_expandable(self) {
        let value = self.expansion.expect("expansion");
        assert_eq!(value["policy_origin"]["input"]["kind"], "command");
        assert_eq!(value["history"][0]["input"]["run"], "decision-run");
        assert_eq!(value["history"][0]["input"]["index"], 0);
        assert_eq!(
            value["evidence_history"][0]["assertion"]["evidence"]["text"],
            "Use fixed gain"
        );
        assert!(
            self.human_expansion
                .expect("human")
                .contains("Evidence: Use fixed gain")
        );
    }

    pub fn when_the_subscriber_polls(mut self) -> Self {
        let database = self.directory.path.join("project.sqlite");
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "inbox",
                "poll",
                "--project",
                "P1",
                "--database",
                database.to_str().unwrap(),
                "--agent",
                "dev",
                "--json",
            ])
            .output()
            .expect("poll inbox");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.poll = Some(serde_json::from_slice(&output.stdout).expect("JSON inbox"));
        self
    }

    pub fn then_only_the_accepted_reference_is_delivered(self) {
        let poll = self.poll.expect("poll");
        assert_eq!(poll["entries"].as_array().unwrap().len(), 1);
        assert_eq!(poll["entries"][0]["changes"][0]["ref"], "D1");
        assert_eq!(poll["coverage"]["required_gaps"], 0);
        assert!(!poll.to_string().contains("Use fixed gain"));
    }
}
