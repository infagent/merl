CREATE TABLE policy_conflicts (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (reason_code IN
        ('basis_ahead', 'object_read_changed', 'kind_collection_changed', 'object_write_changed')),
    target_id TEXT,
    expected_revision INTEGER,
    actual_revision INTEGER,
    PRIMARY KEY (project_id, evaluation_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;

CREATE TRIGGER policy_conflicts_no_update
BEFORE UPDATE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
CREATE TRIGGER policy_conflicts_no_delete
BEFORE DELETE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
