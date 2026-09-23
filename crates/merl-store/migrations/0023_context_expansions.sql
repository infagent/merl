-- Each compiler request owns one deterministic successor, even before dispatch.
CREATE TABLE compilation_expansions (
    project_id TEXT NOT NULL,
    parent_run TEXT NOT NULL,
    child_run TEXT NOT NULL,
    round INTEGER NOT NULL CHECK(round > 0),
    PRIMARY KEY(project_id, parent_run),
    UNIQUE(project_id, child_run),
    FOREIGN KEY(project_id, parent_run) REFERENCES compilation_runs(project_id, id)
);
CREATE TABLE compilation_expansion_references (
    project_id TEXT NOT NULL,
    parent_run TEXT NOT NULL,
    request_index INTEGER NOT NULL,
    reference TEXT NOT NULL,
    PRIMARY KEY(project_id, parent_run, request_index),
    FOREIGN KEY(project_id, parent_run) REFERENCES compilation_expansions(project_id, parent_run)
);
CREATE TABLE compilation_expansion_failures (
    project_id TEXT NOT NULL,
    parent_run TEXT NOT NULL,
    code TEXT NOT NULL,
    PRIMARY KEY(project_id, parent_run),
    FOREIGN KEY(project_id, parent_run) REFERENCES compilation_expansions(project_id, parent_run)
);
CREATE TRIGGER immutable_expansion_update BEFORE UPDATE ON compilation_expansions BEGIN SELECT RAISE(ABORT, 'immutable expansion'); END;
CREATE TRIGGER immutable_expansion_delete BEFORE DELETE ON compilation_expansions BEGIN SELECT RAISE(ABORT, 'immutable expansion'); END;
CREATE TRIGGER immutable_expansion_reference_update BEFORE UPDATE ON compilation_expansion_references BEGIN SELECT RAISE(ABORT, 'immutable expansion reference'); END;
CREATE TRIGGER immutable_expansion_reference_delete BEFORE DELETE ON compilation_expansion_references BEGIN SELECT RAISE(ABORT, 'immutable expansion reference'); END;
CREATE TRIGGER immutable_expansion_failure_update BEFORE UPDATE ON compilation_expansion_failures BEGIN SELECT RAISE(ABORT, 'immutable expansion failure'); END;
CREATE TRIGGER immutable_expansion_failure_delete BEFORE DELETE ON compilation_expansion_failures BEGIN SELECT RAISE(ABORT, 'immutable expansion failure'); END;
CREATE TABLE compilation_replay_origins (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    original_run TEXT NOT NULL,
    PRIMARY KEY(project_id, run_id),
    FOREIGN KEY(project_id, run_id) REFERENCES compilation_runs(project_id,id),
    FOREIGN KEY(project_id, original_run) REFERENCES compilation_runs(project_id,id)
);
CREATE TRIGGER immutable_replay_origin_update BEFORE UPDATE ON compilation_replay_origins BEGIN SELECT RAISE(ABORT, 'immutable replay origin'); END;
CREATE TRIGGER immutable_replay_origin_delete BEFORE DELETE ON compilation_replay_origins BEGIN SELECT RAISE(ABORT, 'immutable replay origin'); END;

-- Existing needs-context results already retain validated requests. Recover them
-- without rewriting results or inventing references after payload erasure.
INSERT INTO compilation_expansions
SELECT r.project_id,r.id,'cx_legacy_' || r.attempt_order,1
FROM compilation_runs r JOIN compilation_results c
ON c.project_id=r.project_id AND c.run_id=r.id WHERE c.outcome='needs_context';
INSERT INTO compilation_expansion_references
SELECT e.project_id,e.parent_run,CAST(j.key AS INTEGER),json_extract(j.value,'$.reference')
FROM compilation_expansions e JOIN compilation_results c
ON c.project_id=e.project_id AND c.run_id=e.parent_run
JOIN payloads p ON p.project_id=c.project_id AND p.id=c.response_payload_id,
json_each(CASE WHEN p.bytes IS NOT NULL THEN CAST(p.bytes AS TEXT) ELSE '{}' END,'$.context_required') j;
INSERT INTO compilation_expansion_failures
SELECT e.project_id,e.parent_run,
CASE WHEN r.max_expansion_rounds=0 THEN 'expansion_round_budget' ELSE 'missing_evidence' END
FROM compilation_expansions e JOIN compilation_runs r ON r.project_id=e.project_id AND r.id=e.parent_run
WHERE r.max_expansion_rounds=0 OR NOT EXISTS (
SELECT 1 FROM compilation_expansion_references q WHERE q.project_id=e.project_id AND q.parent_run=e.parent_run);
