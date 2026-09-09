use std::fs;
use std::io::Write;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::process::Stdio;
use std::time::{Duration, Instant};

use merl_live_delivery_spike::HostDelivery;
use merl_live_delivery_spike::host::codex::{ExactThread, QueueDelivery};
use tempfile::TempDir;

const THREAD_ID: &str = "0199aaaa-bbbb-4ccc-8ddd-dddddddddddd";

struct FakeCodex {
    _directory: TempDir,
    executable: std::path::PathBuf,
    recording: std::path::PathBuf,
}

fn fake_codex() -> FakeCodex {
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
    FakeCodex {
        _directory: directory,
        executable,
        recording,
    }
}

#[test]
fn exact_thread_launcher_resumes_stock_codex_with_unchanged_arguments_and_environment() {
    let fake = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "codex",
            "--program",
            fake.executable.to_str().expect("UTF-8 path"),
            "--thread",
            THREAD_ID,
            "--",
            "--model",
            "test-model",
            "--no-alt-screen",
        ])
        .env("MERL_TEST_RECORDING", &fake.recording)
        .env("MERL_TEST_INHERITED", "inherited-marker")
        .status()
        .expect("run launcher");

    assert!(status.success());
    assert_eq!(
        fs::read_to_string(fake.recording).expect("read recording"),
        format!("resume\n{THREAD_ID}\n--model\ntest-model\n--no-alt-screen\ninherited-marker")
    );
}

#[test]
fn exact_thread_launcher_returns_the_stock_codex_exit_status() {
    let fake = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "codex",
            "--program",
            fake.executable.to_str().expect("UTF-8 path"),
            "--thread",
            THREAD_ID,
        ])
        .env("MERL_TEST_RECORDING", fake.recording)
        .env("MERL_TEST_EXIT_STATUS", "23")
        .status()
        .expect("run launcher");

    assert_eq!(status.code(), Some(23));
}

#[test]
fn queue_delivery_targets_only_the_bound_uuid_with_the_complete_envelope() {
    let fake = fake_codex();
    let thread = ExactThread::parse(THREAD_ID).expect("exact UUID");
    let message = "MERL_LIVE_RESULT_V1\nauthorization=none\n\nuntrusted payload";
    let mut delivery = QueueDelivery::new(&fake.executable, thread);

    delivery.deliver(message).expect("queue delivery");

    assert_eq!(
        fs::read_to_string(fake.recording).expect("read recording"),
        format!("queue\n--thread\n{THREAD_ID}\n--message\n{message}\n")
    );
}

#[test]
fn queue_delivery_times_out_a_hanging_codex_process() {
    let directory = tempfile::tempdir().expect("temporary fake directory");
    let executable = directory.path().join("codex");
    executable_write(&executable, "#!/usr/bin/env bash\nsleep 60\n");
    let thread = ExactThread::parse(THREAD_ID).expect("exact UUID");
    let mut delivery = QueueDelivery::with_timeout(&executable, thread, Duration::from_millis(50));
    let started = Instant::now();

    let error = delivery
        .deliver("untrusted result")
        .expect_err("hanging queue must time out");

    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(2));
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
fn fixed_claude_channel_reports_a_notification_write_failure() {
    let message = "x".repeat(100_000);
    let mut child = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args(["claude-channel", "--message", &message])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start channel server");
    let mut stdin = child.stdin.take().expect("channel stdin");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}}
        })
    )
    .expect("send initialize");
    let mut output = BufReader::new(child.stdout.take().expect("channel stdout"));
    let mut response = String::new();
    output
        .read_line(&mut response)
        .expect("initialize response");
    drop(output);
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        })
    )
    .expect("send initialized");
    drop(stdin);

    let status = child.wait().expect("channel exits");
    assert!(!status.success(), "lost notification must fail the server");
}

#[test]
fn claude_launcher_preserves_arguments_and_preview_consent() {
    let fake = fake_codex();
    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "claude",
            "--program",
            fake.executable.to_str().expect("UTF-8 path"),
            "--server-name",
            "merl-test",
            "--message",
            "result",
            "--",
            "--model",
            "test-model",
        ])
        .env("MERL_TEST_RECORDING", &fake.recording)
        .status()
        .expect("run Claude launcher");
    assert!(status.success());

    let arguments = fs::read_to_string(fake.recording).expect("read recording");
    assert!(arguments.contains("--mcp-config\n"));
    assert!(arguments.contains("--strict-mcp-config\n"));
    assert!(arguments.contains("--dangerously-load-development-channels\nserver:merl-test\n"));
    assert!(arguments.ends_with("--model\ntest-model\n"));
    assert!(!arguments.contains("--dangerously-skip-permissions"));
}

#[test]
fn claude_wrapper_initializes_once_and_exposes_exact_identity_and_home() {
    let directory = tempfile::tempdir().expect("temporary wrapper directory");
    let merl = directory.path().join("merl");
    let host = directory.path().join("claude");
    let calls = directory.path().join("calls");
    let recording = directory.path().join("host-recording");
    let home = directory.path().join("board");
    fs::create_dir(&home).expect("create synthetic board");
    executable_write(
        &merl,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$*\" >>\"$MERL_TEST_CALLS\"\nprintf '{\"session_id\":\"ses-wrapper\"}\\n'\n",
    );
    executable_write(
        &host,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'session=%s\\nhome=%s\\ncwd=%s\\n' \"${MERL_SESSION_ID:-}\" \"${MERL_HOME:-}\" \"$PWD\" >\"$MERL_TEST_RECORDING\"\nprintf '%s\\n' \"$@\" >>\"$MERL_TEST_RECORDING\"\nexit 17\n",
    );

    let status = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args(["claude", "--program"])
        .arg(&host)
        .args(["--merl-program"])
        .arg(&merl)
        .args(["--merl-home"])
        .arg(&home)
        .args(["--", "--model", "test-model"])
        .env("MERL_TEST_CALLS", &calls)
        .env("MERL_TEST_RECORDING", &recording)
        .current_dir(directory.path())
        .status()
        .expect("run wrapper");

    assert_eq!(status.code(), Some(17));
    assert_eq!(
        fs::read_to_string(calls).expect("read init calls"),
        format!(
            "--home {} init --cwd {} --json\n",
            home.display(),
            directory.path().display()
        )
    );
    let host_recording = fs::read_to_string(recording).expect("read host recording");
    assert!(host_recording.starts_with(&format!(
        "session=ses-wrapper\nhome={}\ncwd={}\n",
        home.display(),
        directory.path().display()
    )));
    assert!(
        host_recording.contains("\"claude-channel\""),
        "{host_recording}"
    );
    assert!(
        host_recording.contains("\"--session\",\"ses-wrapper\""),
        "{host_recording}"
    );
    assert!(host_recording.ends_with("--model\ntest-model\n"));
}

#[test]
fn claude_channel_reconciles_after_a_board_wakeup_and_deduplicates() {
    let directory = tempfile::tempdir().expect("temporary channel directory");
    let merl = directory.path().join("merl");
    let home = directory.path().join("board");
    let ready = directory.path().join("ready");
    let queries = directory.path().join("queries");
    fs::create_dir_all(home.join("requests")).expect("create synthetic board");
    executable_write(
        &merl,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'query\\n' >>\"$MERL_TEST_QUERIES\"\nif [[ -e \"$MERL_TEST_READY\" ]]; then\n  printf '[{\"request_id\":\"req-1\",\"requester_session_id\":\"ses-wrapper\",\"answer\":{\"summary\":\"done\"}}]\\n'\nelse\n  printf '[]\\n'\nfi\n",
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args([
            "claude-channel",
            "--session",
            "ses-wrapper",
            "--merl-program",
        ])
        .arg(&merl)
        .args(["--merl-home"])
        .arg(&home)
        .env("MERL_TEST_READY", &ready)
        .env("MERL_TEST_QUERIES", &queries)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start live channel");
    let mut stdin = child.stdin.take().expect("channel stdin");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}}
        })
    )
    .expect("initialize");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        })
    )
    .expect("initialized");
    let mut output = BufReader::new(child.stdout.take().expect("channel stdout"));
    let mut line = String::new();
    output.read_line(&mut line).expect("initialize response");
    let initialization: serde_json::Value =
        serde_json::from_str(&line).expect("JSON initialization response");
    assert!(
        initialization["result"]["instructions"]
            .as_str()
            .expect("server instructions")
            .contains("bound to Merl session ses-wrapper")
    );
    line.clear();
    fs::write(&ready, "ready").expect("answer becomes canonical");
    fs::write(home.join("requests/wake"), "one").expect("first wakeup");
    output.read_line(&mut line).expect("channel notification");
    let notification: serde_json::Value = serde_json::from_str(&line).expect("JSON notification");
    assert_eq!(notification["method"], "notifications/claude/channel");
    assert!(
        notification["params"]["content"]
            .as_str()
            .expect("content")
            .contains("request_id=req-1")
    );

    let queries_before_duplicate = query_count(&queries);
    fs::write(home.join("requests/wake-two"), "two").expect("duplicate wakeup");
    wait_until(Duration::from_secs(2), || {
        query_count(&queries) > queries_before_duplicate
    });
    drop(stdin);
    let status = child.wait().expect("channel exits");
    assert!(status.success());
    let mut remainder = String::new();
    output
        .read_to_string(&mut remainder)
        .expect("remaining output");
    assert!(remainder.is_empty(), "duplicate notification: {remainder}");
}

#[test]
fn claude_wrapper_times_out_hanging_init_then_launches_pull_only_host() {
    let directory = tempfile::tempdir().expect("temporary timeout directory");
    let merl = directory.path().join("merl");
    let host = directory.path().join("claude");
    let recording = directory.path().join("host-recording");
    let home = directory.path().join("board");
    fs::create_dir(&home).expect("create synthetic board");
    executable_write(&merl, "#!/usr/bin/env bash\nsleep 60\n");
    executable_write(
        &host,
        "#!/usr/bin/env bash\nprintf 'launched\\n' >\"$MERL_TEST_RECORDING\"\nexit 0\n",
    );

    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args(["claude", "--program"])
        .arg(&host)
        .args(["--merl-program"])
        .arg(&merl)
        .args(["--merl-home"])
        .arg(&home)
        .args(["--child-timeout-ms", "50"])
        .env("MERL_TEST_RECORDING", &recording)
        .output()
        .expect("run timeout wrapper");

    assert!(output.status.success());
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fs::read_to_string(recording).unwrap(), "launched\n");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("timed out")
    );
}

#[test]
fn claude_wrapper_falls_back_to_stock_pull_only_host_when_init_fails() {
    let directory = tempfile::tempdir().expect("temporary fallback directory");
    let merl = directory.path().join("merl");
    let host = directory.path().join("claude");
    let recording = directory.path().join("host-recording");
    let home = directory.path().join("board");
    fs::create_dir(&home).expect("create synthetic board");
    executable_write(&merl, "#!/usr/bin/env bash\nexit 2\n");
    executable_write(
        &host,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$@\" >\"$MERL_TEST_RECORDING\"\nexit 19\n",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_merl-live-delivery-spike"))
        .args(["claude", "--program"])
        .arg(&host)
        .args(["--merl-program"])
        .arg(&merl)
        .args(["--merl-home"])
        .arg(&home)
        .args(["--", "--model", "fallback-model"])
        .env("MERL_TEST_RECORDING", &recording)
        .output()
        .expect("run fallback wrapper");

    assert_eq!(output.status.code(), Some(19));
    assert_eq!(
        fs::read_to_string(recording).expect("read stock host arguments"),
        "--model\nfallback-model\n"
    );
    assert!(
        String::from_utf8(output.stderr)
            .expect("UTF-8 diagnostic")
            .contains("starting stock Claude without a channel")
    );
}

fn executable_write(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).expect("write fake executable");
    let mut permissions = fs::metadata(path).expect("fake metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fake executable");
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn query_count(path: &std::path::Path) -> usize {
    fs::read_to_string(path)
        .map(|contents| contents.lines().count())
        .unwrap_or(0)
}
