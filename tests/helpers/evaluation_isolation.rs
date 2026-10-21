use std::{collections::BTreeMap, net::TcpListener, path::PathBuf};

use merl_eval::isolation::{IsolatedProgram, ProcessLimits};
use serde_json::{Value, json};

pub struct IsolationCase {
    directory: PathBuf,
    listener: TcpListener,
    responses: Vec<Value>,
}

impl IsolationCase {
    pub fn given_two_units_and_private_host_evidence() -> Self {
        use std::os::unix::fs::PermissionsExt;

        let directory = (0..1_000)
            .find_map(|index| {
                let path = std::env::temp_dir().join(format!(
                    "merl-eval-isolation-{}-{index}",
                    std::process::id()
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => Some(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(error) => panic!("create synthetic evidence directory: {error}"),
                }
            })
            .expect("reserve directory");
        std::fs::write(
            directory.join("private-answer.txt"),
            "synthetic private answer",
        )
        .expect("private canary");
        let program = directory.join("reader.py");
        std::fs::write(
            &program,
            r"#!/usr/bin/python3
import json, os, socket, sys
request = json.load(sys.stdin)
outside = request['outside']
connection = socket.socket()
connection.settimeout(0.5)
try:
    connection.connect(('127.0.0.1', request['port']))
    host_service = True
except OSError:
    host_service = False
finally:
    connection.close()
state = '/tmp/previous-unit'
previous_state = os.path.exists(state)
with open(state, 'w') as handle:
    handle.write('synthetic state from a different unit')
json.dump({
    'outside_visible': os.path.exists(outside),
    'proc_escape_visible': os.path.exists('/proc/1/root' + outside),
    'host_service_visible': host_service,
    'previous_state_visible': previous_state,
    'files': sorted(os.listdir('/input')) if os.path.isdir('/input') else [],
    'prompt': json.load(open('/input/prompt.json')) if os.path.isfile('/input/prompt.json') else None,
    'scoring_environment_visible': 'MERL_EVAL_SCORING_SPEC' in os.environ,
}, sys.stdout)
",
        )
        .expect("synthetic hostile reader");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700))
            .expect("executable reader");
        Self {
            directory,
            listener: TcpListener::bind("127.0.0.1:0").expect("synthetic host-only service"),
            responses: vec![],
        }
    }

    pub fn when_each_unit_runs_in_a_fresh_reader(mut self) -> Self {
        let program =
            IsolatedProgram::new(&self.directory.join("reader.py"), ProcessLimits::default())
                .expect("reader program");
        for unit in ["Q1@2", "Q1@1"] {
            let inputs = BTreeMap::from([
                (
                    "prompt.json".to_owned(),
                    serde_json::to_vec(&json!({"unit_id": unit})).unwrap(),
                ),
                ("source.json".to_owned(), b"{\"observations\":[]}".to_vec()),
                ("method.json".to_owned(), b"{}".to_vec()),
                (
                    "instructions.txt".to_owned(),
                    b"Use only this unit.".to_vec(),
                ),
            ]);
            let response = program
                .call(
                    &inputs,
                    &json!({
                        "outside": self.directory.join("private-answer.txt"),
                        "port": self.listener.local_addr().unwrap().port(),
                    }),
                )
                .expect("isolated reader call");
            self.responses.push(response);
        }
        self
    }

    pub fn then_only_the_selected_files_are_visible(self) -> Self {
        for (response, unit) in self.responses.iter().zip(["Q1@2", "Q1@1"]) {
            assert_eq!(
                response["files"],
                json!([
                    "instructions.txt",
                    "method.json",
                    "prompt.json",
                    "source.json"
                ])
            );
            assert_eq!(response["prompt"]["unit_id"], unit);
            assert_eq!(response["scoring_environment_visible"], false);
        }
        self
    }

    pub fn then_host_files_and_services_are_unreachable(self) -> Self {
        for response in &self.responses {
            assert_eq!(response["outside_visible"], false);
            assert_eq!(response["proc_escape_visible"], false);
            assert_eq!(response["host_service_visible"], false);
        }
        self
    }

    pub fn then_later_readers_cannot_inherit_another_units_state(self) {
        for response in &self.responses {
            assert_eq!(response["previous_state_visible"], false);
        }
    }
}

impl Drop for IsolationCase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.directory).expect("remove synthetic evidence");
    }
}
