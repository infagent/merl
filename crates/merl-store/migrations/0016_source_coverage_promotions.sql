CREATE TABLE project_administrators (
    project_id TEXT NOT NULL REFERENCES projects(id),
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128 AND actor_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    PRIMARY KEY (project_id, actor_id)
) STRICT;

CREATE TABLE source_coverage_targets (
    project_id TEXT NOT NULL REFERENCES projects(id),
    object_id TEXT NOT NULL CHECK (length(object_id) BETWEEN 1 AND 128 AND object_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    source_id TEXT NOT NULL CHECK (length(source_id) BETWEEN 1 AND 128 AND source_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    scope_id TEXT NOT NULL CHECK (length(scope_id) BETWEEN 1 AND 512),
    reason_payload_id TEXT NOT NULL,
    reason_digest BLOB NOT NULL CHECK (length(reason_digest) = 32),
    PRIMARY KEY (project_id, object_id),
    UNIQUE (project_id, source_id, scope_id),
    FOREIGN KEY (project_id, reason_payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE TRIGGER project_administrators_no_update
BEFORE UPDATE ON project_administrators
BEGIN SELECT RAISE(ABORT, 'project administrator grant is append-only'); END;

CREATE TRIGGER project_administrators_no_delete
BEFORE DELETE ON project_administrators
BEGIN SELECT RAISE(ABORT, 'project administrator grant is append-only'); END;

CREATE TRIGGER source_coverage_targets_no_update
BEFORE UPDATE ON source_coverage_targets
BEGIN SELECT RAISE(ABORT, 'source coverage target is append-only'); END;

CREATE TRIGGER source_coverage_targets_no_delete
BEFORE DELETE ON source_coverage_targets
BEGIN SELECT RAISE(ABORT, 'source coverage target is append-only'); END;
