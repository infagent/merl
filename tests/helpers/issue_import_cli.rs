use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

pub struct CliIssueHistory {
    directory: TestDirectory,
    latest: Option<serde_json::Value>,
}

impl CliIssueHistory {
    pub fn new() -> Self {
        Self {
            directory: TestDirectory::new(),
            latest: None,
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
