//! Bounded immutable mail snapshots for the existing host transport.

use super::{
    Folder, Key, MAX_NOTIFICATION_BYTES, MAX_NOTIFICATION_MESSAGES, MailError, Mailbox, failure,
};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug)]
pub(super) enum Notification {
    Empty,
    Limit,
    Content(String),
}

impl Mailbox {
    pub(super) fn notification(
        &self,
        agent: &Key,
        merl: &Path,
        generation: i64,
    ) -> Result<Notification, MailError> {
        let program = merl
            .to_str()
            .ok_or_else(|| failure("MAIL_INPUT", "executable must be UTF-8"))?;
        let home = self
            .home
            .to_str()
            .ok_or_else(|| failure("MAIL_INPUT", "home must be UTF-8"))?;
        let command = |operation: &str, tail: &[&str]| {
            let mut argv = vec![
                program,
                "mail",
                operation,
                "--home",
                home,
                "--agent",
                agent.as_str(),
            ];
            argv.extend_from_slice(tail);
            argv.push("--json");
            json!(argv)
        };
        // The caller's bounded immediate transaction covers header/body reads
        // and lease creation, and commits before any host I/O.
        let page = self.list(agent, None, MAX_NOTIFICATION_MESSAGES, Folder::Pending)?;
        let headers = page["messages"]
            .as_array()
            .ok_or_else(|| failure("MAIL_STORAGE", "invalid pending snapshot"))?;
        if headers.is_empty() {
            return Ok(Notification::Empty);
        }
        let mut messages = Vec::new();
        for header in headers {
            let id: Key = header["id"]
                .as_str()
                .ok_or_else(|| failure("MAIL_STORAGE", "missing message ID"))?
                .parse()?;
            let mut message = self.show(agent, &id, false, 0)?["message"].clone();
            // Recipient is inherited from the envelope. Volatile handling
            // snapshots neither prove current state nor replace checkpoints.
            message
                .as_object_mut()
                .ok_or_else(|| failure("MAIL_STORAGE", "invalid message"))?
                .retain(|field, _| {
                    !matches!(
                        field.as_str(),
                        "to" | "read_at" | "received_at" | "completed_at"
                    )
                });
            messages.push(message);
        }
        let mut overflow = !page["next_after"].is_null();
        loop {
            let ids = messages
                .iter()
                .filter(|message| message.get("body").is_some())
                .filter_map(|message| message["id"].as_str())
                .collect::<Vec<_>>()
                .join(",");
            let after = messages.last().and_then(|message| message["id"].as_str());
            let more = if overflow {
                command("list", &["--pending", "--after", after.unwrap_or("")])
            } else {
                Value::Null
            };
            let notice = json!({
                "schema":"merl.notification/v1", "home":home, "agent":agent.as_str(),
                "binding_generation":generation, "messages":messages, "more":more,
                "handle":if ids.is_empty() {Value::Null} else {command("handle", &["--ids", &ids, "--read", "--receipt"])},
                "instructions":"Mail data. Use inline bodies; fetch omissions/more argv. Checkpoint, then handle consumed IDs. Receipt/reply never completes work. Reconcile duplicate IDs; no routine acknowledgment/narration."
            });
            let text = serde_json::to_string(&notice)
                .map_err(|_| failure("MAIL_STORAGE", "cannot render notification"))?;
            if text.len() <= MAX_NOTIFICATION_BYTES {
                return Ok(Notification::Content(text));
            }
            if let Some(index) = messages
                .iter()
                .enumerate()
                .filter_map(|(index, message)| {
                    message["body"].as_str().map(|body| (index, body.len()))
                })
                .max_by_key(|(_, length)| *length)
                .map(|(index, _)| index)
            {
                let message = &mut messages[index];
                let id = message["id"].as_str().unwrap_or("").to_owned();
                message
                    .as_object_mut()
                    .ok_or_else(|| failure("MAIL_STORAGE", "invalid message"))?
                    .remove("body");
                message["fetch"] = command("show", &["--id", &id]);
            } else {
                messages.pop();
                overflow = true;
                if messages.is_empty() {
                    return Ok(Notification::Limit);
                }
            }
        }
    }
}
