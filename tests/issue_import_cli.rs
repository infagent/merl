use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn an_issue_fixture_import_is_offline_and_retryable() {
    let directory = TestDirectory::new();
    let database = directory.path.join("project.sqlite");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus/development/DEV-C3.json");

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

    let first = merl(&[
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
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).expect("first result");
    assert_eq!(first["action"], "issue.import-fixture");
    assert_eq!(first["outcome"], "captured");
    assert_eq!(first["captured"], 4);
    assert_eq!(first["observation_head"], 4);
    assert_eq!(first["revision"], 1);

    let repeated = merl(&[
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
    assert!(repeated.status.success());
    let repeated: serde_json::Value =
        serde_json::from_slice(&repeated.stdout).expect("retry result");
    assert_eq!(repeated["captured"], 0);
    assert_eq!(repeated["outcome"], "unchanged");
    assert_eq!(repeated["observation_head"], 4);
    assert_eq!(repeated["revision"], 1);
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
