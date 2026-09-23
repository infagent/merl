use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};

pub struct Capture {
    directory: PathBuf,
    pages: Value,
    latest: Value,
    first_binding: Option<Value>,
    original: Option<String>,
    flags: Vec<String>,
    calls_before: usize,
    tick: u8,
    human: Option<String>,
    last_args: Vec<String>,
    changes_before: Value,
    changes_after: Value,
}

impl Capture {
    pub fn given_a_new_project() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "merl-live-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap()
        ));
        fs::create_dir(&directory).expect("isolated directory");
        let mut pages: Value =
            serde_json::from_str(include_str!("../fixtures/github_two_page_edit.json")).unwrap();
        pages.as_array_mut().unwrap().truncate(1);
        let issue = &mut pages[0]["data"]["repository"]["issue"];
        issue["lastEditedAt"] = Value::Null;
        issue["userContentEdits"]["nodes"] = json!([]);
        issue["comments"]["nodes"] = json!([]);
        issue["comments"]["pageInfo"] = json!({"hasNextPage": false, "endCursor": null});
        let case = Self {
            directory,
            pages,
            latest: Value::Null,
            first_binding: None,
            original: None,
            flags: vec![],
            calls_before: 0,
            tick: 0,
            human: None,
            last_args: Vec::new(),
            changes_before: Value::Null,
            changes_after: Value::Null,
        };
        case.script("gh", "#!/bin/sh\ncat \"$(dirname \"$0\")/pages.json\"\n");
        case.script("compiler", r"#!/usr/bin/python3
import json,sys,pathlib
request=json.load(sys.stdin)
with open(pathlib.Path(__file__).with_name('calls.jsonl'),'a') as f:
    f.write(json.dumps(request)+'\n')
print(json.dumps({'schema':'merl.compiler-response/v1','assertions':[],'context_required':[],'unresolved':[],'relations':[]}))
");
        case.command(&["project", "init", "--id", "P1"]);
        case.command(&["inbox", "subscribe", "--project", "P1", "--agent", "reader"]);
        case
    }
    pub fn given_a_project_with_a_supported_comment() -> Self {
        use merl_core::{
            ActorId, BatchId, CompilationRunId, DomainEvent, EventId, ObjectId, ObjectKind,
            ObjectLifecycle, PolicyEvaluationId, PolicyInputId, PolicyVersion, ProjectId,
            SourceVersionId,
        };
        let case = Self::given_a_new_project();
        case.script("compiler", r"#!/usr/bin/python3
import json,sys,pathlib
request=json.load(sys.stdin)
context=request['context']
source=next(s for s in context['sources'] if s['id']==context['trigger'])
assertions=[]
if source['body']=='What gain?':
    assertions=[{'source':source['id'],'span_start':0,'span_end':10,'subject':'D1','predicate':'decision','value':'none','act':'request','epistemic_basis':'reported','polarity':'positive','confidence_millis':900,'attributed_to':None}]
print(json.dumps({'schema':'merl.compiler-response/v1','assertions':assertions}))
");
        let case = case.when_a_comment_arrives().when_the_issue_is_captured();
        let mut store = merl_store::Store::open(&case.directory.join("project.sqlite")).unwrap();
        let project = ProjectId::try_from("P1").unwrap();
        let version = SourceVersionId::try_from(case.original.as_deref().unwrap()).unwrap();
        let author = store
            .source_version(&project, &version)
            .unwrap()
            .unwrap()
            .source_author
            .unwrap();
        let actor = ActorId::try_from(author.as_str()).unwrap();
        let source = case.latest["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["entity"] == "comment-1")
            .unwrap();
        let prepared = merl_policy::evaluate(
            &store,
            &project,
            &actor,
            PolicyEvaluationId::try_from("decision-evaluation").unwrap(),
            BatchId::try_from("decision-batch").unwrap(),
            1,
            &merl_policy::PolicyRules {
                version: PolicyVersion::try_from("test-v1").unwrap(),
                decision_authors: vec![author],
                command_actors: vec![],
                administrators: vec![],
            },
            &[merl_policy::Proposal::ObservedAssertion {
                id: PolicyInputId::try_from("decision-input").unwrap(),
                run: CompilationRunId::try_from(source["run"].as_str().unwrap()).unwrap(),
                index: 0,
                event: DomainEvent::PutObject {
                    id: EventId::try_from("decision-event").unwrap(),
                    object: ObjectId::try_from("D1").unwrap(),
                    kind: ObjectKind::try_from("decision").unwrap(),
                    payload: None,
                    issue_scope: Some("issue-1".into()),
                    lifecycle: ObjectLifecycle::Active,
                },
            }],
        )
        .unwrap();
        prepared.commit(&mut store).unwrap();
        case
    }
    pub fn when_the_decision_is_read(mut self) -> Self {
        self.latest = self.command(&["show", "D1", "--project", "P1", "--source"]);
        let store = merl_store::Store::open(&self.directory.join("project.sqlite")).unwrap();
        let impacts = store
            .pending_evidence_impacts(&merl_core::ProjectId::try_from("P1").unwrap())
            .unwrap();
        self.latest["pending_impacts"] = json!(impacts.len());
        self
    }
    pub fn then_requires_evidence_revalidation(self) -> Self {
        assert_eq!(self.latest["pending_impacts"], 1, "{}", self.latest);
        assert_eq!(self.latest["support"], "unsupported");
        assert_eq!(self.latest["lifecycle"], "active");
        self
    }
    pub fn given_a_provider_that_starts_an_overlapping_capture(self) -> Self {
        let command = json!([
            env!("CARGO_BIN_EXE_merl"),
            "issue",
            "capture",
            "--project",
            "P1",
            "--database",
            self.directory.join("project.sqlite").to_str().unwrap(),
            "--repository",
            "example/project",
            "--issue",
            "17",
            "--github-program",
            self.directory.join("gh").to_str().unwrap(),
            "--mode",
            "capture_only",
            "--coverage",
            "optional",
            "--observed-at",
            "2026-01-02T00:00:01Z",
            "--json"
        ]);
        self.script("gh", &format!(r"#!/usr/bin/python3
import json,os,pathlib,subprocess
root=pathlib.Path(__file__).parent
if 'MERL_TEST_NESTED_CAPTURE' not in os.environ:
    result=subprocess.run({command},env=dict(os.environ,MERL_TEST_NESTED_CAPTURE='1'),capture_output=True)
    (root/'nested.json').write_bytes(result.stdout)
print((root/'pages.json').read_text())
"));
        self
    }
    pub fn then_the_overlapping_capture_is_busy(self) -> Self {
        let nested: Value =
            serde_json::from_slice(&fs::read(self.directory.join("nested.json")).unwrap()).unwrap();
        assert_eq!(nested["code"], "AUTHORITY_BUSY");
        self
    }
    pub fn when_capture_is_attempted_without_a_compiler(mut self) -> Self {
        fs::write(
            self.directory.join("pages.json"),
            serde_json::to_vec(&self.pages).unwrap(),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args([
                "issue",
                "capture",
                "--project",
                "P1",
                "--database",
                self.directory.join("project.sqlite").to_str().unwrap(),
                "--repository",
                "example/project",
                "--issue",
                "17",
                "--github-program",
                self.directory.join("gh").to_str().unwrap(),
                "--observed-at",
                "2026-01-02T00:00:00Z",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        self.latest = serde_json::from_slice(&output.stdout).unwrap();
        self
    }
    pub fn then_requires_compiler_configuration(self) -> Self {
        assert_eq!(self.latest["code"], "COMPILER_REQUIRED");
        self
    }
    pub fn when_optional_capture_is_selected(self) -> Self {
        self.given_optional_capture_policy()
    }
    pub fn when_an_older_comment_snapshot_arrives(mut self) -> Self {
        let head = self.latest["observation_head"].clone();
        let pages: Value =
            serde_json::from_str(include_str!("../fixtures/github_two_page_edit.json")).unwrap();
        self.pages[0]["data"]["repository"]["issue"]["comments"] =
            pages[0]["data"]["repository"]["issue"]["comments"].clone();
        self.pages[0]["data"]["repository"]["issue"]["comments"]["pageInfo"] =
            json!({"hasNextPage": false});
        fs::write(
            self.directory.join("pages.json"),
            serde_json::to_vec(&self.pages).unwrap(),
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(&self.last_args)
            .args([
                "--database",
                self.directory.join("project.sqlite").to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap();
        self.latest = serde_json::from_slice(&output.stdout).unwrap();
        self.latest["expected_head"] = head;
        self.latest["view"] = self.command(&["project", "view", "--project", "P1"]);
        self
    }
    pub fn then_rejects_the_stale_body_without_a_new_observation(self) -> Self {
        assert_eq!(
            self.latest["code"], "STALE_PROVIDER_OBSERVATION",
            "{}",
            self.latest
        );
        assert_eq!(
            self.latest["view"]["coverage"]["observation_head"],
            self.latest["expected_head"]
        );
        self
    }
    fn script(&self, name: &str, text: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.directory.join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn command(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(args)
            .args([
                "--database",
                self.directory.join("project.sqlite").to_str().unwrap(),
                "--json",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn calls(&self) -> usize {
        fs::read_to_string(self.directory.join("calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .count()
    }
    pub fn when_the_issue_is_captured(self) -> Self {
        self.capture(true)
    }
    pub fn when_the_issue_is_captured_for_humans(self) -> Self {
        self.capture(false)
    }
    pub fn then_reports_to_humans(self, outcome: &str) -> Self {
        assert!(
            self.human
                .as_ref()
                .unwrap()
                .contains(&format!("Issue capture {outcome}:")),
            "{:?}",
            self.human
        );
        self
    }
    pub fn when_the_provider_is_unavailable(self) -> Self {
        self.script("gh", "#!/bin/sh\nexit 1\n");
        self
    }
    fn read_changes(&self) -> Value {
        json!({"delta": self.command(&["project", "delta", "--project", "P1", "--since", "0"]), "inbox": self.command(&["inbox", "poll", "--project", "P1", "--agent", "reader"])})
    }
    fn capture(mut self, json_output: bool) -> Self {
        self.changes_before = self.read_changes();
        fs::write(
            self.directory.join("pages.json"),
            serde_json::to_vec(&self.pages).unwrap(),
        )
        .unwrap();
        self.calls_before = self.calls();
        self.tick += 1;
        let mut args = vec![
            "issue".into(),
            "capture".into(),
            "--project".into(),
            "P1".into(),
            "--repository".into(),
            "example/project".into(),
            "--issue".into(),
            "17".into(),
            "--github-program".into(),
            self.directory.join("gh").display().to_string(),
            "--observed-at".into(),
            format!("2026-01-02T00:00:{:02}Z", self.tick),
            "--program".into(),
            self.directory.join("compiler").display().to_string(),
            "--compiler-version".into(),
            "v1".into(),
            "--model".into(),
            "test".into(),
            "--prompt-digest".into(),
            format!("sha256:{}", "0".repeat(64)),
        ];
        args.extend(self.flags.clone());
        self.last_args.clone_from(&args);
        if !json_output {
            let output = Command::new(env!("CARGO_BIN_EXE_merl"))
                .args(&args)
                .args([
                    "--database",
                    self.directory.join("project.sqlite").to_str().unwrap(),
                ])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            self.human = Some(String::from_utf8(output.stdout).unwrap());
            return self;
        }
        self.latest = self.command(&args.iter().map(String::as_str).collect::<Vec<_>>());
        self.changes_after = self.read_changes();
        if self.first_binding.is_none() {
            self.first_binding = Some(self.latest["binding"].clone());
        }
        if self.original.is_none() {
            self.original = self.latest["sources"]
                .as_array()
                .and_then(|sources| {
                    sources
                        .iter()
                        .find(|source| source["entity"] == "comment-1")
                })
                .and_then(|source| source["version"].as_str())
                .map(str::to_owned);
        }
        self
    }
    pub fn when_a_comment_arrives(mut self) -> Self {
        let pages: Value =
            serde_json::from_str(include_str!("../fixtures/github_two_page_edit.json")).unwrap();
        self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"] =
            pages[0]["data"]["repository"]["issue"]["comments"]["nodes"].clone();
        self
    }
    pub fn when_the_compiler_inputs_are_read(mut self) -> Self {
        self.latest = serde_json::from_str(
            fs::read_to_string(self.directory.join("calls.jsonl"))
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        self
    }
    pub fn then_context_records_the_deletion_and_stays_within_its_cutoff(self) -> Self {
        let context = &self.latest["context"];
        let sources = context["sources"].as_array().unwrap();
        assert!(
            sources
                .iter()
                .any(|source| source["observed_deletion_of"].is_string())
        );
        assert!(
            sources
                .iter()
                .all(|source| source["observation"].as_u64().unwrap()
                    <= context["source_observation_cutoff"].as_u64().unwrap())
        );
        self
    }
    pub fn then_captures_and_compiles(self, count: u64) -> Self {
        assert_eq!(self.latest["outcome"], "captured", "{}", self.latest);
        assert_eq!(self.latest["captured"], count);
        assert_eq!(self.latest["compiled"], count);
        assert_eq!(
            self.calls() - self.calls_before,
            usize::try_from(count).unwrap()
        );
        self
    }
    pub fn then_reuses_the_binding(self) -> Self {
        assert_eq!(Some(&self.latest["binding"]), self.first_binding.as_ref());
        self
    }
    pub fn then_changes_nothing(self) -> Self {
        assert_eq!(self.latest["outcome"], "unchanged", "{}", self.latest);
        assert_eq!(self.latest["captured"], 0);
        assert_eq!(self.latest["provider_changed"], false);
        assert_eq!(self.changes_after, self.changes_before);
        assert_eq!(self.calls(), self.calls_before);
        self
    }
    pub fn when_the_comment_is_edited(mut self) -> Self {
        let comment = &mut self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"][0];
        comment["body"] = json!("Fixed gain.");
        comment["lastEditedAt"] = json!("2026-01-01T13:00:00Z");
        comment["updatedAt"] = json!("2026-01-01T13:00:00Z");
        self
    }
    pub fn when_the_comment_disappears(mut self) -> Self {
        self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"] = json!([]);
        self
    }
    pub fn then_records_one_deletion(self) -> Self {
        assert_eq!(self.latest["deleted"], 1, "{}", self.latest);
        assert_eq!(self.latest["compiled"], 0);
        assert_eq!(
            self.latest["sources"][0]["upstream_deleted_at"],
            Value::Null
        );
        self
    }
    pub fn when_the_original_comment_is_read(mut self) -> Self {
        self.latest = self.command(&[
            "source",
            "show",
            "--project",
            "P1",
            "--version",
            self.original.as_deref().unwrap(),
        ]);
        self
    }
    pub fn then_keeps_the_original_body(self) -> Self {
        assert!(
            self.latest.to_string().contains("What gain?"),
            "{}",
            self.latest
        );
        self
    }
    pub fn when_the_provider_closes_the_issue(mut self) -> Self {
        let issue = &mut self.pages[0]["data"]["repository"]["issue"];
        issue["state"] = json!("CLOSED");
        issue["closedAt"] = json!("2026-01-01T14:00:00Z");
        issue["updatedAt"] = json!("2026-01-01T14:00:00Z");
        issue["labels"]["nodes"] = json!([{ "id": "label-2", "name": "done" }]);
        issue["assignees"]["nodes"] = json!([]);
        self
    }
    pub fn then_changes_only_provider_state(self) -> Self {
        assert_eq!(self.latest["captured"], 0);
        assert_eq!(self.latest["provider_changed"], true);
        assert_eq!(self.latest["revision"], 2);
        assert_eq!(self.calls(), self.calls_before);
        self
    }
    pub fn when_changes_are_read(mut self) -> Self {
        self.latest = json!({"delta": self.command(&["project","delta","--project","P1","--since","0"]), "inbox": self.command(&["inbox","poll","--project","P1","--agent","reader"])});
        self
    }
    pub fn then_delta_and_inbox_reach_the_same_revision(self) -> Self {
        assert_eq!(self.latest["delta"]["head_revision"], 2, "{}", self.latest);
        assert_eq!(self.latest["inbox"]["entries"].as_array().unwrap().len(), 2);
        self
    }
    pub fn given_optional_eager_policy(mut self) -> Self {
        self.flags = vec![
            "--mode".into(),
            "eager".into(),
            "--coverage".into(),
            "optional".into(),
        ];
        self
    }
    pub fn then_optional_work_leaves_required_coverage_complete(self) -> Self {
        assert_eq!(self.latest["policy"]["mode"], "eager");
        assert_eq!(self.latest["policy"]["coverage"], "optional");
        let coverage = &self.changes_after["delta"]["coverage"];
        assert_eq!(coverage["required_gaps"], 0);
        assert_eq!(coverage["required_failed"], 0);
        assert_eq!(coverage["required_pending"], 0);
        self
    }
    pub fn given_optional_capture_policy(mut self) -> Self {
        self.flags = vec![
            "--mode".into(),
            "capture_only".into(),
            "--coverage".into(),
            "optional".into(),
        ];
        self
    }
    pub fn when_capture_policy_flags_are_omitted(mut self) -> Self {
        self.flags.clear();
        self
    }
    pub fn then_leaves_optional_sources_cold(self) -> Self {
        assert_eq!(self.latest["captured"], 1);
        assert_eq!(self.latest["compiled"], 0);
        assert_eq!(self.latest["policy"]["mode"], "capture_only");
        assert_eq!(self.latest["policy"]["coverage"], "optional");
        assert_eq!(self.calls(), 0);
        self
    }
    pub fn when_the_provider_page_is_truncated(mut self) -> Self {
        self.pages[0]["data"]["repository"]["issue"]["comments"]["nodes"] = json!([]);
        self.pages[0]["data"]["repository"]["issue"]["comments"]["pageInfo"]["hasNextPage"] =
            json!(true);
        self
    }
    pub fn then_reports_incomplete_without_changes(self) -> Self {
        assert_eq!(self.latest["outcome"], "incomplete");
        assert_eq!(self.latest["captured"], 0);
        assert_eq!(self.latest["deleted"], 0);
        assert_eq!(self.calls(), self.calls_before);
        self
    }
    pub fn given_a_failing_compiler(self) -> Self {
        self.script(
            "compiler",
            "#!/bin/sh\ncat >/dev/null\necho called >> \"$(dirname \"$0\")/calls.jsonl\"\nexit 1\n",
        );
        self
    }
    pub fn then_reports_failed_compilation(self) -> Self {
        assert_eq!(self.latest["outcome"], "failed", "{}", self.latest);
        assert_eq!(self.latest["failed"], 1);
        assert_eq!(self.calls(), 1);
        self
    }
    pub fn then_reports_the_same_failure_without_dispatch(self) -> Self {
        assert_eq!(self.latest["outcome"], "failed");
        assert_eq!(self.latest["captured"], 0);
        assert_eq!(self.calls(), 1);
        self
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
