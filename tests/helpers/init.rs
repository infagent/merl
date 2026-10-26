use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const SKILL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/skills/merl-mail/SKILL.md"
));

#[derive(Debug)]
struct Outcome {
    success: bool,
    value: Value,
}

pub struct SetupCase {
    root: PathBuf,
    profile: PathBuf,
    tools: PathBuf,
    outcomes: Vec<Outcome>,
    before: BTreeMap<PathBuf, Vec<u8>>,
    captured_skills: Vec<Vec<u8>>,
}

impl SetupCase {
    fn new(name: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("merl init {} {name} {serial}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let profile = root.join("user profile");
        fs::create_dir(&profile).unwrap();
        let tools = root.join("tools");
        fs::create_dir(&tools).unwrap();
        Self {
            root,
            profile,
            tools,
            outcomes: Vec::new(),
            before: BTreeMap::new(),
            captured_skills: Vec::new(),
        }
    }
    pub fn given_supported_local_hosts() -> Self {
        let case = Self::new("hosts");
        case.host("codex");
        case.host("claude");
        for (name, bytes) in [
            (
                ".codex/config.toml",
                b"# unrelated synthetic config\n".as_slice(),
            ),
            (".claude/settings.json", b"{\"synthetic\":\"preserve\"}\n"),
        ] {
            let p = case.profile.join(name);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, bytes).unwrap();
        }
        case
    }
    pub fn given_an_unconfigured_user() -> Self {
        let mut case = Self::new("preview");
        case.before = snapshot(&case.profile);
        case
    }
    pub fn given_broken_user_settings() -> Self {
        let case = Self::new("config");
        let p = case.config();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"invalid synthetic JSON\n").unwrap();
        case
    }
    pub fn given_a_borrowed_matching_skill_and_existing_mail() -> Self {
        let case = Self::new("ownership");
        case.host("codex");
        case.host("claude");
        let p = case.codex_skill();
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, SKILL).unwrap();
        fs::create_dir(case.mail()).unwrap();
        fs::write(
            case.mail().join("keep-mail"),
            b"synthetic durable mail bytes",
        )
        .unwrap();
        case
    }
    pub fn given_overlapping_and_symlinked_destinations() -> Self {
        let mut case = Self::new("conflicts");
        let target = case.profile.join("borrowed skill");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("SKILL.md"), b"unrelated skill").unwrap();
        let parent = case.profile.join("symlink skills");
        fs::create_dir(&parent).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, parent.join("merl-mail")).unwrap();
        case.before = snapshot(&case.profile);
        case
    }
    pub fn given_only_a_local_claude_host() -> Self {
        let case = Self::new("claude");
        case.host("claude");
        case
    }
    fn host(&self, name: &str) {
        let p = self.tools.join(name);
        fs::write(
            &p,
            "#!/bin/sh\necho unexpected-host-execution >&2\nexit 74\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    fn mail(&self) -> PathBuf {
        self.profile.join("durable mail")
    }
    fn config(&self) -> PathBuf {
        if cfg!(target_os = "macos") {
            self.profile
                .join("Library/Application Support/merl/config.json")
        } else {
            self.profile.join(".config/merl/config.json")
        }
    }
    fn codex_skill(&self) -> PathBuf {
        self.profile.join(".agents/skills/merl-mail/SKILL.md")
    }
    fn claude_skill(&self) -> PathBuf {
        self.profile.join(".claude/skills/merl-mail/SKILL.md")
    }
    fn command(&self, args: &[String]) -> Outcome {
        let out = Command::new(env!("CARGO_BIN_EXE_merl"))
            .args(args)
            .env("HOME", &self.profile)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("XDG_CONFIG_HOME", self.profile.join(".config"))
            .env("XDG_DATA_HOME", self.profile.join(".local/share"))
            .env("PATH", format!("{}:/usr/bin:/bin", self.tools.display()))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "non-JSON outcome {e}: stdout={} stderr={}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        Outcome {
            success: out.status.success(),
            value,
        }
    }
    fn run(&mut self, args: &[&str]) {
        self.outcomes
            .push(self.command(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>()));
    }
    fn setup(&mut self, extra: &[&str]) {
        let mut args = vec![
            "init".into(),
            "--global".into(),
            "--host".into(),
            "codex".into(),
            "--host".into(),
            "claude".into(),
            "--home".into(),
            self.mail().display().to_string(),
            "--dispatcher".into(),
            "foreground".into(),
            "--yes".into(),
            "--json".into(),
        ];
        args.extend(extra.iter().map(|s| (*s).into()));
        self.outcomes.push(self.command(&args));
    }
    pub fn when_setup_is_applied_used_and_uninstalled(mut self) -> Self {
        self.setup(&[]);
        for p in [self.codex_skill(), self.claude_skill()] {
            self.captured_skills.push(fs::read(p).unwrap_or_default());
        }
        self.run(&["init", "--show", "--json"]);
        self.run(&["mail", "register", "--agent", "worker", "--json"]);
        self.run(&["init", "--uninstall", "--yes", "--json"]);
        let home = self.mail().display().to_string();
        self.run(&[
            "mail", "status", "--home", &home, "--agent", "worker", "--json",
        ]);
        self
    }
    pub fn when_preview_and_show_are_requested_as_json(mut self) -> Self {
        let home = self.mail().display().to_string();
        self.run(&[
            "init",
            "--global",
            "--host",
            "codex",
            "--home",
            &home,
            "--dry-run",
            "--json",
        ]);
        self.run(&["init", "--show", "--json"]);
        self
    }
    pub fn when_explicit_and_default_mailbox_commands_and_help_are_used(mut self) -> Self {
        let home = self.mail().display().to_string();
        let config = self.config().display().to_string();
        self.run(&[
            "mail", "register", "--home", &home, "--config", &config, "--agent", "explicit",
            "--json",
        ]);
        self.run(&["mail", "register", "--agent", "implicit", "--json"]);
        self.run(&["help", "mail", "register", "--config", &config, "--json"]);
        self.run(&["retired-operation", "--config", &config, "--json"]);
        self
    }
    pub fn when_setup_is_retried_after_a_local_edit_and_removed(mut self) -> Self {
        self.setup(&[]);
        self.setup(&[]);
        if self.claude_skill().exists() {
            let mut bytes = SKILL.to_vec();
            bytes.extend_from_slice(b"\nlocal synthetic edit\n");
            fs::write(self.claude_skill(), bytes).unwrap();
        }
        self.setup(&[]);
        self.run(&["init", "--uninstall", "--yes", "--json"]);
        self
    }
    pub fn when_each_invalid_setup_plan_is_applied(mut self) -> Self {
        let home = self.mail().display().to_string();
        let shared = self.profile.join("shared skills").display().to_string();
        let under_mail = self.mail().join("skills").display().to_string();
        let linked = self.profile.join("symlink skills").display().to_string();
        for (codex, claude) in [
            (&shared, &shared),
            (&under_mail, &shared),
            (&linked, &shared),
        ] {
            self.run(&[
                "init",
                "--global",
                "--host",
                "codex",
                "--host",
                "claude",
                "--home",
                &home,
                "--codex-skills-dir",
                codex,
                "--claude-skills-dir",
                claude,
                "--yes",
                "--json",
            ]);
        }
        self
    }
    pub fn when_claude_setup_requests_foreground_instructions(mut self) -> Self {
        let home = self.mail().display().to_string();
        self.run(&[
            "init",
            "--global",
            "--host",
            "claude",
            "--home",
            &home,
            "--dispatcher",
            "foreground",
            "--yes",
            "--json",
        ]);
        self
    }
    pub fn when_publication_is_interrupted_then_recovered_and_upgraded(mut self) -> Self {
        use merl_setup::{Bundle, Plan, Settings};
        let settings = Settings {
            mail_home: self.mail(),
            skills: BTreeMap::from([
                ("codex".into(), self.codex_skill()),
                ("claude".into(), self.claude_skill()),
            ]),
            foreground: false,
        };
        let old = Bundle {
            version: "synthetic-v1",
            skill: std::str::from_utf8(SKILL).unwrap(),
        };
        let plan = Plan::new(self.config(), settings.clone(), old).unwrap();
        let error = plan
            .apply(&mut |_| {
                Err(std::io::Error::other(
                    "synthetic cancellation after publication",
                ))
            })
            .unwrap_err();
        self.captured_skills.push(error.code.as_bytes().to_vec());
        self.run(&["init", "--show", "--json"]);
        // Recovery uses the same public plan; no receipt editing or private hooks.
        plan.apply(&mut |_| Ok(())).unwrap();
        let new = Bundle {
            version: "synthetic-v2",
            skill: "new synthetic version-bound skill\n",
        };
        self.captured_skills.push(
            Plan::new(self.config(), settings, new)
                .unwrap_err()
                .code
                .as_bytes()
                .to_vec(),
        );
        plan.uninstall(&["codex".into()], &mut |_| Ok(())).unwrap();
        let remaining = merl_setup::load(&self.config()).unwrap().unwrap();
        let upgraded = Plan::new(self.config(), remaining, new).unwrap();
        upgraded.apply(&mut |_| Ok(())).unwrap();
        self.captured_skills
            .push(fs::read(self.claude_skill()).unwrap());
        upgraded.uninstall(&[], &mut |_| Ok(())).unwrap();
        self
    }
    pub fn when_json_omits_choices_then_terminal_setup_selects_claude(mut self) -> Self {
        self.run(&["init", "--json"]);
        let home = self.mail().display().to_string();
        self.run(&["init", "--home", &home, "--yes", "--json"]);
        let output = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/helpers/init_terminal.py"
            ))
            .arg(env!("CARGO_BIN_EXE_merl"))
            .arg(&self.profile)
            .arg(&self.tools)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        self.outcomes.push(Outcome {
            success: value["success"] == true,
            value,
        });
        self
    }
    pub fn then_only_the_terminal_selected_host_is_installed(self) {
        assert_eq!(self.outcomes[0].value["code"], "INIT_INPUT");
        assert_eq!(self.outcomes[1].value["code"], "INIT_INPUT");
        assert!(self.outcomes[2].success, "{}", self.outcomes[2].value);
        assert_eq!(self.outcomes[2].value["answered"], 4);
        assert_eq!(fs::read(self.claude_skill()).unwrap(), SKILL);
        assert!(!self.codex_skill().exists());
    }
    pub fn when_custom_settings_and_an_empty_mail_home_are_created_then_removed(mut self) -> Self {
        let config = self
            .profile
            .join("custom/settings.json")
            .display()
            .to_string();
        let home = self.profile.join("custom/mail").display().to_string();
        self.run(&[
            "init", "--global", "--host", "none", "--config", &config, "--home", &home, "--yes",
            "--json",
        ]);
        self.run(&[
            "init",
            "--config",
            &config,
            "--uninstall",
            "--yes",
            "--json",
        ]);
        self
    }
    pub fn then_custom_setup_is_removed_but_the_empty_mail_home_is_retained(self) {
        assert!(
            self.outcomes.iter().all(|o| o.success),
            "{:?}",
            self.outcomes
        );
        assert!(!self.profile.join("custom/settings.json").exists());
        let home = self.profile.join("custom/mail");
        assert!(home.is_dir());
        assert_eq!(fs::read_dir(home).unwrap().count(), 0);
    }
    pub fn when_a_host_is_uninstalled_then_installed_at_a_new_destination(mut self) -> Self {
        self.setup(&[]);
        self.run(&["init", "--uninstall", "--host", "codex", "--yes", "--json"]);
        let root = self.profile.join("new codex skills").display().to_string();
        self.run(&[
            "init",
            "--host",
            "codex",
            "--codex-skills-dir",
            &root,
            "--yes",
            "--json",
        ]);
        self.run(&["init", "--uninstall", "--yes", "--json"]);
        self
    }
    pub fn then_only_the_new_owned_skill_is_removed_and_the_borrowed_skill_remains(self) {
        assert!(
            self.outcomes.iter().all(|o| o.success),
            "{:?}",
            self.outcomes
        );
        assert_eq!(fs::read(self.codex_skill()).unwrap(), SKILL);
        assert!(
            !self
                .profile
                .join("new codex skills/merl-mail/SKILL.md")
                .exists()
        );
        assert!(!self.claude_skill().exists());
        assert!(!self.config().exists());
    }
    pub fn then_recovery_is_visible_and_uninstall_preserves_borrowed_skill_and_mail(self) {
        assert_eq!(self.captured_skills[0], b"INIT_IO");
        assert_eq!(self.outcomes[0].value["outcome"], "incomplete");
        assert_eq!(self.captured_skills[1], b"INIT_CONFLICT");
        assert_eq!(
            self.captured_skills[2],
            b"new synthetic version-bound skill\n"
        );
        assert_eq!(fs::read(self.codex_skill()).unwrap(), SKILL);
        assert!(!self.claude_skill().exists());
        assert!(!self.config().exists());
        assert_eq!(
            fs::read(self.mail().join("keep-mail")).unwrap(),
            b"synthetic durable mail bytes"
        );
    }
    pub fn then_skills_match_the_binary_defaults_work_and_mail_survives(self) {
        for result in &self.outcomes {
            assert!(result.success, "{}", result.value);
        }
        assert!(self.captured_skills.iter().all(|bytes| bytes == SKILL));
        assert_eq!(self.outcomes[0].value["schema"], "merl.init/v1");
        assert_eq!(self.outcomes[0].value["dispatcher"]["state"], "not_started");
        assert_eq!(
            self.outcomes[1].value["mail_home"],
            self.mail().display().to_string()
        );
        assert_eq!(self.outcomes[2].value["outcome"], "registered");
        assert_eq!(self.outcomes[4].value["agent"], "worker");
        assert!(
            !self.codex_skill().exists()
                && !self.claude_skill().exists()
                && !self.config().exists()
        );
        assert!(self.mail().is_dir());
        assert_eq!(
            fs::read(self.profile.join(".codex/config.toml")).unwrap(),
            b"# unrelated synthetic config\n"
        );
        assert_eq!(
            fs::read(self.profile.join(".claude/settings.json")).unwrap(),
            b"{\"synthetic\":\"preserve\"}\n"
        );
    }
    pub fn then_the_user_files_are_unchanged(self) {
        for result in &self.outcomes {
            assert!(result.success, "{}", result.value);
        }
        assert_eq!(self.outcomes[0].value["outcome"], "preview");
        assert_eq!(self.outcomes[1].value["outcome"], "not_configured");
        assert_eq!(snapshot(&self.profile), self.before);
    }
    pub fn then_only_the_explicit_mailbox_opens_and_help_stays_pure(self) {
        assert!(self.outcomes[0].success, "{}", self.outcomes[0].value);
        assert_eq!(self.outcomes[1].value["code"], "INIT_CONFIG");
        assert!(self.outcomes[2].success);
        assert_eq!(self.outcomes[3].value["code"], "UNKNOWN_COMMAND");
        assert_eq!(
            fs::read(self.config()).unwrap(),
            b"invalid synthetic JSON\n"
        );
        assert!(!self.profile.join(".local/share/merl").exists());
    }
    pub fn then_borrowed_and_modified_content_and_all_mail_are_preserved(self) {
        assert!(self.outcomes[0].success, "{}", self.outcomes[0].value);
        assert!(self.outcomes[1].success, "{}", self.outcomes[1].value);
        assert_eq!(self.outcomes[1].value["outcome"], "unchanged");
        assert_eq!(self.outcomes[2].value["code"], "INIT_CONFLICT");
        assert!(!self.outcomes[3].success);
        assert_eq!(fs::read(self.codex_skill()).unwrap(), SKILL);
        let mut expected = SKILL.to_vec();
        expected.extend_from_slice(b"\nlocal synthetic edit\n");
        assert_eq!(fs::read(self.claude_skill()).unwrap(), expected);
        assert_eq!(
            fs::read(self.mail().join("keep-mail")).unwrap(),
            b"synthetic durable mail bytes"
        );
    }
    pub fn then_every_conflict_preserves_the_original_files(self) {
        assert!(self.outcomes.iter().all(|r| !r.success));
        for result in &self.outcomes {
            assert!(
                matches!(
                    result.value["code"].as_str(),
                    Some("INIT_INPUT" | "INIT_CONFLICT")
                ),
                "{}",
                result.value
            );
        }
        assert_eq!(snapshot(&self.profile), self.before);
    }
    pub fn then_claude_is_configured_and_missing_codex_is_reported_truthfully(self) {
        assert!(self.outcomes[0].success, "{}", self.outcomes[0].value);
        assert_eq!(fs::read(self.claude_skill()).unwrap(), SKILL);
        assert!(!self.codex_skill().exists());
        let dispatcher = &self.outcomes[0].value["dispatcher"];
        assert_eq!(dispatcher["state"], "missing_codex");
        assert!(dispatcher.get("start_command").is_none_or(Value::is_null));
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut result = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        let metadata = fs::symlink_metadata(&p).unwrap();
        if metadata.file_type().is_symlink() {
            result.insert(
                p,
                fs::read_link(entry.path())
                    .unwrap()
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec(),
            );
        } else if metadata.is_dir() {
            result.insert(p.clone(), b"directory".to_vec());
            result.extend(snapshot(&p));
        } else {
            result.insert(p, fs::read(entry.path()).unwrap());
        }
    }
    result
}
impl Drop for SetupCase {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
