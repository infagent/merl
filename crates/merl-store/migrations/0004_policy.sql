CREATE TABLE policy_evaluations (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    basis_project_revision INTEGER NOT NULL CHECK (basis_project_revision >= 0),
    evaluation_digest BLOB NOT NULL CHECK (length(evaluation_digest) = 32),
    batch_id TEXT,
    committed_revision INTEGER,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, batch_id) REFERENCES domain_event_batches(project_id, id),
    CHECK ((batch_id IS NULL) = (committed_revision IS NULL))
) STRICT;

CREATE TABLE policy_evaluation_inputs (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    input_index INTEGER NOT NULL CHECK (input_index >= 0),
    input_kind TEXT NOT NULL CHECK (input_kind IN
        ('observed_assertion', 'command', 'provider_observation', 'administrative_action')),
    input_id TEXT NOT NULL,
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    disposition TEXT NOT NULL CHECK (disposition IN
        ('accepted', 'candidate', 'rejected', 'duplicate', 'conflict')),
    reason_code TEXT NOT NULL,
    PRIMARY KEY (project_id, evaluation_id, input_index),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE INDEX policy_input_history ON policy_evaluation_inputs(project_id, input_kind, input_id);

-- The first receipt fixes a command's meaning even when policy rejects it.
CREATE TABLE policy_input_receipts (
    project_id TEXT NOT NULL,
    input_kind TEXT NOT NULL,
    input_id TEXT NOT NULL,
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    first_evaluation_id TEXT NOT NULL,
    PRIMARY KEY (project_id, input_kind, input_id),
    FOREIGN KEY (project_id, first_evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE TABLE policy_assertion_inputs (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    input_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    assertion_index INTEGER NOT NULL CHECK (assertion_index >= 0),
    PRIMARY KEY (project_id, evaluation_id, input_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id),
    FOREIGN KEY (project_id, run_id, assertion_index)
        REFERENCES observed_assertions(project_id, run_id, assertion_index)
) STRICT;

CREATE TABLE accepted_policy_inputs (
    project_id TEXT NOT NULL,
    input_kind TEXT NOT NULL,
    input_id TEXT NOT NULL,
    input_digest BLOB NOT NULL CHECK (length(input_digest) = 32),
    evaluation_id TEXT NOT NULL,
    PRIMARY KEY (project_id, input_kind, input_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

-- An assertion cannot create two accepted transitions under different input handles.
CREATE TABLE accepted_assertions (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    assertion_index INTEGER NOT NULL CHECK (assertion_index >= 0),
    evaluation_id TEXT NOT NULL,
    PRIMARY KEY (project_id, run_id, assertion_index),
    FOREIGN KEY (project_id, run_id, assertion_index)
        REFERENCES observed_assertions(project_id, run_id, assertion_index),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE TABLE policy_evaluation_reads (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    read_index INTEGER NOT NULL CHECK (read_index >= 0),
    read_kind TEXT NOT NULL CHECK (read_kind IN ('object', 'kind_collection')),
    target_id TEXT NOT NULL,
    expected_revision INTEGER,
    PRIMARY KEY (project_id, evaluation_id, read_index),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE TABLE policy_evaluation_writes (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    expected_revision INTEGER,
    PRIMARY KEY (project_id, evaluation_id, object_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE TABLE policy_evaluation_domain_events (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (project_id, evaluation_id, event_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id),
    FOREIGN KEY (project_id, event_id) REFERENCES domain_events(project_id, id)
) STRICT;

CREATE TABLE inbox_subscriptions (
    project_id TEXT NOT NULL REFERENCES projects(id),
    agent_id TEXT NOT NULL,
    PRIMARY KEY (project_id, agent_id)
) STRICT;

CREATE TABLE inbox_entries (
    project_id TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_revision INTEGER NOT NULL CHECK (project_revision > 0),
    PRIMARY KEY (project_id, batch_id, agent_id),
    FOREIGN KEY (project_id, batch_id) REFERENCES domain_event_batches(project_id, id),
    FOREIGN KEY (project_id, agent_id) REFERENCES inbox_subscriptions(project_id, agent_id)
) STRICT;

CREATE TRIGGER policy_evaluations_no_update
BEFORE UPDATE ON policy_evaluations BEGIN SELECT RAISE(ABORT, 'policy evaluation is append-only'); END;
CREATE TRIGGER policy_evaluations_no_delete
BEFORE DELETE ON policy_evaluations BEGIN SELECT RAISE(ABORT, 'policy evaluation is append-only'); END;
CREATE TRIGGER policy_evaluation_inputs_no_update
BEFORE UPDATE ON policy_evaluation_inputs BEGIN SELECT RAISE(ABORT, 'policy input decision is append-only'); END;
CREATE TRIGGER policy_evaluation_inputs_no_delete
BEFORE DELETE ON policy_evaluation_inputs BEGIN SELECT RAISE(ABORT, 'policy input decision is append-only'); END;
CREATE TRIGGER policy_input_receipts_no_update
BEFORE UPDATE ON policy_input_receipts BEGIN SELECT RAISE(ABORT, 'policy input receipt is append-only'); END;
CREATE TRIGGER policy_input_receipts_no_delete
BEFORE DELETE ON policy_input_receipts BEGIN SELECT RAISE(ABORT, 'policy input receipt is append-only'); END;
CREATE TRIGGER policy_assertion_inputs_no_update
BEFORE UPDATE ON policy_assertion_inputs BEGIN SELECT RAISE(ABORT, 'policy assertion link is append-only'); END;
CREATE TRIGGER policy_assertion_inputs_no_delete
BEFORE DELETE ON policy_assertion_inputs BEGIN SELECT RAISE(ABORT, 'policy assertion link is append-only'); END;
CREATE TRIGGER accepted_policy_inputs_no_update
BEFORE UPDATE ON accepted_policy_inputs BEGIN SELECT RAISE(ABORT, 'accepted policy input is append-only'); END;
CREATE TRIGGER accepted_policy_inputs_no_delete
BEFORE DELETE ON accepted_policy_inputs BEGIN SELECT RAISE(ABORT, 'accepted policy input is append-only'); END;
CREATE TRIGGER accepted_assertions_no_update
BEFORE UPDATE ON accepted_assertions BEGIN SELECT RAISE(ABORT, 'accepted assertion is append-only'); END;
CREATE TRIGGER accepted_assertions_no_delete
BEFORE DELETE ON accepted_assertions BEGIN SELECT RAISE(ABORT, 'accepted assertion is append-only'); END;
CREATE TRIGGER policy_evaluation_reads_no_update
BEFORE UPDATE ON policy_evaluation_reads BEGIN SELECT RAISE(ABORT, 'policy read is append-only'); END;
CREATE TRIGGER policy_evaluation_reads_no_delete
BEFORE DELETE ON policy_evaluation_reads BEGIN SELECT RAISE(ABORT, 'policy read is append-only'); END;
CREATE TRIGGER policy_evaluation_writes_no_update
BEFORE UPDATE ON policy_evaluation_writes BEGIN SELECT RAISE(ABORT, 'policy write is append-only'); END;
CREATE TRIGGER policy_evaluation_writes_no_delete
BEFORE DELETE ON policy_evaluation_writes BEGIN SELECT RAISE(ABORT, 'policy write is append-only'); END;
CREATE TRIGGER policy_evaluation_domain_events_no_update
BEFORE UPDATE ON policy_evaluation_domain_events BEGIN SELECT RAISE(ABORT, 'policy event link is append-only'); END;
CREATE TRIGGER policy_evaluation_domain_events_no_delete
BEFORE DELETE ON policy_evaluation_domain_events BEGIN SELECT RAISE(ABORT, 'policy event link is append-only'); END;
