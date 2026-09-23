-- Targets are structural intent. Only an accepted authority_grant object grants permission.
CREATE TABLE authority_grant_targets (
    project_id TEXT NOT NULL REFERENCES projects(id),
    object_id TEXT NOT NULL,
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128 AND actor_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    permission TEXT NOT NULL CHECK (permission IN ('decision_author','command_actor')),
    PRIMARY KEY (project_id, object_id),
    UNIQUE (project_id, actor_id, permission)
) STRICT;

CREATE TRIGGER authority_grant_targets_no_update
BEFORE UPDATE ON authority_grant_targets
BEGIN SELECT RAISE(ABORT, 'authority grant target is immutable'); END;

CREATE TRIGGER authority_grant_targets_no_delete
BEFORE DELETE ON authority_grant_targets
BEGIN SELECT RAISE(ABORT, 'authority grant target is immutable'); END;
