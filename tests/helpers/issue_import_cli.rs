use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

pub struct CliIssueHistory {
    directory: TestDirectory,
    latest: Option<serde_json::Value>,
    purge_digest: Option<String>,
}

impl CliIssueHistory {
    pub fn new() -> Self {
        Self {
            directory: TestDirectory::new(),
            latest: None,
            purge_digest: None,
        }
    }

    fn database(&self) -> PathBuf {
        self.directory.path.join("project.sqlite")
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
