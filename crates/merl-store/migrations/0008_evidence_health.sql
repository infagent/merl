CREATE TABLE evidence_supports (
    project_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    PRIMARY KEY (project_id, event_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, event_id) REFERENCES domain_events(project_id, id)
) STRICT;

CREATE INDEX evidence_supports_by_source
    ON evidence_supports(project_id, source_version_id, object_id);

-- An impact doubles as a durable revalidation intent. A resolution is appended separately.
CREATE TABLE evidence_impacts (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    support_event_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    replacement_version_id TEXT,
    next_action TEXT NOT NULL CHECK (next_action IN ('recompile', 'reevaluate')),
    recorded_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, support_event_id) REFERENCES domain_events(project_id, id),
    FOREIGN KEY (project_id, replacement_version_id) REFERENCES source_versions(project_id, id)
) STRICT;

CREATE TABLE evidence_revalidations (
    project_id TEXT NOT NULL,
    impact_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('current', 'unsupported')),
    PRIMARY KEY (project_id, impact_id),
    FOREIGN KEY (project_id, impact_id) REFERENCES evidence_impacts(project_id, id),
    FOREIGN KEY (project_id, event_id) REFERENCES domain_events(project_id, id)
) STRICT;

CREATE TRIGGER evidence_supports_no_update BEFORE UPDATE ON evidence_supports
BEGIN SELECT RAISE(ABORT, 'evidence support is append-only'); END;
CREATE TRIGGER evidence_supports_no_delete BEFORE DELETE ON evidence_supports
BEGIN SELECT RAISE(ABORT, 'evidence support is append-only'); END;
CREATE TRIGGER evidence_impacts_no_update BEFORE UPDATE ON evidence_impacts
BEGIN SELECT RAISE(ABORT, 'evidence impact is append-only'); END;
CREATE TRIGGER evidence_impacts_no_delete BEFORE DELETE ON evidence_impacts
BEGIN SELECT RAISE(ABORT, 'evidence impact is append-only'); END;
CREATE TRIGGER evidence_revalidations_no_update BEFORE UPDATE ON evidence_revalidations
BEGIN SELECT RAISE(ABORT, 'evidence revalidation is append-only'); END;
CREATE TRIGGER evidence_revalidations_no_delete BEFORE DELETE ON evidence_revalidations
BEGIN SELECT RAISE(ABORT, 'evidence revalidation is append-only'); END;
