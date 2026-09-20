CREATE TABLE policy_event_origins_v6 (
    project_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    input_index INTEGER NOT NULL CHECK (input_index >= 0),
    PRIMARY KEY (project_id, evaluation_id, event_id),
    FOREIGN KEY (project_id, evaluation_id, input_index)
        REFERENCES policy_evaluation_inputs(project_id, evaluation_id, input_index),
    FOREIGN KEY (project_id, event_id) REFERENCES domain_events(project_id, id)
) STRICT;

-- Before this migration, the policy builder emitted one event per accepted
-- input in input order. Recover that pairing from the stored event index.
WITH ranked_inputs AS (
    SELECT project_id, evaluation_id, input_index,
           ROW_NUMBER() OVER (
               PARTITION BY project_id, evaluation_id ORDER BY input_index
           ) - 1 AS event_index
    FROM policy_evaluation_inputs
    WHERE disposition = 'accepted'
)
INSERT INTO policy_event_origins_v6 (project_id, evaluation_id, event_id, input_index)
SELECT old.project_id, old.evaluation_id, old.event_id, ranked.input_index
FROM policy_evaluation_domain_events old
JOIN domain_events event ON event.project_id = old.project_id AND event.id = old.event_id
JOIN ranked_inputs ranked
  ON ranked.project_id = old.project_id
 AND ranked.evaluation_id = old.evaluation_id
 AND ranked.event_index = event.event_index;

CREATE TABLE policy_origin_migration_guard (
    old_count INTEGER NOT NULL,
    new_count INTEGER NOT NULL,
    CHECK (old_count = new_count)
) STRICT;
INSERT INTO policy_origin_migration_guard
SELECT (SELECT COUNT(*) FROM policy_evaluation_domain_events),
       (SELECT COUNT(*) FROM policy_event_origins_v6);
DROP TABLE policy_origin_migration_guard;

DROP TRIGGER policy_evaluation_domain_events_no_update;
DROP TRIGGER policy_evaluation_domain_events_no_delete;
DROP TABLE policy_evaluation_domain_events;
ALTER TABLE policy_event_origins_v6 RENAME TO policy_evaluation_domain_events;

CREATE TRIGGER policy_evaluation_domain_events_no_update
BEFORE UPDATE ON policy_evaluation_domain_events
BEGIN SELECT RAISE(ABORT, 'policy event link is append-only'); END;
CREATE TRIGGER policy_evaluation_domain_events_no_delete
BEFORE DELETE ON policy_evaluation_domain_events
BEGIN SELECT RAISE(ABORT, 'policy event link is append-only'); END;
