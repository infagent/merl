use merl_setup::{Bundle, Change, Plan, Settings};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};

const CANONICAL: &str = include_str!("../../skills/merl/SKILL.md");
const LEGACY: &str = include_str!("../../skills/merl-mail/SKILL.md");

struct User {
    profile: PathBuf,
    kind: &'static str,
}
impl User {
    fn config(&self) -> PathBuf {
        self.profile.join(if cfg!(target_os = "macos") {
            "Library/Application Support/merl/config.json"
        } else {
            ".config/merl/config.json"
        })
    }
    fn receipt(&self) -> PathBuf {
        self.config().with_file_name(".config.json.merl-init.json")
    }
    fn skill(&self, host: &str, name: &str) -> PathBuf {
        self.profile
            .join(format!("custom {host} root/{name}/SKILL.md"))
    }
    fn settings(&self) -> Settings {
        Settings {
            mail_home: self.profile.join("durable mail"),
            skills: ["codex", "claude"]
                .into_iter()
                .map(|h| (h.into(), self.skill(h, "merl")))
                .collect(),
            foreground: true,
        }
    }
    fn seed(&self) {
        let legacy_primary = self.kind == "v1";
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut aliases = BTreeMap::new();
        let settings = json!({"schema":"merl.config/v1","mail_home":self.settings().mail_home,"skills":{"codex":self.skill("codex",if legacy_primary{"merl-mail"}else{"merl"}),"claude":self.skill("claude",if legacy_primary{"merl-mail"}else{"merl"})},"dispatcher":"foreground"});
        for host in ["codex", "claude"] {
            if !legacy_primary {
                let path = self.skill(host, "merl");
                write(&path, CANONICAL);
                files.push(json!({"path":path,"text":CANONICAL,"owned":true,"pending":null}));
            }
            if self.kind != "current" {
                let path = self.skill(host, "merl-mail");
                write(&path, LEGACY);
                if self.kind != "unmanaged" {
                    files.push(json!({"path":path,"text":LEGACY,"owned":self.kind!="borrowed","pending":null}));
                }
                if !legacy_primary {
                    aliases.insert(host, path.clone());
                }
                if !matches!(self.kind, "borrowed" | "unmanaged") {
                    let dir = path.parent().unwrap();
                    let metadata = fs::metadata(dir).unwrap();
                    directories
                        .push(json!({"path":dir,"identity":[metadata.dev(),metadata.ino()]}));
                }
            }
        }
        let text = format!("{settings}\n");
        write(&self.config(), &text);
        files.push(json!({"path":self.config(),"text":text,"owned":true,"pending":null}));
        let mut receipt = json!({"schema":if legacy_primary{"merl.init-receipt/v1"}else{"merl.init-receipt/v2"},"version":"0.3.0","complete":true,"settings":settings,"files":files,"directories":directories});
        if !legacy_primary {
            receipt["legacy_skills"] = json!(aliases);
        }
        write(&self.receipt(), &format!("{receipt}\n"));
        fs::create_dir_all(self.settings().mail_home.clone()).unwrap();
        write(
            &self.settings().mail_home.join("keep"),
            "Synthetic durable mail.\n",
        );
        match self.kind {
            "edited" => write(&self.skill("codex", "merl-mail"), "Synthetic local edit.\n"),
            "nonempty" => write(
                &self.skill("codex", "merl-mail").with_file_name("unrelated"),
                "Keep unrelated file.\n",
            ),
            "replaced" => {
                let original = self
                    .skill("codex", "merl-mail")
                    .parent()
                    .unwrap()
                    .to_owned();
                fs::rename(&original, original.with_file_name("preserved-original")).unwrap();
                write(&self.skill("codex", "merl-mail"), LEGACY);
            }
            "symlink" => {
                let path = self.skill("codex", "merl-mail");
                fs::remove_file(&path).unwrap();
                let target = self.profile.join("unrelated skill");
                write(&target, "Unrelated bytes.\n");
                std::os::unix::fs::symlink(target, path).unwrap();
            }
            _ => (),
        }
    }
    fn plan(&self) -> Result<Plan<'static>, merl_setup::SetupError> {
        Plan::new(
            self.config(),
            self.settings(),
            Bundle {
                version: env!("CARGO_PKG_VERSION"),
                skill: CANONICAL,
            },
        )
    }
    fn snapshot(&self) -> Value {
        let mut entries = BTreeMap::new();
        for path in [
            self.config().parent().unwrap().to_owned(),
            self.profile.join("custom codex root"),
            self.profile.join("custom claude root"),
            self.settings().mail_home,
        ] {
            snapshot(&path, &mut entries);
        }
        json!(entries)
    }
    fn installer(&self, accept: bool) -> Value {
        self.installer_answer(if accept { "y" } else { "n" })
    }
    fn installer_answer(&self, answer: &str) -> Value {
        let output = Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/helpers/setup_upgrade_terminal.py"
            ))
            .arg(env!("CARGO_BIN_EXE_merl"))
            .arg(env!("CARGO_MANIFEST_DIR"))
            .arg(&self.profile)
            .arg(answer)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

pub struct Upgrade {
    root: PathBuf,
    users: Vec<User>,
    records: Vec<Value>,
}
impl Upgrade {
    fn given(name: &str, kinds: &[&'static str]) -> Self {
        let root =
            std::env::temp_dir().join(format!("merl upgrade169 {} {name}", std::process::id()));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        let users = kinds
            .iter()
            .map(|kind| {
                let user = User {
                    profile: root.join(kind),
                    kind,
                };
                fs::create_dir(&user.profile).unwrap();
                user.seed();
                user
            })
            .collect();
        Self {
            root,
            users,
            records: Vec::new(),
        }
    }
    pub fn given_current_and_older_saved_setups() -> Self {
        Self::given(
            "terminal",
            &["current", "borrowed", "unmanaged", "v1", "owned"],
        )
    }
    pub fn given_legacy_alias_ownership_and_interruption_cases() -> Self {
        Self::given(
            "ownership",
            &[
                "owned",
                "file-interruption",
                "dir-interruption",
                "borrowed",
                "unmanaged",
                "edited",
                "nonempty",
                "replaced",
                "symlink",
            ],
        )
    }

    pub fn when_the_verified_installer_runs_again(mut self) -> Self {
        for user in &self.users {
            let before = user.snapshot();
            let declined = if matches!(user.kind, "current" | "borrowed" | "unmanaged") {
                Value::Null
            } else {
                user.installer(false)
            };
            let cancel_pure = before == user.snapshot();
            let accepted = user.installer(true);
            let after = user.snapshot();
            let repeated = user.installer(true);
            let repeat_pure = after == user.snapshot();
            self.records.push(json!({"kind":user.kind,"declined":declined,"cancel_pure":cancel_pure,"accepted":accepted,"repeated":repeated,"repeat_pure":repeat_pure,"current_pure":before==after,"settings":read_json(&user.config()),"receipt":read_json(&user.receipt()),"legacy_exists":user.skill("codex","merl-mail").exists()||user.skill("claude","merl-mail").exists(),"canonical":fs::read_to_string(user.skill("codex","merl")).ok(),"mail":fs::read_to_string(user.settings().mail_home.join("keep")).unwrap()}));
        }
        let fresh = User {
            profile: self.root.join("fresh-decline"),
            kind: "fresh",
        };
        fs::create_dir(&fresh.profile).unwrap();
        let before = fresh.snapshot();
        let declined = fresh.installer_answer("entry-no");
        let mut invalid = Vec::new();
        for mode in ["--show", "--dry-run", "--uninstall"] {
            let out = Command::new(env!("CARGO_BIN_EXE_merl"))
                .args(["init", "--offer", mode, "--json"])
                .env_remove("HOME")
                .env("XDG_CONFIG_HOME", "bad-relative")
                .output()
                .unwrap();
            let error: Value = serde_json::from_slice(&out.stdout).unwrap();
            invalid.push(json!({"code":error["code"],"exit":out.status.code()}));
        }
        self.records.push(json!({"kind":"fresh-decline","accepted":declined,"pure":before==fresh.snapshot(),"invalid_modes":invalid}));
        self
    }

    pub fn then_current_setup_is_quiet_and_real_changes_have_one_specific_confirmation(self) {
        for record in &self.records {
            let transcript = record["accepted"]["transcript"].as_str().unwrap();
            if record["kind"] == "fresh-decline" {
                assert_eq!(record["accepted"]["code"], 0);
                assert_eq!(transcript.matches("Run guided setup now?").count(), 1);
                assert!(
                    !transcript.contains("Select hosts")
                        && !transcript.contains("Apply this user setup?")
                );
                assert_eq!(record["pure"], true);
                assert!(
                    record["invalid_modes"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|mode| mode["code"] == "INIT_INPUT" && mode["exit"] == 2)
                );
                continue;
            }
            assert_eq!(record["accepted"]["code"], 0, "{record}");
            for redundant in [
                "Run guided setup now?",
                "Show dispatcher start instructions?",
                "Select hosts",
                "Durable mailbox home [",
            ] {
                assert!(!transcript.contains(redundant), "{transcript}");
            }
            if matches!(
                record["kind"].as_str(),
                Some("current" | "borrowed" | "unmanaged")
            ) {
                assert!(
                    !transcript.contains("Apply this user setup?"),
                    "{transcript}"
                );
                assert_eq!(record["current_pure"], true);
            } else {
                assert_eq!(
                    transcript.matches("Apply this user setup?").count(),
                    1,
                    "{transcript}"
                );
                assert!(
                    transcript.contains("Preview") && transcript.contains("will remove"),
                    "{transcript}"
                );
                assert_eq!(record["cancel_pure"], true);
                assert_ne!(record["declined"]["code"], 0);
            }
            let repeat = record["repeated"]["transcript"].as_str().unwrap();
            assert!(!repeat.contains('?'), "{repeat}");
            assert_eq!(record["repeat_pure"], true);
            assert!(
                transcript.contains("Not checked") && transcript.contains("merl usage"),
                "{transcript}"
            );
            assert_eq!(record["settings"]["dispatcher"], "foreground");
            assert_eq!(
                record["legacy_exists"],
                matches!(record["kind"].as_str(), Some("borrowed" | "unmanaged"))
            );
            assert_eq!(record["canonical"], CANONICAL);
            assert_eq!(record["receipt"]["complete"], true);
            assert_eq!(record["mail"], "Synthetic durable mail.\n");
        }
    }

    pub fn when_owned_alias_cleanup_is_planned_interrupted_and_retried(mut self) -> Self {
        for user in &self.users {
            let before = user.snapshot();
            let plan = match user.plan() {
                Ok(plan) => plan,
                Err(error) => {
                    self.records.push(
                        json!({"kind":user.kind,"error":error.code,"pure":before==user.snapshot()}),
                    );
                    continue;
                }
            };
            let actions=plan.changes(&[]).unwrap().into_iter().filter(|change|change.retained_legacy).map(|change|json!({"remove":change.action==Change::Remove,"ownership":format!("{:?}",change.ownership)})).collect::<Vec<_>>();
            let mut interrupted = false;
            let first = plan.apply(&mut |path| {
                let alias = path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|p| p == "merl-mail");
                let dir = path.file_name().is_some_and(|p| p == "merl-mail");
                if !interrupted
                    && ((user.kind == "file-interruption" && alias)
                        || (user.kind == "dir-interruption" && dir))
                {
                    interrupted = true;
                    return Err(io::Error::from(io::ErrorKind::Interrupted));
                }
                Ok(())
            });
            let pending = read_json(&user.receipt());
            let canonical_valid = fs::read_to_string(user.skill("codex", "merl")).unwrap()
                == CANONICAL
                && read_json(&user.config())["skills"]["codex"]
                    == user.skill("codex", "merl").to_str().unwrap();
            let resumed = user.plan().unwrap().apply(&mut |_| Ok(())).unwrap();
            self.records.push(json!({"kind":user.kind,"actions":actions,"first_error":first.err().map(|e|e.code),"interrupted":interrupted,"pending":pending,"canonical_valid":canonical_valid,"resumed":resumed,"pure":before==user.snapshot(),"receipt":read_json(&user.receipt()),"codex_legacy":user.skill("codex","merl-mail").exists(),"claude_legacy":user.skill("claude","merl-mail").exists(),"codex_dir":user.skill("codex","merl-mail").parent().unwrap().exists(),"unrelated":user.skill("codex","merl-mail").with_file_name("unrelated").exists(),"preserved_original":user.skill("codex","merl-mail").parent().unwrap().with_file_name("preserved-original").join("SKILL.md").exists(),"mail":fs::read_to_string(user.settings().mail_home.join("keep")).unwrap()}));
        }
        self
    }

    pub fn then_only_eligible_aliases_and_records_are_removed_safely(self) {
        for record in &self.records {
            let kind = record["kind"].as_str().unwrap();
            if matches!(kind, "edited" | "symlink") {
                assert_eq!(record["error"], "INIT_CONFLICT");
                assert_eq!(record["pure"], true);
                continue;
            }
            let keep = matches!(kind, "borrowed" | "unmanaged");
            if keep {
                assert_eq!(record["pure"], true);
            }
            assert_eq!(record["codex_legacy"], keep, "{record}");
            assert_eq!(record["claude_legacy"], keep);
            assert!(
                record["actions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|action| action["remove"] == !keep)
            );
            if kind.ends_with("interruption") {
                assert_eq!(record["interrupted"], true, "{record}");
                assert_eq!(record["first_error"], "INIT_IO");
                assert_eq!(record["pending"]["complete"], false);
                assert_eq!(record["canonical_valid"], true);
                let intent = record["pending"]["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["pending"]["remove"] == true)
                    .unwrap();
                assert_eq!(intent["owned"], true);
                assert!(
                    record["pending"]["legacy_skills"]
                        .as_object()
                        .unwrap()
                        .values()
                        .any(|path| path == &intent["path"])
                );
                if kind == "dir-interruption" {
                    let parent = Path::new(intent["path"].as_str().unwrap())
                        .parent()
                        .unwrap();
                    assert!(
                        record["pending"]["directories"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|directory| directory["path"] == parent.to_str().unwrap())
                    );
                }
            }
            assert_eq!(record["receipt"]["complete"], true);
            assert_eq!(
                record["receipt"]["legacy_skills"]
                    .as_object()
                    .unwrap()
                    .len(),
                if keep { 2 } else { 0 }
            );
            if !keep {
                assert!(
                    !record["receipt"]["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|entry| entry["path"].as_str().unwrap().contains("/merl-mail/"))
                );
            }
            assert_eq!(
                record["codex_dir"],
                keep || matches!(kind, "nonempty" | "replaced")
            );
            if kind == "nonempty" {
                assert_eq!(record["unrelated"], true);
            }
            if kind == "replaced" {
                assert_eq!(record["preserved_original"], true);
            }
            assert_eq!(record["mail"], "Synthetic durable mail.\n");
        }
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn snapshot(path: &Path, entries: &mut BTreeMap<PathBuf, Value>) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        entries.insert(path.into(), json!({"link":fs::read_link(path).unwrap()}));
    } else if metadata.is_dir() {
        for child in fs::read_dir(path).unwrap() {
            snapshot(&child.unwrap().path(), entries);
        }
    } else {
        entries.insert(path.into(),json!({"bytes":fs::read(path).unwrap(),"device":metadata.dev(),"inode":metadata.ino(),"mode":metadata.mode(),"modified":metadata.mtime_nsec()}));
    }
}
impl Drop for Upgrade {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
