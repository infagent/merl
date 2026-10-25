use merl_cli::{CliResponse, run_with_clock};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

pub struct Mailbox {
    home: PathBuf,
    results: Vec<Value>,
    #[cfg(unix)]
    socket: Option<std::os::unix::net::UnixListener>,
}

impl Mailbox {
    pub fn given_three_named_agents(name: &str) -> Self {
        let home = std::env::temp_dir().join(format!("merl-mail-{}-{name}", std::process::id()));
        if home.exists() {
            fs::remove_dir_all(&home).unwrap();
        }
        fs::create_dir(&home).unwrap();
        let case = Self {
            home,
            results: Vec::new(),
            #[cfg(unix)]
            socket: None,
        };
        for agent in ["alice", "bob", "eve"] {
            assert_eq!(
                case.call(&["register", "--agent", agent], 1000)["outcome"],
                "registered"
            );
        }
        case
    }

    pub fn when_processing_stops_after_reading_and_before_receipt(mut self) -> Self {
        self.send("interrupted-1", "bob", "assign", None);
        self.results
            .push(self.call(&["read", "--agent", "bob", "--id", "interrupted-1"], 1000));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--unread"], 2000));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--pending"], 2000));
        self.results.push(self.call(
            &["receipt", "--agent", "bob", "--id", "interrupted-1"],
            2000,
        ));
        self.results.push(self.call(
            &["receipt", "--agent", "bob", "--id", "interrupted-1"],
            3000,
        ));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--pending"], 3000));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--outstanding"], 3000));
        self.results.push(self.call(
            &["complete", "--agent", "bob", "--id", "interrupted-1"],
            3000,
        ));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--outstanding"], 4000));
        self.send("interrupted-note", "bob", "note", None);
        self.results.push(self.call(
            &["read", "--agent", "bob", "--id", "interrupted-note"],
            1000,
        ));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--pending"], 2000));
        self.results.push(self.call(
            &["receipt", "--agent", "bob", "--id", "interrupted-note"],
            2000,
        ));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--pending"], 3000));
        self
    }

    pub fn then_pending_and_outstanding_folders_preserve_the_right_work(self) {
        assert_eq!(self.results[1]["messages"], json!([]));
        assert_eq!(self.results[2]["messages"][0]["id"], "interrupted-1");
        assert_eq!(self.results[5]["messages"], json!([]));
        assert_eq!(self.results[6]["messages"][0]["read_at"], 1000);
        assert_eq!(self.results[6]["messages"][0]["received_at"], 2000);
        assert_eq!(self.results[8]["messages"], json!([]));
        assert_eq!(self.results[10]["messages"][0]["id"], "interrupted-note");
        assert_eq!(self.results[12]["messages"], json!([]));
    }

    fn call(&self, args: &[&str], now: i64) -> Value {
        call(&self.home, args, now)
    }

    fn send(&self, id: &str, to: &str, kind: &str, reply: Option<&str>) -> Value {
        let mut args = vec![
            "send",
            "--from",
            "alice",
            "--to",
            to,
            "--id",
            id,
            "--kind",
            kind,
            "--body-file",
        ];
        let body = self.home.join("body.txt");
        fs::write(&body, "Synthetic bounded message.\n").unwrap();
        let path = body.to_str().unwrap();
        args.push(path);
        if let Some(reply) = reply {
            args.extend(["--in-reply-to", reply]);
        }
        self.call(&args, 1000)
    }

    pub fn when_an_assignment_is_read_acknowledged_and_replied_to(mut self) -> Self {
        assert_eq!(
            self.send("work-1", "bob", "assign", None)["outcome"],
            "sent"
        );
        self.results
            .push(self.call(&["show", "--agent", "eve", "--id", "work-1"], 1000));
        self.results
            .push(self.call(&["read", "--agent", "bob", "--id", "work-1"], 1000));
        self.results
            .push(self.call(&["receipt", "--agent", "bob", "--id", "work-1"], 1000));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1000));
        let body = self.home.join("reply.txt");
        fs::write(&body, "Synthetic reply.").unwrap();
        self.results.push(self.call(
            &[
                "send",
                "--from",
                "bob",
                "--to",
                "alice",
                "--id",
                "reply-1",
                "--kind",
                "reply",
                "--body-file",
                body.to_str().unwrap(),
                "--in-reply-to",
                "work-1",
            ],
            1000,
        ));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1000));
        self.results
            .push(self.call(&["complete", "--agent", "eve", "--id", "work-1"], 1000));
        self.results.push(self.call(
            &[
                "complete", "--agent", "bob", "--id", "work-1", "--reply", "reply-1",
            ],
            1000,
        ));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1000));
        self.results
            .push(self.send("wrong-reply", "eve", "reply", Some("reply-1")));
        self
    }

    pub fn then_only_the_recipient_can_explicitly_complete_that_assignment(self) {
        assert_eq!(self.results[0]["code"], "MAIL_NOT_RECIPIENT");
        assert_eq!(
            self.results[1]["message"]["body"],
            "Synthetic bounded message.\n"
        );
        assert_eq!(self.results[2]["outcome"], "received");
        assert_eq!(self.results[3]["unread"], 0);
        assert_eq!(self.results[3]["outstanding"], 1);
        assert_eq!(self.results[4]["outcome"], "sent");
        assert_eq!(self.results[5]["outstanding"], 1);
        assert_eq!(self.results[6]["code"], "MAIL_NOT_RECIPIENT");
        assert_eq!(self.results[7]["outcome"], "completed");
        assert_eq!(self.results[8]["outstanding"], 0);
        assert_eq!(self.results[9]["code"], "MAIL_REPLY_MISMATCH");
    }

    #[cfg(unix)]
    pub fn given_a_bound_recipient_and_unavailable_host(mut self) -> Self {
        use std::os::unix::fs::PermissionsExt;
        self.socket =
            Some(std::os::unix::net::UnixListener::bind(self.home.join("host.sock")).unwrap());
        let mock = self.home.join("codex-mock");
        fs::write(&mock, "#!/usr/bin/python3\nimport pathlib,sys,json,os\np=pathlib.Path(__file__).parent\nif (p/'offline').exists(): sys.exit(1)\na=sys.argv[1:]\nassert a[0]=='queue'\nentry={'thread':a[a.index('--thread')+1],'pointer':a[a.index('--message')+1]}\nfd=os.open(p/'calls.jsonl',os.O_CREAT|os.O_APPEND|os.O_WRONLY,0o600)\nos.write(fd,(json.dumps(entry)+'\\n').encode());os.close(fd)\nprint('Queued synthetic notification.')\n").unwrap();
        fs::set_permissions(&mock, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(self.home.join("offline"), "").unwrap();
        assert_eq!(self.bind(0)["generation"], 1);
        self
    }

    #[cfg(unix)]
    pub fn when_another_dispatch_temporarily_owns_the_host_boundary(mut self) -> Self {
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
            sync::mpsc,
            time::{Duration, Instant},
        };
        self.send("watch-1", "bob", "note", None);
        fs::remove_file(self.home.join("offline")).unwrap();
        let mock = self.home.join("codex-mock");
        fs::write(&mock,"#!/usr/bin/python3\nimport pathlib,time\np=pathlib.Path(__file__).parent\n(p/'entered').touch()\nwhile not (p/'release').exists(): time.sleep(0.01)\n").unwrap();
        let command = |operation: &str| {
            let mut c = Command::new(env!("CARGO_BIN_EXE_merl"));
            c.args([
                "mail",
                operation,
                "--home",
                self.home.to_str().unwrap(),
                "--codex",
                mock.to_str().unwrap(),
                "--json",
            ]);
            c
        };
        let mut owner = command("dispatch").stdout(Stdio::null()).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.home.join("entered").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if !self.home.join("entered").exists() {
            let _ = owner.kill();
            owner.wait().unwrap();
            panic!("controlled host did not start");
        }
        let mut watcher = command("watch").stdout(Stdio::piped()).spawn().unwrap();
        let stdout = watcher.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        for _ in 0..2 {
            let value = rx
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null);
            self.results.push(value);
        }
        fs::write(self.home.join("release"), "").unwrap();
        owner.wait().unwrap();
        if self.results[1]["outcome"] == "waiting" {
            let value = rx
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(Value::Null);
            self.results.push(value);
        }
        let _ = watcher.kill();
        watcher.wait().unwrap();
        reader.join().unwrap();
        self
    }

    #[cfg(unix)]
    pub fn then_the_watcher_waits_and_resumes_without_restarting(self) {
        assert_eq!(self.results[0]["outcome"], "watching");
        assert_eq!(self.results[1]["outcome"], "waiting");
        assert_eq!(self.results[1]["reason"], "MAIL_BUSY");
        assert_eq!(self.results[2]["outcome"], "resumed");
    }

    #[cfg(unix)]
    pub fn when_the_host_accepts_a_notification_before_the_agent_receipts_it(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("grace-1", "bob", "note", None);
        self.results.push(self.dispatch(1000));
        self.results.push(self.dispatch(10_000));
        self.results.push(self.dispatch(80_000));
        self.results
            .push(self.call(&["receipt", "--agent", "bob", "--id", "grace-1"], 80_000));
        self.results.push(self.dispatch(200_000));
        self
    }

    #[cfg(unix)]
    pub fn then_rapid_duplicates_are_coalesced_and_missing_receipts_remain_retryable(self) {
        assert_eq!(self.results[0]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[1]["notifications"], json!([]));
        assert_eq!(self.results[2]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[4]["notifications"], json!([]));
        assert_eq!(
            fs::read_to_string(self.home.join("calls.jsonl"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[cfg(unix)]
    pub fn when_new_messages_arrive_before_and_after_receipt(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("batch-1", "bob", "note", None);
        self.results.push(self.dispatch(1000));
        let body = self.home.join("body.txt");
        let send = |id: &str, now| {
            self.call(
                &[
                    "send",
                    "--from",
                    "alice",
                    "--to",
                    "bob",
                    "--id",
                    id,
                    "--kind",
                    "note",
                    "--body-file",
                    body.to_str().unwrap(),
                ],
                now,
            )
        };
        assert_eq!(send("batch-2", 5000)["outcome"], "sent");
        self.results.push(self.dispatch(6000));
        for id in ["batch-1", "batch-2"] {
            self.call(&["receipt", "--agent", "bob", "--id", id], 7000);
        }
        self.results.push(self.call(
            &[
                "send",
                "--from",
                "alice",
                "--to",
                "bob",
                "--id",
                "batch-3",
                "--kind",
                "note",
                "--body-file",
                body.to_str().unwrap(),
            ],
            8000,
        ));
        self.results.push(self.dispatch(8000));
        self
    }

    #[cfg(unix)]
    pub fn then_pending_mail_coalesces_and_a_new_batch_wakes_immediately(self) {
        assert_eq!(self.results[0]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[1]["notifications"], json!([]));
        assert_eq!(self.results[2]["outcome"], "sent");
        assert_eq!(self.results[3]["notifications"][0]["state"], "enqueued");
        assert_eq!(
            fs::read_to_string(self.home.join("calls.jsonl"))
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[cfg(unix)]
    fn bind(&self, generation: u64) -> Value {
        self.call(
            &[
                "bind",
                "--agent",
                "bob",
                "--session",
                "00000000-0000-4000-8000-000000000001",
                "--socket",
                self.home.join("host.sock").to_str().unwrap(),
                "--expected-generation",
                &generation.to_string(),
            ],
            1000,
        )
    }

    #[cfg(unix)]
    fn dispatch(&self, now: i64) -> Value {
        self.call(
            &[
                "dispatch",
                "--codex",
                self.home.join("codex-mock").to_str().unwrap(),
            ],
            now,
        )
    }

    #[cfg(unix)]
    pub fn when_two_messages_arrive_and_dispatch_restarts(mut self) -> Self {
        assert_eq!(self.send("one", "bob", "assign", None)["outcome"], "sent");
        assert_eq!(self.send("two", "bob", "note", None)["outcome"], "sent");
        self.results.push(self.dispatch(1000));
        fs::remove_file(self.home.join("offline")).unwrap();
        self.results.push(self.dispatch(200_000));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--limit", "1"], 200_000));
        self.results.push(self.call(
            &["list", "--agent", "bob", "--limit", "1", "--after", "one"],
            200_000,
        ));
        self.results.push(self.dispatch(400_000));
        self.results
            .push(self.call(&["receipt", "--agent", "bob", "--id", "one"], 400_000));
        self.results
            .push(self.call(&["receipt", "--agent", "bob", "--id", "two"], 400_000));
        self.results.push(self.dispatch(600_000));
        self
    }

    #[cfg(unix)]
    pub fn then_one_pointer_wakes_the_recipient_and_receipts_stop_retries(self) {
        assert_eq!(self.results[0]["notifications"][0]["state"], "failed");
        assert_eq!(self.results[1]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[2]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(self.results[2]["next_after"], "one");
        assert_eq!(self.results[3]["messages"][0]["id"], "two");
        assert_eq!(self.results[4]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[7]["notifications"], json!([]));
        let log = fs::read_to_string(self.home.join("calls.jsonl")).unwrap();
        let calls: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            calls.len(),
            2,
            "one coalesced notification per dispatch; duplicate wakes allowed until receipt"
        );
        for call in calls {
            let p = call["pointer"].as_str().unwrap();
            assert!(p.contains("mail list"));
            assert!(p.contains("--pending"));
            assert!(!p.contains("Synthetic bounded"));
        }
    }

    #[cfg(unix)]
    pub fn when_a_session_is_rebound_and_its_host_socket_is_replaced(mut self) -> Self {
        self.send("pending", "bob", "note", None);
        self.results.push(self.bind(0));
        self.socket.take();
        fs::remove_file(self.home.join("host.sock")).unwrap();
        self.socket =
            Some(std::os::unix::net::UnixListener::bind(self.home.join("host.sock")).unwrap());
        fs::remove_file(self.home.join("offline")).unwrap();
        self.results.push(self.dispatch(200_000));
        self.results.push(self.bind(1));
        self.results.push(self.dispatch(400_000));
        self
    }

    #[cfg(unix)]
    pub fn then_dispatch_waits_for_explicit_rebinding(self) {
        assert_eq!(self.results[0]["code"], "MAIL_BINDING_CONFLICT");
        assert_eq!(
            self.results[1]["notifications"][0]["state"],
            "stale_binding"
        );
        assert_eq!(self.results[2]["generation"], 2);
        assert_eq!(self.results[3]["notifications"][0]["state"], "enqueued");
    }

    pub fn when_concurrent_messages_and_conflicting_retries_are_sent(mut self) -> Self {
        let body = self.home.join("body.txt");
        fs::write(&body, "Complete concurrent body.").unwrap();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for i in 0..8 {
                let home = &self.home;
                let body = &body;
                handles.push(scope.spawn(move || {
                    call(
                        home,
                        &[
                            "send",
                            "--from",
                            "alice",
                            "--to",
                            "bob",
                            "--id",
                            &format!("parallel-{i}"),
                            "--kind",
                            "note",
                            "--body-file",
                            body.to_str().unwrap(),
                        ],
                        1000,
                    )
                }));
            }
            for h in handles {
                self.results.push(h.join().unwrap());
            }
        });
        self.results.push(self.call(
            &[
                "send",
                "--from",
                "alice",
                "--to",
                "bob",
                "--id",
                "parallel-0",
                "--kind",
                "note",
                "--body-file",
                body.to_str().unwrap(),
            ],
            1000,
        ));
        fs::write(&body, "Changed retry.").unwrap();
        self.results.push(self.call(
            &[
                "send",
                "--from",
                "alice",
                "--to",
                "bob",
                "--id",
                "parallel-0",
                "--kind",
                "note",
                "--body-file",
                body.to_str().unwrap(),
            ],
            1000,
        ));
        self.results
            .push(self.call(&["list", "--agent", "bob"], 1000));
        self.results
            .push(self.call(&["show", "--agent", "bob", "--id", "parallel-0"], 1000));
        self
    }

    pub fn then_all_messages_are_complete_unique_and_addressed_correctly(self) {
        for r in &self.results[..8] {
            assert_eq!(r["outcome"], "sent");
        }
        assert_eq!(self.results[8]["outcome"], "unchanged");
        assert_eq!(self.results[9]["code"], "MAIL_ID_CONFLICT");
        let msgs = self.results[10]["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 8);
        for m in msgs {
            assert_eq!(m["to"], "bob");
            assert!(m.get("body").is_none());
        }
        assert_eq!(
            self.results[11]["message"]["body"],
            "Complete concurrent body."
        );
    }
}

fn call(home: &std::path::Path, args: &[&str], now: i64) -> Value {
    let mut command: Vec<String> = ["mail", "--home", home.to_str().unwrap(), "--json"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    command.extend(args.iter().map(|s| (*s).to_owned()));
    match run_with_clock(&command, &|| Ok(now)) {
        CliResponse::Success(s) => serde_json::from_str(&s).unwrap(),
        CliResponse::JsonError(e) => serde_json::from_str(&e.as_json()).unwrap(),
        CliResponse::HumanError(e) => panic!("JSON mode lost: {e}"),
    }
}

impl Drop for Mailbox {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.home).unwrap();
    }
}
