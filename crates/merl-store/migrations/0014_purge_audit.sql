CREATE TABLE purge_intents (
    project_id TEXT NOT NULL REFERENCES projects(id),
    source_version_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    requested_at_millis INTEGER NOT NULL,
    reason_payload_id TEXT NOT NULL,
    preview_digest BLOB NOT NULL CHECK (length(preview_digest) = 32),
    PRIMARY KEY (project_id, source_version_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, reason_payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE TABLE purge_intent_payloads (
    project_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    payload_id TEXT NOT NULL,
    digest BLOB NOT NULL CHECK (length(digest) = 32),
    PRIMARY KEY (project_id, source_version_id, payload_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES purge_intents(project_id, source_version_id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE TABLE purge_completions (
    project_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    completed_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, source_version_id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES purge_intents(project_id, source_version_id)
) STRICT;

CREATE TRIGGER purge_intents_no_update BEFORE UPDATE ON purge_intents
BEGIN SELECT RAISE(ABORT, 'purge intent is append-only'); END;
CREATE TRIGGER purge_intents_no_delete BEFORE DELETE ON purge_intents
BEGIN SELECT RAISE(ABORT, 'purge intent is append-only'); END;
CREATE TRIGGER purge_intent_payloads_no_update BEFORE UPDATE ON purge_intent_payloads
BEGIN SELECT RAISE(ABORT, 'purge payload receipt is append-only'); END;
CREATE TRIGGER purge_intent_payloads_no_delete BEFORE DELETE ON purge_intent_payloads
BEGIN SELECT RAISE(ABORT, 'purge payload receipt is append-only'); END;
CREATE TRIGGER purge_completions_no_update BEFORE UPDATE ON purge_completions
BEGIN SELECT RAISE(ABORT, 'purge completion is append-only'); END;
CREATE TRIGGER purge_completions_no_delete BEFORE DELETE ON purge_completions
BEGIN SELECT RAISE(ABORT, 'purge completion is append-only'); END;
