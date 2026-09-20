ALTER TABLE evidence_impacts ADD COLUMN affected_run_id TEXT;

UPDATE evidence_impacts
SET affected_run_id = (
    SELECT pa.run_id
    FROM policy_evaluation_domain_events pe
    JOIN policy_evaluation_inputs pi
      ON pi.project_id=pe.project_id AND pi.evaluation_id=pe.evaluation_id
     AND pi.input_index=pe.input_index
    JOIN policy_assertion_inputs pa
      ON pa.project_id=pi.project_id AND pa.evaluation_id=pi.evaluation_id
     AND pa.input_id=pi.input_id
    WHERE pe.project_id=evidence_impacts.project_id
      AND pe.event_id=evidence_impacts.support_event_id
);

CREATE INDEX evidence_impacts_by_run
    ON evidence_impacts(project_id, affected_run_id, id);
