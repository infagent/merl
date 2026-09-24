-- A command promotes one reviewed hindsight result without changing the compiler record.
CREATE TABLE revalidation_reviews (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    impact_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('confirm','weaken','supersede','invalidate','unavailable')),
    run_id TEXT,
    assertion_index INTEGER,
    PRIMARY KEY(project_id,id),
    FOREIGN KEY(project_id,impact_id) REFERENCES evidence_impacts(project_id,id),
    FOREIGN KEY(project_id,run_id) REFERENCES compilation_runs(project_id,id),
    CHECK ((action IN ('confirm','supersede')) = (assertion_index IS NOT NULL)),
    CHECK ((action='unavailable') = (run_id IS NULL))
) STRICT;
CREATE TRIGGER revalidation_reviews_no_update BEFORE UPDATE ON revalidation_reviews
BEGIN SELECT RAISE(ABORT,'revalidation reviews are append-only'); END;
CREATE TRIGGER revalidation_reviews_no_delete BEFORE DELETE ON revalidation_reviews
BEGIN SELECT RAISE(ABORT,'revalidation reviews are append-only'); END;

CREATE TABLE revalidation_attempts (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    impact_id TEXT NOT NULL,
    PRIMARY KEY(project_id,run_id),
    FOREIGN KEY(project_id,impact_id) REFERENCES evidence_impacts(project_id,id)
) STRICT;
CREATE TRIGGER revalidation_attempts_no_update BEFORE UPDATE ON revalidation_attempts
BEGIN SELECT RAISE(ABORT,'revalidation attempts are append-only'); END;
CREATE TRIGGER revalidation_attempts_no_delete BEFORE DELETE ON revalidation_attempts
BEGIN SELECT RAISE(ABORT,'revalidation attempts are append-only'); END;
