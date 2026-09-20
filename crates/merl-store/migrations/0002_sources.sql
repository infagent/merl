ALTER TABLE projects ADD COLUMN source_observation_head INTEGER NOT NULL DEFAULT 0
    CHECK (source_observation_head >= 0);

CREATE TABLE source_bindings (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 128 AND provider NOT GLOB '*[^A-Za-z0-9_-]*'),
    provider_namespace_id TEXT NOT NULL CHECK (length(provider_namespace_id) BETWEEN 1 AND 512),
    namespace_digest BLOB NOT NULL CHECK (length(namespace_digest) = 32),
    PRIMARY KEY (project_id, id)
) STRICT;

CREATE TABLE source_versions (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    source_id TEXT NOT NULL CHECK (length(source_id) BETWEEN 1 AND 128 AND source_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    provider_entity_id TEXT NOT NULL CHECK (length(provider_entity_id) BETWEEN 1 AND 512),
    provider_version_id TEXT NOT NULL CHECK (length(provider_version_id) BETWEEN 1 AND 512),
    binding_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 128 AND kind NOT GLOB '*[^A-Za-z0-9_-]*'),
    supersedes_id TEXT,
    ambiguous_order_with_previous INTEGER NOT NULL CHECK (ambiguous_order_with_previous IN (0, 1)),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    occurred_at_millis INTEGER NOT NULL,
    created_at_millis INTEGER NOT NULL,
    observed_at_millis INTEGER NOT NULL,
    actor_id TEXT CHECK (actor_id IS NULL OR (length(actor_id) BETWEEN 1 AND 128 AND actor_id NOT GLOB '*[^A-Za-z0-9_-]*')),
    provider_actor_id TEXT CHECK (provider_actor_id IS NULL OR length(provider_actor_id) BETWEEN 1 AND 512),
    body_digest BLOB CHECK (body_digest IS NULL OR length(body_digest) = 32),
    edit_diff_digest BLOB CHECK (edit_diff_digest IS NULL OR length(edit_diff_digest) = 32),
    capture_digest BLOB NOT NULL CHECK (length(capture_digest) = 32),
    payload_id TEXT,
    edit_diff_payload_id TEXT,
    edit_deleted_at_millis INTEGER,
    missing_body_reason TEXT CHECK (missing_body_reason IS NULL OR missing_body_reason IN ('prior_version_unavailable', 'deleted_by_provider')),
    compilation_mode TEXT NOT NULL CHECK (compilation_mode IN ('capture_only', 'on_demand', 'eager')),
    coverage_requirement TEXT NOT NULL CHECK (coverage_requirement IN ('required', 'optional')),
    capture_policy_version TEXT NOT NULL CHECK (length(capture_policy_version) BETWEEN 1 AND 128 AND capture_policy_version NOT GLOB '*[^A-Za-z0-9_-]*'),
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, sequence),
    FOREIGN KEY (project_id, binding_id) REFERENCES source_bindings(project_id, id),
    FOREIGN KEY (project_id, supersedes_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id),
    FOREIGN KEY (project_id, edit_diff_payload_id) REFERENCES payloads(project_id, id),
    CHECK ((body_digest IS NULL) = (payload_id IS NULL)),
    CHECK ((edit_diff_digest IS NULL) = (edit_diff_payload_id IS NULL)),
    CHECK ((body_digest IS NULL) = (missing_body_reason IS NOT NULL))
) STRICT;

CREATE INDEX source_versions_by_entity ON source_versions(project_id, source_id, sequence);

CREATE TABLE provider_observations (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    binding_id TEXT NOT NULL,
    issue_id TEXT NOT NULL CHECK (length(issue_id) BETWEEN 1 AND 128 AND issue_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    issue_state TEXT NOT NULL CHECK (issue_state IN ('open', 'closed')),
    snapshot_payload_id TEXT NOT NULL,
    observed_at_millis INTEGER NOT NULL,
    accepted_batch_id TEXT NOT NULL,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, binding_id) REFERENCES source_bindings(project_id, id),
    FOREIGN KEY (project_id, snapshot_payload_id) REFERENCES payloads(project_id, id),
    FOREIGN KEY (project_id, accepted_batch_id) REFERENCES domain_event_batches(project_id, id)
) STRICT;

CREATE TABLE provider_issue_heads (
    project_id TEXT NOT NULL REFERENCES projects(id),
    issue_id TEXT NOT NULL,
    observation_id TEXT NOT NULL,
    observed_at_millis INTEGER NOT NULL,
    last_seen_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, issue_id),
    FOREIGN KEY (project_id, observation_id) REFERENCES provider_observations(project_id, id)
) STRICT;

CREATE TRIGGER source_bindings_no_update
BEFORE UPDATE ON source_bindings BEGIN SELECT RAISE(ABORT, 'source binding identity is append-only'); END;
CREATE TRIGGER source_bindings_no_delete
BEFORE DELETE ON source_bindings BEGIN SELECT RAISE(ABORT, 'source binding identity is append-only'); END;
CREATE TRIGGER source_versions_no_update
BEFORE UPDATE ON source_versions BEGIN SELECT RAISE(ABORT, 'source version is append-only'); END;
CREATE TRIGGER source_versions_no_delete
BEFORE DELETE ON source_versions BEGIN SELECT RAISE(ABORT, 'source version is append-only'); END;
CREATE TRIGGER provider_observations_no_update
BEFORE UPDATE ON provider_observations BEGIN SELECT RAISE(ABORT, 'provider observation is append-only'); END;
CREATE TRIGGER provider_observations_no_delete
BEFORE DELETE ON provider_observations BEGIN SELECT RAISE(ABORT, 'provider observation is append-only'); END;
