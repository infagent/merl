ALTER TABLE domain_events ADD COLUMN issue_scope_id TEXT;
ALTER TABLE objects ADD COLUMN issue_scope_id TEXT;
CREATE INDEX objects_by_issue_scope ON objects(project_id, issue_scope_id, id);
