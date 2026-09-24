ALTER TABLE source_versions ADD COLUMN author_time TEXT;
ALTER TABLE observed_assertions ADD COLUMN temporal TEXT NOT NULL DEFAULT '[]';
ALTER TABLE observed_assertions ADD COLUMN deferral TEXT;
ALTER TABLE semantic_commands ADD COLUMN planning_evidence TEXT;
