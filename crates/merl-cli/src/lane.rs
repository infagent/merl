//! Named reviewer-lane configuration, bounded scans and the combined foreground watch.

use crate::{CliError, render_json};
use merl_lane::{Configuration, Gh, Labels, LaneError, Name, Store};
use merl_mail::{MailError, Mailbox};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write, path::Path, thread, time::Duration};

impl From<LaneError> for CliError {
    fn from(error: LaneError) -> Self {
        Self {
            code: error.code,
            message: error.to_string(),
        }
    }
}

pub(super) fn handles(args: &[String]) -> bool {
    args.first().is_some_and(|s| s == "lane")
        || (args.first().is_some_and(|s| s == "help") && args.get(1).is_some_and(|s| s == "lane"))
}

struct Arguments<'a> {
    operation: &'a str,
    values: BTreeMap<&'a str, &'a str>,
    labels: Vec<String>,
    help: bool,
}
impl Arguments<'_> {
    fn get(&self, key: &str) -> Result<&str, CliError> {
        self.values
            .get(key)
            .copied()
            .ok_or_else(|| input(&format!("missing {key}")))
    }
    fn generation(&self) -> Result<Option<i64>, CliError> {
        self.values
            .get("--expected-generation")
            .map(|s| {
                s.parse::<i64>()
                    .ok()
                    .filter(|g| *g >= 0)
                    .ok_or_else(|| input("generation must be nonnegative"))
            })
            .transpose()
    }
    fn name(&self) -> Result<Name, CliError> {
        Ok(self.get("--name")?.parse()?)
    }
}

fn parse<'a>(args: &'a [String], json_output: &mut bool) -> Result<Arguments<'a>, CliError> {
    let mut result = Arguments {
        operation: "",
        values: BTreeMap::new(),
        labels: Vec::new(),
        help: args[0] == "help",
    };
    let mut index = if result.help { 2 } else { 1 };
    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--json" => *json_output = true,
            "--format" => {
                index += 1;
                match args.get(index).map(String::as_str) {
                    Some("json") => *json_output = true,
                    Some("human") => *json_output = false,
                    _ => return Err(input("format must be human or json")),
                }
            }
            "--help" | "-h" => result.help = true,
            "--home"
            | "--config"
            | "--name"
            | "--repo"
            | "--agent"
            | "--label"
            | "--expected-generation"
            | "--gh"
            | "--codex"
            | "--max-requests"
            | "--after"
            | "--limit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| input("option requires a value"))?
                    .as_str();
                if argument == "--label" {
                    result.labels.push(value.into());
                } else if result.values.insert(argument, value).is_some() {
                    return Err(input("duplicate option"));
                }
            }
            flag if flag.starts_with('-') => return Err(input("unknown lane option")),
            word if result.operation.is_empty() => result.operation = word,
            _ => return Err(input("load one lane operation at a time")),
        }
        index += 1;
    }
    Ok(result)
}

pub(super) fn execute(
    args: &[String],
    json_output: &mut bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    let args = parse(args, json_output)?;
    if args.help || args.operation.is_empty() {
        return help(args.operation, *json_output);
    }
    let (_, flags, _) = contract(args.operation)
        .ok_or_else(|| input("unknown lane operation; run merl help lane"))?;
    if args
        .values
        .keys()
        .any(|k| !matches!(*k, "--home" | "--config") && !flags.contains(k))
        || (!args.labels.is_empty() && !flags.contains(&"--label"))
    {
        return Err(input("option does not apply to this lane operation"));
    }
    let home = merl_setup::resolve_home(
        args.values.get("--home").copied(),
        args.values.get("--config").copied(),
    )?;
    let now = clock()?;
    let mut store = Store::open(&home)?;
    let value = match args.operation {
        "start" => start(&mut store, &args, now)?,
        "update" => store.update(
            &args.name()?,
            &Labels::new(args.labels.clone())?,
            args.generation()?
                .ok_or_else(|| input("missing --expected-generation"))?,
            now,
        )?,
        "stop" => store.stop(
            &args.name()?,
            args.generation()?
                .ok_or_else(|| input("missing --expected-generation"))?,
        )?,
        "status" => store.status(
            args.values
                .get("--name")
                .map(|s| s.parse::<Name>())
                .transpose()?
                .as_ref(),
        )?,
        "queue" => store.queue(
            &args.name()?,
            args.values
                .get("--after")
                .unwrap_or(&"0")
                .parse()
                .map_err(|_| input("after must be a nonnegative PR number"))?,
            args.values
                .get("--limit")
                .unwrap_or(&"100")
                .parse()
                .map_err(|_| input("limit must be 1..100"))?,
        )?,
        "scan" | "watch" => {
            let mut github = Gh::new(Path::new(args.get("--gh")?), store.home())?;
            let mut mailbox = Mailbox::open(&home)?;
            let budget = args
                .values
                .get("--max-requests")
                .unwrap_or(&"16")
                .parse()
                .map_err(|_| input("max requests must be 1..16"))?;
            if args.operation == "watch" {
                return watch(
                    &mut store,
                    &mut github,
                    &mut mailbox,
                    Path::new(args.get("--codex")?),
                    budget,
                    *json_output,
                    clock,
                );
            }
            store.scan(
                &mut github,
                &mut mailbox,
                &|| clock().map_err(|_| LaneError::clock_unavailable()),
                budget,
            )?
        }
        _ => return Err(input("unknown lane operation")),
    };
    render(value, args.operation, *json_output)
}

fn start(store: &mut Store, args: &Arguments<'_>, now: i64) -> Result<Value, CliError> {
    let config = if args.values.contains_key("--repo")
        || args.values.contains_key("--agent")
        || !args.labels.is_empty()
    {
        let config = Configuration {
            repository: args.get("--repo")?.parse()?,
            agent: args.get("--agent")?.parse()?,
            labels: Labels::new(args.labels.clone())?,
        };
        Mailbox::open(store.home())?.status(&config.agent)?;
        Some(config)
    } else {
        None
    };
    Ok(store.start(&args.name()?, config, args.generation()?, now)?)
}

fn watch(
    store: &mut Store,
    github: &mut Gh,
    mail: &mut Mailbox,
    codex: &Path,
    budget: usize,
    json_output: bool,
    clock: &impl Fn() -> Result<i64, CliError>,
) -> Result<String, CliError> {
    if !codex.is_absolute() || !(1..=16).contains(&budget) {
        return Err(input(
            "use an absolute Codex executable and max requests 1..16",
        ));
    }
    let _guard = mail.watch_guard()?;
    let executable = std::env::current_exe().map_err(MailError::from)?;
    let mut stdout = std::io::stdout().lock();
    emit(
        &mut stdout,
        json!({"outcome":"watching","stop":"Ctrl-C or SIGTERM this foreground process"}),
        json_output,
    )?;
    let mut last_error = None;
    loop {
        match store.scan(
            github,
            mail,
            &|| clock().map_err(|_| LaneError::clock_unavailable()),
            budget,
        ) {
            Ok(value) => {
                if value["requests"].as_u64().is_some_and(|n| n > 0)
                    || value["published_batches"].as_u64().is_some_and(|n| n > 0)
                {
                    emit(&mut stdout, value, json_output)?;
                    last_error = None;
                }
            }
            Err(error)
                if matches!(
                    error.code,
                    "LANE_STORAGE" | "LANE_SCHEMA" | "LANE_IO" | "LANE_CLOCK"
                ) =>
            {
                return Err(error.into());
            }
            Err(error) => {
                if last_error != Some(error.code) {
                    emit(
                        &mut stdout,
                        json!({"outcome":"waiting","code":error.code,"retry_at":error.retry_at}),
                        json_output,
                    )?;
                    last_error = Some(error.code);
                }
            }
        }
        match mail.dispatch(codex, &executable, clock()?) {
            Ok(value)
                if value["notifications"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()) =>
            {
                emit(
                    &mut stdout,
                    json!({"outcome":"dispatch","notifications":value["notifications"]}),
                    json_output,
                )?;
            }
            Ok(_) => (),
            Err(error) if error.code == "MAIL_BUSY" => (),
            Err(error) => return Err(error.into()),
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn emit(stdout: &mut impl Write, value: Value, json_output: bool) -> Result<(), CliError> {
    stdout
        .write_all(render(value, "watch", json_output)?.as_bytes())
        .map_err(LaneError::from)?;
    stdout.flush().map_err(LaneError::from)?;
    Ok(())
}
fn input(message: &str) -> CliError {
    CliError {
        code: "LANE_INPUT",
        message: message.into(),
    }
}
fn render(mut value: Value, operation: &str, json_output: bool) -> Result<String, CliError> {
    value["schema"] = json!("merl.lane/v1");
    value["action"] = json!(format!("lane.{operation}"));
    if json_output {
        render_json(&value)
    } else {
        serde_json::to_string_pretty(&value)
            .map(|s| format!("{s}\n"))
            .map_err(|_| input("cannot render lane record"))
    }
}

const COMMANDS: &[&str] = &[
    "start", "update", "stop", "status", "queue", "scan", "watch",
];
fn contract(operation: &str) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    Some(match operation {
        "start" => (
            "Create a durable repository/required-label-AND subscription, or resume by name and expected generation. Recipient session binding remains separate.",
            &[
                "--name",
                "--repo",
                "--agent",
                "--label",
                "--expected-generation",
            ],
            "--name backend --repo example/widgets --agent reviewer --label review-ready --label backend",
        ),
        "update" => (
            "Replace required labels under a generation guard; preserve still-matching PR cursors and all durable delivery intent.",
            &["--name", "--label", "--expected-generation"],
            "--name backend --expected-generation 1 --label review-ready --label backend",
        ),
        "stop" => (
            "Pause polling and unclaimed publication without discarding checkpoints. In-flight or published mail still follows normal handling.",
            &["--name", "--expected-generation"],
            "--name backend --expected-generation 1",
        ),
        "status" => (
            "Inspect bounded lane health, last complete discovery, initializations, failures and provider cooldown. Use queue for independent PR progress.",
            &["--name"],
            "--name backend",
        ),
        "queue" => (
            "Page retained PR snapshots and independent cursors, including scope exits, terminal state and resynchronization gaps.",
            &["--name", "--after", "--limit"],
            "--name backend --limit 100",
        ),
        "scan" => (
            "Perform one fair bounded read-only GitHub pass with an absolute authenticated gh executable and retry frozen Merl mail. No host wake by this command.",
            &["--gh", "--max-requests"],
            "--gh /usr/local/bin/gh --max-requests 16",
        ),
        "watch" => (
            "Poll due lanes and reuse mail dispatch in one opt-in foreground process. Shares the mail watch lock; no idle model calls, GitHub writes or session fallback.",
            &["--gh", "--codex", "--max-requests"],
            "--gh /usr/local/bin/gh --codex /usr/local/bin/codex",
        ),
        _ => return None,
    })
}
fn help(operation: &str, json_output: bool) -> Result<String, CliError> {
    let command = if operation.is_empty() {
        "lane".to_owned()
    } else {
        format!("lane {operation}")
    };
    let (summary, flags, example) = if operation.is_empty() {
        (
            "Durable named reviewer lanes discover and follow many PRs; session binding, receipt and review completion remain separate.",
            &[][..],
            "",
        )
    } else {
        contract(operation).ok_or_else(|| input("unknown lane help topic"))?
    };
    let children = if operation.is_empty() {
        COMMANDS
            .iter()
            .map(|op| format!("lane {op}"))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let arguments = flags
        .iter()
        .map(|flag| argument(operation, flag))
        .collect::<Vec<_>>();
    let usage = format!(
        "merl {command}{} [--home <absolute-path>] [--config <absolute-path>] [--format human|json] [--json]",
        if operation.is_empty() {
            " <operation>".to_owned()
        } else {
            format!(" {}", arguments.join(" "))
        }
    );
    let example = if operation.is_empty() {
        None
    } else {
        Some(format!(
            "merl {command} --home /tmp/team-mail {example} --json"
        ))
    };
    let errors = errors(operation);
    let outcomes = vec!["durable lane configuration or explicit scan/progress/delivery status"];
    let related = vec!["lane status", "lane queue", "mail list", "mail status"];
    if json_output {
        render_json(
            &json!({"schema":"merl.help/v1","command":command,"kind":if operation.is_empty(){"group"}else{"command"},"usage":usage,"summary":summary,"description":summary,"arguments":arguments,"example":example,"examples":example.iter().collect::<Vec<_>>(),"errors":errors,"outcomes":outcomes,"related":related,"children":children,"commands":if operation.is_empty(){COMMANDS}else{&[]}}),
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
fn argument(operation: &str, flag: &str) -> String {
    let value = match flag {
        "--gh" | "--codex" => "<path>",
        "--expected-generation" | "--max-requests" | "--after" | "--limit" => "<integer>",
        _ => "<value>",
    };
    let optional = matches!(flag, "--max-requests" | "--after" | "--limit")
        || (operation == "status" && flag == "--name")
        || (operation == "start" && flag != "--name");
    if optional {
        format!("[{flag} {value}]")
    } else {
        format!("{flag} {value}")
    }
}
fn errors(operation: &str) -> Vec<&'static str> {
    let mut errors = vec![
        "LANE_INPUT",
        "LANE_IO",
        "LANE_STORAGE",
        "LANE_SCHEMA",
        "INIT_INPUT",
        "INIT_CONFIG",
        "INIT_CONFLICT",
    ];
    match operation {
        "start" => errors.extend([
            "LANE_CONFLICT",
            "LANE_LIMIT",
            "MAIL_INPUT",
            "MAIL_IO",
            "MAIL_STORAGE",
            "MAIL_SCHEMA",
            "MAIL_IDENTITY_MISSING",
        ]),
        "update" | "stop" => errors.push("LANE_CONFLICT"),
        "status" | "queue" => errors.push("LANE_NOT_FOUND"),
        "scan" | "watch" => errors.extend([
            "LANE_BUSY",
            "LANE_CLOCK",
            "LANE_GITHUB",
            "LANE_GITHUB_PROTOCOL",
            "LANE_RATE_LIMITED",
            "LANE_CURSOR_GAP",
            "LANE_INCOMPLETE",
            "LANE_SOURCE_UNAVAILABLE",
            "LANE_DELIVERY",
            "LANE_LIMIT",
            "MAIL_IO",
            "MAIL_STORAGE",
            "MAIL_SCHEMA",
            "MAIL_INPUT",
            "MAIL_BUSY",
        ]),
        _ => (),
    }
    errors
}
