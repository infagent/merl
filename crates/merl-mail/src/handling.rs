//! Explicit bounded handling of content already consumed from an inline notice.

use super::{Key, MAX_PAGE, MailError, Mailbox, failure, require_recipient, validate_completion};
use rusqlite::{TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Explicit handling facts to record atomically for a selected set of messages.
#[derive(Debug)]
pub struct Handling {
    read: bool,
    receipt: bool,
    complete: bool,
    reply: Option<Key>,
}

impl Handling {
    /// Selects independent read, receipt and completion intents.
    ///
    /// A reply links completion to an already published response; it publishes
    /// nothing and never implicitly selects completion.
    ///
    /// # Errors
    /// Rejects an empty selection or a reply without explicit completion.
    pub fn new(
        read: bool,
        receipt: bool,
        complete: bool,
        reply: Option<Key>,
    ) -> Result<Self, MailError> {
        if !(read || receipt || complete) || (reply.is_some() && !complete) {
            return Err(failure(
                "MAIL_INPUT",
                "choose --read, --receipt or --complete; --reply requires --complete",
            ));
        }
        Ok(Self {
            read,
            receipt,
            complete,
            reply,
        })
    }
}

impl Mailbox {
    /// Records selected facts for 1..100 unique addressed IDs in one transaction.
    ///
    /// Content must already have been consumed and checkpointed by the caller.
    /// No bodies are fetched, replies published or facts inferred from transport.
    /// Retries preserve first timestamps and the original completion reply.
    ///
    /// # Errors
    /// Rejects invalid/duplicate selections, foreign IDs and invalid completion
    /// links. A failure leaves every selected delivery unchanged.
    pub fn handle(
        &mut self,
        agent: &Key,
        ids: &[Key],
        handling: &Handling,
        now: i64,
    ) -> Result<Value, MailError> {
        if ids.is_empty() || ids.len() > MAX_PAGE || (handling.reply.is_some() && ids.len() != 1) {
            return Err(failure(
                "MAIL_INPUT",
                "choose 1..100 unique IDs; a completion reply selects one ID",
            ));
        }
        let unique: BTreeSet<_> = ids.iter().map(Key::as_str).collect();
        if unique.len() != ids.len() {
            return Err(failure(
                "MAIL_INPUT",
                "choose unique IDs without duplicates",
            ));
        }
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for id in ids {
            require_recipient(&tx, agent, id)?;
            if handling.complete {
                validate_completion(&tx, agent, id, handling.reply.as_ref())?;
            }
        }
        let mut messages = Vec::new();
        for id in ids {
            tx.execute(
                "UPDATE handling SET read_at=CASE WHEN ?2 THEN coalesce(read_at,?5) ELSE read_at END,received_at=CASE WHEN ?3 THEN coalesce(received_at,?5) ELSE received_at END,completed_at=CASE WHEN ?4 THEN coalesce(completed_at,?5) ELSE completed_at END,completion_reply=CASE WHEN ?4 THEN ?6 ELSE completion_reply END WHERE message=?1",
                params![id.as_str(),handling.read,handling.receipt,handling.complete,now,handling.reply.as_ref().map(Key::as_str)],
            )?;
            messages.push(tx.query_row(
                "SELECT read_at,received_at,completed_at FROM handling WHERE message=?1",
                [id.as_str()], |r| Ok(json!({"id":id.as_str(),"read_at":r.get::<_,Option<i64>>(0)?,"received_at":r.get::<_,Option<i64>>(1)?,"completed_at":r.get::<_,Option<i64>>(2)?})),
            )?);
        }
        tx.commit()?;
        Ok(json!({"outcome":"handled","agent":agent.as_str(),"messages":messages}))
    }
}
