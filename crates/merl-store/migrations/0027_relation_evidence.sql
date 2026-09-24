CREATE TABLE relation_evidence_supports (
    project_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    relation_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    relation_index INTEGER NOT NULL,
    PRIMARY KEY(project_id,event_id),
    FOREIGN KEY(project_id,event_id) REFERENCES relation_events(project_id,id),
    FOREIGN KEY(project_id,run_id,relation_index) REFERENCES observed_relations(project_id,run_id,relation_index)
) STRICT;
CREATE TABLE relation_evidence_impacts (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    support_event_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    replacement_version_id TEXT,
    PRIMARY KEY(project_id,id),
    FOREIGN KEY(project_id,support_event_id) REFERENCES relation_evidence_supports(project_id,event_id),
    FOREIGN KEY(project_id,source_version_id) REFERENCES source_versions(project_id,id),
    FOREIGN KEY(project_id,replacement_version_id) REFERENCES source_versions(project_id,id)
) STRICT;
CREATE TABLE relation_withdrawals (
    project_id TEXT NOT NULL,
    id TEXT NOT NULL,
    impact_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    PRIMARY KEY(project_id,id),
    FOREIGN KEY(project_id,impact_id) REFERENCES relation_evidence_impacts(project_id,id)
) STRICT;
CREATE TABLE relation_evidence_resolutions (
    project_id TEXT NOT NULL,
    support_event_id TEXT NOT NULL,
    evaluation_id TEXT NOT NULL,
    replacement_relation_id TEXT,
    PRIMARY KEY(project_id,support_event_id),
    FOREIGN KEY(project_id,support_event_id) REFERENCES relation_evidence_supports(project_id,event_id),
    FOREIGN KEY(project_id,evaluation_id) REFERENCES policy_evaluations(project_id,id)
) STRICT;
CREATE INDEX relation_impacts_by_support ON relation_evidence_impacts(project_id,support_event_id);
CREATE INDEX relation_supports_by_run ON relation_evidence_supports(project_id,run_id);

CREATE VIEW relation_support_availability AS
SELECT s.*, r.source_version_id AS trigger,
    (context.bytes IS NOT NULL AND response.bytes IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM compilation_context_sources cs
        JOIN source_versions v ON v.project_id=cs.project_id AND v.id=cs.source_version_id
        LEFT JOIN payloads p ON p.project_id=v.project_id AND p.id=v.payload_id
        WHERE cs.project_id=s.project_id AND cs.run_id=s.run_id
          AND v.payload_id IS NOT NULL AND p.bytes IS NULL)) AS available
FROM relation_evidence_supports s
JOIN compilation_runs r ON r.project_id=s.project_id AND r.id=s.run_id
JOIN compilation_results result ON result.project_id=s.project_id AND result.run_id=s.run_id
JOIN payloads context ON context.project_id=r.project_id AND context.id=r.context_payload_id
JOIN payloads response ON response.project_id=result.project_id AND response.id=result.response_payload_id;

-- Impacts retire this derivation from current graph use even after review closes the work.
CREATE VIEW current_relations AS SELECT r.* FROM relations r WHERE NOT EXISTS (
    SELECT 1 FROM relation_evidence_supports s
    JOIN relation_events e ON e.project_id=s.project_id AND e.id=s.event_id
    JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
    WHERE s.project_id=r.project_id AND s.relation_id=r.id AND b.revision=r.project_revision
      AND EXISTS (SELECT 1 FROM relation_evidence_impacts i
                  WHERE i.project_id=s.project_id AND i.support_event_id=s.event_id));
CREATE TRIGGER relation_evidence_supports_no_update BEFORE UPDATE ON relation_evidence_supports
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_evidence_supports_no_delete BEFORE DELETE ON relation_evidence_supports
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_evidence_impacts_no_update BEFORE UPDATE ON relation_evidence_impacts
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_evidence_impacts_no_delete BEFORE DELETE ON relation_evidence_impacts
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_withdrawals_no_update BEFORE UPDATE ON relation_withdrawals
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_withdrawals_no_delete BEFORE DELETE ON relation_withdrawals
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_evidence_resolutions_no_update BEFORE UPDATE ON relation_evidence_resolutions
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
CREATE TRIGGER relation_evidence_resolutions_no_delete BEFORE DELETE ON relation_evidence_resolutions
BEGIN SELECT RAISE(ABORT,'relation evidence history is append-only'); END;
