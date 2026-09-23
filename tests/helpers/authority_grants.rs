use std::process::Command;

use serde_json::Value;
use std::path::PathBuf;

pub struct AuthorityScenario {
    directory: TestDirectory,
    grants: Vec<Value>,
    changes: Vec<Value>,
    reads: Vec<Value>,
    rejected: Vec<Value>,
    retry: Option<Value>,
    audit: Vec<merl_store::RecordedPolicyEvaluation>,
    human: Vec<String>,
}

impl AuthorityScenario {
    pub fn given_two_projects_and_an_administrator() -> Self {
        let scenario = Self {
            directory: TestDirectory::new(),
            grants: Vec::new(),
            changes: Vec::new(),
            reads: Vec::new(),
            rejected: Vec::new(),
            retry: None,
            audit: Vec::new(),
            human: Vec::new(),
        };
        for project in ["P1", "P2"] {
            scenario.run(&[
                "project",
                "init",
                "--id",
                project,
                "--administrator",
                "admin",
            ]);
        }
        scenario
    }

    pub fn when_both_semantic_permissions_are_granted(mut self) -> Self {
        for permission in ["decision_author", "command_actor"] {
            self.grants
                .push(self.change("grant", "admin", "alice", permission, permission));
        }
        self
    }

    pub fn when_grants_are_read_after_restart(mut self) -> Self {
        self.reads = ["P1", "P2"]
            .iter()
            .map(|project| self.run(&["project", "authority", "list", "--project", project]))
            .collect();
        let store = merl_store::Store::open(&self.directory.path.join("authority.sqlite"))
            .expect("restart");
        let project = merl_core::ProjectId::try_from("P1").expect("project");
        self.audit = self
            .grants
            .iter()
            .map(|grant| {
                let evaluation = merl_core::PolicyEvaluationId::try_from(
                    grant["evaluation"].as_str().expect("evaluation ID"),
                )
                .expect("ID");
                store
                    .policy_evaluation(&project, &evaluation)
                    .expect("read audit")
                    .expect("original grant retained")
            })
            .collect();
        self
    }

    pub fn then_grants_and_audit_are_visible_only_in_the_target_project(self) -> Self {
        assert_eq!(self.reads[0]["schema"], "merl.authority/v1");
        assert_eq!(
            self.reads[0]["decision_authors"],
            serde_json::json!(["alice"])
        );
        assert_eq!(
            self.reads[0]["command_actors"],
            serde_json::json!(["alice"])
        );
        assert_eq!(
            self.reads[0]["administrators"],
            serde_json::json!(["admin"])
        );
        assert_eq!(self.reads[1]["decision_authors"], serde_json::json!([]));
        assert_eq!(self.reads[1]["command_actors"], serde_json::json!([]));
        for (index, grant) in self.grants.iter().enumerate() {
            assert_eq!(grant["outcome"], "accepted");
            assert_eq!(grant["actor"], "admin");
            assert_eq!(grant["revision"], index + 1);
            assert_eq!(grant["policy_version"], "authority_v1");
            assert!(grant["evaluation"].is_string());
        }
        assert_ne!(
            self.grants[0]["configuration_digest"],
            self.grants[1]["configuration_digest"]
        );
        self
    }

    pub fn when_both_permissions_are_revoked(mut self) -> Self {
        for permission in ["decision_author", "command_actor"] {
            self.changes.push(self.change(
                "revoke",
                "admin",
                "alice",
                permission,
                &format!("revoke_{permission}"),
            ));
        }
        self
    }

    pub fn then_revocation_changes_the_digest_and_preserves_history(self) {
        assert_eq!(self.reads[0]["decision_authors"], serde_json::json!([]));
        assert_eq!(self.reads[0]["command_actors"], serde_json::json!([]));
        assert_eq!(self.reads[0]["revision"], 4);
        assert_eq!(
            self.reads[0]["configuration_digest"],
            self.reads[1]["configuration_digest"]
        );
        assert_ne!(
            self.changes[0]["configuration_digest"],
            self.changes[1]["configuration_digest"]
        );
        assert_eq!(self.changes[1]["revision"], 4);
        assert_eq!(self.audit.len(), 2);
        for (index, record) in self.audit.iter().enumerate() {
            assert_eq!(
                record.inputs[0].disposition,
                merl_core::PolicyDisposition::Accepted
            );
            assert_eq!(record.actor.as_str(), "admin");
            assert_eq!(
                record.committed_revision.expect("accepted revision").get(),
                (index + 1) as u64
            );
        }
    }

    pub fn when_an_outsider_attempts_grant_and_revoke(mut self) -> Self {
        self.rejected.push(self.change(
            "grant",
            "outsider",
            "outsider",
            "command_actor",
            "bad_grant",
        ));
        self.rejected.push(self.change(
            "revoke",
            "outsider",
            "alice",
            "decision_author",
            "bad_revoke",
        ));
        self.reads
            .push(self.run(&["project", "authority", "list", "--project", "P1"]));
        self
    }

    pub fn then_rejections_leave_grants_and_revision_unchanged(self) {
        for result in &self.rejected {
            assert_eq!(result["outcome"], "rejected");
            assert!(result["revision"].is_null());
        }
        assert_eq!(self.reads[0]["revision"], 2);
        assert_eq!(
            self.reads[0]["decision_authors"],
            serde_json::json!(["alice"])
        );
        assert_eq!(
            self.reads[0]["command_actors"],
            serde_json::json!(["alice"])
        );
    }

    pub fn when_the_original_grant_is_retried(mut self) -> Self {
        self.retry = Some(self.change("grant", "admin", "alice", "command_actor", "command_actor"));
        self.reads
            .push(self.run(&["project", "authority", "list", "--project", "P1"]));
        self
    }

    pub fn then_the_original_result_returns_without_restoring_authority(self) -> Self {
        assert_eq!(self.retry.as_ref().expect("retry"), &self.grants[1]);
        assert_eq!(self.reads[0]["revision"], 4);
        assert_eq!(self.reads[0]["command_actors"], serde_json::json!([]));
        self
    }

    pub fn when_the_request_id_is_reused_for_another_actor(mut self) -> Self {
        self.retry = Some(self.change("grant", "admin", "bob", "command_actor", "command_actor"));
        self
    }

    pub fn then_the_retry_conflicts(self) {
        assert_eq!(
            self.retry.expect("conflicting retry")["code"],
            "POLICY_INPUT_CONFLICT"
        );
    }

    pub fn when_provider_capture_and_an_administrative_change_are_evaluated(mut self) -> Self {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");
        let imported = self.run(&[
            "issue",
            "import-fixture",
            "--project",
            "P1",
            "--fixture",
            fixture.to_str().expect("fixture path"),
        ]);
        let store =
            merl_store::Store::open(&self.directory.path.join("authority.sqlite")).expect("store");
        let project = merl_core::ProjectId::try_from("P1").expect("project");
        let issue =
            merl_core::ObjectId::try_from(imported["issue"].as_str().expect("imported Issue"))
                .expect("issue ID");
        let evaluation = store
            .object_policy_evaluation(&project, &issue)
            .expect("provider policy")
            .expect("evaluation ID");
        self.audit.push(
            store
                .policy_evaluation(&project, &evaluation)
                .expect("provider audit")
                .expect("record"),
        );
        self.changes
            .push(self.change("grant", "admin", "bob", "decision_author", "grant_bob"));
        self
    }

    pub fn then_both_boundaries_record_the_same_configuration(self) -> Self {
        let record = &self.audit[0];
        let digest = record
            .configuration_digest
            .iter()
            .fold(String::new(), |mut text, byte| {
                use std::fmt::Write as _;
                write!(text, "{byte:02x}").expect("hex digest");
                text
            });
        assert_eq!(self.changes[0]["configuration_digest"], digest);
        assert_eq!(record.version.as_str(), self.changes[0]["policy_version"]);
        self
    }

    pub fn when_the_provider_capture_is_retried_after_the_grant_change(mut self) -> Self {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");
        self.retry = Some(self.run(&[
            "issue",
            "import-fixture",
            "--project",
            "P1",
            "--fixture",
            fixture.to_str().expect("fixture path"),
        ]));
        self.reads
            .push(self.run(&["project", "revision", "--project", "P1"]));
        self
    }

    pub fn then_the_capture_retry_preserves_accepted_history(self) {
        assert_eq!(
            self.retry.as_ref().expect("capture retry")["schema"],
            "merl.result/v1"
        );
        assert_eq!(self.reads[0]["revision"], self.changes[0]["revision"]);
    }

    pub fn when_authority_help_and_human_grant_results_are_read(mut self) -> Self {
        for operation in ["list", "grant", "revoke"] {
            self.reads
                .push(self.run(&["help", "project", "authority", operation]));
        }
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .arg("--database")
            .arg(self.directory.path.join("authority.sqlite"))
            .args([
                "project",
                "authority",
                "grant",
                "--project",
                "P1",
                "--id",
                "human_request",
                "--actor",
                "admin",
                "--subject",
                "alice",
                "--permission",
                "command_actor",
                "--reason",
                "Assigned role",
            ])
            .output()
            .expect("human command");
        self.human
            .push(String::from_utf8(output.stdout).expect("human output"));
        self
    }

    pub fn then_help_and_results_name_the_permission_outcome_and_revision(self) {
        for help in &self.reads {
            assert_eq!(help["schema"], "merl.help/v1");
            assert!(
                help["example"]
                    .as_str()
                    .expect("example")
                    .contains("project authority")
            );
            assert!(
                help["trust_boundary"]
                    .as_str()
                    .expect("trust boundary")
                    .contains("trusted client claim")
            );
        }
        assert!(self.human[0].contains("accepted: grant command_actor for alice by admin"));
        assert!(self.human[0].contains("accepted revision 1"));
    }

    fn change(
        &self,
        operation: &str,
        actor: &str,
        subject: &str,
        permission: &str,
        id: &str,
    ) -> Value {
        self.run(&[
            "project",
            "authority",
            operation,
            "--project",
            "P1",
            "--actor",
            actor,
            "--subject",
            subject,
            "--permission",
            permission,
            "--id",
            id,
            "--reason",
            "Project owner approved the role",
        ])
    }

    fn run(&self, arguments: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(["--json", "--database"])
            .arg(self.directory.path.join("authority.sqlite"))
            .args(arguments)
            .output()
            .expect("run public command");
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("CLI output: {output:?}"))
    }
}

struct TestDirectory {
    path: PathBuf,
}
impl TestDirectory {
    fn new() -> Self {
        for index in 0..1000 {
            let path =
                std::env::temp_dir().join(format!("merl-authority-{}-{index}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("create test directory: {error}"),
            }
        }
        panic!("no free test directory")
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).expect("remove test directory");
    }
}
