//! Shared human setup summary and read-only top-level status.

use crate::{CliError, init::BUNDLE, render_json};
use merl_setup::{
    Change, DirectoryState, FileChange, FileState, Ownership, Plan, Settings, SetupState,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub(super) fn handles(args: &[String]) -> bool {
    args.first().is_some_and(|s| s == "status")
        || (args.first().is_some_and(|s| s == "help") && args.get(1).is_some_and(|s| s == "status"))
}
fn input(message: &str) -> CliError {
    CliError {
        code: "STATUS_INPUT",
        message: message.into(),
    }
}
pub(super) fn execute(args: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut home = None;
    let mut config = None;
    let mut help = args[0] == "help";
    let mut index = if help { 2 } else { 1 };
    while index < args.len() {
        match args[index].as_str() {
            "--verbose" | "-v" | "--json" => *json_output = true,
            "--help" | "-h" => help = true,
            "--format" => {
                index += 1;
                match args.get(index).map(String::as_str) {
                    Some("json") => *json_output = true,
                    Some("human") => *json_output = false,
                    _ => return Err(input("format must be human or json")),
                }
            }
            flag @ ("--home" | "--config") => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| input("option requires a value"))?;
                let target = if flag == "--home" {
                    &mut home
                } else {
                    &mut config
                };
                if target.replace(value.as_str()).is_some() {
                    return Err(input("duplicate option"));
                }
            }
            _ => return Err(input("unknown status option; run merl help status")),
        }
        index += 1;
    }
    if help {
        return help_record(*json_output);
    }
    let record = if let Some(home) = home {
        let path = merl_setup::resolve_home(Some(home), None)?;
        json!({"schema":"merl.status/v1","setup":{"state":"not_checked","pending":null,"issues":[]},"config":{"path":null,"source":"bypassed","state":"not_checked"},"mailbox":{"path":path,"source":"explicit","state":directory_name(DirectoryState::inspect(&path)?)},"skills":[],"dispatcher":{"runtime":"not_checked","guidance":"unknown"},"next_step":"merl help mail"})
    } else {
        let path = config
            .map(PathBuf::from)
            .map_or_else(merl_setup::default_config, Ok)?;
        observe(
            &path,
            if config.is_some() {
                "explicit"
            } else {
                "user_default"
            },
        )?
    };
    if *json_output {
        render_json(&record)
    } else {
        Ok(summary(&record))
    }
}

pub(super) fn observe(config: &Path, source: &str) -> Result<Value, CliError> {
    let observed = merl_setup::inspect(config, BUNDLE)?;
    let state = match observed.state {
        SetupState::NotConfigured => "not_configured",
        SetupState::Configured => "configured",
        SetupState::Incomplete => "incomplete",
    };
    let mailbox = if let Some(settings) = &observed.settings {
        json!({"path":settings.mail_home,"source":if observed.config_present {"saved_default"} else {"receipt"},"state":observed.mailbox.map(directory_name)})
    } else {
        json!({"path":null,"source":"none","state":"not_selected"})
    };
    let skills=observed.skills.iter().map(|skill|json!({"host":skill.host,"path":skill.path,"state":file_name(skill.state),"ownership":match skill.ownership {Ownership::Owned=>"owned",Ownership::Borrowed=>"borrowed",Ownership::Unmanaged=>"unmanaged",Ownership::Pending=>"pending"},"matches_bundle":skill.matches_bundle})).collect::<Vec<_>>();
    let guidance = observed.settings.as_ref().map_or("unknown", |s| {
        if s.foreground { "foreground" } else { "none" }
    });
    let next = match observed.state {
        SetupState::NotConfigured => "merl init",
        SetupState::Incomplete => "merl help init",
        SetupState::Configured => {
            if guidance == "foreground" {
                "merl help mail bind"
            } else {
                "merl help mail"
            }
        }
    };
    let mut issues = Vec::new();
    if observed.pending {
        issues.push(json!({"kind":"unfinished_setup"}));
    }
    if observed.config_state != FileState::Installed && observed.settings.is_some() {
        issues.push(
            json!({"kind":"settings","state":file_name(observed.config_state),"path":config}),
        );
    }
    for skill in &observed.skills {
        if !matches!(skill.state, FileState::Installed | FileState::Outdated) {
            issues.push(json!({"kind":"skill","host":skill.host,"state":file_name(skill.state),"path":skill.path}));
        }
    }
    if observed
        .mailbox
        .is_some_and(|directory| directory != DirectoryState::Exists)
    {
        issues.push(json!({"kind":"mailbox","state":observed.mailbox.map(directory_name)}));
    }
    Ok(
        json!({"schema":"merl.status/v1","setup":{"state":state,"pending":observed.pending,"issues":issues},"config":{"path":config,"source":source,"state":file_name(observed.config_state)},"mailbox":mailbox,"skills":skills,"dispatcher":{"runtime":"not_checked","guidance":guidance},"next_step":next}),
    )
}

fn file_name(state: FileState) -> &'static str {
    match state {
        FileState::Installed => "installed",
        FileState::Outdated => "outdated",
        FileState::Missing => "missing",
        FileState::Pending => "pending",
        FileState::Modified => "modified",
        FileState::Different => "different",
        FileState::Conflict => "conflict",
    }
}
fn directory_name(state: DirectoryState) -> &'static str {
    match state {
        DirectoryState::Exists => "exists",
        DirectoryState::Missing => "missing",
        DirectoryState::Conflict => "conflict",
    }
}

pub(super) fn preview(
    config: &Path,
    settings: &Settings,
    plan: &Plan<'_>,
    hosts: &[String],
    removing: bool,
) -> Result<Value, CliError> {
    let changes = plan.changes(hosts)?;
    let skills=settings.skills.iter().map(|(host,path)| {
        let action=changes.iter().find(|change|&change.path==path).map(|change|change.action);
        json!({"host":host,"path":path,"state":"proposed","action":action.map_or("unchanged",|action|action_name(action,false))})
    }).collect::<Vec<_>>();
    let config_action = changes
        .iter()
        .find(|change| change.path == config)
        .map_or("unchanged", |change| action_name(change.action, true));
    let directory = DirectoryState::inspect(&settings.mail_home)?;
    Ok(
        json!({"setup":{"state":if removing {"removal_preview"} else {"preview"}},"skills":skills,"config":{"path":config,"state":"proposed","action":config_action},"mailbox":{"path":settings.mail_home,"source":if removing {"retained"} else {"proposed"},"state":directory_name(directory),"action":if removing {"will retain all mail"} else if directory==DirectoryState::Missing {"will create; will use as default"} else {"will use as default"}},"dispatcher":{"runtime":"not_checked","guidance":if settings.foreground {"foreground"} else {"none"}},"next_step":null}),
    )
}
fn action_name(action: Change, config: bool) -> &'static str {
    match action {
        Change::Create => {
            if config {
                "will save"
            } else {
                "will install"
            }
        }
        Change::Update => "will update",
        Change::Unchanged => "unchanged",
        Change::Borrowed => "unchanged; borrowed",
        Change::Remove => "will remove",
        Change::Retain => "will retain; borrowed",
    }
}

pub(super) fn removal_result(
    config: &Path,
    settings: &Settings,
    changes: &[FileChange],
    hosts: &[String],
) -> Result<Value, CliError> {
    let mut record = observe(config, "selected")?;
    let mut skills = record["skills"].as_array().cloned().unwrap_or_default();
    for (host, path) in &settings.skills {
        if !hosts.is_empty() && !hosts.contains(host) {
            continue;
        }
        if let Some(change) = changes.iter().find(|change| &change.path == path) {
            skills.push(json!({"host":host,"path":path,"state":if change.action==Change::Remove {"removed"} else {"retained"},"ownership":if change.action==Change::Retain {"borrowed"} else {"owned"}}));
        }
    }
    record["skills"] = json!(skills);
    record["mailbox"] = json!({"path":settings.mail_home,"source":if hosts.is_empty() {"retained"} else {"saved_default"},"state":directory_name(DirectoryState::inspect(&settings.mail_home)?)});
    if hosts.is_empty() {
        record["setup"]["state"] = json!("removed");
    }
    Ok(record)
}

pub(super) fn display_path(path: &Path) -> String {
    if let Some(profile) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        && let Ok(relative) = path.strip_prefix(&profile)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.display().to_string()
}
fn path_label(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| "Not selected".into(), |s| display_path(Path::new(s)))
}
fn text(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

/// Every human setup screen uses these same fields; proposals label their actions.
pub(super) fn summary(record: &Value) -> String {
    let state = text(&record["setup"]["state"]);
    let title = match state {
        "configured" => "Configured",
        "not_configured" => "Not configured",
        "incomplete" => "Incomplete",
        "preview" => "Preview (proposed changes)",
        "removal_preview" => "Preview (proposed removals)",
        "removed" => "Removed",
        _ => "Not checked",
    };
    let mut lines = vec![format!("Merl setup: {title}")];
    lines.extend(skill_rows(record));
    let mailbox = &record["mailbox"];
    let source = match text(&mailbox["source"]) {
        "saved_default" => "saved default",
        "receipt" => "planned home; not an active default",
        "explicit" => "explicit home",
        "retained" => "retained; no saved default",
        _ => "",
    };
    let directory = match text(&mailbox["state"]) {
        "exists" => "directory exists",
        "missing" => "directory missing",
        "conflict" => "path conflict",
        _ => "",
    };
    let description = mailbox["action"].as_str().map_or_else(
        || {
            [source, directory]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("; ")
        },
        str::to_owned,
    );
    lines.push(format!(
        "  Mailbox: {}{}",
        path_label(&mailbox["path"]),
        if description.is_empty() {
            String::new()
        } else {
            format!(" ({description})")
        }
    ));
    let config = &record["config"];
    let effect = config["action"]
        .as_str()
        .unwrap_or_else(|| match text(&config["state"]) {
            "installed" => "saved",
            "missing" => "not found",
            "modified" => "modified; preserved",
            "pending" => "pending publication",
            "conflict" => "conflict; preserved",
            _ => "Not checked",
        });
    lines.push(if config["source"] == "bypassed" {
        "  Settings: Not checked (--home supplied)".into()
    } else {
        format!("  Settings: {} ({effect})", path_label(&config["path"]))
    });
    let dispatcher = if matches!(state, "preview" | "removal_preview") {
        "Not checked; setup will not start it."
    } else if record["dispatcher"]["guidance"] == "foreground" {
        "Not checked; foreground guidance saved. No process started by setup."
    } else {
        "Not checked. No process started by setup."
    };
    lines.push(format!("  Dispatcher: {dispatcher}"));
    if let Some(next) = record["next_step"].as_str() {
        lines.push(format!("Next: {next}"));
    }
    format!("{}\n", lines.join("\n"))
}
fn skill_rows(record: &Value) -> Vec<String> {
    let state = text(&record["setup"]["state"]);
    let mut lines = Vec::new();
    if let Some(skills) = record["skills"].as_array() {
        if skills.is_empty() {
            lines.push(if state == "not_checked" {
                "  Skills: Not checked (--home supplied)".into()
            } else {
                "  Skills: Not selected".into()
            });
        }
        for host in ["codex", "claude"] {
            for skill in skills.iter().filter(|skill| skill["host"] == host) {
                let label = if host == "codex" { "Codex" } else { "Claude" };
                let mut observed = skill["action"]
                    .as_str()
                    .unwrap_or_else(|| match text(&skill["state"]) {
                        "modified" => "modified; preserved",
                        "conflict" => "conflict; preserved",
                        "different" => "different; preserved",
                        state => state,
                    })
                    .to_owned();
                if skill["action"].is_null() && skill["ownership"] == "borrowed" {
                    observed.push_str("; borrowed; preserved");
                }
                lines.push(format!(
                    "  {label} skill: {observed} ({})",
                    path_label(&skill["path"])
                ));
            }
        }
    }
    lines
}

fn help_record(json_output: bool) -> Result<String, CliError> {
    let record = json!({"schema":"merl.help/v1","command":"status","kind":"command","usage":"merl status [--home <absolute-path>] [--config <absolute-path>] [--verbose|-v|--json] [--format human|json]","summary":"Summarize local setup without changing files or checking dispatcher/session health.","children":[],"arguments":["--home","--config","--verbose","-v","--json","--format"],"outcomes":["not_configured","configured","incomplete","not_checked"],"errors":["STATUS_INPUT","INIT_INPUT","INIT_CONFIG","INIT_CONFLICT","INIT_IO"],"related":["init","mail status"],"example":"merl status --verbose","examples":["merl status","merl status -v"]});
    if json_output {
        render_json(&record)
    } else {
        Ok(format!(
            "{}\n{}\nOutcomes: {}\nErrors: {}\nRelated: {}\nExample: {}\n",
            record["usage"].as_str().unwrap_or_default(),
            record["summary"].as_str().unwrap_or_default(),
            record["outcomes"],
            record["errors"],
            record["related"],
            record["example"].as_str().unwrap_or_default()
        ))
    }
}
