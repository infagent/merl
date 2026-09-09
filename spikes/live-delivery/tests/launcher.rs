use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::process::Stdio;

use merl_live_delivery_spike::HostDelivery;
use merl_live_delivery_spike::host::codex::{ExactThread, QueueDelivery};
use tempfile::TempDir;

const THREAD_ID: &str = "0199aaaa-bbbb-4ccc-8ddd-dddddddddddd";

fn fake_codex() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("temporary fake directory");
    let executable = directory.path().join("codex");
    let recording = executable.with_extension("recording");
    fs::write(
        &executable,
        "#!/usr/bin/env bash\nset -euo pipefail\nrecording=${MERL_TEST_RECORDING:-$0.recording}\nprintf '%s\\n' \"$@\" >\"$recording\"\nprintf '%s' \"${MERL_TEST_INHERITED:-}\" >>\"$recording\"\nexit \"${MERL_TEST_EXIT_STATUS:-0}\"\n",
    )
    .expect("write fake Codex");
    let mut permissions = fs::metadata(&executable)
        .expect("fake metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).expect("make fake executable");
    (directory, executable, recording)
}

#[test]
fn exact_thread_launcher_resumes_stock_codex_with_unchanged_arguments_and_environment() {
    let (_directory, executable, recording) = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "codex",
            "--program",
            executable.to_str().expect("UTF-8 path"),
            "--thread",
            THREAD_ID,
            "--",
            "--model",
            "test-model",
            "--no-alt-screen",
        ])
        .env("MERL_TEST_RECORDING", &recording)
        .env("MERL_TEST_INHERITED", "inherited-marker")
        .status()
        .expect("run launcher");

    assert!(status.success());
    assert_eq!(
        fs::read_to_string(recording).expect("read recording"),
        format!("resume\n{THREAD_ID}\n--model\ntest-model\n--no-alt-screen\ninherited-marker")
    );
}

#[test]
fn exact_thread_launcher_returns_the_stock_codex_exit_status() {
    let (_directory, executable, recording) = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "codex",
            "--program",
            executable.to_str().expect("UTF-8 path"),
            "--thread",
            THREAD_ID,
        ])
        .env("MERL_TEST_RECORDING", recording)
        .env("MERL_TEST_EXIT_STATUS", "23")
        .status()
        .expect("run launcher");

    assert_eq!(status.code(), Some(23));
}

#[test]
fn queue_delivery_targets_only_the_bound_uuid_with_the_complete_envelope() {
    let (_directory, executable, recording) = fake_codex();
    let thread = ExactThread::parse(THREAD_ID).expect("exact UUID");
    let message = "MERL_LIVE_RESULT_V1\nauthorization=none\n\nuntrusted payload";
    let mut delivery = QueueDelivery::new(executable, thread);

    delivery.deliver(message).expect("queue delivery");

    assert_eq!(
        fs::read_to_string(recording).expect("read recording"),
        format!("queue\n--thread\n{THREAD_ID}\n--message\n{message}\n")
    );
}

#[test]
fn non_uuid_binding_is_rejected_instead_of_using_a_name_or_inference() {
    let error = ExactThread::parse("last").expect_err("recency alias must fail");

    assert!(error.contains("exact UUID"));
}

#[test]
fn claude_channel_declares_only_the_channel_capability_and_emits_one_notification() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args(["claude-channel", "--message", "untrusted result"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start channel server");
    let stdin = child.stdin.as_mut().expect("channel stdin");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "fake-claude", "version": "0"}
            }
        })
    )
    .expect("send initialize");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        })
    )
    .expect("send initialized");
    drop(child.stdin.take());

    let output = child.wait_with_output().expect("read channel output");
    assert!(output.status.success());
    let messages: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .expect("UTF-8 protocol output")
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON-RPC message"))
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["id"], 1);
    assert_eq!(
        messages[0]["result"]["capabilities"]["experimental"]["claude/channel"],
        serde_json::json!({})
    );
    assert!(messages[0]["result"]["capabilities"].get("tools").is_none());
    assert!(
        messages[0]["result"]["capabilities"]["experimental"]
            .get("claude/channel/permission")
            .is_none()
    );
    assert_eq!(messages[1]["method"], "notifications/claude/channel");
    assert_eq!(messages[1]["params"]["content"], "untrusted result");
    assert_eq!(messages[1]["params"]["meta"]["authorization"], "none");
}

#[test]
fn claude_launcher_preserves_arguments_and_preview_consent() {
    let (_directory, executable, recording) = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "claude",
            "--program",
            executable.to_str().expect("UTF-8 path"),
            "--server-name",
            "merl-test",
            "--message",
            "result",
            "--",
            "--model",
            "test-model",
        ])
        .env("MERL_TEST_RECORDING", &recording)
        .status()
        .expect("run Claude launcher");
    assert!(status.success());

    let arguments = fs::read_to_string(recording).expect("read recording");
    assert!(arguments.contains("--mcp-config\n"));
    assert!(arguments.contains("--strict-mcp-config\n"));
    assert!(arguments.contains("--dangerously-load-development-channels\nserver:merl-test\n"));
    assert!(arguments.ends_with("--model\ntest-model\n"));
    assert!(!arguments.contains("--dangerously-skip-permissions"));
}
