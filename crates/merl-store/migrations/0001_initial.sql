CREATE TABLE projects (
    id TEXT PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    current_revision INTEGER NOT NULL DEFAULT 0 CHECK (current_revision >= 0)
) STRICT;

-- Protected bytes live here alone. No other table stores copied prose.
CREATE TABLE payloads (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    digest BLOB NOT NULL CHECK (length(digest) = 32),
    bytes BLOB,
    erased INTEGER NOT NULL DEFAULT 0 CHECK (erased IN (0, 1)),
    PRIMARY KEY (project_id, id),
    CHECK ((bytes IS NULL) = (erased = 1))
) STRICT;

CREATE TABLE domain_event_batches (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    revision INTEGER NOT NULL CHECK (revision > 0),
    actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 128 AND actor_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    occurred_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, revision)
) STRICT;

CREATE TABLE domain_events (
    project_id TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    event_kind TEXT NOT NULL CHECK (event_kind = 'put_object'),
    object_id TEXT NOT NULL CHECK (length(object_id) BETWEEN 1 AND 128 AND object_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    object_kind TEXT NOT NULL CHECK (length(object_kind) BETWEEN 1 AND 128 AND object_kind NOT GLOB '*[^A-Za-z0-9_-]*'),
    payload_id TEXT CHECK (payload_id IS NULL OR (length(payload_id) BETWEEN 1 AND 128 AND payload_id NOT GLOB '*[^A-Za-z0-9_-]*')),
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, batch_id, event_index),
    FOREIGN KEY (project_id, batch_id) REFERENCES domain_event_batches(project_id, id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE TABLE objects (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 128 AND kind NOT GLOB '*[^A-Za-z0-9_-]*'),
    payload_id TEXT CHECK (payload_id IS NULL OR (length(payload_id) BETWEEN 1 AND 128 AND payload_id NOT GLOB '*[^A-Za-z0-9_-]*')),
    object_revision INTEGER NOT NULL CHECK (object_revision > 0),
    project_revision INTEGER NOT NULL CHECK (project_revision > 0),
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id)
) STRICT;

-- Accepted envelopes are immutable; corrections are new events.
CREATE TRIGGER domain_event_batches_no_update
BEFORE UPDATE ON domain_event_batches BEGIN SELECT RAISE(ABORT, 'accepted batch is append-only'); END;
CREATE TRIGGER domain_event_batches_no_delete
BEFORE DELETE ON domain_event_batches BEGIN SELECT RAISE(ABORT, 'accepted batch is append-only'); END;
CREATE TRIGGER domain_events_no_update
BEFORE UPDATE ON domain_events BEGIN SELECT RAISE(ABORT, 'domain event is append-only'); END;
CREATE TRIGGER domain_events_no_delete
BEFORE DELETE ON domain_events BEGIN SELECT RAISE(ABORT, 'domain event is append-only'); END;
