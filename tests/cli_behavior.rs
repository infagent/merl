use std::process::Command;

#[test]
fn cli_help_is_incremental_and_machine_readable() {
    let top = run(&["--help"]);
    assert!(top.status.success());
    let top_text = String::from_utf8(top.stdout).expect("UTF-8 help");
    assert!(top_text.contains("project"));
    assert!(!top_text.contains("--database"));

    let group = run(&["help", "project"]);
    assert!(group.status.success());
    assert!(
        String::from_utf8(group.stdout)
            .expect("UTF-8 help")
            .contains("revision")
    );

    let command = run(&["help", "project", "revision", "--format", "json"]);
    assert!(command.status.success());
    let help: serde_json::Value = serde_json::from_slice(&command.stdout).expect("JSON help");
    assert_eq!(help["schema"], "merl.help/v1");
    assert_eq!(help["command"], "project revision");

    let initialized = run(&[
        "project",
        "init",
        "--id",
        "P1",
        "--database",
        ":memory:",
        "--format",
        "json",
    ]);
    assert!(initialized.status.success());
    let result: serde_json::Value =
        serde_json::from_slice(&initialized.stdout).expect("JSON result");
    assert_eq!(result["schema"], "merl.result/v1");
    assert_eq!(result["action"], "project.init");
    assert_eq!(result["revision"], 0);

    let bad = run(&["project", "unknown", "--format", "json"]);
    assert!(!bad.status.success());
    let error: serde_json::Value = serde_json::from_slice(&bad.stdout).expect("JSON error");
    assert_eq!(error["schema"], "merl.error/v1");
    assert_eq!(error["code"], "INVALID_INPUT");
}

fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_merl"))
        .args(arguments)
        .output()
        .expect("run Merl")
}
