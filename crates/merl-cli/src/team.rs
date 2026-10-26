//! Bounded team onboarding and read-only scope/roster discovery.

use crate::{CliError, render_json};
use merl_mail::{
    Key, MAX_BODY_BYTES, MailError, Mailbox,
    team::{MAX_REFS, MemberSpec, TeamSpec},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt::Write, fs::File, io::Read};

pub(super) fn handles(a: &[String]) -> bool {
    a.first().is_some_and(|s| s == "team")
        || (a.first().is_some_and(|s| s == "help") && a.get(1).is_some_and(|s| s == "team"))
}
fn input(message: &str) -> CliError {
    CliError {
        code: "TEAM_INPUT",
        message: message.into(),
    }
}

pub(super) fn missing() -> CliError {
    CliError {
        code: "TEAM_NOT_FOUND",
        message: "team does not exist".into(),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one bounded parser validates the five team leaf contracts before I/O"
)]
pub(super) fn execute(a: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut options = BTreeMap::new();
    let mut refs = Vec::new();
    let mut words = Vec::new();
    let mut help = a[0] == "help";
    let mut i = if help { 2 } else { 1 };
    while i < a.len() {
        match a[i].as_str() {
            "--json" => *json_output = true,
            "--format" => {
                i += 1;
                *json_output = match a.get(i).map(String::as_str) {
                    Some("json") => true,
                    Some("human") => false,
                    _ => return Err(input("format must be human or json")),
                };
            }
            "--help" | "-h" => help = true,
            flag @ ("--home" | "--config" | "--name" | "--coordinator" | "--scope-file"
            | "--team" | "--member" | "--role" | "--description-file"
            | "--guidance-ref" | "--after" | "--limit") => {
                i += 1;
                let value = a.get(i).ok_or_else(|| input("option requires a value"))?;
                if flag == "--guidance-ref" {
                    if refs.len() >= MAX_REFS {
                        return Err(input("at most 16 guidance references"));
                    }
                    refs.push(value.clone());
                } else if options.insert(flag, value.as_str()).is_some() {
                    return Err(input("duplicate option"));
                }
            }
            flag if flag.starts_with('-') => return Err(input("unknown team option")),
            word => words.push(word),
        }
        i += 1;
    }
    if words.len() > 1 {
        return Err(input("load one team operation at a time"));
    }
    let operation = words.first().copied().unwrap_or("");
    if help || operation.is_empty() {
        return help_record(operation, *json_output);
    }
    let (_, flags, _) =
        contract(operation).ok_or_else(|| input("unknown team operation; run merl help team"))?;
    for flag in options.keys() {
        if !matches!(*flag, "--home" | "--config") && !flags.contains(flag) {
            return Err(input("option does not apply to this team operation"));
        }
    }
    if !refs.is_empty() && !matches!(operation, "create" | "join") {
        return Err(input("guidance references apply only to create/join"));
    }
    let get = |flag: &str| {
        options
            .get(flag)
            .copied()
            .ok_or_else(|| input(&format!("missing {flag}")))
    };
    let key = |flag: &str| -> Result<Key, CliError> {
        get(flag)?.parse().map_err(|_| {
            input("names require 1..64 ASCII letters, digits, dot, underscore or hyphen")
        })
    };
    // Validate all supplied content before opening a mutating mailbox.
    let create = if operation == "create" {
        Some(TeamSpec::new(
            key("--name")?,
            key("--coordinator")?,
            read_brief(get("--scope-file")?)?,
            refs.clone(),
        )?)
    } else {
        None
    };
    let join = if operation == "join" {
        Some(MemberSpec::new(
            key("--member")?,
            options.get("--role").map(|r| (*r).to_owned()),
            read_brief(get("--description-file")?)?,
            refs,
        )?)
    } else {
        None
    };
    let team = if operation == "create" {
        None
    } else {
        Some(key("--team")?)
    };
    let member = if operation == "member" {
        Some(key("--member")?)
    } else {
        None
    };
    let after: Option<Key> = options
        .get("--after")
        .map(|s| s.parse().map_err(|_| input("invalid member cursor")))
        .transpose()?;
    let limit = options
        .get("--limit")
        .unwrap_or(&"100")
        .parse::<usize>()
        .ok()
        .filter(|n| (1..=100).contains(n))
        .ok_or_else(|| input("limit must be 1..100"))?;
    let home = merl_setup::resolve_home(
        options.get("--home").copied(),
        options.get("--config").copied(),
    )?;
    let mut store = if matches!(operation, "create" | "join") {
        Mailbox::open(&home)?
    } else {
        Mailbox::open_existing(&home)?.ok_or_else(missing)?
    };
    let mut data = match operation {
        "create" => {
            store.create_team(create.as_ref().ok_or_else(|| input("missing team scope"))?)?
        }
        "join" => store.join_team(
            team.as_ref().ok_or_else(missing)?,
            join.as_ref()
                .ok_or_else(|| input("missing member guidance"))?,
            &fresh_key()?,
        )?,
        "show" => store.show_team(team.as_ref().ok_or_else(missing)?)?,
        "member" => store.team_member(
            team.as_ref().ok_or_else(missing)?,
            member.as_ref().ok_or_else(|| input("missing member"))?,
        )?,
        "members" => {
            store.team_members(team.as_ref().ok_or_else(missing)?, after.as_ref(), limit)?
        }
        _ => return Err(input("unknown team operation")),
    };
    data["schema"] = json!("merl.team/v1");
    data["action"] = json!(format!("team.{operation}"));
    if *json_output {
        render_json(&data)
    } else {
        human(&data, operation)
    }
}

fn read_brief(path: &str) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(MailError::from)?
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(MailError::from)?;
    String::from_utf8(bytes).map_err(|_| input("guidance must be UTF-8"))
}
fn fresh_key() -> Result<Key, CliError> {
    // The supported Linux/macOS release targets provide OS randomness here.
    // Fail closed rather than deriving identity keys from clocks or session names.
    let mut bytes = [0_u8; 16];
    File::open("/dev/urandom")
        .map_err(MailError::from)?
        .read_exact(&mut bytes)
        .map_err(MailError::from)?;
    let mut key = String::from("t-");
    for byte in bytes {
        write!(key, "{byte:02x}").map_err(|_| input("cannot format identity"))?;
    }
    Ok(key.parse()?)
}

fn human(data: &Value, operation: &str) -> Result<String, CliError> {
    let outcome = data["outcome"].as_str().unwrap_or("saved");
    let mut text = match operation {
        "create" | "show" => format!(
            "Team {} ({outcome}, generation {})\nCoordinator: {} (local convention)\nScope:\n{}\n",
            data["team"].as_str().unwrap_or(""),
            data["generation"],
            data["coordinator"].as_str().unwrap_or(""),
            data["scope"].as_str().unwrap_or("")
        ),
        "join" | "member" => format!(
            "Member {} ({outcome}, generation {})\nRole: {}\nSaved starting guidance:\n{}\nDelivery: inspect merl mail status --team {} --agent {}\n",
            data["qualified"].as_str().unwrap_or(""),
            data["generation"],
            data["role"].as_str().unwrap_or("Unspecified"),
            data["description"].as_str().unwrap_or(""),
            data["team"].as_str().unwrap_or(""),
            data["member"].as_str().unwrap_or("")
        ),
        "members" => {
            let mut text = format!("Team {}: members\n", data["team"].as_str().unwrap_or(""));
            if let Some(members) = data["members"].as_array() {
                for m in members {
                    writeln!(
                        text,
                        "  {}/{}  {}",
                        data["team"].as_str().unwrap_or(""),
                        m["member"].as_str().unwrap_or(""),
                        m["role"].as_str().unwrap_or("Unspecified")
                    )
                    .map_err(|_| input("cannot format member"))?;
                }
            }
            if let Some(after) = data["next_after"].as_str() {
                writeln!(text, "More members: --after {after}")
                    .map_err(|_| input("cannot format cursor"))?;
            }
            text
        }
        _ => return Err(input("unknown team output")),
    };
    if let Some(refs) = data["guidance_refs"].as_array() {
        for reference in refs {
            writeln!(
                text,
                "Guidance reference: {}",
                reference.as_str().unwrap_or("")
            )
            .map_err(|_| input("cannot format reference"))?;
        }
    }
    Ok(text)
}

const COMMANDS: &[&str] = &["create", "join", "show", "members", "member"];
fn contract(op: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    Some(match op {
        "create" => (
            "Save a shared team scope and coordinator convention; exact retries are unchanged.",
            &["--name", "--coordinator", "--scope-file", "--guidance-ref"],
            "--name alpha --coordinator planner --scope-file /tmp/scope.txt",
        ),
        "join" => (
            "Join with durable starting guidance and a fresh individual mail identity; changed inputs conflict. Does not bind a session.",
            &[
                "--team",
                "--member",
                "--role",
                "--description-file",
                "--guidance-ref",
            ],
            "--team alpha --member dev-a --role implementation --description-file /tmp/dev.txt",
        ),
        "show" => (
            "Read shared scope and inert guidance references.",
            &["--team"],
            "--team alpha",
        ),
        "members" => (
            "Page roster headers without loading each member's guidance.",
            &["--team", "--after", "--limit"],
            "--team alpha --limit 100",
        ),
        "member" => (
            "Read one member's saved starting guidance and stable qualified identity.",
            &["--team", "--member"],
            "--team alpha --member dev-a",
        ),
        _ => return None,
    })
}
fn help_record(op: &str, json_output: bool) -> Result<String, CliError> {
    let (summary, flags, example) = if op.is_empty() {
        (
            "Persistent shared scope and roster; work stays addressed to one member. Roles are labels, not permissions.",
            &[][..],
            "",
        )
    } else {
        contract(op).ok_or_else(|| input("unknown team help topic"))?
    };
    let command = if op.is_empty() {
        "team".into()
    } else {
        format!("team {op}")
    };
    let arguments = flags
        .iter()
        .map(|f| {
            if matches!(*f, "--role" | "--guidance-ref" | "--after" | "--limit") {
                format!("[{f} <value>]")
            } else {
                format!("{f} <value>")
            }
        })
        .collect::<Vec<_>>();
    let usage = format!(
        "merl {command} {} [--home <absolute-path>] [--config <absolute-path>] [--format human|json] [--json]",
        if op.is_empty() {
            "<operation>".into()
        } else {
            arguments.join(" ")
        }
    );
    let example = format!("merl {command} --home /tmp/team-mail {example} --json");
    let children = if op.is_empty() {
        COMMANDS
            .iter()
            .map(|c| format!("team {c}"))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let errors = [
        "TEAM_INPUT",
        "TEAM_NOT_FOUND",
        "TEAM_MEMBER_MISSING",
        "TEAM_CONFLICT",
        "TEAM_IDENTITY_CONFLICT",
        "TEAM_SCHEMA",
        "MAIL_INPUT",
        "MAIL_IO",
        "MAIL_STORAGE",
        "MAIL_SCHEMA",
        "INIT_INPUT",
        "INIT_CONFIG",
    ];
    let outcomes = ["created", "joined", "unchanged", "scope or roster metadata"];
    if json_output {
        render_json(
            &json!({"schema":"merl.help/v1","command":command,"kind":if op.is_empty(){"group"}else{"command"},"usage":usage,"summary":summary,"description":summary,"arguments":arguments,"children":children,"commands":if op.is_empty(){COMMANDS}else{&[]},"example":example,"examples":[example],"outcomes":outcomes,"errors":errors,"related":["team","mail send","mail status"]}),
        )
    } else {
        Ok(format!(
            "{usage}\n{summary}\nCommands: {}\nOutcomes: {}\nErrors: {}\nExample: {example}\nRelated: team, mail send, mail status\nReferences: repeat --guidance-ref, maximum 16; pointers are never opened.\n",
            children.join(", "),
            outcomes.join(", "),
            errors.join(", ")
        ))
    }
}
