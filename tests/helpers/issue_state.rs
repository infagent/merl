use merl_core::{
    ActorId, BatchId, DomainEvent, DomainEventBatch, EventId, ObjectId, ObjectKind,
    PolicyEvaluationId, PolicyInputId, PolicyVersion, ProjectId,
};
use merl_policy::{PolicyRules, Proposal, evaluate};
use merl_store::{IssueState, Store};

pub struct IssueScenario {
    store: Store,
    project: ProjectId,
    issue: ObjectId,
    scope: String,
    state: Option<IssueState>,
    command_rejected: bool,
}

impl IssueScenario {
    pub fn given_an_imported_issue_with_a_local_decision() -> Self {
        let mut store = Store::open_in_memory().expect("store");
        let project = ProjectId::try_from("IssueScenario").expect("project");
        store.create_project(&project).expect("project");
        let fixture = merl_corpus::github::fixture_from_graphql_pages(
            "DEV-FAKE",
            "2026-01-02T00:00:00Z",
            include_bytes!("../fixtures/github_two_page_edit.json"),
        )
        .expect("fixture");
        let issue = merl_ingest::fixture_issue_id(&fixture).expect("issue ID");
        let scope = fixture.source.issue_provider_id.clone();
        merl_ingest::import_fixture(&mut store, &project, &fixture).expect("import Issue");
        store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("decision-batch").expect("batch"),
                project: project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 1_767_300_000_000,
                events: vec![DomainEvent::PutObject {
                    id: EventId::try_from("decision-event").expect("event"),
                    object: ObjectId::try_from("D18").expect("decision"),
                    kind: ObjectKind::try_from("decision").expect("kind"),
                    payload: None,
                    issue_scope: Some(scope.clone()),
                }],
            })
            .expect("accepted decision");
        Self {
            store,
            project,
            issue,
            scope,
            state: None,
            command_rejected: false,
        }
    }

    pub fn when_the_issue_state_is_read(&mut self) -> &mut Self {
        self.state = Some(
            self.store
                .issue_state(&self.project, &self.issue, &self.scope)
                .expect("Issue state"),
        );
        self
    }

    pub fn then_provider_facts_and_decision_are_separate(&mut self) -> &mut Self {
        let state = self.state.as_ref().expect("state");
        assert!(state.provider.is_some());
        assert_eq!(state.semantics.len(), 1);
        assert_eq!(state.semantics[0].id.as_str(), "D18");
        assert_eq!(state.semantics[0].kind.as_str(), "decision");
        self
    }

    pub fn when_the_projection_is_rebuilt(&mut self) -> &mut Self {
        self.store
            .rebuild_projection(&self.project)
            .expect("rebuild");
        self.when_the_issue_state_is_read()
    }

    pub fn when_a_command_tries_to_replace_the_provider_mirror(&mut self) -> &mut Self {
        let actor = ActorId::try_from("owner").expect("actor");
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("policy version"),
            decision_authors: vec![],
            command_actors: vec![actor.clone()],
            administrators: vec![],
        };
        self.command_rejected = evaluate(
            &self.store,
            &self.project,
            &actor,
            PolicyEvaluationId::try_from("invalid-provider-write").expect("evaluation"),
            BatchId::try_from("invalid-provider-batch").expect("batch"),
            1_767_300_000_001,
            &rules,
            &[Proposal::Command {
                id: PolicyInputId::try_from("invalid-provider-command").expect("command"),
                event: DomainEvent::PutObject {
                    id: EventId::try_from("invalid-provider-event").expect("event"),
                    object: self.issue.clone(),
                    kind: ObjectKind::try_from("provider_issue").expect("kind"),
                    payload: None,
                    issue_scope: None,
                },
            }],
        )
        .is_err();
        self
    }

    pub fn then_the_command_is_rejected_and_provider_state_is_unchanged(&mut self) -> &mut Self {
        assert!(self.command_rejected);
        let state = self
            .store
            .issue_state(&self.project, &self.issue, &self.scope)
            .expect("Issue state");
        assert!(state.provider.is_some());
        assert_eq!(state.semantics.len(), 1);
        self
    }
}
