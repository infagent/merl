use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

use merl_live_delivery_spike::SessionId;
use merl_live_delivery_spike::board::{Board, BoardError, ResultItem};

pub struct MemoryBoard {
    results: Vec<ResultItem>,
}

impl MemoryBoard {
    pub fn new(results: Vec<ResultItem>) -> Self {
        Self { results }
    }
}

impl Board for MemoryBoard {
    fn unread_results(&mut self, _session_id: &SessionId) -> Result<Vec<ResultItem>, BoardError> {
        Ok(self.results.clone())
    }
}

pub fn result_item(request_id: &str, requester_session_id: &str, summary: &str) -> ResultItem {
    ResultItem {
        request_id: request_id.to_owned(),
        requester_session_id: SessionId::new(requester_session_id),
        answer: serde_json::json!({"summary": summary}),
    }
}

pub struct SyntheticBoard {
    root: TempDir,
    home: PathBuf,
}

impl SyntheticBoard {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("synthetic root");
        let home = root.path().join("merl-home");
        Self { root, home }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn project(&self, name: &str) -> PathBuf {
        let path = self.root.path().join(name);
        std::fs::create_dir_all(&path).expect("synthetic project");
        path
    }

    pub fn merl(&self, arguments: &[&str]) -> Value {
        let output = Command::new("uv")
            .current_dir(repository_root())
            .env("UV_CACHE_DIR", self.root.path().join("uv-cache"))
            .args(["run", "merl", "--home"])
            .arg(&self.home)
            .args(arguments)
            .output()
            .expect("run existing Merl CLI");
        assert!(
            output.status.success(),
            "Merl CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("structured Merl output")
    }

    pub fn initialize(&self, project: &Path) -> SessionId {
        SessionId::new(
            self.merl(&[
                "init",
                "--cwd",
                project.to_str().expect("UTF-8 project path"),
                "--json",
            ])["session_id"]
                .as_str()
                .expect("session id")
                .to_owned(),
        )
    }

    pub fn answered_request(
        &self,
        requester: &SessionId,
        worker: &SessionId,
        summary: &str,
    ) -> String {
        let request_id = self.claimed_request(requester, worker);
        self.answer_request(worker, &request_id, summary);
        request_id
    }

    pub fn claimed_request(&self, requester: &SessionId, worker: &SessionId) -> String {
        let request = self.merl(&[
            "ask",
            "--session",
            requester.as_str(),
            "--title",
            "Synthetic live delivery",
            "--outcome",
            "The exact requester sees the answer",
            "--evidence",
            "Synthetic evidence",
            "--artifact",
            "synthetic.txt",
            "--needs",
            "testing",
            "--blocking",
            "--json",
        ]);
        let request_id = request["request_id"].as_str().expect("request id");
        self.merl(&[
            "claim",
            "--session",
            worker.as_str(),
            "--request",
            request_id,
            "--json",
        ]);
        request_id.to_owned()
    }

    pub fn answer_request(&self, worker: &SessionId, request_id: &str, summary: &str) {
        self.merl(&[
            "answer",
            "--session",
            worker.as_str(),
            "--request",
            request_id,
            "--summary",
            summary,
            "--evidence",
            "Synthetic verification passed",
            "--json",
        ]);
    }

    pub fn results(&self, session: &SessionId) -> Value {
        self.merl(&["results", "--session", session.as_str(), "--json"])
    }

    pub fn request_bytes(&self, request_id: &str) -> Vec<u8> {
        std::fs::read(self.home.join("requests").join(format!("{request_id}.md")))
            .expect("request document")
    }
}

pub fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
