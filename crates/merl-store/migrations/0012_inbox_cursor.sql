CREATE TABLE inbox_cursors (
    project_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    PRIMARY KEY (project_id, agent_id),
    FOREIGN KEY (project_id, agent_id) REFERENCES inbox_subscriptions(project_id, agent_id)
) STRICT;

INSERT INTO inbox_cursors (project_id, agent_id, revision)
    SELECT project_id, agent_id, 0 FROM inbox_subscriptions;
