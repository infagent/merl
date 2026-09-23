ALTER TABLE source_compilation_intents ADD COLUMN adapter_config_digest BLOB CHECK (adapter_config_digest IS NULL OR length(adapter_config_digest) = 32);
ALTER TABLE source_compilation_intents ADD COLUMN max_input_bytes INTEGER CHECK (max_input_bytes IS NULL OR max_input_bytes >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_output_bytes INTEGER CHECK (max_output_bytes IS NULL OR max_output_bytes >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_output_tokens INTEGER CHECK (max_output_tokens IS NULL OR max_output_tokens >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_assertions INTEGER CHECK (max_assertions IS NULL OR max_assertions >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_context_requests INTEGER CHECK (max_context_requests IS NULL OR max_context_requests >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_expansion_rounds INTEGER CHECK (max_expansion_rounds IS NULL OR max_expansion_rounds >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_payload_bytes INTEGER CHECK (max_payload_bytes IS NULL OR max_payload_bytes >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_source_window INTEGER CHECK (max_source_window IS NULL OR max_source_window >= 0);
ALTER TABLE source_compilation_intents ADD COLUMN max_objects INTEGER CHECK (max_objects IS NULL OR max_objects >= 0);

ALTER TABLE source_compilation_authorizations ADD COLUMN adapter_config_digest BLOB CHECK (adapter_config_digest IS NULL OR length(adapter_config_digest) = 32);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_input_bytes INTEGER CHECK (max_input_bytes IS NULL OR max_input_bytes >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_output_bytes INTEGER CHECK (max_output_bytes IS NULL OR max_output_bytes >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_output_tokens INTEGER CHECK (max_output_tokens IS NULL OR max_output_tokens >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_assertions INTEGER CHECK (max_assertions IS NULL OR max_assertions >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_context_requests INTEGER CHECK (max_context_requests IS NULL OR max_context_requests >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_expansion_rounds INTEGER CHECK (max_expansion_rounds IS NULL OR max_expansion_rounds >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_payload_bytes INTEGER CHECK (max_payload_bytes IS NULL OR max_payload_bytes >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_source_window INTEGER CHECK (max_source_window IS NULL OR max_source_window >= 0);
ALTER TABLE source_compilation_authorizations ADD COLUMN max_objects INTEGER CHECK (max_objects IS NULL OR max_objects >= 0);

ALTER TABLE compilation_runs ADD COLUMN adapter_config_digest BLOB CHECK (adapter_config_digest IS NULL OR length(adapter_config_digest) = 32);

PRAGMA user_version = 17;
