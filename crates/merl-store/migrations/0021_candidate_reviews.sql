-- A request fixes the reviewer's intent before policy runs. Accepted command
-- receipts determine resolution; merely recording this row grants no authority.
CREATE TABLE candidate_reviews (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    candidate_id TEXT NOT NULL,
    original_evaluation_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    assertion_index INTEGER NOT NULL CHECK (assertion_index >= 0),
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('accept','reject','correct')),
    object_id TEXT,
    object_kind TEXT,
    payload_id TEXT,
    reason_payload_id TEXT,
    PRIMARY KEY (project_id,id),
    FOREIGN KEY (project_id,original_evaluation_id) REFERENCES policy_evaluations(project_id,id),
    FOREIGN KEY (project_id,run_id,assertion_index) REFERENCES observed_assertions(project_id,run_id,assertion_index),
    FOREIGN KEY (project_id,payload_id) REFERENCES payloads(project_id,id),
    FOREIGN KEY (project_id,reason_payload_id) REFERENCES payloads(project_id,id),
    CHECK ((action='correct') = (object_id IS NOT NULL AND object_kind IS NOT NULL)),
    CHECK (action='accept' OR reason_payload_id IS NOT NULL)
) STRICT;
CREATE INDEX candidate_review_history ON candidate_reviews(project_id,candidate_id);
CREATE TRIGGER candidate_reviews_no_update
BEFORE UPDATE ON candidate_reviews BEGIN SELECT RAISE(ABORT,'candidate review is append-only'); END;
CREATE TRIGGER candidate_reviews_no_delete
BEFORE DELETE ON candidate_reviews BEGIN SELECT RAISE(ABORT,'candidate review is append-only'); END;
