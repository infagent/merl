-- Each accepted policy version retains the selector snapshot used by later captures.
ALTER TABLE binding_policy_changes ADD COLUMN selectors TEXT NOT NULL DEFAULT '{}';

CREATE TABLE source_policy_selections (
    project_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    selection TEXT NOT NULL,
    PRIMARY KEY (project_id, source_version_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id)
) STRICT;
CREATE TRIGGER source_policy_selections_no_update BEFORE UPDATE ON source_policy_selections
BEGIN SELECT RAISE(ABORT, 'capture policy selections are immutable'); END;
CREATE TRIGGER source_policy_selections_no_delete BEFORE DELETE ON source_policy_selections
BEGIN SELECT RAISE(ABORT, 'capture policy selections are immutable'); END;
