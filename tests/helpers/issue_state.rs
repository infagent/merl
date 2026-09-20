use merl_core::{
    ActorId, BatchId, DomainEvent, DomainEventBatch, EventId, ObjectId, ObjectKind,
    PolicyEvaluationId, PolicyInputId, PolicyVersion, ProjectId, Relation, RelationId,
    RelationKind,
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
                    lifecycle: merl_core::ObjectLifecycle::Active,
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
                    lifecycle: merl_core::ObjectLifecycle::Active,
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

    pub fn when_the_decision_is_linked_to_the_issue(&mut self) -> &mut Self {
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("relation-batch").expect("batch"),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 1_767_300_000_002,
                events: vec![DomainEvent::PutRelation {
                    id: EventId::try_from("relation-event").expect("event"),
                    relation: Relation {
                        id: RelationId::try_from("R1").expect("relation"),
                        project: self.project.clone(),
                        subject: ObjectId::try_from("D18").expect("decision"),
                        kind: RelationKind::try_from("addresses").expect("kind"),
                        object: self.issue.clone(),
                    },
                }],
            })
            .expect("accept relation");
        self.when_the_issue_state_is_read()
    }

    pub fn then_the_issue_view_lists_the_relation(&mut self) -> &mut Self {
        let state = self.state.as_ref().expect("state");
        assert_eq!(state.relations.len(), 1);
        assert_eq!(state.relations[0].relation.kind.as_str(), "addresses");
        self
    }

    pub fn when_an_authorized_command_links_the_decision(&mut self) -> &mut Self {
        let actor = ActorId::try_from("owner").expect("actor");
        let rules = PolicyRules {
            version: PolicyVersion::try_from("v1").expect("policy version"),
            decision_authors: vec![],
            command_actors: vec![actor.clone()],
            administrators: vec![],
        };
        let prepared = evaluate(
            &self.store,
            &self.project,
            &actor,
            PolicyEvaluationId::try_from("relation-eval").expect("evaluation"),
            BatchId::try_from("relation-policy-batch").expect("batch"),
            1_767_300_000_002,
            &rules,
            &[Proposal::Command {
                id: PolicyInputId::try_from("relation-command").expect("command"),
                event: DomainEvent::PutRelation {
                    id: EventId::try_from("relation-policy-event").expect("event"),
                    relation: Relation {
                        id: RelationId::try_from("R2").expect("relation"),
                        project: self.project.clone(),
                        subject: ObjectId::try_from("D18").expect("decision"),
                        kind: RelationKind::try_from("addresses").expect("kind"),
                        object: self.issue.clone(),
                    },
                },
            }],
        )
        .expect("evaluate relation command");
        prepared
            .commit(&mut self.store)
            .expect("accept relation command");
        self.when_the_issue_state_is_read()
    }

    pub fn then_the_relation_is_accepted_with_its_input_origin(&mut self) {
        let state = self.state.as_ref().expect("state");
        assert_eq!(state.relations.len(), 1);
        let origin = self
            .store
            .relation_policy_origin(
                &self.project,
                &RelationId::try_from("R2").expect("relation"),
            )
            .expect("origin lookup")
            .expect("origin");
        assert_eq!(origin.input.id().as_str(), "relation-command");
    }

    pub fn when_a_later_decision_supersedes_the_original(&mut self) -> &mut Self {
        self.store
            .commit_unchecked_bootstrap(&DomainEventBatch {
                id: BatchId::try_from("replacement-batch").expect("batch"),
                project: self.project.clone(),
                actor: ActorId::try_from("owner").expect("actor"),
                occurred_at_millis: 1_767_300_000_003,
                events: vec![
                    DomainEvent::PutObject {
                        id: EventId::try_from("replacement-old-event").expect("event"),
                        object: ObjectId::try_from("D18").expect("decision"),
                        kind: ObjectKind::try_from("decision").expect("kind"),
                        payload: None,
                        issue_scope: Some(self.scope.clone()),
                        lifecycle: merl_core::ObjectLifecycle::Superseded,
                    },
                    DomainEvent::PutObject {
                        id: EventId::try_from("replacement-new-event").expect("event"),
                        object: ObjectId::try_from("D19").expect("decision"),
                        kind: ObjectKind::try_from("decision").expect("kind"),
                        payload: None,
                        issue_scope: Some(self.scope.clone()),
                        lifecycle: merl_core::ObjectLifecycle::Active,
                    },
                    DomainEvent::PutRelation {
                        id: EventId::try_from("replacement-relation-event").expect("event"),
                        relation: Relation {
                            id: RelationId::try_from("R3").expect("relation"),
                            project: self.project.clone(),
                            subject: ObjectId::try_from("D19").expect("decision"),
                            kind: RelationKind::try_from("supersedes").expect("kind"),
                            object: ObjectId::try_from("D18").expect("decision"),
                        },
                    },
                ],
            })
            .expect("accept replacement");
        self.when_the_issue_state_is_read()
    }

    pub fn then_the_original_is_superseded_but_still_supported(&mut self) -> &mut Self {
        let state = self.state.as_ref().expect("state");
        let original = state
            .semantics
            .iter()
            .find(|item| item.id.as_str() == "D18")
            .expect("original");
        assert_eq!(original.lifecycle, merl_core::ObjectLifecycle::Superseded);
        assert_eq!(original.support, merl_store::SupportStatus::Current);
        let replacement = state
            .semantics
            .iter()
            .find(|item| item.id.as_str() == "D19")
            .expect("replacement");
        assert_eq!(replacement.lifecycle, merl_core::ObjectLifecycle::Active);
        assert!(
            state
                .relations
                .iter()
                .any(|item| item.relation.kind.as_str() == "supersedes")
        );
        self
    }
}
