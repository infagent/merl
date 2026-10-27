//! A pure first-use primer; detailed contracts remain incremental CLI help.

use crate::{CliError, render_json};
use serde_json::json;

pub(super) fn handles(args: &[String]) -> bool {
    args.first().is_some_and(|s| s == "usage")
        || (args.first().is_some_and(|s| s == "help") && args.get(1).is_some_and(|s| s == "usage"))
}

const PRIMER: &str = "Merl: start an existing agent and hand work to one owner.\n\n\
1. Select access\n\
   merl init                         Optional user setup; review before applying.\n\
   Use an explicit --home or your selected saved mailbox default.\n\n\
2. Choose a scope and identity\n\
   merl team list                    Page scopes; --project NAME filters exact references.\n\
   merl team members --team SCOPE     Discover members; roles are optional labels.\n\
   merl team member --team SCOPE --member NAME\n\
   Resume saved guidance. Ask only for missing access, scope or identity.\n\n\
3. Join only if new\n\
   merl team join --team SCOPE --member NAME --description-file PATH\n\
   Supply starting guidance once; changed saved inputs conflict.\n\
   Create shared scope if needed: merl help team create\n\n\
4. Hand off and resume work\n\
   merl mail send --from SCOPE/NAME --to OTHER/MEMBER --id ID --kind assign --body-file PATH\n\
   merl mail status --agent SCOPE/NAME\n\
   merl mail list --agent SCOPE/NAME --pending\n\
   merl mail list --agent SCOPE/NAME --outstanding\n\
   Drain pending pages, read each ID, persist a checkpoint, then receipt.\n\
   Receipt and replies do not complete work; use explicit mail complete.\n\n\
5. Optional host delivery\n\
   merl help mail bind               Existing UUID/socket and current generation required.\n\
   Joining/resuming never replaces a binding or starts a dispatcher.\n\
   Codex wake is verified; other hosts can use the CLI without claimed wake.\n\n\
Skill startup: $merl SCOPE/NAME starting guidance, or /merl in Claude.\n\
Load details only when needed: merl help GROUP OPERATION --json\n";

pub(super) fn execute(args: &[String], json_output: &mut bool) -> Result<String, CliError> {
    let mut help = args[0] == "help";
    let mut i = if help { 2 } else { 1 };
    while i < args.len() {
        match args[i].as_str() {
            "--json" => *json_output = true,
            "--help" | "-h" => help = true,
            "--format" => {
                i += 1;
                *json_output = match args.get(i).map(String::as_str) {
                    Some("human") => false,
                    Some("json") => true,
                    _ => return Err(input("format must be human or json")),
                };
            }
            _ => {
                return Err(input(
                    "usage reads no setup or runtime; run merl help usage",
                ));
            }
        }
        i += 1;
    }
    if !*json_output {
        return Ok(if help {
            "merl usage [--format human|json] [--json]\nRead a short onboarding primer without accessing user setup or runtime.\nOutcomes: primer\nErrors: USAGE_INPUT\nExample: merl usage --json\nRelated: team, mail, init\n".into()
        } else {
            PRIMER.into()
        });
    }
    if help {
        render_json(
            &json!({"schema":"merl.help/v1","command":"usage","kind":"command","usage":"merl usage [--format human|json] [--json]","summary":"Read a short onboarding primer without accessing user setup or runtime.","description":"Pure documentation; load one command contract when needed.","children":[],"arguments":["--format human|json","--json"],"outcomes":["primer"],"errors":["USAGE_INPUT"],"related":["team","mail","init"],"example":"merl usage --json","examples":["merl usage --json"]}),
        )
    } else {
        render_json(
            &json!({"schema":"merl.usage/v1","action":"usage","text":PRIMER,"related":["init","team list","team join","team member","mail send","mail list","mail bind"]}),
        )
    }
}

fn input(message: &str) -> CliError {
    CliError {
        code: "USAGE_INPUT",
        message: message.into(),
    }
}
