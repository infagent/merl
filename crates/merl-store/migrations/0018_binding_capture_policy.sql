CREATE TABLE binding_capture_policies (
    project_id TEXT NOT NULL,
    binding_id TEXT NOT NULL,
    compilation_mode TEXT NOT NULL CHECK (compilation_mode IN ('eager', 'capture_only', 'on_demand')),
    coverage_requirement TEXT NOT NULL CHECK (coverage_requirement IN ('required', 'optional')),
    policy_version TEXT NOT NULL,
    PRIMARY KEY (project_id, binding_id),
    FOREIGN KEY (project_id, binding_id) REFERENCES source_bindings(project_id, id)
);
