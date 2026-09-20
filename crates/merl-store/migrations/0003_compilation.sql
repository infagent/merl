ALTER TABLE source_versions ADD COLUMN interpretation_basis_revision INTEGER NOT NULL DEFAULT 0
    CHECK (interpretation_basis_revision >= 0);
ALTER TABLE source_versions ADD COLUMN interpretation_basis_known INTEGER NOT NULL DEFAULT 0
    CHECK (interpretation_basis_known IN (0, 1));
ALTER TABLE source_versions ADD COLUMN context_scope_id TEXT NOT NULL DEFAULT '';
CREATE INDEX source_versions_by_context_scope
    ON source_versions(project_id, context_scope_id, sequence);

CREATE TABLE compilation_runs (
    project_id TEXT NOT NULL REFERENCES projects(id),
    id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    context_digest BLOB NOT NULL CHECK (length(context_digest) = 32),
    context_payload_id TEXT NOT NULL,
    interpretation_basis_revision INTEGER NOT NULL,
    source_observation_cutoff INTEGER NOT NULL,
    renderer_version TEXT NOT NULL,
    selector_version TEXT NOT NULL,
    max_input_bytes INTEGER NOT NULL,
    max_output_bytes INTEGER NOT NULL,
    max_output_tokens INTEGER NOT NULL,
    max_assertions INTEGER NOT NULL,
    max_context_requests INTEGER NOT NULL,
    max_expansion_rounds INTEGER NOT NULL,
    max_payload_bytes INTEGER NOT NULL,
    compiler_id TEXT NOT NULL,
    compiler_version TEXT NOT NULL,
    model_id TEXT NOT NULL,
    prompt_digest BLOB NOT NULL CHECK (length(prompt_digest) = 32),
    mode TEXT NOT NULL CHECK (mode IN ('live', 'replay', 'eval', 'hindsight')),
    started_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id),
    FOREIGN KEY (project_id, context_payload_id) REFERENCES payloads(project_id, id)
) STRICT;

CREATE INDEX compilation_runs_by_source ON compilation_runs(project_id, source_version_id);

CREATE TABLE compilation_results (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed')),
    failure_code TEXT,
    response_digest BLOB CHECK (response_digest IS NULL OR length(response_digest) = 32),
    response_payload_id TEXT,
    completed_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, run_id),
    FOREIGN KEY (project_id, run_id) REFERENCES compilation_runs(project_id, id),
    FOREIGN KEY (project_id, response_payload_id) REFERENCES payloads(project_id, id),
    CHECK ((outcome = 'succeeded' AND failure_code IS NULL AND response_payload_id IS NOT NULL)
        OR (outcome = 'failed' AND failure_code IS NOT NULL AND response_payload_id IS NULL))
) STRICT;

CREATE TABLE compilation_context_sources (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    window_index INTEGER NOT NULL,
    source_version_id TEXT NOT NULL,
    PRIMARY KEY (project_id, run_id, window_index),
    FOREIGN KEY (project_id, run_id) REFERENCES compilation_runs(project_id, id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id)
) STRICT;

CREATE TABLE compilation_context_objects (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    object_revision INTEGER NOT NULL,
    PRIMARY KEY (project_id, run_id, object_id),
    FOREIGN KEY (project_id, run_id) REFERENCES compilation_runs(project_id, id)
) STRICT;

CREATE TABLE observed_assertions (
    project_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    assertion_index INTEGER NOT NULL,
    source_version_id TEXT NOT NULL,
    span_start INTEGER NOT NULL,
    span_end INTEGER NOT NULL,
    subject_id TEXT NOT NULL,
    predicate_id TEXT NOT NULL,
    value_id TEXT NOT NULL,
    act TEXT NOT NULL,
    epistemic_basis TEXT NOT NULL,
    polarity TEXT NOT NULL,
    confidence_millis INTEGER NOT NULL,
    asserted_by TEXT NOT NULL,
    attributed_to TEXT,
    attribution_verified INTEGER NOT NULL,
    PRIMARY KEY (project_id, run_id, assertion_index),
    FOREIGN KEY (project_id, run_id) REFERENCES compilation_runs(project_id, id),
    FOREIGN KEY (project_id, source_version_id) REFERENCES source_versions(project_id, id)
) STRICT;

CREATE TRIGGER compilation_runs_no_update
BEFORE UPDATE ON compilation_runs BEGIN SELECT RAISE(ABORT, 'compilation run is append-only'); END;
CREATE TRIGGER compilation_runs_no_delete
BEFORE DELETE ON compilation_runs BEGIN SELECT RAISE(ABORT, 'compilation run is append-only'); END;
CREATE TRIGGER compilation_results_no_update
BEFORE UPDATE ON compilation_results BEGIN SELECT RAISE(ABORT, 'compilation result is append-only'); END;
CREATE TRIGGER compilation_results_no_delete
BEFORE DELETE ON compilation_results BEGIN SELECT RAISE(ABORT, 'compilation result is append-only'); END;
CREATE TRIGGER compilation_context_sources_no_update
BEFORE UPDATE ON compilation_context_sources BEGIN SELECT RAISE(ABORT, 'compilation context is append-only'); END;
CREATE TRIGGER compilation_context_sources_no_delete
BEFORE DELETE ON compilation_context_sources BEGIN SELECT RAISE(ABORT, 'compilation context is append-only'); END;
CREATE TRIGGER compilation_context_objects_no_update
BEFORE UPDATE ON compilation_context_objects BEGIN SELECT RAISE(ABORT, 'compilation context is append-only'); END;
CREATE TRIGGER compilation_context_objects_no_delete
BEFORE DELETE ON compilation_context_objects BEGIN SELECT RAISE(ABORT, 'compilation context is append-only'); END;
CREATE TRIGGER observed_assertions_no_update
BEFORE UPDATE ON observed_assertions BEGIN SELECT RAISE(ABORT, 'observed assertion is append-only'); END;
CREATE TRIGGER observed_assertions_no_delete
BEFORE DELETE ON observed_assertions BEGIN SELECT RAISE(ABORT, 'observed assertion is append-only'); END;
