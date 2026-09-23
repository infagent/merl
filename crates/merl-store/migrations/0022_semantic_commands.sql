-- Receipts retain structural proposals; prose has its own erasure boundary.
CREATE TABLE semantic_commands (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL,
    actor TEXT NOT NULL,
    operation TEXT NOT NULL CHECK (operation IN ('create', 'resolve', 'accept', 'defer', 'start', 'complete')),
    object_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    issue_scope TEXT,
    payload_id TEXT,
    reason_id TEXT,
    source_version_id TEXT,
    expected_revision INTEGER,
    request_digest BLOB NOT NULL CHECK (length(request_digest) = 32),
    occurred_at_millis INTEGER NOT NULL,
    commitment TEXT CHECK (commitment IN ('pending', 'accepted', 'declined')),
    scheduling TEXT CHECK (scheduling IN ('unscheduled', 'deferred', 'scheduled')),
    execution TEXT CHECK (execution IN ('not_started', 'in_progress', 'blocked', 'completed', 'cancelled')),
    review_at TEXT,
    rejection TEXT,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id),
    FOREIGN KEY (project_id, reason_id) REFERENCES payloads(project_id, id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id)
) STRICT;
CREATE UNIQUE INDEX semantic_command_source ON semantic_commands(project_id, source_version_id);
CREATE TRIGGER semantic_commands_no_update BEFORE UPDATE ON semantic_commands
BEGIN SELECT RAISE(ABORT, 'semantic commands are immutable'); END;
CREATE TRIGGER semantic_commands_no_delete BEFORE DELETE ON semantic_commands
BEGIN SELECT RAISE(ABORT, 'semantic commands are immutable'); END;

-- Preserve prior conflicts while adding the protected command-content guard.
CREATE TABLE policy_conflicts_v3 (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (reason_code IN
        ('basis_ahead', 'object_read_changed', 'kind_collection_changed',
         'object_write_changed', 'accepted_input_overlap', 'assertion_evidence_changed', 'command_content_unavailable')),
    target_id TEXT,
    expected_revision INTEGER,
    actual_revision INTEGER,
    PRIMARY KEY (project_id, evaluation_id),
    FOREIGN KEY (project_id, evaluation_id) REFERENCES policy_evaluations(project_id, id)
) STRICT;
INSERT INTO policy_conflicts_v3 SELECT * FROM policy_conflicts;
DROP TRIGGER policy_conflicts_no_update;
DROP TRIGGER policy_conflicts_no_delete;
DROP TABLE policy_conflicts;
ALTER TABLE policy_conflicts_v3 RENAME TO policy_conflicts;
CREATE TRIGGER policy_conflicts_no_update
BEFORE UPDATE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
CREATE TRIGGER policy_conflicts_no_delete
BEFORE DELETE ON policy_conflicts BEGIN SELECT RAISE(ABORT, 'policy conflict is append-only'); END;
