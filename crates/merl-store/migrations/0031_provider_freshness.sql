-- Legacy sightings carry only local observation time. Their accepted observation
-- supplies the upstream watermark until a new sighting records a later one.
ALTER TABLE provider_issue_sightings ADD COLUMN upstream_updated_at_millis INTEGER
    CHECK (upstream_updated_at_millis IS NULL OR upstream_updated_at_millis <= seen_at_millis);

-- Polling history can grow without accepted revisions. Watermark reads should
-- seek to the latest timestamp instead of scanning that history on every poll.
CREATE INDEX provider_sighting_watermarks ON provider_issue_sightings
    (project_id, issue_id, observation_id, upstream_updated_at_millis);

CREATE VIEW provider_issue_freshness AS
SELECT h.*, COALESCE((
    SELECT MAX(s.upstream_updated_at_millis)
    FROM provider_issue_sightings s
    WHERE s.project_id = h.project_id AND s.issue_id = h.issue_id
      AND s.observation_id = h.observation_id
), o.upstream_updated_at_millis) AS upstream_updated_at_millis
FROM provider_issue_heads h JOIN provider_observations o
ON o.project_id = h.project_id AND o.id = h.observation_id;
