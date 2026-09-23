CREATE TABLE source_coverage_promotions (
    project_id TEXT NOT NULL REFERENCES projects(id),
    source_version_id TEXT NOT NULL,
    scope_id TEXT NOT NULL CHECK (length(scope_id) BETWEEN 1 AND 512),
    actor_id TEXT NOT NULL,
    reason_payload_id TEXT NOT NULL,
    reason_digest BLOB NOT NULL CHECK (length(reason_digest) = 32),
    promoted_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, source_version_id, scope_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, reason_payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE TRIGGER source_coverage_promotions_no_update
BEFORE UPDATE ON source_coverage_promotions
BEGIN SELECT RAISE(ABORT, 'source coverage promotion is append-only'); END;

CREATE TRIGGER source_coverage_promotions_no_delete
BEFORE DELETE ON source_coverage_promotions
BEGIN SELECT RAISE(ABORT, 'source coverage promotion is append-only'); END;
