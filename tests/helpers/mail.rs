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
    #[cfg(unix)]
    pub fn when_even_a_fetch_reference_exceeds_the_notice_budget(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("cannot-inline", "bob", "assign", None);
        let program = format!("/{}", "x".repeat(20_000));
        let store = merl_mail::Mailbox::open(&self.home).unwrap();
        self.results.push(
            store
                .dispatch(
                    &self.home.join("codex-mock"),
                    std::path::Path::new(&program),
                    1000,
                )
                .unwrap(),
        );
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1001));
        self.results
            .push(json!(self.home.join("calls.jsonl").exists()));
        self
    }

    #[cfg(unix)]
    pub fn then_the_limit_is_reported_without_discarding_or_handling_mail(self) {
        assert_eq!(
            self.results[0]["notifications"][0]["state"],
            "pointer_limit"
        );
        assert_eq!(self.results[1]["unread"], 1);
        assert_eq!(self.results[1]["unreceipted"], 1);
        assert_eq!(self.results[1]["outstanding"], 1);
        assert_eq!(self.results[2], false);
    }

    #[cfg(unix)]
    pub fn when_more_mail_arrives_before_the_inline_selection_is_handled(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("consumed", "bob", "assign", None);
        self.dispatch(1000);
        self.send("arrived-later", "bob", "assign", None);
        self.results.push(self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "consumed",
                "--read",
                "--receipt",
            ],
            2000,
        ));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 2000));
        self.results.push(self.dispatch(61_000));
        let calls = fs::read_to_string(self.home.join("calls.jsonl")).unwrap();
        let call: Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
        self.results
            .push(serde_json::from_str(call["pointer"].as_str().unwrap()).unwrap());
        self.results
            .push(self.call(&["show", "--agent", "bob", "--id", "arrived-later"], 62_000));
        self
    }

    #[cfg(unix)]
    pub fn then_only_consumed_ids_are_receipted_and_later_content_is_delivered(self) {
        assert_eq!(self.results[0]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(self.results[1]["unread"], 1);
        assert_eq!(self.results[1]["unreceipted"], 1);
        assert_eq!(self.results[1]["outstanding"], 2);
        assert_eq!(self.results[2]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[3]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(self.results[3]["messages"][0]["id"], "arrived-later");
        assert_eq!(
            self.results[3]["messages"][0]["body"],
            "Synthetic bounded message.\n"
        );
        assert_eq!(self.results[4]["message"]["read_at"], Value::Null);
        assert_eq!(self.results[4]["message"]["received_at"], Value::Null);
    }

    pub fn when_compact_completion_is_retried_with_valid_and_invalid_selections(mut self) -> Self {
        self.send("compact-a", "bob", "assign", None);
        self.send("compact-b", "bob", "assign", None);
        self.send("compact-note", "bob", "note", None);
        let mixed = self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "compact-a,compact-note",
                "--read",
                "--receipt",
                "--complete",
            ],
            2000,
        );
        let unchanged = self.call(&["show", "--agent", "bob", "--id", "compact-a"], 2000);
        self.publish_compact_replies();
        let handle = [
            "handle",
            "--agent",
            "bob",
            "--ids",
            "compact-a",
            "--read",
            "--receipt",
            "--complete",
            "--reply",
            "compact-reply-a",
        ];
        let handled = self.call(&handle, 3000);
        let retried = self.call(&handle, 4000);
        let changed = self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "compact-a",
                "--complete",
            ],
            4000,
        );
        let wrong = self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "compact-a",
                "--complete",
                "--reply",
                "compact-reply-b",
            ],
            4000,
        );
        let missing = self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "compact-b,not-found",
                "--receipt",
            ],
            4000,
        );
        let empty = self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "compact-a,,compact-b",
                "--receipt",
            ],
            4000,
        );
        let too_many = (0..101)
            .map(|i| format!("bounded-{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let overflow = self.call(
            &["handle", "--agent", "bob", "--ids", &too_many, "--receipt"],
            4000,
        );
        self.results.push(
            json!({"mixed":mixed,"unchanged":unchanged,"handled":handled,"retried":retried,
            "changed":changed,"wrong":wrong,"missing":missing,"empty":empty,"overflow":overflow,
            "untouched":self.call(&["show", "--agent", "bob", "--id", "compact-b"], 4000)}),
        );
        self
    }

    fn publish_compact_replies(&self) {
        for (id, request) in [
            ("compact-reply-a", "compact-a"),
            ("compact-reply-b", "compact-b"),
        ] {
            assert_eq!(
                self.call(
                    &[
                        "send",
                        "--from",
                        "bob",
                        "--to",
                        "alice",
                        "--id",
                        id,
                        "--kind",
                        "reply",
                        "--body-file",
                        self.home.join("body.txt").to_str().unwrap(),
                        "--in-reply-to",
                        request
                    ],
                    2000
                )["outcome"],
                "sent"
            );
        }
    }

    pub fn then_retries_keep_the_original_link_and_failed_batches_change_nothing(self) {
        let result = &self.results[0];
        assert_eq!(result["mixed"]["code"], "MAIL_INPUT");
        for field in ["read_at", "received_at", "completed_at"] {
            assert_eq!(result["unchanged"]["message"][field], Value::Null);
            assert_eq!(result["untouched"]["message"][field], Value::Null);
            assert_eq!(result["handled"]["messages"][0][field], 3000);
            assert_eq!(result["retried"]["messages"][0][field], 3000);
        }
        assert_eq!(result["changed"]["code"], "MAIL_ID_CONFLICT");
        assert_eq!(result["wrong"]["code"], "MAIL_REPLY_MISMATCH");
        assert_eq!(result["missing"]["code"], "MAIL_NOT_FOUND");
        assert_eq!(result["empty"]["code"], "MAIL_INPUT");
        assert_eq!(result["overflow"]["code"], "MAIL_INPUT");
    }

    #[cfg(unix)]
    pub fn when_a_busy_host_accepts_duplicates_before_receipt(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("queued-1", "bob", "assign", None);
        self.results.push(self.dispatch(1000));
        self.results.push(self.dispatch(61_000));
        let events: Vec<Value> = fs::read_to_string(self.home.join("calls.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        self.results.push(json!(events));
        self.call(&["read", "--agent", "bob", "--id", "queued-1"], 62_000);
        self.call(&["receipt", "--agent", "bob", "--id", "queued-1"], 62_000);
        self.results.push(self.dispatch(122_000));
        self.results
            .push(self.call(&["list", "--agent", "bob", "--pending"], 122_000));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 122_000));
        self
    }

    #[cfg(unix)]
    pub fn then_accepted_events_can_be_obsolete_without_a_new_dispatch(self) {
        assert_eq!(self.results[0]["notifications"][0]["attempts"], 1);
        assert_eq!(self.results[1]["notifications"][0]["attempts"], 2);
        assert_eq!(self.results[2].as_array().unwrap().len(), 2);
        for event in self.results[2].as_array().unwrap() {
            let notice: Value = serde_json::from_str(event["pointer"].as_str().unwrap()).unwrap();
            assert_eq!(notice["binding_generation"], 1);
            assert_eq!(notice["messages"][0]["id"], "queued-1");
        }
        assert_eq!(self.results[3]["notifications"], json!([]));
        assert_eq!(self.results[4]["messages"], json!([]));
        assert_eq!(self.results[5]["unreceipted"], 0);
        assert_eq!(self.results[5]["outstanding"], 1);
        assert_eq!(self.results[5]["notification"]["state"], "enqueued");
    }

    #[cfg(unix)]
    pub fn when_a_later_recipient_receipts_mail_during_host_io(mut self) -> Self {
        use std::io::{Read, Write};
        self.call(
            &[
                "bind",
                "--agent",
                "eve",
                "--session",
                "00000000-0000-4000-8000-000000000002",
                "--socket",
                self.home.join("host.sock").to_str().unwrap(),
                "--expected-generation",
                "0",
            ],
            1000,
        );
        self.send("selected-bob", "bob", "assign", None);
        self.send("selected-eve", "eve", "assign", None);
        let gate =
            std::os::unix::net::UnixListener::bind(self.home.join("selection-gate.sock")).unwrap();
        let mock = self.home.join("codex-mock");
        fs::write(&mock, "#!/usr/bin/python3\nimport pathlib,socket,sys,json\np=pathlib.Path(__file__).parent\na=sys.argv[1:]\nwith (p/'selected-calls.jsonl').open('a') as f: f.write(json.dumps(a)+'\\n')\nif a[a.index('--thread')+1].endswith('000000000001'):\n s=socket.socket(socket.AF_UNIX);s.connect(str(p/'selection-gate.sock'));s.sendall(b'R');assert s.recv(1)==b'G'\n").unwrap();
        let home = self.home.clone();
        let owner = std::thread::spawn(move || {
            call(
                &home,
                &["dispatch", "--codex", mock.to_str().unwrap()],
                1000,
            )
        });
        let (mut signal, _) = gate.accept().unwrap();
        let mut ready = [0];
        signal.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [b'R']);
        self.call(&["receipt", "--agent", "eve", "--id", "selected-eve"], 1001);
        signal.write_all(b"G").unwrap();
        self.results.push(owner.join().unwrap());
        self.results.push(json!(
            fs::read_to_string(self.home.join("selected-calls.jsonl"))
                .unwrap()
                .lines()
                .count()
        ));
        self.results
            .push(self.call(&["status", "--agent", "eve"], 1002));
        self
    }

    #[cfg(unix)]
    pub fn then_that_obsolete_selection_creates_no_host_event(self) {
        assert_eq!(self.results[0]["notifications"][0]["state"], "enqueued");
        assert_eq!(self.results[0]["notifications"][1]["state"], "obsolete");
        assert_eq!(self.results[0]["notifications"][1]["attempts"], 0);
        assert_eq!(self.results[1], 1);
        assert_eq!(self.results[2]["unreceipted"], 0);
        assert_eq!(self.results[2]["outstanding"], 1);
    }

    #[cfg(unix)]
    pub fn when_large_unicode_and_escaped_messages_exceed_one_notice(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        let body_file = self.home.join("large.txt");
        let mut expected = serde_json::Map::new();
        for index in 0..10 {
            let id = format!("large-{index:02}");
            let body = match index {
                0 | 1 => "🦉".repeat(2048),
                2 => "\\".repeat(8192),
                _ => format!("Synthetic short content {index}."),
            };
            fs::write(&body_file, &body).unwrap();
            assert_eq!(
                self.call(
                    &[
                        "send",
                        "--from",
                        "alice",
                        "--to",
                        "bob",
                        "--id",
                        &id,
                        "--kind",
                        "assign",
                        "--body-file",
                        body_file.to_str().unwrap()
                    ],
                    1000
                )["outcome"],
                "sent"
            );
            expected.insert(id, json!(body));
        }
        self.results.push(self.dispatch(1000));
        let calls = fs::read_to_string(self.home.join("calls.jsonl")).unwrap();
        let call: Value = serde_json::from_str(calls.lines().next().unwrap()).unwrap();
        let text = call["pointer"].as_str().unwrap();
        let notice: Value = serde_json::from_str(text).unwrap();
        let mut fetched = serde_json::Map::new();
        for message in notice["messages"].as_array().unwrap() {
            if let Some(argv) = message["fetch"].as_array() {
                let args: Vec<String> = argv
                    .iter()
                    .skip(1)
                    .map(|s| s.as_str().unwrap().to_owned())
                    .collect();
                let CliResponse::Success(output) = run_with_clock(&args, &|| Ok(1001)) else {
                    panic!("overflow reference failed at public CLI");
                };
                fetched.insert(
                    message["id"].as_str().unwrap().to_owned(),
                    serde_json::from_str::<Value>(&output).unwrap()["message"].clone(),
                );
            }
        }
        let args: Vec<String> = notice["more"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .map(|s| s.as_str().unwrap().to_owned())
            .collect();
        let CliResponse::Success(more) = run_with_clock(&args, &|| Ok(1001)) else {
            panic!("continuation failed at public CLI");
        };
        self.results.extend([
            notice,
            json!(text.len()),
            Value::Object(expected),
            Value::Object(fetched),
            serde_json::from_str(&more).unwrap(),
        ]);
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1002));
        self
    }

    #[cfg(unix)]
    pub fn then_complete_bodies_and_exact_fetch_references_cover_the_bounded_selection(self) {
        let notice = &self.results[1];
        let messages = notice["messages"].as_array().unwrap();
        let complete: Vec<_> = messages
            .iter()
            .filter(|m| m.get("body").is_some())
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        let handle = notice["handle"].as_array().unwrap();
        let ids = handle.iter().position(|v| v == "--ids").unwrap() + 1;
        assert_eq!(
            handle[ids].as_str().unwrap(),
            complete.join(","),
            "never suggest receipting omitted bodies"
        );
        assert!(messages.len() <= 8);
        assert!(self.results[2].as_u64().unwrap() <= 16 * 1024);
        assert_eq!(notice["home"], self.home.to_str().unwrap());
        assert!(!self.results[4].as_object().unwrap().is_empty());
        for message in messages {
            let id = message["id"].as_str().unwrap();
            let body = message.get("body").unwrap_or(&self.results[4][id]["body"]);
            assert_eq!(body, &self.results[3][id]);
        }
        let mut selected: Vec<_> = messages.iter().map(|m| m["id"].as_str().unwrap()).collect();
        selected.extend(
            self.results[5]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["id"].as_str().unwrap()),
        );
        assert_eq!(
            selected,
            (0..10).map(|i| format!("large-{i:02}")).collect::<Vec<_>>()
        );
        assert_eq!(self.results[6]["unread"], 10);
        assert_eq!(self.results[6]["unreceipted"], 10);
    }

    pub fn when_inline_messages_are_handled_together_and_retried(mut self) -> Self {
        self.send("handle-a", "bob", "assign", None);
        self.send("handle-b", "bob", "assign", None);
        self.send("handle-foreign", "eve", "assign", None);
        for now in [2000, 3000] {
            self.results.push(self.call(
                &[
                    "handle",
                    "--agent",
                    "bob",
                    "--ids",
                    "handle-a,handle-b",
                    "--read",
                    "--receipt",
                ],
                now,
            ));
        }
        self.results
            .push(self.call(&["status", "--agent", "bob"], 3000));
        self.results.push(self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "handle-a,handle-foreign",
                "--complete",
            ],
            4000,
        ));
        self.results
            .push(self.call(&["show", "--agent", "bob", "--id", "handle-a"], 4000));
        self.results
            .push(self.call(&["handle", "--agent", "bob", "--ids", "handle-a"], 4000));
        self.results.push(self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "handle-a,handle-a",
                "--receipt",
            ],
            4000,
        ));
        self.results.push(self.call(
            &[
                "handle",
                "--agent",
                "bob",
                "--ids",
                "handle-a,handle-b",
                "--complete",
            ],
            5000,
        ));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 5000));
        self.results
            .push(self.call(&["status", "--agent", "eve"], 5000));
        self
    }

    pub fn then_only_the_selected_handling_facts_change_atomically(self) {
        for value in &self.results[..2] {
            assert_eq!(value["outcome"], "handled");
            for message in value["messages"].as_array().unwrap() {
                assert_eq!(message["read_at"], 2000);
                assert_eq!(message["received_at"], 2000);
                assert_eq!(message["completed_at"], Value::Null);
                assert!(message.get("body").is_none());
            }
        }
        assert_eq!(self.results[2]["unread"], 0);
        assert_eq!(self.results[2]["unreceipted"], 0);
        assert_eq!(self.results[2]["outstanding"], 2);
        assert_eq!(self.results[3]["code"], "MAIL_NOT_RECIPIENT");
        assert_eq!(self.results[4]["message"]["completed_at"], Value::Null);
        assert_eq!(self.results[5]["code"], "MAIL_INPUT");
        assert_eq!(self.results[6]["code"], "MAIL_INPUT");
        assert_eq!(self.results[7]["outcome"], "handled");
        assert_eq!(self.results[8]["outstanding"], 0);
        assert_eq!(self.results[9]["unread"], 1);
        assert_eq!(self.results[9]["unreceipted"], 1);
        assert_eq!(self.results[9]["outstanding"], 1);
    }

    #[cfg(unix)]
    pub fn when_a_short_assignment_is_delivered(mut self) -> Self {
        fs::remove_file(self.home.join("offline")).unwrap();
        self.send("inline-short", "bob", "assign", None);
        self.results.push(self.dispatch(1000));
        let calls = fs::read_to_string(self.home.join("calls.jsonl")).unwrap();
        let call: Value = serde_json::from_str(calls.lines().next().unwrap()).unwrap();
        self.results
            .push(serde_json::from_str(call["pointer"].as_str().unwrap()).unwrap_or(Value::Null));
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1001));
        self
    }

    #[cfg(unix)]
    pub fn then_its_complete_content_is_available_without_an_inbox_read(self) {
        assert_eq!(self.results[0]["notifications"][0]["state"], "enqueued");
        let notice = &self.results[1];
        assert_eq!(notice["schema"], "merl.notification/v1");
        assert_eq!(notice["agent"], "bob");
        assert_eq!(notice["binding_generation"], 1);
        assert_eq!(notice["messages"][0]["id"], "inline-short");
        assert_eq!(notice["messages"][0]["from"], "alice");
        assert_eq!(notice["messages"][0]["kind"], "assign");
        assert_eq!(
            notice["messages"][0]["body"],
            "Synthetic bounded message.\n"
        );
        assert_eq!(notice["messages"][0]["in_reply_to"], Value::Null);
        for field in ["to", "read_at", "received_at", "completed_at"] {
            assert!(
                notice["messages"][0].get(field).is_none(),
                "recipient is inherited; handling state belongs in explicit results"
            );
        }
        assert_eq!(self.results[2]["unread"], 1);
        assert_eq!(self.results[2]["unreceipted"], 1);
        assert_eq!(self.results[2]["outstanding"], 1);
    }

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

    pub fn when_human_and_json_output_flags_are_mixed(mut self) -> Self {
        for (flags, fail) in [
            (vec!["--json", "--format", "human"], false),
            (vec!["--format", "human", "--json"], false),
            (vec!["--json", "--format", "human"], true),
            (vec!["--format", "human", "--json"], true),
        ] {
            let mut arguments: Vec<String> = vec!["mail".into()];
            arguments.extend(flags.into_iter().map(str::to_owned));
            arguments.push(if fail { "--unknown" } else { "--help" }.into());
            self.results.push(match run_with_clock(&arguments,&||Ok(0)) {
                CliResponse::Success(output)=>json!({"kind":"success","is_json":serde_json::from_str::<Value>(&output).is_ok()}),
                CliResponse::HumanError(_)=>json!({"kind":"human_error"}),
                CliResponse::JsonError(_)=>json!({"kind":"json_error"}),
            });
        }
        self
    }

    pub fn then_the_last_flag_controls_success_and_error_output(self) {
        assert_eq!(self.results[0], json!({"kind":"success","is_json":false}));
        assert_eq!(self.results[1], json!({"kind":"success","is_json":true}));
        assert_eq!(self.results[2]["kind"], "human_error");
        assert_eq!(self.results[3]["kind"], "json_error");
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
    pub fn when_a_receipted_batch_is_replaced_while_its_enqueue_is_in_flight(mut self) -> Self {
        use std::io::{Read, Write};
        let gate = std::os::unix::net::UnixListener::bind(self.home.join("gate.sock")).unwrap();
        let mock = self.home.join("codex-mock");
        fs::write(&mock,"#!/usr/bin/python3\nimport pathlib,socket\np=pathlib.Path(__file__).parent\ns=socket.socket(socket.AF_UNIX)\ns.connect(str(p/'gate.sock'))\ns.sendall(b'R')\nassert s.recv(1)==b'G'\n").unwrap();
        self.send("race-old", "bob", "note", None);
        let home = self.home.clone();
        let executable = mock.clone();
        let owner = std::thread::spawn(move || {
            call(
                &home,
                &["dispatch", "--codex", executable.to_str().unwrap()],
                1000,
            )
        });
        let (mut signal, _) = gate.accept().unwrap();
        let mut ready = [0];
        signal.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [b'R']);
        self.call(&["receipt", "--agent", "bob", "--id", "race-old"], 1001);
        self.call(
            &[
                "send",
                "--from",
                "alice",
                "--to",
                "bob",
                "--id",
                "race-new",
                "--kind",
                "note",
                "--body-file",
                self.home.join("body.txt").to_str().unwrap(),
            ],
            1002,
        );
        signal.write_all(b"G").unwrap();
        self.results.push(owner.join().unwrap());
        self.results
            .push(self.call(&["status", "--agent", "bob"], 1003));
        fs::write(&mock, "#!/usr/bin/python3\n").unwrap();
        self.results.push(self.dispatch(1003));
        self
    }

    #[cfg(unix)]
    pub fn then_the_new_batch_remains_immediately_due(self) {
        assert_eq!(
            self.results[2]["notifications"].as_array().unwrap().len(),
            1,
            "older host completion postponed new mail"
        );
        assert_eq!(self.results[1]["notification"]["next_attempt_at"], 1002);
        assert_eq!(self.results[0]["state"], Value::Null);
        assert_eq!(self.results[0]["notifications"][0]["state"], "pending");
        assert_eq!(
            self.results[0]["notifications"][0]["host_outcome"],
            "enqueued"
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
            let notice: Value = serde_json::from_str(p).unwrap();
            assert_eq!(notice["schema"], "merl.notification/v1");
            assert_eq!(notice["messages"].as_array().unwrap().len(), 2);
            assert_eq!(
                notice["messages"][0]["body"],
                "Synthetic bounded message.\n"
            );
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
