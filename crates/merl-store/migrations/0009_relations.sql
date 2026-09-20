CREATE TABLE relation_events (
    project_id TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    id TEXT NOT NULL,
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    relation_id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, batch_id, event_index),
    FOREIGN KEY (project_id, batch_id) REFERENCES domain_event_batches(project_id, id)
) STRICT;

CREATE TABLE relations (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    object_id TEXT NOT NULL,
    relation_revision INTEGER NOT NULL CHECK (relation_revision > 0),
    project_revision INTEGER NOT NULL CHECK (project_revision > 0),
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, subject_id) REFERENCES objects(project_id, id),
    FOREIGN KEY (project_id, object_id) REFERENCES objects(project_id, id)
) STRICT;

CREATE INDEX relations_by_subject ON relations(project_id, subject_id, id);
CREATE INDEX relations_by_object ON relations(project_id, object_id, id);

CREATE TABLE policy_evaluation_writes_v2 (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    expected_revision INTEGER,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('object', 'relation')),
    PRIMARY KEY (project_id, evaluation_id, target_kind, object_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;
INSERT INTO policy_evaluation_writes_v2
    (project_id,evaluation_id,object_id,expected_revision,target_kind)
    SELECT project_id,evaluation_id,object_id,expected_revision,'object'
    FROM policy_evaluation_writes;
DROP TRIGGER policy_evaluation_writes_no_update;
DROP TRIGGER policy_evaluation_writes_no_delete;
DROP TABLE policy_evaluation_writes;
ALTER TABLE policy_evaluation_writes_v2 RENAME TO policy_evaluation_writes;
CREATE TRIGGER policy_evaluation_writes_no_update BEFORE UPDATE ON policy_evaluation_writes
BEGIN SELECT RAISE(ABORT, 'policy write is append-only'); END;
CREATE TRIGGER policy_evaluation_writes_no_delete BEFORE DELETE ON policy_evaluation_writes
BEGIN SELECT RAISE(ABORT, 'policy write is append-only'); END;

CREATE TABLE policy_evaluation_relation_events (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    input_index INTEGER NOT NULL CHECK (input_index >= 0),
    PRIMARY KEY (project_id, evaluation_id, event_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id),
    FOREIGN KEY (project_id, event_id) REFERENCES relation_events(project_id, id)
) STRICT;

CREATE TRIGGER relation_events_no_update BEFORE UPDATE ON relation_events
BEGIN SELECT RAISE(ABORT, 'relation event is append-only'); END;
CREATE TRIGGER relation_events_no_delete BEFORE DELETE ON relation_events
BEGIN SELECT RAISE(ABORT, 'relation event is append-only'); END;
CREATE TRIGGER policy_evaluation_relation_events_no_update
BEFORE UPDATE ON policy_evaluation_relation_events
BEGIN SELECT RAISE(ABORT, 'policy relation link is append-only'); END;
CREATE TRIGGER policy_evaluation_relation_events_no_delete
BEFORE DELETE ON policy_evaluation_relation_events
BEGIN SELECT RAISE(ABORT, 'policy relation link is append-only'); END;
