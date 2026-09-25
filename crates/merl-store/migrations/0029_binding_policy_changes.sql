-- The accepted object's payload selects an immutable policy proposal. Merely
-- retaining a proposal does not change capture behavior.
CREATE TABLE binding_policy_changes (
    project_id TEXT NOT NULL,
    reason_payload_id TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    object_id TEXT NOT NULL,
    compilation_mode TEXT NOT NULL CHECK (compilation_mode IN ('eager','capture_only','on_demand')),
    coverage_requirement TEXT NOT NULL CHECK (coverage_requirement IN ('optional','required')),
    policy_version TEXT NOT NULL,
    expected_version TEXT NOT NULL,
    PRIMARY KEY (project_id, reason_payload_id),
    FOREIGN KEY (project_id, binding_id) REFERENCES binding_capture_policies(project_id, binding_id),
    FOREIGN KEY (project_id, reason_payload_id) REFERENCES payloads(project_id, id)
) STRICT;
CREATE INDEX binding_policy_changes_by_binding ON binding_policy_changes(project_id,binding_id);
CREATE TRIGGER binding_policy_changes_no_update BEFORE UPDATE ON binding_policy_changes
BEGIN SELECT RAISE(ABORT, 'binding policy proposals are immutable'); END;
CREATE TRIGGER binding_policy_changes_no_delete BEFORE DELETE ON binding_policy_changes
BEGIN SELECT RAISE(ABORT, 'binding policy proposals are immutable'); END;
