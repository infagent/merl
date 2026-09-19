use merl_core::{ActorId, BatchId, DomainEvent, DomainEventBatch, EventId, ObjectId, ProjectId};
use merl_store::{PayloadRead, Store};

#[test]
fn accepted_history_rebuilds_and_a_protected_payload_can_disappear() {
    let mut project = ProjectScenario::new("P1");

    project
        .given_protected_statement("D1", b"Keep the baseline gain fixed")
        .when_an_accepted_batch_puts_the_decision("B1", "E1", "D1")
        .then_revision_is(1)
        .then_decision_is_at_revision("D1", 1)
        .when_projection_is_rebuilt()
        .then_decision_is_at_revision("D1", 1)
        .when_the_statement_is_erased()
        .then_history_still_names_the_decision()
        .then_statement_is_unavailable();
}

#[test]
fn identical_statements_in_two_projects_have_separate_erasure_fates() {
    let mut store = Store::open_in_memory().expect("open local store");
    let first = ProjectId::try_from("P1").expect("valid project ID");
    let second = ProjectId::try_from("P2").expect("valid project ID");
    let statement = merl_core::PayloadId::try_from("S1").expect("valid payload ID");
    store.create_project(&first).expect("create first project");
    store
        .create_project(&second)
        .expect("create second project");
    store
        .put_payload(&first, &statement, b"same protected bytes")
        .expect("capture first copy");
    store
        .put_payload(&second, &statement, b"same protected bytes")
        .expect("capture second copy");

    store
        .erase_payload(&first, &statement)
        .expect("erase first copy");

    assert_eq!(
        store
            .read_payload(&first, &statement)
            .expect("first status"),
        PayloadRead::Unavailable
    );
    assert_eq!(
        store
            .read_payload(&second, &statement)
            .expect("second status"),
        PayloadRead::Available(b"same protected bytes".to_vec())
    );
}

#[test]
fn a_bad_event_cannot_leave_half_an_accepted_batch() {
    let mut store = Store::open_in_memory().expect("open local store");
    let project = ProjectId::try_from("P1").expect("valid project ID");
    store.create_project(&project).expect("create project");
    let batch = DomainEventBatch {
        id: BatchId::try_from("B1").expect("valid batch ID"),
        project: project.clone(),
        actor: ActorId::try_from("alice").expect("valid actor ID"),
        occurred_at_millis: 1_700_000_000_000,
        events: ["first", "second"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| DomainEvent::PutObject {
                id: EventId::try_from(format!("E{index}").as_str()).expect("valid event ID"),
                object: ObjectId::try_from(name).expect("valid object ID"),
                kind: merl_core::ObjectKind::try_from("decision").expect("valid kind"),
                payload: (index == 1)
                    .then(|| merl_core::PayloadId::try_from("missing").expect("valid payload ID")),
            })
            .collect(),
    };

    assert!(store.commit(&batch).is_err());
    assert_eq!(store.project_revision(&project).expect("revision").get(), 0);
    assert_eq!(store.accepted_event_count(&project).expect("history"), 0);
    assert_eq!(
        store
            .object(
                &project,
                &ObjectId::try_from("first").expect("valid object ID")
            )
            .expect("projection"),
        None
    );
}

struct ProjectScenario {
    store: Store,
    project: ProjectId,
    statement: Option<merl_core::PayloadId>,
}

impl ProjectScenario {
    fn new(project: &str) -> Self {
        let mut store = Store::open_in_memory().expect("open local store");
        let project = ProjectId::try_from(project).expect("valid project ID");
        store.create_project(&project).expect("create project");
        Self {
            store,
            project,
            statement: None,
        }
    }

    fn given_protected_statement(&mut self, id: &str, text: &[u8]) -> &mut Self {
        let payload = merl_core::PayloadId::try_from(id).expect("valid payload ID");
        self.store
            .put_payload(&self.project, &payload, text)
            .expect("store protected statement");
        self.statement = Some(payload);
        self
    }

    fn when_an_accepted_batch_puts_the_decision(
        &mut self,
        batch: &str,
        event: &str,
        object: &str,
    ) -> &mut Self {
        let batch = DomainEventBatch {
            id: BatchId::try_from(batch).expect("valid batch ID"),
            project: self.project.clone(),
            actor: ActorId::try_from("alice").expect("valid actor ID"),
            occurred_at_millis: 1_700_000_000_000,
            events: vec![DomainEvent::PutObject {
                id: EventId::try_from(event).expect("valid event ID"),
                object: ObjectId::try_from(object).expect("valid object ID"),
                kind: merl_core::ObjectKind::try_from("decision").expect("valid kind"),
                payload: self.statement.clone(),
            }],
        };
        self.store.commit(&batch).expect("accept batch");
        self
    }

    fn then_revision_is(&mut self, expected: u64) -> &mut Self {
        assert_eq!(
            self.store
                .project_revision(&self.project)
                .expect("revision")
                .get(),
            expected
        );
        self
    }

    fn then_decision_is_at_revision(&mut self, id: &str, expected: u64) -> &mut Self {
        let object = ObjectId::try_from(id).expect("valid object ID");
        let decision = self
            .store
            .object(&self.project, &object)
            .expect("read object")
            .expect("decision exists");
        assert_eq!(decision.revision.get(), expected);
        assert_eq!(decision.payload, self.statement);
        self
    }

    fn when_projection_is_rebuilt(&mut self) -> &mut Self {
        self.store
            .rebuild_projection(&self.project)
            .expect("rebuild accepted state");
        self
    }

    fn when_the_statement_is_erased(&mut self) -> &mut Self {
        self.store
            .erase_payload(&self.project, self.statement.as_ref().expect("payload"))
            .expect("erase protected bytes");
        self
    }

    fn then_history_still_names_the_decision(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .accepted_event_count(&self.project)
                .expect("history"),
            1
        );
        self.then_decision_is_at_revision("D1", 1)
    }

    fn then_statement_is_unavailable(&mut self) -> &mut Self {
        assert_eq!(
            self.store
                .read_payload(&self.project, self.statement.as_ref().expect("payload"))
                .expect("read payload status"),
            PayloadRead::Unavailable
        );
        self
    }
}
