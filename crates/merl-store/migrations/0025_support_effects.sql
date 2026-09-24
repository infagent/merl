-- Support effects share ordering and policy provenance with semantic events,
-- but cannot carry semantic content or increment an object's version.
CREATE TABLE domain_events_v25 (
    project_id TEXT NOT NULL,
    batch_id TEXT NOT NULL,
    id TEXT NOT NULL CHECK (length(id) BETWEEN 1 AND 128 AND id NOT GLOB '*[^A-Za-z0-9_-]*'),
    event_index INTEGER NOT NULL CHECK (event_index >= 0),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('put_object', 'resolve_support')),
    object_id TEXT NOT NULL CHECK (length(object_id) BETWEEN 1 AND 128 AND object_id NOT GLOB '*[^A-Za-z0-9_-]*'),
    object_kind TEXT CHECK (object_kind IS NULL OR (length(object_kind) BETWEEN 1 AND 128 AND object_kind NOT GLOB '*[^A-Za-z0-9_-]*')),
    payload_id TEXT CHECK (payload_id IS NULL OR (length(payload_id) BETWEEN 1 AND 128 AND payload_id NOT GLOB '*[^A-Za-z0-9_-]*')),
    issue_scope_id TEXT,
    lifecycle TEXT CHECK (lifecycle IN ('active', 'superseded', 'invalidated')),
    review_id TEXT,
    CHECK ((event_kind='put_object' AND object_kind IS NOT NULL AND lifecycle IS NOT NULL AND review_id IS NULL)
        OR (event_kind='resolve_support' AND object_kind IS NULL AND payload_id IS NULL AND issue_scope_id IS NULL AND lifecycle IS NULL AND review_id IS NOT NULL)),
    FOREIGN KEY (project_id, review_id) REFERENCES revalidation_reviews(project_id, id),
    PRIMARY KEY (project_id, id),
    UNIQUE (project_id, batch_id, event_index),
    FOREIGN KEY (project_id, batch_id) REFERENCES domain_event_batches(project_id, id),
    FOREIGN KEY (project_id, payload_id) REFERENCES payloads(project_id, id)
) STRICT;

INSERT INTO domain_events_v25(project_id,batch_id,id,event_index,event_kind,object_id,object_kind,payload_id,issue_scope_id,lifecycle)
SELECT project_id,batch_id,id,event_index,event_kind,object_id,object_kind,payload_id,issue_scope_id,lifecycle FROM domain_events;
DROP TABLE domain_events;
ALTER TABLE domain_events_v25 RENAME TO domain_events;
CREATE TRIGGER domain_events_no_update BEFORE UPDATE ON domain_events
BEGIN SELECT RAISE(ABORT, 'domain event is append-only'); END;
CREATE TRIGGER domain_events_no_delete BEFORE DELETE ON domain_events
BEGIN SELECT RAISE(ABORT, 'domain event is append-only'); END;
CREATE VIEW semantic_object_events AS SELECT * FROM domain_events WHERE event_kind='put_object';
