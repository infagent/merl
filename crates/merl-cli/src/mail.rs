//! The public agent-mail command vocabulary and bounded help contracts.

use crate::{CliError, render_json};
use merl_mail::{Binding, Draft, Folder, Key, MAX_BODY_BYTES, MailError, Mailbox};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::Path,
    thread,
    time::Duration,
};

impl From<MailError> for CliError {
    fn from(e: MailError) -> Self {
        Self {
            code: e.code,
            message: e.to_string(),
        }
    }
}

pub(super) fn handles(a: &[String]) -> bool {
    a.first().is_some_and(|s| s == "mail")
        || (a.first().is_some_and(|s| s == "help") && a.get(1).is_some_and(|s| s == "mail"))
}

#[expect(
    clippy::too_many_lines,
    reason = "one bounded parser dispatches the mail command contract"
)]
pub(super) fn execute(
    a: &[String],
    json_output: &mut bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    *json_output = false;
    let mut options = BTreeMap::new();
    let mut words = Vec::new();
    let mut help = a[0] == "help";
    let mut i = if help { 2 } else { 1 };
    while i < a.len() {
        let arg = a[i].as_str();
        match arg {
            "--json" => *json_output = true,
            "--format" => {
                i += 1;
                match a.get(i).map(String::as_str) {
                    Some("json") => *json_output = true,
                    Some("human") => *json_output = false,
                    _ => return Err(input("format must be human or json")),
                }
            }
            "--unread" | "--pending" | "--outstanding" => {
                if options.insert(arg, "true").is_some() {
                    return Err(input("duplicate option"));
                }
            }
            "--help" | "-h" => help = true,
            "--home"
            | "--config"
            | "--team"
            | "--agent"
            | "--from"
            | "--to"
            | "--id"
            | "--kind"
            | "--body-file"
            | "--in-reply-to"
            | "--reply"
            | "--session"
            | "--socket"
            | "--expected-generation"
            | "--after"
            | "--limit"
            | "--codex" => {
                i += 1;
                let value = a.get(i).ok_or_else(|| input("option requires a value"))?;
                if options.insert(arg, value.as_str()).is_some() {
                    return Err(input("duplicate option"));
                }
            }
            flag if flag.starts_with('-') => return Err(input("unknown mail option")),
            word => words.push(word),
        }
        i += 1;
    }
    if words.len() > 1 {
        return Err(input("load one mail operation at a time"));
    }
    let operation = words.first().copied().unwrap_or("");
    if help || operation.is_empty() {
        return help_record(operation, *json_output);
    }
    let contract =
        contract(operation).ok_or_else(|| input("unknown mail operation; run merl help mail"))?;
    for key in options.keys() {
        if !(matches!(*key, "--home" | "--config" | "--json" | "--format")
            || (*key == "--team" && qualified_operation(operation))
            || contract.1.contains(key))
        {
            return Err(input("option does not apply to this mail operation"));
        }
    }
    let get = |key: &str| {
        options
            .get(key)
            .copied()
            .ok_or_else(|| input(&format!("missing {key}")))
    };
    let key = |name: &str| -> Result<Key, CliError> { Ok(get(name)?.parse()?) };
    let opt_key = |name: &str| -> Result<Option<Key>, CliError> {
        options
            .get(name)
            .map(|s| s.parse().map_err(CliError::from))
            .transpose()
    };
    let home = merl_setup::resolve_home(
        options.get("--home").copied(),
        options.get("--config").copied(),
    )?;
    let mut resolved = serde_json::Map::new();
    let mut identities = BTreeMap::new();
    let default_team: Option<Key> = options.get("--team").map(|s| s.parse()).transpose()?;
    let scoped = default_team.is_some()
        || (qualified_operation(operation)
            && ["--agent", "--from", "--to"]
                .iter()
                .any(|flag| options.get(flag).is_some_and(|s| s.contains('/'))));
    if scoped {
        let store = Mailbox::open_existing(&home)?.ok_or_else(crate::team::missing)?;
        if let Some(team) = &default_team {
            store.show_team(team)?;
        }
        for name in ["--agent", "--from", "--to"] {
            if let Some(value) = options.get(name) {
                let (team, member) = if let Some((team, member)) = value.split_once('/') {
                    (Some(team.parse::<Key>()?), member.parse::<Key>()?)
                } else {
                    (default_team.clone(), value.parse::<Key>()?)
                };
                let Some(team) = team else {
                    continue;
                };
                let record = store.team_member(&team, &member)?;
                let mail_key: Key = record["mail_key"]
                    .as_str()
                    .ok_or_else(|| input("member identity is missing"))?
                    .parse()?;
                resolved.insert(name.into(), json!({"team":team.as_str(),"member":member.as_str(),"qualified":format!("{team}/{member}"),"mail_key":mail_key.as_str()}));
                identities.insert(name, mail_key);
            }
        }
    }
    let actor = |name: &str| -> Result<Key, CliError> {
        identities.get(name).cloned().map_or_else(|| key(name), Ok)
    };
    let mut store = Mailbox::open(&home)?;
    let now = clock()?;
    let result = match operation {
        "register" => store.register(&key("--agent")?)?,
        "bind" => store.bind(
            &actor("--agent")?,
            &Binding::new(get("--session")?, Path::new(get("--socket")?))?,
            generation(get("--expected-generation")?)?,
            now,
        )?,
        "unbind" => store.unbind(
            &actor("--agent")?,
            generation(get("--expected-generation")?)?,
        )?,
        "send" => {
            let mut body = Vec::new();
            File::open(get("--body-file")?)
                .map_err(MailError::from)?
                .take((MAX_BODY_BYTES + 1) as u64)
                .read_to_end(&mut body)
                .map_err(MailError::from)?;
            let body = String::from_utf8(body).map_err(|_| input("body must be UTF-8"))?;
            let draft = Draft::new(
                key("--id")?,
                actor("--from")?,
                actor("--to")?,
                get("--kind")?,
                body,
                opt_key("--in-reply-to")?,
            )?;
            store.send(&draft, now)?
        }
        "list" => store.list(
            &actor("--agent")?,
            opt_key("--after")?.as_ref(),
            options
                .get("--limit")
                .unwrap_or(&"100")
                .parse()
                .map_err(|_| input("limit must be 1..100"))?,
            {
                let chosen: Vec<_> = [
                    ("--unread", Folder::Unread),
                    ("--pending", Folder::Pending),
                    ("--outstanding", Folder::Outstanding),
                ]
                .into_iter()
                .filter(|(flag, _)| options.contains_key(flag))
                .collect();
                if chosen.len() > 1 {
                    return Err(input("choose one mail folder filter"));
                }
                chosen.first().map_or(Folder::All, |(_, folder)| *folder)
            },
        )?,
        "show" | "read" => {
            store.show(&actor("--agent")?, &key("--id")?, operation == "read", now)?
        }
        "receipt" => store.receipt(&actor("--agent")?, &key("--id")?, now)?,
        "complete" => store.complete(
            &actor("--agent")?,
            &key("--id")?,
            opt_key("--reply")?.as_ref(),
            now,
        )?,
        "status" => store.status(&actor("--agent")?)?,
        "dispatch" => store.dispatch(
            Path::new(get("--codex")?),
            &std::env::current_exe().map_err(MailError::from)?,
            now,
        )?,
        "watch" => {
            let _guard = store.watch_guard()?;
            let program = Path::new(get("--codex")?);
            let merl = std::env::current_exe().map_err(MailError::from)?;
            let stdout = std::io::stdout();
            let mut stdout = stdout.lock();
            stdout.write_all(render(json!({"outcome":"watching","stop":"Ctrl-C or SIGTERM this foreground process"}),operation,*json_output)?.as_bytes()).map_err(MailError::from)?;
            stdout.flush().map_err(MailError::from)?;
            let mut waiting = false;
            loop {
                let result = match store.dispatch(program, &merl, clock()?) {
                    Ok(result) => {
                        if waiting {
                            stdout
                                .write_all(
                                    render(json!({"outcome":"resumed"}), operation, *json_output)?
                                        .as_bytes(),
                                )
                                .map_err(MailError::from)?;
                            stdout.flush().map_err(MailError::from)?;
                            waiting = false;
                        }
                        result
                    }
                    Err(e) if e.code == "MAIL_BUSY" => {
                        if !waiting {
                            stdout
                                .write_all(
                                    render(
                                        json!({"outcome":"waiting","reason":"MAIL_BUSY"}),
                                        operation,
                                        *json_output,
                                    )?
                                    .as_bytes(),
                                )
                                .map_err(MailError::from)?;
                            stdout.flush().map_err(MailError::from)?;
                            waiting = true;
                        }
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                    Err(e) => return Err(e.into()),
                };
                if result["notifications"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                {
                    stdout
                        .write_all(render(result, operation, *json_output)?.as_bytes())
                        .map_err(MailError::from)?;
                    stdout.flush().map_err(MailError::from)?;
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
        _ => return Err(input("unknown operation")),
    };
    let mut result = result;
    if !resolved.is_empty() {
        if *json_output {
            result["resolved"] = Value::Object(resolved);
        } else {
            qualify_human(&mut result, &store)?;
        }
    }
    render(result, operation, *json_output)
}

fn qualified_operation(op: &str) -> bool {
    matches!(
        op,
        "bind" | "unbind" | "send" | "list" | "show" | "read" | "receipt" | "complete" | "status"
    )
}

fn qualify_human(value: &mut Value, store: &Mailbox) -> Result<(), CliError> {
    match value {
        Value::Object(object) => {
            for (field, value) in object {
                if matches!(field.as_str(), "agent" | "from" | "to") {
                    if let Some(key) = value.as_str()
                        && let Some(qualified) = store.qualified_member(&key.parse()?)?
                    {
                        *value = json!(qualified);
                    }
                } else if matches!(field.as_str(), "messages" | "message") {
                    qualify_human(value, store)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                qualify_human(value, store)?;
            }
        }
        _ => (),
    }
    Ok(())
}

fn input(s: &str) -> CliError {
    CliError {
        code: "MAIL_INPUT",
        message: s.into(),
    }
}
fn generation(s: &str) -> Result<i64, CliError> {
    s.parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
        .ok_or_else(|| input("generation must be a nonnegative integer"))
}
fn render(mut data: Value, operation: &str, json_output: bool) -> Result<String, CliError> {
    data["schema"] = json!("merl.mail/v1");
    data["action"] = json!(format!("mail.{operation}"));
    if json_output {
        render_json(&data)
    } else {
        serde_json::to_string_pretty(&data)
            .map(|s| format!("{s}\n"))
            .map_err(|_| input("cannot render mail record"))
    }
}

const COMMANDS: &[&str] = &[
    "register", "bind", "unbind", "send", "list", "show", "read", "receipt", "complete", "status",
    "dispatch", "watch",
];
fn contract(op: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    Some(match op {
        "register" => (
            "Register an explicit durable identity without a session.",
            &["--agent"],
            "--agent alice",
        ),
        "bind" => (
            "Explicitly bind a name to an existing session UUID and local socket; inspect status generation first.",
            &["--agent", "--session", "--socket", "--expected-generation"],
            "--agent alice --session 00000000-0000-4000-8000-000000000001 --socket /tmp/codex.sock --expected-generation 0",
        ),
        "unbind" => (
            "Disable notification for one explicit binding without deleting mail.",
            &["--agent", "--expected-generation"],
            "--agent alice --expected-generation 1",
        ),
        "send" => (
            "Atomically publish immutable UTF-8 mail and notification intent. Local sender claims are trusted, not authentication. Labels: assign, ready, review, blocked, question, reply, note; maximum body 8192 bytes.",
            &[
                "--from",
                "--to",
                "--id",
                "--kind",
                "--body-file",
                "--in-reply-to",
            ],
            "--from alice --to bob --id handoff-1 --kind assign --body-file /tmp/handoff.txt",
        ),
        "list" => (
            "Read bounded recipient headers; follow next_after until null. No body or read-state mutation.",
            &[
                "--agent",
                "--limit",
                "--after",
                "--unread",
                "--pending",
                "--outstanding",
            ],
            "--agent bob --pending --limit 100",
        ),
        "show" => (
            "Inspect one recipient's full message without marking it read.",
            &["--agent", "--id"],
            "--agent bob --id handoff-1",
        ),
        "read" => (
            "Read one message and mark it read; receipt and completion remain separate.",
            &["--agent", "--id"],
            "--agent bob --id handoff-1",
        ),
        "receipt" => (
            "Record application receipt, stop notification retries for this ID, and create no acknowledgment loop. Does not complete work.",
            &["--agent", "--id"],
            "--agent bob --id handoff-1",
        ),
        "complete" => (
            "Explicitly complete one addressed coordination request; an optional reply must match this conversation.",
            &["--agent", "--id", "--reply"],
            "--agent bob --id handoff-1 --reply response-1",
        ),
        "status" => (
            "Inspect binding generation, unread, unreceipted, outstanding and last notification independently.",
            &["--agent"],
            "--agent bob",
        ),
        "dispatch" => (
            "Attempt due notifications once with an absolute Codex executable; queue exit success is host enqueue only. No fallback session is created.",
            &["--codex"],
            "--codex /usr/local/bin/codex",
        ),
        "watch" => (
            "Run an opt-in foreground dispatcher; waiting makes no model calls. Stop only this process with Ctrl-C or SIGTERM. No system service is installed.",
            &["--codex"],
            "--codex /usr/local/bin/codex",
        ),
        _ => return None,
    })
}
fn describe_actors(summary: &str, op: &str) -> String {
    if qualified_operation(op) {
        format!(
            "{summary} Actors accept scope/member independently; --team supplies the scope only for bare names. Other bare actors use legacy identities. Unknown qualified actors never fall back or join implicitly."
        )
    } else {
        summary.to_owned()
    }
}

fn help_outcomes(op: &str) -> Vec<&'static str> {
    match op {
        "register" => vec!["registered"],
        "bind" => vec!["bound"],
        "unbind" => vec!["unbound"],
        "send" => vec!["sent", "unchanged"],
        "receipt" => vec!["received"],
        "complete" => vec!["completed"],
        "dispatch" => vec![
            "pending",
            "offline",
            "failed",
            "stale_binding",
            "enqueued",
            "pointer_limit",
        ],
        "watch" => vec![
            "watching",
            "waiting",
            "resumed",
            "recorded notification states",
        ],
        _ => vec!["mail headers, body or handling status"],
    }
}

fn help_record(op: &str, json_output: bool) -> Result<String, CliError> {
    let command = if op.is_empty() {
        "mail".into()
    } else {
        format!("mail {op}")
    };
    let (summary, flags, example) = if op.is_empty() {
        (
            "Durable local agent mail; transport receipt and explicit work completion are independent. Same-user local identity claims are trusted.",
            &[][..],
            "",
        )
    } else {
        contract(op).ok_or_else(|| input("unknown mail help topic"))?
    };
    let summary = describe_actors(summary, op);
    let mut arguments: Vec<String> = flags
        .iter()
        .map(|flag| {
            let value = match *flag {
                "--unread" | "--pending" | "--outstanding" => "",
                "--body-file" | "--codex" | "--socket" => " <path>",
                "--limit" | "--expected-generation" => " <integer>",
                _ => " <value>",
            };
            let optional = matches!(
                *flag,
                "--after"
                    | "--limit"
                    | "--unread"
                    | "--pending"
                    | "--outstanding"
                    | "--reply"
                    | "--in-reply-to"
            );
            if optional {
                format!("[{flag}{value}]")
            } else {
                format!("{flag}{value}")
            }
        })
        .collect();
    if qualified_operation(op) {
        arguments.push("[--team <name>]".into());
    }
    let usage = format!(
        "merl {command}{} [--home <absolute-path>] [--config <absolute-path>] [--format human|json] [--json]",
        if op.is_empty() {
            " <operation>".into()
        } else {
            format!(" {}", arguments.join(" "))
        }
    );
    let example =
        (!op.is_empty()).then(|| format!("merl {command} --home /tmp/team-mail {example} --json"));
    let children = if op.is_empty() {
        COMMANDS
            .iter()
            .map(|c| format!("mail {c}"))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let errors = help_errors(op);
    let outcomes = help_outcomes(op);
    let related = vec!["mail list", "mail status"];
    if json_output {
        render_json(
            &json!({"schema":"merl.help/v1","command":command,"kind":if op.is_empty(){"group"}else{"command"},"usage":usage,"summary":summary,"description":summary,"example":example,"examples":example.iter().collect::<Vec<_>>(),"arguments":arguments,"outcomes":outcomes,"errors":errors,"related":related,"children":children,"commands":if op.is_empty(){COMMANDS}else{&[]}}),
        )
    } else {
        Ok(format!(
            "{usage}\n{summary}\nCommands: {}\nOutcomes: {}\nErrors: {}\nExample: {}\nRelated: {}\n",
            children.join(", "),
            outcomes.join(", "),
            errors.join(", "),
            example.unwrap_or_default(),
            related.join(", ")
        ))
    }
}

fn help_errors(op: &str) -> Vec<&'static str> {
    let mut errors = vec![
        "MAIL_INPUT",
        "MAIL_IO",
        "MAIL_STORAGE",
        "MAIL_SCHEMA",
        "INIT_INPUT",
        "INIT_CONFIG",
        "INIT_CONFLICT",
    ];
    if qualified_operation(op) {
        errors.extend(["TEAM_NOT_FOUND", "TEAM_MEMBER_MISSING", "TEAM_SCHEMA"]);
    }
    match op {
        "bind" => errors.extend([
            "MAIL_BUSY",
            "MAIL_IDENTITY_MISSING",
            "MAIL_BINDING_CONFLICT",
            "MAIL_HOST_UNAVAILABLE",
        ]),
        "unbind" => errors.extend([
            "MAIL_BUSY",
            "MAIL_IDENTITY_MISSING",
            "MAIL_BINDING_CONFLICT",
        ]),
        "send" => errors.extend([
            "MAIL_IDENTITY_MISSING",
            "MAIL_ID_CONFLICT",
            "MAIL_REPLY_MISMATCH",
        ]),
        "list" | "status" => errors.push("MAIL_IDENTITY_MISSING"),
        "show" | "read" | "receipt" => errors.extend(["MAIL_NOT_FOUND", "MAIL_NOT_RECIPIENT"]),
        "complete" => errors.extend([
            "MAIL_NOT_FOUND",
            "MAIL_NOT_RECIPIENT",
            "MAIL_REPLY_MISMATCH",
            "MAIL_ID_CONFLICT",
        ]),
        "dispatch" | "watch" => errors.push("MAIL_BUSY"),
        _ => (),
    }
    errors
}
