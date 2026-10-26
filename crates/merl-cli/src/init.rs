//! Opt-in user setup and controlling-terminal guidance.

use crate::{CliError, render_json};
use merl_setup::{Bundle, Locations, Plan, Settings, SetupError, setup_settings};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};

const SKILL: &str = include_str!("../../../skills/merl-mail/SKILL.md");
const VERSION: &str = env!("CARGO_PKG_VERSION");

impl From<SetupError> for CliError {
    fn from(error: SetupError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}
fn input(message: &str) -> CliError {
    CliError {
        code: "INIT_INPUT",
        message: message.into(),
    }
}
pub(super) fn handles(a: &[String]) -> bool {
    a.first().is_some_and(|s| s == "init")
        || (a.first().is_some_and(|s| s == "help") && a.get(1).is_some_and(|s| s == "init"))
}

#[derive(Default)]
struct Arguments {
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
    hosts: Vec<String>,
    help: bool,
}
fn parse(a: &[String], json_output: &mut bool) -> Result<Arguments, CliError> {
    let mut args = Arguments {
        help: a[0] == "help",
        ..Arguments::default()
    };
    let mut index = if args.help { 2 } else { 1 };
    while index < a.len() {
        let flag = a[index].as_str();
        match flag {
            "--json" => *json_output = true,
            "--help" | "-h" => args.help = true,
            "--global" | "--yes" | "--dry-run" | "--show" | "--uninstall" => {
                if !args.flags.insert(flag.into()) {
                    return Err(input("duplicate option"));
                }
            }
            "--home"
            | "--config"
            | "--host"
            | "--dispatcher"
            | "--codex-skills-dir"
            | "--claude-skills-dir"
            | "--format" => {
                index += 1;
                let value = a
                    .get(index)
                    .ok_or_else(|| input("option requires a value"))?;
                match flag {
                    "--host" => {
                        if !matches!(value.as_str(), "codex" | "claude" | "none")
                            || args.hosts.contains(value)
                        {
                            return Err(input("choose each host once: codex, claude or none"));
                        }
                        args.hosts.push(value.clone());
                    }
                    "--format" => match value.as_str() {
                        "human" => *json_output = false,
                        "json" => *json_output = true,
                        _ => return Err(input("format must be human or json")),
                    },
                    _ => {
                        if args.values.insert(flag.into(), value.clone()).is_some() {
                            return Err(input("duplicate option"));
                        }
                    }
                }
            }
            _ => return Err(input("unknown init option; run merl help init")),
        }
        index += 1;
    }
    if args.hosts.contains(&"none".into()) && args.hosts.len() != 1 {
        return Err(input("host none cannot be combined with another host"));
    }
    if args.flags.contains("--show")
        && (args.flags.contains("--dry-run") || args.flags.contains("--uninstall"))
    {
        return Err(input("show cannot be combined with dry-run or uninstall"));
    }
    Ok(args)
}

#[expect(
    clippy::too_many_lines,
    reason = "setup selection, preview and confirmation share one bounded command flow"
)]
pub(super) fn execute(a: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let args = parse(a, json_output)?;
    if args.help {
        return help(*json_output);
    }
    let locations = Locations::from_environment()?;
    let config = args
        .values
        .get("--config")
        .map_or_else(|| locations.config.clone(), PathBuf::from);
    let saved = setup_settings(&config)?;
    let show = args.flags.contains("--show");
    let preview = args.flags.contains("--dry-run");
    let uninstall = args.flags.contains("--uninstall");
    let yes = args.flags.contains("--yes");
    if show {
        return report(
            saved.as_ref(),
            &config,
            if saved.is_some() {
                if merl_setup::setup_is_complete(&config)? {
                    "configured"
                } else {
                    "incomplete"
                }
            } else {
                "not_configured"
            },
            *json_output,
        );
    }
    if uninstall && saved.is_none() {
        return report(None, &config, "not_configured", *json_output);
    }
    let mut terminal = if !yes && !preview && !*json_output {
        Some(Terminal::open()?)
    } else {
        None
    };
    let mut settings = saved.clone().unwrap_or(Settings {
        mail_home: locations.mail.clone(),
        skills: BTreeMap::new(),
        foreground: false,
    });
    if !uninstall {
        let mut hosts = args.hosts.clone();
        if hosts.is_empty() && saved.is_none() {
            if let Some(terminal) = &mut terminal {
                let detected = ["codex", "claude"]
                    .into_iter()
                    .filter(|name| {
                        find_executable(name).is_some()
                            || match *name {
                                "codex" => locations.codex_skills.exists(),
                                _ => locations.claude_skills.exists(),
                            }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let detected = if detected.is_empty() {
                    "none"
                } else {
                    &detected
                };
                let answer = terminal.ask("Select hosts (codex,claude or none)", detected)?;
                hosts = answer.split(',').map(|s| s.trim().into()).collect();
                if hosts
                    .iter()
                    .any(|h| !matches!(h.as_str(), "codex" | "claude" | "none"))
                    || (hosts.contains(&"none".into()) && hosts.len() != 1)
                {
                    return Err(input("invalid host selection"));
                }
            } else {
                return Err(input(
                    "first unattended setup requires --host and --home; apply also requires --yes",
                ));
            }
        }
        // Explicit host selection augments existing setup. Removal is --uninstall.
        for host in hosts.iter().filter(|h| h.as_str() != "none") {
            let key = format!("--{host}-skills-dir");
            let root = args.values.get(&key).map_or_else(
                || {
                    if host == "codex" {
                        locations.codex_skills.clone()
                    } else {
                        locations.claude_skills.clone()
                    }
                },
                PathBuf::from,
            );
            let path = args.values.get(&key).map_or_else(
                || {
                    settings
                        .skills
                        .get(host)
                        .cloned()
                        .unwrap_or_else(|| root.join("merl-mail/SKILL.md"))
                },
                |_| root.join("merl-mail/SKILL.md"),
            );
            settings.skills.insert(host.clone(), path);
        }
        if let Some(home) = args.values.get("--home") {
            settings.mail_home = home.into();
        } else if saved.is_none() {
            if let Some(terminal) = &mut terminal {
                settings.mail_home = terminal
                    .ask(
                        "Durable mailbox home",
                        &settings.mail_home.to_string_lossy(),
                    )?
                    .into();
            } else {
                return Err(input("first unattended setup requires an explicit --home"));
            }
        }
        if let Some(dispatcher) = args.values.get("--dispatcher") {
            settings.foreground = match dispatcher.as_str() {
                "foreground" => true,
                "none" => false,
                _ => return Err(input("dispatcher must be foreground or none")),
            };
        } else if let Some(terminal) = &mut terminal {
            settings.foreground = terminal.confirm(
                "Show dispatcher start instructions? No process or service will start",
                settings.foreground,
            )?;
        }
    } else if args.values.keys().any(|k| k != "--config") {
        return Err(input(
            "uninstall uses saved paths; select only --host or the whole setup",
        ));
    }
    if uninstall && args.hosts.iter().any(|host| host == "none") {
        return Err(input(
            "uninstall selects codex/claude or all hosts when omitted",
        ));
    }
    let bundle = Bundle {
        version: VERSION,
        skill: SKILL,
    };
    for host in ["codex", "claude"] {
        if args.values.contains_key(&format!("--{host}-skills-dir"))
            && !args.hosts.iter().any(|h| h == host)
        {
            return Err(input(
                "a skill-directory override requires its explicit --host",
            ));
        }
    }
    let plan = if uninstall {
        Plan::removal(config.clone(), settings.clone(), bundle)?
    } else {
        Plan::new(config.clone(), settings.clone(), bundle)?
    };
    if preview {
        return report(Some(&settings), &config, "preview", *json_output);
    }
    if !yes {
        let Some(terminal) = &mut terminal else {
            return Err(input("unattended apply/uninstall requires --yes"));
        };
        terminal.write(&report(Some(&settings), &config, "preview", false)?)?;
        if !terminal.confirm(
            if uninstall {
                "Remove unchanged owned setup files? Mail is retained"
            } else {
                "Apply this user setup?"
            },
            false,
        )? {
            return Err(input("setup cancelled; no changes applied"));
        }
    }
    let changed = if uninstall {
        plan.uninstall(&args.hosts, &mut |_| Ok(()))?
    } else {
        plan.apply(&mut |_| Ok(()))?
    };
    if uninstall && !args.hosts.is_empty() {
        settings.skills.retain(|host, _| !args.hosts.contains(host));
    }
    report(
        Some(&settings),
        &config,
        if !changed {
            "unchanged"
        } else if uninstall {
            "uninstalled"
        } else {
            "configured"
        },
        *json_output,
    )
}

struct Terminal {
    reader: BufReader<File>,
    writer: File,
}
impl Terminal {
    fn open() -> Result<Self, CliError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|_| {
                input("no controlling terminal; use explicit --host/--home and --yes, or --dry-run")
            })?;
        if !file.is_terminal() {
            return Err(input("setup requires a controlling terminal or --yes"));
        }
        let writer = file.try_clone().map_err(SetupError::from)?;
        Ok(Self {
            reader: BufReader::new(file),
            writer,
        })
    }
    fn write(&mut self, text: &str) -> Result<(), CliError> {
        self.writer
            .write_all(text.as_bytes())
            .map_err(SetupError::from)?;
        self.writer.flush().map_err(SetupError::from)?;
        Ok(())
    }
    fn ask(&mut self, question: &str, default: &str) -> Result<String, CliError> {
        self.write(&format!("{question} [{default}]: "))?;
        let mut line = String::new();
        self.reader
            .by_ref()
            .take(4097)
            .read_line(&mut line)
            .map_err(SetupError::from)?;
        if line.len() > 4096 || !line.ends_with('\n') {
            return Err(input(
                "terminal input ended or exceeded 4096 bytes; no confirmation assumed",
            ));
        }
        let answer = line.trim();
        Ok(if answer.is_empty() {
            default.into()
        } else {
            answer.into()
        })
    }
    fn confirm(&mut self, question: &str, default: bool) -> Result<bool, CliError> {
        let answer = self.ask(question, if default { "y/N: y" } else { "y/N" })?;
        Ok(answer.eq_ignore_ascii_case("y")
            || answer.eq_ignore_ascii_case("yes")
            || (default && answer == "y/N: y"))
    }
}
fn find_executable(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(name))
        .find_map(|path| {
            let metadata = path.metadata().ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return None;
                }
            }
            metadata
                .is_file()
                .then(|| path.canonicalize().ok())
                .flatten()
        })
}
fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
fn dispatcher(settings: &Settings) -> Result<Value, CliError> {
    if !settings.foreground {
        return Ok(json!({"state":"not_requested"}));
    }
    let Some(codex) = find_executable("codex") else {
        return Ok(
            json!({"state":"missing_codex","instructions":"Skill setup is available. Install Codex and rerun init --show for its foreground dispatcher instructions. Claude automatic wake is not supported."}),
        );
    };
    let merl = std::env::current_exe()
        .map_err(SetupError::from)?
        .canonicalize()
        .map_err(SetupError::from)?;
    let base = format!("{} mail", quoted(&merl));
    let home = quoted(&settings.mail_home);
    Ok(
        json!({"state":"not_started","codex":codex,"start_command":format!("{base} watch --home {home} --codex {}",quoted(&codex)),"steps":["Choose AGENT, an existing Codex session UUID, and its local Unix socket. Replace placeholders below; inspect status for the expected generation.",format!("{base} register --home {home} --agent AGENT --json"),format!("{base} status --home {home} --agent AGENT --json"),format!("{base} bind --home {home} --agent AGENT --session UUID --socket /absolute/codex.sock --expected-generation GENERATION --json"),"Run start_command in the foreground from a stable directory. Stop that process with Ctrl-C. No service has been installed. Claude skill installation does not enable automatic Claude wake."]}),
    )
}
fn report(
    settings: Option<&Settings>,
    config: &Path,
    outcome: &str,
    json_output: bool,
) -> Result<String, CliError> {
    let value = if let Some(settings) = settings {
        json!({"schema":"merl.init/v1","outcome":outcome,"config":config,"mail_home":settings.mail_home,"skills":settings.skills,"dispatcher":dispatcher(settings)?})
    } else {
        json!({"schema":"merl.init/v1","outcome":outcome,"config":config,"dispatcher":{"state":"not_requested"}})
    };
    if json_output {
        render_json(&value)
    } else {
        Ok(format!(
            "{}\n",
            serde_json::to_string_pretty(&value)
                .map_err(|_| input("cannot render setup report"))?
        ))
    }
}
fn help(json_output: bool) -> Result<String, CliError> {
    let value = json!({"schema":"merl.help/v1","command":"init","kind":"command","usage":"merl init [--global] [--host codex|claude|none] [--home <absolute-path>] [--config <absolute-path>] [--dispatcher foreground|none] [--codex-skills-dir <absolute-path>] [--claude-skills-dir <absolute-path>] [--yes] [--dry-run|--show|--uninstall] [--format human|json] [--json]","summary":"Set up matching user skills and a durable mailbox default; optionally show dispatcher start instructions.","description":"Interactive input uses the controlling terminal. JSON never prompts. Setup never binds sessions, starts services, or migrates mail. Uninstall preserves all mail and borrowed/modified files.","children":[],"arguments":["--host (repeatable)","--home","--config","--dispatcher","--codex-skills-dir","--claude-skills-dir","--global","--yes","--dry-run","--show","--uninstall"],"outcomes":["preview","configured","not_configured","unchanged","uninstalled"],"errors":["INIT_INPUT","INIT_CONFIG","INIT_CONFLICT","INIT_BUSY","INIT_IO"],"related":["mail bind","mail watch"],"example":"merl init --global --host codex --home /absolute/durable-mail --dispatcher foreground --yes --json","examples":["merl init","merl init --show --json","merl init --uninstall --yes"]});
    if json_output {
        render_json(&value)
    } else {
        Ok(format!(
            "{}\n{}\n{}\nOutcomes: {}\nErrors: {}\nRelated: {}\nExample: {}\n",
            value["usage"].as_str().unwrap_or_default(),
            value["summary"].as_str().unwrap_or_default(),
            value["description"].as_str().unwrap_or_default(),
            value["outcomes"],
            value["errors"],
            value["related"],
            value["example"].as_str().unwrap_or_default()
        ))
    }
}
