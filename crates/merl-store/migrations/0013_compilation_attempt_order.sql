ALTER TABLE compilation_runs ADD COLUMN attempt_order INTEGER NOT NULL DEFAULT 0
    CHECK (attempt_order >= 0);

-- The one-time backfill preserves the order in which the old store accepted intents.
UPDATE compilation_runs SET attempt_order = rowid;

CREATE UNIQUE INDEX compilation_runs_by_attempt
    ON compilation_runs(project_id, attempt_order);
