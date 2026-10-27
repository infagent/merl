use merl_setup::{Bundle, Plan, Settings};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const LEGACY: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/skills/merl-mail/SKILL.md"
));

struct User {
    root: PathBuf,
    kind: &'static str,
}
impl User {
    fn config(&self) -> PathBuf {
        self.root.join("settings/config.json")
    }
    fn receipt(&self) -> PathBuf {
        self.root.join("settings/.config.json.merl-init.json")
    }
    fn mail(&self) -> PathBuf {
        self.root.join("mail")
    }
    fn skill(&self, host: &str, name: &str) -> PathBuf {
        self.root
            .join(format!("custom {host} root/{name}/SKILL.md"))
    }
    fn settings(&self, name: &str) -> Settings {
        Settings {
            mail_home: self.mail(),
            skills: ["codex", "claude"]
                .into_iter()
                .map(|h| (h.into(), self.skill(h, name)))
                .collect(),
            foreground: false,
        }
    }
    fn legacy(&self) {
        let mut files = Vec::new();
        for host in ["codex", "claude"] {
            let path = self.skill(host, "merl-mail");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, LEGACY).unwrap();
            if self.kind != "unmanaged" {
                files.push(
                    json!({"path":path,"text":LEGACY,"owned":self.kind!="borrowed","pending":null}),
                );
            }
        }
        fs::create_dir_all(self.config().parent().unwrap()).unwrap();
        fs::create_dir(self.mail()).unwrap();
        fs::write(self.mail().join("keep"), "Synthetic durable mail.\n").unwrap();
        let settings = json!({"schema":"merl.config/v1","mail_home":self.mail(),"skills":{"codex":self.skill("codex","merl-mail"),"claude":self.skill("claude","merl-mail")},"dispatcher":"none"});
        let text = format!("{settings}\n");
        fs::write(self.config(), &text).unwrap();
        files.push(json!({"path":self.config(),"text":text,"owned":self.kind!="borrowed-config","pending":null}));
        fs::write(self.receipt(),format!("{}\n",json!({"schema":"merl.init-receipt/v1","version":"0.3.0","complete":self.kind!="incomplete","settings":settings,"files":files,"directories":[]}))).unwrap();
        if self.kind == "modified" {
            fs::write(self.skill("codex", "merl-mail"), "Synthetic local edit.\n").unwrap();
        }
        if self.kind == "edited-config" {
            let mut edited = read_json(&self.config());
            edited["synthetic_user_field"] = json!("preserve this edit");
            fs::write(self.config(), format!("{edited}\n")).unwrap();
        }
        if self.kind == "collision" {
            let p = self.skill("codex", "merl");
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, "Unrelated synthetic skill.\n").unwrap();
        }
    }
    fn call(&self, args: &[&str], broken: bool) -> Value {
        let mut command = Command::new(env!("CARGO_BIN_EXE_merl"));
        command
            .args(args)
            .env("HOME", &self.root)
            .env_remove("CODEX_HOME")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("XDG_CONFIG_HOME", self.root.join(".config"))
            .env("XDG_DATA_HOME", self.root.join(".data"))
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null());
        if broken {
            command
                .env_remove("HOME")
                .env("XDG_CONFIG_HOME", "bad-relative");
        }
        let output = command.output().unwrap();
        json!({"success":output.status.success(),"stdout":String::from_utf8(output.stdout).unwrap(),"stderr":String::from_utf8(output.stderr).unwrap()})
    }
    fn init(&self, extra: &[&str]) -> Value {
        let config = self.config();
        let home = self.mail();
        let mut args = vec![
            "init",
            "--json",
            "--config",
            config.to_str().unwrap(),
            "--yes",
        ];
        args.extend(extra);
        if !extra.contains(&"--uninstall") && !config.exists() {
            args.extend([
                "--home",
                home.to_str().unwrap(),
                "--host",
                "codex",
                "--host",
                "claude",
            ]);
        }
        let output = self.call(&args, false);
        let mut value: Value = serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap();
        value["success"] = output["success"].clone();
        value
    }
    fn status(&self) -> Value {
        let p = self.config();
        let output = self.call(
            &["status", "--config", p.to_str().unwrap(), "--json"],
            false,
        );
        assert_eq!(output["success"], true, "{output}");
        serde_json::from_str(output["stdout"].as_str().unwrap()).unwrap()
    }
}

pub struct Onboarding {
    root: PathBuf,
    users: Vec<User>,
    records: Vec<Value>,
}
impl Onboarding {
    fn new(label: &str, kinds: &[&'static str], legacy: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("merl-onboarding-{}-{label}", std::process::id()));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        let users = kinds
            .iter()
            .map(|kind| {
                let user = User {
                    root: root.join(kind),
                    kind,
                };
                fs::create_dir(&user.root).unwrap();
                if legacy {
                    user.legacy();
                }
                user
            })
            .collect();
        Self {
            root,
            users,
            records: Vec::new(),
        }
    }
    pub fn given_an_unconfigured_user() -> Self {
        Self::new("usage", &["fresh"], false)
    }
    pub fn given_completed_legacy_setups() -> Self {
        Self::new("legacy", &["owned", "borrowed", "unmanaged"], true)
    }
    pub fn given_incomplete_and_conflicting_legacy_setups() -> Self {
        Self::new(
            "conflicts",
            &[
                "incomplete",
                "modified",
                "borrowed-config",
                "edited-config",
                "collision",
            ],
            true,
        )
    }
    pub fn given_one_completed_legacy_setup() -> Self {
        Self::new("recovery", &["owned"], true)
    }
    pub fn when_usage_and_help_are_requested_without_a_user_environment(mut self) -> Self {
        let user = &self.users[0];
        let before = snapshot(&user.root);
        for args in [
            vec!["usage"],
            vec!["usage", "--json"],
            vec!["help", "usage", "--json"],
            vec!["usage", "--json", "--format", "human"],
            vec!["usage", "--format", "human", "--json"],
        ] {
            self.records.push(user.call(&args, true));
        }
        self.records
            .push(json!({"pure":before==snapshot(&user.root)}));
        self
    }
    pub fn then_the_primer_and_json_are_pure_and_discoverable(self) {
        for result in &self.records[..5] {
            assert_eq!(result["success"], true, "{result}");
        }
        assert_eq!(self.records[5]["pure"], true);
        let machine: Value =
            serde_json::from_str(self.records[1]["stdout"].as_str().unwrap()).unwrap();
        assert_eq!(machine["schema"], "merl.usage/v1");
        let help: Value =
            serde_json::from_str(self.records[2]["stdout"].as_str().unwrap()).unwrap();
        assert_eq!(help["command"], "usage");
        assert_eq!(self.records[0]["stdout"], self.records[3]["stdout"]);
        assert_eq!(self.records[1]["stdout"], self.records[4]["stdout"]);
        let text = self.records[0]["stdout"].as_str().unwrap();
        assert!((20..=40).contains(&text.lines().count()));
        for command in [
            "merl init",
            "merl team list",
            "merl team member",
            "merl mail send",
            "merl help",
        ] {
            assert!(text.contains(command), "{text}");
        }
    }
    pub fn when_setup_is_upgraded_retried_and_uninstalled(mut self) -> Self {
        for user in &self.users {
            let upgrade = user.init(&[]);
            let before_status = snapshot(&user.root);
            let status = user.status();
            let status_pure = before_status == snapshot(&user.root);
            let receipt = read_json(&user.receipt());
            let legacy = fs::read_to_string(user.skill("codex", "merl-mail")).unwrap();
            let canonical = fs::read(user.skill("codex", "merl")).unwrap_or_default();
            let retry = user.init(&[]);
            let selected = user.init(&["--uninstall", "--host", "codex"]);
            let kept_claude = user.skill("claude", "merl").exists();
            let after_selected = read_json(&user.receipt());
            let full = user.init(&["--uninstall"]);
            self.records.push(json!({"upgrade":upgrade,"status":status,"status_pure":status_pure,"receipt":receipt,"legacy":legacy,"canonical":canonical,"retry":retry,"selected":selected,"kept_claude":kept_claude,"after_selected":after_selected,"full":full,"canonical_gone":!user.skill("codex","merl").exists()&&!user.skill("claude","merl").exists(),"legacy_exists":user.skill("codex","merl-mail").exists(),"mail":fs::read_to_string(user.mail().join("keep")).unwrap()}));
        }
        self
    }
    pub fn then_canonical_setup_works_and_only_owned_files_are_removed(self) {
        let source =
            fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/merl/SKILL.md")).unwrap();
        for (user, result) in self.users.iter().zip(&self.records) {
            for field in ["upgrade", "retry", "selected", "full"] {
                assert_eq!(result[field]["success"], true, "{field}: {result}");
            }
            assert_eq!(result["receipt"]["schema"], "merl.init-receipt/v2");
            assert_eq!(result["legacy"], LEGACY);
            assert_eq!(result["status_pure"], true);
            if user.kind == "unmanaged" {
                assert!(
                    result["receipt"]["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|entry| entry["path"]
                            != user.skill("codex", "merl-mail").to_str().unwrap()),
                    "unmanaged contents must not become a receipt backup"
                );
            }
            assert_eq!(result["canonical"], json!(source));
            assert_eq!(
                result["receipt"]["settings"]["skills"]["codex"],
                user.skill("codex", "merl").to_str().unwrap()
            );
            assert_eq!(
                result["receipt"]["legacy_skills"]["codex"],
                user.skill("codex", "merl-mail").to_str().unwrap()
            );
            assert_eq!(result["legacy_exists"], user.kind != "owned");
            assert_eq!(result["kept_claude"], true);
            assert_eq!(result["canonical_gone"], true);
            assert!(
                result["after_selected"]["legacy_skills"]
                    .get("codex")
                    .is_none()
            );
            assert_eq!(
                result["after_selected"]["legacy_skills"]["claude"],
                user.skill("claude", "merl-mail").to_str().unwrap()
            );
            let skills = result["status"]["skills"].as_array().unwrap();
            assert_eq!(skills.len(), 4);
            let retained = skills
                .iter()
                .find(|s| s["host"] == "codex" && s["retained_legacy"] == true)
                .unwrap();
            assert_eq!(retained["matches_bundle"], false);
            assert_eq!(retained["ownership"], user.kind);
            assert_eq!(result["mail"], "Synthetic durable mail.\n");
        }
    }
    pub fn when_upgrade_is_attempted_and_incomplete_setup_is_safely_removed(mut self) -> Self {
        for user in &self.users {
            let before = snapshot(&user.root);
            let result = user.init(&[]);
            let pure = before == snapshot(&user.root);
            let mut record = json!({"result":result,"pure":pure});
            if user.kind == "incomplete" {
                record["remove"] = user.init(&["--uninstall", "--host", "codex"]);
                record["remaining"] = read_json(&user.receipt());
                record["finish_remove"] = user.init(&["--uninstall"]);
                record["retry"] = user.init(&[]);
            }
            self.records.push(record);
        }
        self
    }
    pub fn then_conflicts_write_nothing_and_v1_recovery_stays_available(self) {
        for (user, record) in self.users.iter().zip(&self.records) {
            assert_eq!(record["result"]["success"], false, "{record}");
            assert_eq!(record["pure"], true, "{record}");
            if user.kind == "incomplete" {
                let message = record["result"]["message"].as_str().unwrap();
                assert!(
                    message.contains("--uninstall") && message.contains("--config"),
                    "{message}"
                );
                assert_eq!(record["remove"]["success"], true);
                assert_eq!(record["remaining"]["schema"], "merl.init-receipt/v1");
                assert_eq!(record["finish_remove"]["success"], true);
                assert_eq!(record["retry"]["success"], true);
            }
        }
    }
    pub fn when_publication_and_removal_are_interrupted_then_retried(mut self) -> Self {
        let user = &self.users[0];
        let text =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/merl/SKILL.md"))
                .unwrap_or_else(|_| "synthetic canonical skill".into());
        let bundle = Bundle {
            version: "synthetic-new",
            skill: &text,
        };
        let settings = user.settings("merl");
        let plan = Plan::new(user.config(), settings.clone(), bundle).unwrap();
        // Occupied, documented staging slots make publication unavailable at
        // the filesystem boundary without relying on permissions or timing.
        let directory = user.skill("claude", "merl").parent().unwrap().to_owned();
        fs::create_dir_all(&directory).unwrap();
        let stages = (0..32)
            .map(|n| directory.join(format!(".SKILL.md.merl-stage-{n}")))
            .collect::<Vec<_>>();
        for path in &stages {
            fs::write(path, "Synthetic occupied slot.\n").unwrap();
        }
        let unavailable = plan.apply(&mut |_| Ok(())).unwrap_err();
        let before_publication = read_json(&user.receipt());
        let unpublished_status = user.status();
        let canonical_absent = !user.skill("claude", "merl").exists();
        for path in stages {
            fs::remove_file(path).unwrap();
        }
        let error = plan
            .apply(&mut |_| Err(std::io::Error::other("synthetic interruption")))
            .unwrap_err();
        let pending = read_json(&user.receipt());
        let old_config = read_json(&user.config());
        let resumed = user.init(&[]);
        let removal = Plan::removal(user.config(), settings, bundle).unwrap();
        let removal_error = removal
            .uninstall(&["codex".into()], &mut |_| {
                Err(std::io::Error::other("synthetic removal interruption"))
            })
            .unwrap_err();
        let interrupted = read_json(&user.receipt());
        let retry = user.init(&["--uninstall", "--host", "codex"]);
        let remaining = read_json(&user.receipt());
        self.records.push(json!({"unavailable":unavailable.code,"before_publication":before_publication,"unpublished_status":unpublished_status,"canonical_absent":canonical_absent,"publication_error":error.code,"pending":pending,"old_config":old_config,"resumed":resumed,"removal_error":removal_error.code,"interrupted":interrupted,"retry":retry,"remaining":remaining,"claude_exists":user.skill("claude","merl").exists(),"codex_legacy_gone":!user.skill("codex","merl-mail").exists(),"mail":fs::read_to_string(user.mail().join("keep")).unwrap()}));
        self
    }
    pub fn then_ownership_and_unselected_host_state_survive_recovery(self) {
        let r = &self.records[0];
        assert_eq!(r["unavailable"], "INIT_CONFLICT");
        assert_eq!(r["before_publication"]["schema"], "merl.init-receipt/v2");
        assert_eq!(r["canonical_absent"], true);
        let pending = r["unpublished_status"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|skill| skill["host"] == "claude" && skill["retained_legacy"] == false)
            .unwrap();
        assert_eq!(pending["state"], "pending");
        assert_eq!(
            r["unpublished_status"]["mailbox"]["source"],
            "saved_default"
        );
        assert_eq!(r["publication_error"], "INIT_IO");
        assert_eq!(r["pending"]["schema"], "merl.init-receipt/v2");
        assert_eq!(r["pending"]["complete"], false);
        assert!(
            r["old_config"]["skills"]["codex"]
                .as_str()
                .unwrap()
                .contains("merl-mail/")
        );
        assert_eq!(r["resumed"]["success"], true);
        assert_eq!(r["removal_error"], "INIT_IO");
        assert!(r["interrupted"]["legacy_skills"].get("codex").is_some());
        assert_eq!(r["retry"]["success"], true);
        assert!(r["remaining"]["legacy_skills"].get("codex").is_none());
        assert!(r["remaining"]["legacy_skills"].get("claude").is_some());
        assert_eq!(r["claude_exists"], true);
        assert_eq!(r["codex_legacy_gone"], true);
        assert_eq!(r["mail"], "Synthetic durable mail.\n");
    }
}
fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.insert(path.clone(), vec![]);
            out.extend(snapshot(&path));
        } else {
            out.insert(path.clone(), fs::read(path).unwrap());
        }
    }
    out
}
impl Drop for Onboarding {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
