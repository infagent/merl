use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

const SKILL: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/skills/merl/SKILL.md"));

#[derive(Debug)]
struct Output {
    success: bool,
    text: String,
    json: Option<Value>,
}
struct User {
    profile: PathBuf,
    config: PathBuf,
    expected: &'static str,
}
pub struct SummaryCase {
    root: PathBuf,
    tools: PathBuf,
    users: Vec<User>,
    outputs: Vec<Vec<Output>>,
    pure: Vec<bool>,
    transcripts: Vec<Value>,
}
impl SummaryCase {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "merl summary {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let tools = root.join("tools");
        fs::create_dir(&tools).unwrap();
        for host in ["codex", "claude"] {
            let path = tools.join(host);
            fs::write(
                &path,
                "#!/bin/sh\necho unexpected-host-execution >&2\nexit 74\n",
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        Self {
            root,
            tools,
            users: Vec::new(),
            outputs: Vec::new(),
            pure: Vec::new(),
            transcripts: Vec::new(),
        }
    }
    fn user(&mut self, name: &'static str) -> usize {
        let profile = self.root.join(name);
        fs::create_dir(&profile).unwrap();
        let config = profile.join(if cfg!(target_os = "macos") {
            "Library/Application Support/merl/config.json"
        } else {
            ".config/merl/config.json"
        });
        self.users.push(User {
            profile,
            config,
            expected: name,
        });
        self.users.len() - 1
    }
    fn command(&self, index: usize, args: &[&str]) -> Output {
        self.command_environment(index, args, false)
    }
    fn command_environment(&self, index: usize, args: &[&str], broken_environment: bool) -> Output {
        let user = &self.users[index];
        let mut command = Command::new(env!("CARGO_BIN_EXE_merl"));
        command
            .args(args)
            .env("HOME", &user.profile)
            .env("XDG_CONFIG_HOME", user.profile.join(".config"))
            .env("XDG_DATA_HOME", user.profile.join(".local/share"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()))
            .stdin(std::process::Stdio::null());
        if broken_environment {
            command
                .env_remove("HOME")
                .env("XDG_CONFIG_HOME", "bad-relative-path")
                .env("CLAUDE_CONFIG_DIR", "bad-relative-host");
        }
        let out = command.output().unwrap();
        let text = String::from_utf8(out.stdout).unwrap();
        let error = String::from_utf8(out.stderr).unwrap();
        Output {
            success: out.status.success(),
            json: serde_json::from_str(&text).ok(),
            text: if text.is_empty() { error } else { text },
        }
    }
    fn setup(&self, index: usize) {
        let home = self.users[index].profile.join("mail");
        let result = self.command(
            index,
            &[
                "init",
                "--host",
                "codex",
                "--host",
                "claude",
                "--home",
                home.to_str().unwrap(),
                "--dispatcher",
                "foreground",
                "--yes",
                "--json",
            ],
        );
        assert!(result.success, "{result:?}");
    }
    pub fn given_a_fresh_user_with_local_hosts() -> Self {
        let mut case = Self::new();
        case.user("absent");
        case
    }
    pub fn given_absent_configured_modified_partial_and_invalid_setups() -> Self {
        let mut case = Self::new();
        case.user("absent");
        let configured = case.user("configured");
        case.setup(configured);
        let modified = case.user("modified");
        case.setup(modified);
        fs::write(
            case.users[modified]
                .profile
                .join(".agents/skills/merl/SKILL.md"),
            "synthetic user edit preserved\n",
        )
        .unwrap();
        let partial = case.user("partial");
        let user = &case.users[partial];
        let settings = merl_setup::Settings {
            mail_home: user.profile.join("mail"),
            skills: BTreeMap::from([(
                "codex".into(),
                user.profile.join(".agents/skills/merl/SKILL.md"),
            )]),
            foreground: true,
        };
        let plan = merl_setup::Plan::new(
            user.config.clone(),
            settings,
            merl_setup::Bundle {
                version: env!("CARGO_PKG_VERSION"),
                skill: SKILL,
            },
        )
        .unwrap();
        assert_eq!(
            plan.apply(&mut |_| Err(std::io::Error::other("synthetic interruption")))
                .unwrap_err()
                .code,
            "INIT_IO"
        );
        let borrowed = case.user("borrowed");
        let skill = case.users[borrowed]
            .profile
            .join(".agents/skills/merl/SKILL.md");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(skill, SKILL).unwrap();
        case.setup(borrowed);
        let missing = case.user("missing");
        case.setup(missing);
        fs::remove_file(
            case.users[missing]
                .profile
                .join(".claude/skills/merl/SKILL.md"),
        )
        .unwrap();
        let outdated = case.user("outdated");
        let user = &case.users[outdated];
        merl_setup::Plan::new(
            user.config.clone(),
            merl_setup::Settings {
                mail_home: user.profile.join("mail"),
                skills: BTreeMap::from([(
                    "codex".into(),
                    user.profile.join(".agents/skills/merl/SKILL.md"),
                )]),
                foreground: true,
            },
            merl_setup::Bundle {
                version: "synthetic-old",
                skill: "older synthetic skill",
            },
        )
        .unwrap()
        .apply(&mut |_| Ok(()))
        .unwrap();
        let linked = case.user("symlink");
        case.setup(linked);
        let path = case.users[linked]
            .profile
            .join(".agents/skills/merl/SKILL.md");
        fs::remove_file(&path).unwrap();
        let unrelated = case.users[linked].profile.join("unrelated-secret");
        fs::write(&unrelated, "unrelated synthetic secret").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(unrelated, path).unwrap();
        let invalid = case.user("invalid");
        fs::create_dir_all(case.users[invalid].config.parent().unwrap()).unwrap();
        fs::write(
            &case.users[invalid].config,
            "synthetic-secret-value invalid JSON",
        )
        .unwrap();
        case
    }
    pub fn given_an_owned_and_a_borrowed_skill() -> Self {
        let mut case = Self::new();
        let index = case.user("configured");
        let borrowed = case.users[index]
            .profile
            .join(".agents/skills/merl/SKILL.md");
        fs::create_dir_all(borrowed.parent().unwrap()).unwrap();
        fs::write(borrowed, SKILL).unwrap();
        case.setup(index);
        case
    }
    pub fn given_borrowed_settings_with_owned_and_borrowed_skills() -> Self {
        let mut case = Self::new();
        let index = case.user("borrowed settings");
        let user = &case.users[index];
        let codex = user.profile.join(".agents/skills/merl/SKILL.md");
        let claude = user.profile.join(".claude/skills/merl/SKILL.md");
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::write(&codex, SKILL).unwrap();
        fs::create_dir_all(user.config.parent().unwrap()).unwrap();
        // A manually supplied, documented config is usable but never adopted.
        fs::write(&user.config,format!("{}\n",json!({"schema":"merl.config/v1","mail_home":user.profile.join("mail"),"skills":{"codex":codex,"claude":claude},"dispatcher":"foreground"}))).unwrap();
        case.setup(index);
        let manual = case.user("manual settings");
        let user = &case.users[manual];
        fs::create_dir_all(user.config.parent().unwrap()).unwrap();
        fs::create_dir(user.profile.join("mail")).unwrap();
        fs::write(&user.config,format!("{}\n",json!({"schema":"merl.config/v1","mail_home":user.profile.join("mail"),"skills":{},"dispatcher":"none"}))).unwrap();
        case
    }
    pub fn when_borrowed_setup_is_removed_and_status_is_read(mut self) -> Self {
        for index in 0..self.users.len() {
            let before = snapshot(&self.users[index].profile);
            self.outputs.push(vec![
                self.command(index, &["init", "--uninstall", "--yes"]),
                self.command(index, &["status"]),
            ]);
            self.pure
                .push(before == snapshot(&self.users[index].profile));
        }
        self
    }
    pub fn then_removal_reports_the_remaining_default_and_agrees_with_status(self) {
        for (index, results) in self.outputs.iter().enumerate() {
            assert!(results.iter().all(|output| output.success), "{results:?}");
            let removal = &results[0].text;
            let status = &results[1].text;
            assert_eq!(removal.lines().next(), status.lines().next(), "{results:?}");
            assert!(removal.contains("saved default"), "{removal}");
            assert_eq!(
                removal.matches("Codex skill:").count(),
                usize::from(index == 0),
                "{removal}"
            );
            assert!(self.users[index].config.is_file());
            if self.users[index].expected == "manual settings" {
                assert!(self.pure[index], "unowned settings were changed");
            }
        }
    }
    pub fn when_terminal_setup_is_cancelled_then_applied_and_status_is_read(mut self) -> Self {
        for confirm in ["n", "y"] {
            let before = snapshot(&self.users[0].profile);
            let out = Command::new("python3")
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/helpers/summary_terminal.py"
                ))
                .arg(env!("CARGO_BIN_EXE_merl"))
                .arg(&self.users[0].profile)
                .arg(&self.tools)
                .arg(confirm)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            self.transcripts
                .push(serde_json::from_slice(&out.stdout).unwrap());
            self.pure.push(before == snapshot(&self.users[0].profile));
        }
        self.outputs.push(vec![self.command(0, &["status"])]);
        self
    }
    pub fn when_each_setup_is_inspected_in_summary_and_full_json_modes(mut self) -> Self {
        for index in 0..self.users.len() {
            let before = snapshot(&self.users[index].profile);
            let mut outcomes = vec![
                self.command(index, &["status"]),
                self.command(index, &["status", "--verbose"]),
                self.command(index, &["status", "-v"]),
                self.command(index, &["status", "--json"]),
            ];
            if self.users[index].expected == "invalid" {
                let config = self.users[index].config.to_str().unwrap();
                let home = self.users[index].profile.join("explicit mail");
                outcomes.push(self.command(
                    index,
                    &[
                        "status",
                        "--home",
                        home.to_str().unwrap(),
                        "--config",
                        config,
                        "--json",
                    ],
                ));
                outcomes
                    .push(self.command(index, &["help", "status", "--config", config, "--json"]));
                outcomes.push(self.command_environment(
                    index,
                    &[
                        "status",
                        "--home",
                        home.to_str().unwrap(),
                        "--config",
                        config,
                        "--json",
                    ],
                    true,
                ));
            }
            if self.users[index].expected == "configured" {
                outcomes.push(self.command(index, &["status", "-v", "--format", "human"]));
                outcomes.push(self.command(index, &["status", "--format", "human", "--verbose"]));
            }
            self.outputs.push(outcomes);
            self.pure
                .push(before == snapshot(&self.users[index].profile));
        }
        self
    }
    pub fn when_setup_and_uninstall_are_previewed_then_uninstalled(mut self) -> Self {
        let before = snapshot(&self.users[0].profile);
        let mut results = vec![
            self.command(0, &["init", "--dry-run"]),
            self.command(0, &["init", "--uninstall", "--dry-run"]),
        ];
        self.pure.push(before == snapshot(&self.users[0].profile));
        results.push(self.command(0, &["init", "--uninstall", "--yes"]));
        self.outputs.push(results);
        self
    }
    pub fn then_proposed_and_current_summaries_are_clear_and_cancellation_is_pure(self) {
        assert!(self.pure[0], "cancellation changed setup files");
        for (index, transcript) in self.transcripts.iter().enumerate() {
            assert_eq!(transcript["answered"], 4);
            assert_eq!(
                transcript["code"],
                if index == 0 { json!(2) } else { json!(0) }
            );
            let text = transcript["transcript"].as_str().unwrap();
            assert!(
                text.contains("[~/"),
                "displayed default was not abbreviated: {text}"
            );
            for label in [
                "Merl setup",
                "Codex skill:",
                "Claude skill:",
                "Mailbox:",
                "Settings:",
                "Dispatcher:",
            ] {
                assert!(text.contains(label), "missing {label}: {text}");
            }
            assert!(
                text.contains("will install") && text.contains("will create"),
                "{text}"
            );
            for hidden in [
                "merl.init/v1",
                "\"schema\"",
                "\"outcome\"",
                "--session UUID",
                "--expected-generation GENERATION",
                "/absolute/codex.sock",
            ] {
                assert!(
                    !text.contains(hidden),
                    "internal/placeholder output: {text}"
                );
            }
        }
        let result = &self.outputs[0][0];
        assert!(result.success, "{result:?}");
        assert!(!result.text.contains('{'));
        assert!(result.text.contains("Not checked"));
        assert!(
            self.transcripts[1]["transcript"]
                .as_str()
                .unwrap()
                .contains(result.text.trim()),
            "init result and status did not share the summary: {result:?}"
        );
    }
    pub fn then_status_is_read_only_truthful_and_all_json_aliases_agree(self) {
        assert!(self.pure.iter().all(|pure| *pure));
        for (user, results) in self.users.iter().zip(&self.outputs) {
            for result in &results[1..4] {
                assert_eq!(result.json, results[1].json, "{results:?}");
            }
            assert!(!results[0].text.contains("\"schema\""), "{results:?}");
            if user.expected == "invalid" {
                assert!(!results[0].success);
                assert_eq!(results[1].json.as_ref().unwrap()["code"], "INIT_CONFIG");
                assert!(!results[0].text.contains("synthetic-secret-value"));
                let bypass = results[4].json.as_ref().unwrap();
                assert!(results[4].success, "{results:?}");
                assert_eq!(bypass["setup"]["state"], "not_checked");
                assert_eq!(bypass["config"]["source"], "bypassed");
                assert_eq!(bypass["mailbox"]["source"], "explicit");
                assert!(results[5].success);
                assert!(results[6].success, "{results:?}");
                continue;
            }
            assert!(results.iter().all(|r| r.success), "{results:?}");
            let value = results[1].json.as_ref().unwrap();
            assert_eq!(value["schema"], "merl.status/v1");
            assert_eq!(
                value["setup"]["state"],
                match user.expected {
                    "absent" => "not_configured",
                    "configured" | "borrowed" | "outdated" => "configured",
                    _ => "incomplete",
                }
            );
            assert_eq!(value["dispatcher"]["runtime"], "not_checked");
            if user.expected == "configured" {
                assert_eq!(results[4].text, results[0].text);
                assert_eq!(results[5].json, results[1].json);
            }
            if user.expected == "partial" {
                assert_eq!(value["mailbox"]["source"], "receipt");
                assert!(results[0].text.contains("not an active default"));
            }
            if user.expected == "borrowed" {
                assert!(
                    value["skills"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|skill| skill["ownership"] == "borrowed")
                );
            }
            if matches!(user.expected, "missing" | "outdated" | "symlink") {
                let state = if user.expected == "symlink" {
                    "conflict"
                } else {
                    user.expected
                };
                assert!(
                    value["skills"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|skill| skill["state"] == state)
                );
                assert!(!results[1].text.contains("unrelated synthetic secret"));
            }
            if user.expected == "modified" {
                assert!(
                    value["skills"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|s| s["state"] == "modified")
                );
                assert!(results[0].text.contains("preserved"));
            }
        }
    }
    pub fn then_the_shared_summary_describes_changes_and_retained_mail(self) {
        assert!(self.pure[0]);
        let results = &self.outputs[0];
        assert!(results.iter().all(|r| r.success), "{results:?}");
        assert!(results[0].text.contains("unchanged"), "{results:?}");
        assert!(
            results[1].text.contains("will retain") && results[1].text.contains("will remove"),
            "{results:?}"
        );
        for result in results {
            assert!(!result.text.contains("\"schema\""));
            assert!(result.text.contains("Mailbox:"));
            assert!(result.text.contains("Dispatcher:"));
        }
        assert!(self.users[0].profile.join("mail").is_dir());
    }
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut entries = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::symlink_metadata(&path).unwrap();
        if meta.file_type().is_symlink() {
            entries.insert(
                path.clone(),
                fs::read_link(&path)
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            );
        } else if meta.is_dir() {
            entries.insert(path.clone(), b"directory".to_vec());
            entries.extend(snapshot(&path));
        } else {
            entries.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    entries
}
impl Drop for SummaryCase {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
