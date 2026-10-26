#![cfg(unix)]

use merl_cli::{CliResponse, run_with_clock};
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct Dispatcher {
    home: PathBuf,
    caller: PathBuf,
    gate: UnixListener,
    _host: UnixListener,
    result: Option<Value>,
}

impl Dispatcher {
    pub fn given_mail_and_a_removable_launch_directory(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("merl-launch-{name}-{}", std::process::id()));
        let home = root.join("mailbox");
        let caller = root.join("checkout");
        fs::create_dir_all(&caller).unwrap();
        let call = |words: &[&str]| {
            let mut args = words.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
            args.extend([
                "--home".into(),
                home.to_str().unwrap().into(),
                "--json".into(),
            ]);
            match run_with_clock(&args, &|| Ok(0)) {
                CliResponse::Success(_) => (),
                response => panic!("setup failed: {response:?}"),
            }
        };
        for agent in ["author", "reviewer"] {
            call(&["mail", "register", "--agent", agent]);
        }
        let socket = home.join("host.sock");
        let host = UnixListener::bind(&socket).unwrap();
        call(&[
            "mail",
            "bind",
            "--agent",
            "reviewer",
            "--session",
            "00000000-0000-4000-8000-000000000001",
            "--socket",
            socket.to_str().unwrap(),
            "--expected-generation",
            "0",
        ]);
        fs::write(home.join("work.txt"), "Synthetic work").unwrap();
        call(&[
            "mail",
            "send",
            "--id",
            "work-1",
            "--from",
            "author",
            "--to",
            "reviewer",
            "--kind",
            "review",
            "--body-file",
            home.join("work.txt").to_str().unwrap(),
        ]);
        let gate = UnixListener::bind(home.join("gate.sock")).unwrap();
        fs::write(home.join("host.py"), "#!/usr/bin/env python3\nimport os,pathlib,socket\nhome=pathlib.Path(__file__).parent\ns=socket.socket(socket.AF_UNIX)\ns.connect(str(home/'gate.sock'))\ns.sendall(b'ready')\ns.recv(1)\n(home/'host-cwd').write_text(os.getcwd())\n").unwrap();
        fs::set_permissions(home.join("host.py"), fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            home,
            caller,
            gate,
            _host: host,
            result: None,
        }
    }

    pub fn when_the_launch_directory_is_removed_during_host_delivery(mut self) -> Self {
        self.remove_during("--codex");
        self
    }

    pub fn given_a_reviewer_lane_and_synthetic_github(self) -> Self {
        let words = [
            "lane",
            "start",
            "--home",
            self.home.to_str().unwrap(),
            "--name",
            "reviews",
            "--repo",
            "example/widgets",
            "--agent",
            "reviewer",
            "--label",
            "review-ready",
            "--json",
        ];
        let args = words.into_iter().map(str::to_owned).collect::<Vec<_>>();
        assert!(matches!(
            run_with_clock(&args, &|| Ok(0)),
            CliResponse::Success(_)
        ));
        let script = fs::read_to_string(self.home.join("host.py"))
            .unwrap()
            .replace("host-cwd", "gh-cwd");
        fs::write(self.home.join("host.py"),format!("{script}\nimport json,sys\nrequest=json.load(sys.stdin)\nassert request['operationName']=='Discover'\nprint('HTTP/2.0 200 OK\\r\\n\\r\\n'+json.dumps({{'data':{{'repository':{{'pullRequests':{{'edges':[],'pageInfo':{{'endCursor':None,'hasNextPage':False}}}}}}}}}}))\n")).unwrap();
        self
    }

    pub fn when_the_launch_directory_is_removed_during_source_polling(mut self) -> Self {
        self.remove_during("--gh");
        self
    }

    fn remove_during(&mut self, flag: &str) {
        let (group, operation) = if flag == "--gh" {
            ("lane", "scan")
        } else {
            ("mail", "dispatch")
        };
        let mut child = Command::new(env!("CARGO_BIN_EXE_merl"))
            .current_dir(&self.caller)
            .args([
                group,
                operation,
                "--home",
                self.home.to_str().unwrap(),
                flag,
                self.home.join("host.py").to_str().unwrap(),
                "--json",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        self.gate.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match self.gate.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("configured subprocess did not reach the handshake: {error}");
                }
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut ready = [0; 5];
        stream.read_exact(&mut ready).unwrap();
        fs::remove_dir(&self.caller).unwrap();
        stream.write_all(b"g").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        self.result = Some(serde_json::from_slice(&output.stdout).unwrap());
    }

    pub fn then_the_host_uses_the_stable_mailbox_directory_and_delivery_succeeds(self) {
        assert_eq!(
            self.result.as_ref().unwrap()["notifications"][0]["state"],
            "enqueued"
        );
        assert_eq!(
            fs::read_to_string(self.home.join("host-cwd")).unwrap(),
            self.home.canonicalize().unwrap().to_str().unwrap()
        );
    }

    pub fn then_github_uses_the_stable_mailbox_directory_and_polling_succeeds(self) {
        assert_eq!(self.result.as_ref().unwrap()["requests"], 1);
        assert_eq!(
            fs::read_to_string(self.home.join("gh-cwd")).unwrap(),
            self.home.canonicalize().unwrap().to_str().unwrap()
        );
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        fs::remove_dir_all(self.home.parent().unwrap()).unwrap();
    }
}
