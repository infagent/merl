CREATE TABLE policy_evaluation_inputs_v2 (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    input_index INTEGER NOT NULL CHECK (input_index >= 0),
    input_kind TEXT NOT NULL CHECK (input_kind IN
        ('observed_assertion', 'observed_relation', 'command', 'provider_observation', 'administrative_action')),
    input_id TEXT NOT NULL,
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    disposition TEXT NOT NULL CHECK (disposition IN
        ('accepted', 'candidate', 'rejected', 'duplicate', 'conflict')),
    reason_code TEXT NOT NULL,
    PRIMARY KEY (project_id, evaluation_id, input_index),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

INSERT INTO policy_evaluation_inputs_v2 SELECT * FROM policy_evaluation_inputs;
DROP TRIGGER policy_evaluation_inputs_no_update;
DROP TRIGGER policy_evaluation_inputs_no_delete;
DROP TABLE policy_evaluation_inputs;
ALTER TABLE policy_evaluation_inputs_v2 RENAME TO policy_evaluation_inputs;
CREATE INDEX policy_input_history ON policy_evaluation_inputs(project_id,input_kind,input_id);
CREATE TRIGGER policy_evaluation_inputs_no_update BEFORE UPDATE ON policy_evaluation_inputs
BEGIN SELECT RAISE(ABORT,'policy input decision is append-only'); END;
CREATE TRIGGER policy_evaluation_inputs_no_delete BEFORE DELETE ON policy_evaluation_inputs
BEGIN SELECT RAISE(ABORT,'policy input decision is append-only'); END;

CREATE TABLE observed_relations (
 project_id TEXT NOT NULL, run_id TEXT NOT NULL, relation_index INTEGER NOT NULL,
 subject_id TEXT, predicate TEXT, object_id TEXT,
 subject_assertion INTEGER, subject_revision INTEGER,
 object_assertion INTEGER, object_revision INTEGER,
 rejection TEXT,
 PRIMARY KEY(project_id,run_id,relation_index),
 FOREIGN KEY(project_id,run_id) REFERENCES compilation_runs(project_id,id)
) STRICT;
CREATE TABLE policy_relation_inputs (
 project_id TEXT NOT NULL, evaluation_id TEXT NOT NULL, input_id TEXT NOT NULL,
 run_id TEXT NOT NULL, relation_index INTEGER NOT NULL,
 PRIMARY KEY(project_id,evaluation_id,input_id),
 FOREIGN KEY(project_id,evaluation_id) REFERENCES policy_evaluations(project_id,id),
 FOREIGN KEY(project_id,run_id,relation_index) REFERENCES observed_relations(project_id,run_id,relation_index)
) STRICT;
CREATE TABLE relation_reviews (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    original_evaluation_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    relation_index INTEGER NOT NULL CHECK (relation_index >= 0),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('accept','reject','correct')),
    object_id TEXT,
    object_kind TEXT,
    payload_id TEXT,
    reason_payload_id TEXT,
    PRIMARY KEY (project_id,id),
    FOREIGN KEY (project_id,original_evaluation_id) REFERENCES policy_evaluations(project_id,id),
    FOREIGN KEY (project_id,run_id,relation_index) REFERENCES observed_relations(project_id,run_id,relation_index),
    FOREIGN KEY (project_id,payload_id) REFERENCES payloads(project_id,id),
    FOREIGN KEY (project_id,reason_payload_id) REFERENCES payloads(project_id,id),
    CHECK ((action='correct') = (object_id IS NOT NULL AND object_kind IS NOT NULL)),
    CHECK (action='accept' OR reason_payload_id IS NOT NULL)
) STRICT;
CREATE TRIGGER observed_relations_no_update BEFORE UPDATE ON observed_relations
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;
CREATE TRIGGER observed_relations_no_delete BEFORE DELETE ON observed_relations
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;
CREATE TRIGGER policy_relation_inputs_no_update BEFORE UPDATE ON policy_relation_inputs
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;
CREATE TRIGGER policy_relation_inputs_no_delete BEFORE DELETE ON policy_relation_inputs
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;
CREATE TRIGGER relation_reviews_no_update BEFORE UPDATE ON relation_reviews
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;
CREATE TRIGGER relation_reviews_no_delete BEFORE DELETE ON relation_reviews
BEGIN SELECT RAISE(ABORT,'compiler relation history is append-only'); END;

CREATE TABLE policy_conflicts_v4 (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (reason_code IN
        ('basis_ahead', 'object_read_changed', 'kind_collection_changed',
         'object_write_changed', 'accepted_input_overlap', 'assertion_evidence_changed', 'command_content_unavailable', 'relation_evidence_changed', 'relation_write_changed')),
    target_id TEXT,
    expected_revision INTEGER,
    actual_revision INTEGER,
    PRIMARY KEY (project_id, evaluation_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;
INSERT INTO policy_conflicts_v4 SELECT * FROM policy_conflicts;
DROP TRIGGER policy_conflicts_no_update;
DROP TRIGGER policy_conflicts_no_delete;
DROP TABLE policy_conflicts;
ALTER TABLE policy_conflicts_v4 RENAME TO policy_conflicts;
CREATE TRIGGER policy_conflicts_no_update
BEFORE UPDATE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
CREATE TRIGGER policy_conflicts_no_delete
BEFORE DELETE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
