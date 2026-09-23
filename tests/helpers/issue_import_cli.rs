use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

use merl_core::{
    ActorId, CapturePolicyVersion, CompilationMode, CoverageRequirement, ProjectId,
    SourceBindingId, SourceId, SourceKind, SourceProvider, SourceVersionId,
};
use merl_store::{SourceBinding, SourceCapture, Store};
use sha2::{Digest, Sha256};

pub struct CliIssueHistory {
    directory: TestDirectory,
    latest: Option<serde_json::Value>,
    purge_digest: Option<String>,
    compiler_program: Option<PathBuf>,
    first_promotion: Option<serde_json::Value>,
}

impl CliIssueHistory {
    pub fn new() -> Self {
        Self {
            directory: TestDirectory::new(),
            latest: None,
            purge_digest: None,
            compiler_program: None,
            first_promotion: None,
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path.join("project.sqlite")
    }

    pub fn given_an_optional_issue_note_and_project_administrator(&mut self) -> &mut Self {
        let output = merl(&[
            "project",
            "init",
            "--id",
            "P1",
            "--database",
            path(&self.database()),
            "--administrator",
            "pm",
            "--json",
        ]);
        assert!(output.status.success(), "project initialization failed");
        let mut store = Store::open(&self.database()).expect("authority");
        let project = ProjectId::try_from("P1").expect("project");
        store
            .capture_source_version(
                &project,
                &SourceCapture {
                    binding: SourceBinding {
                        id: SourceBindingId::try_from("binding").expect("binding"),
                        provider: SourceProvider::try_from("controlled").expect("provider"),
                        provider_namespace_id: "fixture".into(),
                        namespace_digest: Sha256::digest(b"fixture").into(),
                    },
                    source: SourceId::try_from("optional-note").expect("source"),
                    provider_entity_id: "optional-note",
                    context_scope_id: "issue-204",
                    version: SourceVersionId::try_from("optional-note-v1").expect("version"),
                    provider_version_id: "optional-note-v1",
                    kind: SourceKind::try_from("issue_comment").expect("kind"),
                    supersedes: None,
                    ambiguous_order_with_previous: false,
                    created_at_millis: 1,
                    occurred_at_millis: 1,
                    upstream_updated_at_millis: Some(1),
                    observed_at_millis: 1,
                    actor: Some(ActorId::try_from("researcher").expect("actor")),
                    provider_actor_id: Some("researcher"),
                    source_author: Some(ActorId::try_from("researcher").expect("author")),
                    provider_source_author_id: Some("researcher"),
                    body: Some(b"Optional safety note."),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::CaptureOnly,
                    coverage_requirement: CoverageRequirement::Optional,
                    policy_version: CapturePolicyVersion::try_from("fixture-v1").expect("policy"),
                },
            )
            .expect("capture optional note");
        self
    }

    pub fn when_a_non_administrator_requires_the_note(&mut self) -> &mut Self {
        self.latest = Some(require_note(
            &self.database(),
            "researcher",
            "Untrusted reason",
        ));
        self
    }

    pub fn then_the_requirement_is_rejected_and_the_note_stays_optional(&mut self) -> &mut Self {
        assert_eq!(self.latest.as_ref().expect("result")["outcome"], "rejected");
        let store = Store::open(&self.database()).expect("authority");
        let coverage = store
            .semantic_coverage_in_scope(&ProjectId::try_from("P1").expect("project"), "issue-204")
            .expect("coverage");
        assert_eq!(coverage.required_gaps, 0);
        assert_eq!(coverage.optional_cold, 1);
        self
    }

    pub fn when_issue_coverage_is_read(&mut self) -> &mut Self {
        let store = Store::open(&self.database()).expect("authority");
        let coverage = store
            .semantic_coverage_in_scope(&ProjectId::try_from("P1").expect("project"), "issue-204")
            .expect("coverage");
        self.latest = Some(serde_json::json!({
            "required_gaps": coverage.required_gaps,
            "optional_cold": coverage.optional_cold
        }));
        self
    }

    pub fn then_the_cold_note_is_optional(&mut self) -> &mut Self {
        let coverage = self.latest.as_ref().expect("coverage");
        assert_eq!(coverage["required_gaps"], 0);
        assert_eq!(coverage["optional_cold"], 1);
        self
    }

    pub fn when_the_note_is_required_for_the_issue(&mut self) -> &mut Self {
        let promotion = require_note(&self.database(), "pm", "Required safety evidence");
        self.first_promotion = Some(promotion.clone());
        self.latest = Some(promotion);
        self
    }

    pub fn then_the_note_becomes_a_required_gap_with_an_audit_record(&mut self) -> &mut Self {
        let promotion = self.latest.as_ref().expect("promotion");
        assert_eq!(promotion["outcome"], "promoted");
        let store = Store::open(&self.database()).expect("authority");
        let project = ProjectId::try_from("P1").expect("project");
        let source = SourceVersionId::try_from("optional-note-v1").expect("source");
        let coverage = store
            .semantic_coverage_in_scope(&project, "issue-204")
            .expect("coverage");
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.optional_cold, 0);
        let audit = store
            .coverage_promotion(&project, &source, "issue-204")
            .expect("promotion audit")
            .expect("promotion");
        assert_eq!(audit.actor.as_str(), "pm");
        assert_eq!(audit.reason.as_str(), promotion["reason_payload"]);
        self
    }

    pub fn when_ordinary_project_views_are_read(&mut self) -> &mut Self {
        let database = self.database();
        let project_view = merl(&[
            "project",
            "view",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--role",
            "pm",
            "--json",
        ]);
        let issue_view = merl(&[
            "issue",
            "view",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--issue",
            "issue-204",
            "--scope",
            "issue-204",
            "--role",
            "researcher",
            "--json",
        ]);
        let delta = merl(&[
            "project",
            "delta",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--since",
            "0",
            "--json",
        ]);
        for output in [&project_view, &issue_view, &delta] {
            assert!(output.status.success(), "view command failed");
        }
        self.latest = Some(serde_json::json!({
            "project": serde_json::from_slice::<serde_json::Value>(&project_view.stdout)
                .expect("project view"),
            "issue": serde_json::from_slice::<serde_json::Value>(&issue_view.stdout)
                .expect("issue view"),
            "delta": serde_json::from_slice::<serde_json::Value>(&delta.stdout)
                .expect("project delta")
        }));
        self
    }

    pub fn then_the_coverage_change_is_auditable_but_not_semantic_context(&mut self) -> &mut Self {
        let views = self.latest.as_ref().expect("views");
        assert_eq!(views["project"]["project_revision"], 1);
        assert_eq!(views["issue"]["project_revision"], 1);
        assert_eq!(
            views["delta"]["batches"]
                .as_array()
                .expect("accepted batches")
                .len(),
            1
        );
        for view in [&views["project"], &views["issue"]] {
            let objects = view["objects"].as_array().expect("semantic objects");
            assert!(objects.iter().all(|object| {
                object["kind"] != "source_coverage_requirement"
                    && object["summary"] != "Required safety evidence"
            }));
        }
        self
    }

    pub fn when_the_same_requirement_is_retried(&mut self) -> &mut Self {
        let output = merl(&[
            "source",
            "require",
            "--project",
            "P1",
            "--database",
            path(&self.database()),
            "--version",
            "optional-note-v1",
            "--scope",
            "issue-204",
            "--actor",
            "pm",
            "--reason",
            "Required safety evidence",
            "--json",
        ]);
        assert!(
            output.status.success(),
            "stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("retry result"));
        self
    }

    pub fn then_the_retry_returns_the_original_promotion(&mut self) {
        let first = self.first_promotion.as_ref().expect("first promotion");
        let retry = self.latest.as_ref().expect("retry result");
        assert_eq!(retry["outcome"], "unchanged");
        assert_eq!(retry["promoted_at_millis"], first["promoted_at_millis"]);
        assert_eq!(retry["reason_payload"], first["reason_payload"]);
    }

    pub fn when_the_required_note_is_compiled(&mut self) -> &mut Self {
        self.latest =
            Some(self.compile_optional_note("optional-note-v1", "optional-note-run-v1", "pm"));
        self
    }

    pub fn when_a_non_administrator_compiles_the_required_note(&mut self) -> &mut Self {
        self.latest = Some(self.compile_optional_note(
            "optional-note-v1",
            "rejected-optional-note-run",
            "researcher",
        ));
        self
    }

    pub fn then_the_compile_request_is_rejected_without_spending_compiler_work(
        &mut self,
    ) -> &mut Self {
        assert_eq!(
            self.latest.as_ref().expect("compile result")["outcome"],
            "rejected"
        );
        let store = Store::open(&self.database()).expect("authority");
        let project = ProjectId::try_from("P1").expect("project");
        let source = SourceVersionId::try_from("optional-note-v1").expect("source");
        assert_eq!(
            store
                .compilation_run_count(&project, &source)
                .expect("compiler runs"),
            0
        );
        self
    }

    pub fn then_required_coverage_is_complete_with_an_authorization_record(&mut self) -> &mut Self {
        assert_eq!(
            self.latest.as_ref().expect("compile result")["outcome"],
            "compiled"
        );
        let store = Store::open(&self.database()).expect("authority");
        let project = ProjectId::try_from("P1").expect("project");
        let coverage = store
            .semantic_coverage_in_scope(&project, "issue-204")
            .expect("coverage");
        assert_eq!(coverage.required_gaps, 0);
        let run = self.latest.as_ref().expect("compile result")["run"]
            .as_str()
            .expect("run");
        let authorization = store
            .compilation_authorization(&project, run)
            .expect("authorization lookup")
            .expect("authorization");
        assert_eq!(authorization.actor.as_str(), "pm");
        assert_eq!(authorization.compiler_id, "process");
        assert_eq!(authorization.compiler_version, "v1");
        assert_eq!(authorization.model_id, "deterministic-test");
        assert_eq!(authorization.prompt_digest, [0; 32]);
        assert_eq!(
            store
                .read_payload(&project, &authorization.reason)
                .expect("authorization reason"),
            merl_store::PayloadRead::Available(b"Required compiler processing".to_vec())
        );
        self
    }

    pub fn when_the_optional_note_is_edited(&mut self) -> &mut Self {
        let mut store = Store::open(&self.database()).expect("authority");
        let project = ProjectId::try_from("P1").expect("project");
        store
            .capture_source_version(
                &project,
                &SourceCapture {
                    binding: SourceBinding {
                        id: SourceBindingId::try_from("binding").expect("binding"),
                        provider: SourceProvider::try_from("controlled").expect("provider"),
                        provider_namespace_id: "fixture".into(),
                        namespace_digest: Sha256::digest(b"fixture").into(),
                    },
                    source: SourceId::try_from("optional-note").expect("source"),
                    provider_entity_id: "optional-note",
                    context_scope_id: "issue-204",
                    version: SourceVersionId::try_from("optional-note-v2").expect("version"),
                    provider_version_id: "optional-note-v2",
                    kind: SourceKind::try_from("issue_comment").expect("kind"),
                    supersedes: Some(
                        SourceVersionId::try_from("optional-note-v1").expect("prior version"),
                    ),
                    ambiguous_order_with_previous: false,
                    created_at_millis: 1,
                    occurred_at_millis: 2,
                    upstream_updated_at_millis: Some(2),
                    observed_at_millis: 2,
                    actor: Some(ActorId::try_from("researcher").expect("actor")),
                    provider_actor_id: Some("researcher"),
                    source_author: Some(ActorId::try_from("researcher").expect("author")),
                    provider_source_author_id: Some("researcher"),
                    body: Some(b"Updated optional safety note."),
                    edit_diff: None,
                    edit_deleted_at_millis: None,
                    missing_body_reason: None,
                    compilation_mode: CompilationMode::CaptureOnly,
                    coverage_requirement: CoverageRequirement::Optional,
                    policy_version: CapturePolicyVersion::try_from("fixture-v1").expect("policy"),
                },
            )
            .expect("capture edited note");
        self
    }

    pub fn then_the_edit_is_a_required_gap(&mut self) -> &mut Self {
        let store = Store::open(&self.database()).expect("authority");
        let coverage = store
            .semantic_coverage_in_scope(&ProjectId::try_from("P1").expect("project"), "issue-204")
            .expect("coverage");
        assert_eq!(coverage.required_gaps, 1);
        assert_eq!(coverage.optional_cold, 0);
        self
    }

    pub fn when_the_required_edit_is_compiled(&mut self) -> &mut Self {
        self.latest =
            Some(self.compile_optional_note("optional-note-v2", "optional-note-run-v2", "pm"));
        self
    }

    fn compile_optional_note(&self, version: &str, run: &str, actor: &str) -> serde_json::Value {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let program = self.directory.path.join(format!("compiler-{version}.sh"));
            let response = format!(
                "{{\"schema\":\"merl.compiler-response/v1\",\"assertions\":[],\"context_required\":[],\"unresolved\":[{{\"source\":\"{version}\",\"span_start\":0,\"span_end\":0}}],\"relations\":[]}}"
            );
            std::fs::write(
                &program,
                format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{response}'\n"),
            )
            .expect("test compiler program");
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
                .expect("make compiler executable");
            let output = merl(&[
                "source",
                "compile",
                "--project",
                "P1",
                "--database",
                path(&self.database()),
                "--version",
                version,
                "--run",
                run,
                "--actor",
                actor,
                "--reason",
                "Required compiler processing",
                "--program",
                path(&program),
                "--compiler-version",
                "v1",
                "--model",
                "deterministic-test",
                "--prompt-digest",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "--json",
            ]);
            assert!(
                output.status.success(),
                "stdout: {} stderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            serde_json::from_slice(&output.stdout).expect("compile result")
        }
        #[cfg(not(unix))]
        panic!("the CLI compiler acceptance adapter requires a Unix process");
    }

    pub fn given_a_new_project(&mut self) -> &mut Self {
        let database = self.database();
        let initialized = merl(&[
            "project",
            "init",
            "--id",
            "P1",
            "--database",
            path(&database),
            "--json",
        ]);
        assert!(initialized.status.success());
        self
    }

    pub fn given_two_projects_with_the_same_issue(&mut self) -> &mut Self {
        self.given_a_new_project();
        self.when_importing_an_issue();
        let database = self.database();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");
        let initialized = merl(&[
            "project",
            "init",
            "--id",
            "P2",
            "--database",
            path(&database),
        ]);
        assert!(initialized.status.success());
        let imported = merl(&[
            "issue",
            "import-fixture",
            "--project",
            "P2",
            "--database",
            path(&database),
            "--fixture",
            path(&fixture),
        ]);
        assert!(imported.status.success());
        self
    }

    pub fn when_importing_an_issue(&mut self) -> &mut Self {
        let database = self.database();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");
        let output = merl(&[
            "issue",
            "import-fixture",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--fixture",
            path(&fixture),
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("import result"));
        self
    }

    pub fn when_importing_the_same_issue_again(&mut self) -> &mut Self {
        self.when_importing_an_issue()
    }

    pub fn then_four_versions_are_captured(&mut self) -> &mut Self {
        let result = self.latest.as_ref().expect("import result");
        assert_eq!(result["action"], "issue.import-fixture");
        assert_eq!(result["outcome"], "captured");
        assert_eq!(result["captured"], 4);
        assert_eq!(result["observation_head"], 4);
        assert_eq!(result["revision"], 1);
        assert!(
            result["issue"]
                .as_str()
                .is_some_and(|id| id.starts_with("pi_"))
        );
        assert_eq!(result["scope"], "controlled:DEV-C3:issue");
        self
    }

    pub fn then_nothing_new_is_captured(&mut self) -> &mut Self {
        let result = self.latest.as_ref().expect("import result");
        assert_eq!(result["captured"], 0);
        assert_eq!(result["outcome"], "unchanged");
        assert_eq!(result["observation_head"], 4);
        assert_eq!(result["revision"], 1);
        self
    }

    fn first_version() -> String {
        merl_ingest::fixture_version_id("controlled:DEV-C3:issue:v1")
            .expect("source version")
            .to_string()
    }

    pub fn when_a_source_purge_is_previewed(&mut self) -> &mut Self {
        let database = self.database();
        let version = Self::first_version();
        let output = merl(&[
            "source",
            "purge",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--version",
            &version,
            "--reason",
            "Sensitive text",
            "--dry-run",
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let preview: serde_json::Value = serde_json::from_slice(&output.stdout).expect("preview");
        self.purge_digest = Some(preview["confirm_digest"].as_str().expect("digest").into());
        self.latest = Some(preview);
        self
    }

    pub fn then_the_preview_names_protected_bytes(&mut self) -> &mut Self {
        let preview = self.latest.as_ref().expect("preview");
        assert_eq!(preview["action"], "source.purge.preview");
        assert!(!preview["payloads"].as_array().expect("payloads").is_empty());
        self
    }

    pub fn when_the_preview_is_confirmed(&mut self) -> &mut Self {
        let database = self.database();
        let version = Self::first_version();
        let digest = self.purge_digest.as_deref().expect("preview digest");
        let output = merl(&[
            "source",
            "purge",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--version",
            &version,
            "--reason",
            "Sensitive text",
            "--actor",
            "admin",
            "--confirm-digest",
            digest,
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("purge result"));
        self
    }

    pub fn then_source_content_is_unavailable_and_the_audit_is_complete(&mut self) -> &mut Self {
        assert_eq!(self.latest.as_ref().expect("purge")["completed"], true);
        let database = self.database();
        let version = Self::first_version();
        let output = merl(&[
            "source",
            "show",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--version",
            &version,
            "--json",
        ]);
        assert!(output.status.success());
        let source: serde_json::Value = serde_json::from_slice(&output.stdout).expect("source");
        assert_eq!(source["body"]["status"], "unavailable");
        assert_eq!(source["body"]["reason"], "source_content_unavailable");
        assert_eq!(source["body"]["tombstone"]["source"], version);
        self
    }

    pub fn when_the_purge_audit_is_read(&mut self) -> &mut Self {
        let database = self.database();
        let version = Self::first_version();
        let output = merl(&[
            "source",
            "purge-audit",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--version",
            &version,
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("audit result"));
        self
    }

    pub fn then_the_audit_names_the_actor_reason_and_tombstoned_payload(&mut self) -> &mut Self {
        let audit = self.latest.as_ref().expect("audit result");
        assert_eq!(audit["actor"], "admin");
        assert_eq!(audit["reason"]["text"], "Sensitive text");
        assert_eq!(audit["completed"], true);
        assert!(
            audit["payloads"]
                .as_array()
                .expect("payload receipts")
                .iter()
                .any(|item| item["id"].as_str().is_some_and(|id| id.starts_with("src_")))
        );
        self
    }

    pub fn then_active_store_no_longer_contains_the_source_body(&mut self) {
        let active_file = std::fs::read(self.database()).expect("active store bytes");
        let removed = b"For report A, export CSV with a sample_id column.";
        assert!(
            !active_file
                .windows(removed.len())
                .any(|window| window == removed)
        );
    }

    pub fn then_the_other_projects_copy_remains_available(&mut self) {
        let database = self.database();
        let version = Self::first_version();
        let output = merl(&[
            "source",
            "show",
            "--project",
            "P2",
            "--database",
            path(&database),
            "--version",
            &version,
            "--json",
        ]);
        assert!(output.status.success());
        let source: serde_json::Value = serde_json::from_slice(&output.stdout).expect("source");
        assert_eq!(source["body"]["status"], "available");
    }

    pub fn when_the_project_projection_is_rebuilt(&mut self) -> &mut Self {
        let database = self.database();
        let output = merl(&[
            "project",
            "rebuild",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("rebuild result"));
        self
    }

    pub fn then_the_accepted_revision_and_issue_state_are_unchanged(&mut self) {
        let result = self.latest.as_ref().expect("rebuild result");
        assert_eq!(result["action"], "project.rebuild");
        assert_eq!(result["revision"], 1);
        assert_eq!(result["accepted_events"], 1);
        let database = self.database();
        let store = merl_store::Store::open(&database).expect("reopen authority");
        let project = merl_core::ProjectId::try_from("P1").expect("project");
        let source = merl_core::SourceVersionId::try_from(Self::first_version().as_str())
            .expect("source version");
        assert_eq!(
            store
                .compilation_run_count(&project, &source)
                .expect("compiler runs"),
            0
        );
    }

    pub fn then_rebuild_reports_degraded_provenance_at_the_same_revision(&mut self) {
        let result = self.latest.as_ref().expect("rebuild result");
        assert_eq!(result["action"], "project.rebuild");
        assert_eq!(result["revision"], 1);
        assert_eq!(result["provenance"], "degraded");
        assert!(result["erased_payloads"].as_u64().expect("erased count") > 0);
    }

    pub fn given_an_issue_with_a_recorded_compiler_run(&mut self) -> &mut Self {
        self.given_a_new_project();
        self.when_importing_an_issue();
        let database = self.database();
        let mut store = merl_store::Store::open(&database).expect("authority");
        let project = merl_core::ProjectId::try_from("P1").expect("project");
        let source = merl_core::SourceVersionId::try_from(Self::first_version().as_str())
            .expect("source version");
        let limits = merl_compiler::CompilerLimits {
            input_bytes: 4096,
            output_bytes: 4096,
            output_tokens: 512,
            assertions: 4,
            context_requests: 1,
            expansion_rounds: 1,
            payload_bytes: 2048,
            source_window: 2,
            objects: 4,
        };
        let prepared = merl_compiler::prepare_eager_compilation(
            &mut store,
            &project,
            &source,
            &merl_compiler::FakeCompiler,
            merl_compiler::RunRequest {
                id: "recorded-run",
                limits,
                mode: merl_compiler::RunMode::Live,
                now_millis: 1_800_000_000_000,
            },
        )
        .expect("prepare compiler")
        .expect("new run");
        let response = merl_compiler::execute_compilation(&prepared, &merl_compiler::FakeCompiler);
        merl_compiler::record_compilation_result(
            &mut store,
            &project,
            &prepared,
            response,
            1_800_000_000_001,
        )
        .expect("record compiler response");
        self
    }

    pub fn when_the_recorded_source_is_replayed(&mut self) -> &mut Self {
        let database = self.database();
        let output = merl(&[
            "source",
            "replay",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--run",
            "recorded-run",
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("replay result"));
        self
    }

    pub fn when_the_purged_source_is_replayed(&mut self) -> &mut Self {
        let database = self.database();
        let output = merl(&[
            "source",
            "replay",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--run",
            "recorded-run",
            "--json",
        ]);
        assert!(!output.status.success());
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("replay error"));
        self
    }

    pub fn then_replay_reports_missing_evidence(&mut self) {
        assert_eq!(
            self.latest.as_ref().expect("replay error")["code"],
            "MISSING_EVIDENCE"
        );
    }

    pub fn then_the_input_digest_matches_without_an_accepted_change(&mut self) {
        let result = self.latest.as_ref().expect("replay result");
        assert_eq!(result["action"], "source.replay");
        assert_eq!(result["input_matches"], true);
        assert_eq!(result["interpretation_basis_revision"], 0);
        assert_eq!(result["source_observation_cutoff"], 1);
        assert_eq!(result["accepted_revision"], 1);
    }

    #[cfg(unix)]
    pub fn given_a_deterministic_compiler_process(&mut self) -> &mut Self {
        use std::os::unix::fs::PermissionsExt;
        let program = self.directory.path.join("compiler.sh");
        let source = Self::first_version();
        let response = format!(
            "{{\"schema\":\"merl.compiler-response/v1\",\"assertions\":[],\"context_required\":[],\"unresolved\":[{{\"source\":\"{source}\",\"span_start\":0,\"span_end\":0}}],\"relations\":[]}}"
        );
        // Reading the request avoids a scheduler-dependent broken pipe in the parent writer.
        std::fs::write(
            &program,
            format!("#!/bin/sh\ncat >/dev/null\nprintf '%s' '{response}'\n"),
        )
        .expect("test compiler program");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
            .expect("make compiler executable");
        self.compiler_program = Some(program);
        self
    }

    #[cfg(unix)]
    pub fn when_the_recorded_source_is_recompiled(&mut self) -> &mut Self {
        let database = self.database();
        let program = self.compiler_program.as_ref().expect("compiler program");
        let output = merl(&[
            "source",
            "replay",
            "--project",
            "P1",
            "--database",
            path(&database),
            "--run",
            "recorded-run",
            "--program",
            path(program),
            "--new-run",
            "replay-process",
            "--compiler-version",
            "v1",
            "--model",
            "deterministic-test",
            "--prompt-digest",
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            "--json",
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        self.latest = Some(serde_json::from_slice(&output.stdout).expect("replay result"));
        self
    }

    #[cfg(unix)]
    pub fn then_a_new_replay_run_exists_without_an_accepted_change(&mut self) {
        let result = self.latest.as_ref().expect("replay result");
        assert_eq!(result["rerun"], "replay-process");
        assert_eq!(result["accepted_history_changed"], false);
        assert_eq!(result["accepted_revision"], 1);
        let store = merl_store::Store::open(&self.database()).expect("authority");
        let project = merl_core::ProjectId::try_from("P1").expect("project");
        let rerun = store
            .compilation_run_status(&project, "replay-process")
            .expect("run lookup")
            .expect("run");
        assert_eq!(rerun.mode, "replay");
        assert!(rerun.succeeded);
        let original = store
            .load_compilation_context(&project, "recorded-run")
            .expect("recorded input");
        let replay = store
            .load_compilation_context(&project, "replay-process")
            .expect("new replay input");
        assert_eq!(replay.rendered, original.rendered);
        assert_eq!(replay.source_window, original.source_window);
        assert_eq!(replay.objects, original.objects);
        assert_eq!(rerun.selector_version, "issue_context_v1");
    }
}

fn require_note(database: &Path, actor: &str, reason: &str) -> serde_json::Value {
    let output = merl(&[
        "source",
        "require",
        "--project",
        "P1",
        "--database",
        path(database),
        "--version",
        "optional-note-v1",
        "--scope",
        "issue-204",
        "--actor",
        actor,
        "--reason",
        reason,
        "--json",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("require result")
}

fn merl(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_merl"))
        .args(arguments)
        .output()
        .expect("run Merl")
}

fn path(path: &Path) -> &str {
    path.to_str().expect("UTF-8 test path")
}

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Self {
        for index in 0..1_000 {
            let path = std::env::temp_dir()
                .join(format!("merl-issue-import-{}-{index}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create temporary project directory: {error}"),
            }
        }
        panic!("could not reserve a temporary project directory")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).expect("remove temporary project directory");
    }
}
