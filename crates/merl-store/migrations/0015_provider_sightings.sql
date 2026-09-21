CREATE TABLE provider_issue_sightings (
    project_id TEXT NOT NULL,
    issue_id TEXT NOT NULL,
    observation_id TEXT NOT NULL,
    seen_at_millis INTEGER NOT NULL,
    PRIMARY KEY (project_id, issue_id, observation_id, seen_at_millis),
    FOREIGN KEY (project_id, observation_id) REFERENCES provider_observations(project_id, id)
) STRICT;

INSERT INTO provider_issue_sightings (project_id, issue_id, observation_id, seen_at_millis)
SELECT project_id, issue_id, observation_id, last_seen_at_millis
FROM provider_issue_heads WHERE last_seen_at_millis > observed_at_millis;

CREATE TRIGGER provider_issue_sightings_no_update
BEFORE UPDATE ON provider_issue_sightings BEGIN SELECT RAISE(ABORT, 'provider sighting is append-only'); END;
CREATE TRIGGER provider_issue_sightings_no_delete
BEFORE DELETE ON provider_issue_sightings BEGIN SELECT RAISE(ABORT, 'provider sighting is append-only'); END;
