//! Durable local coordination mail, separate from accepted project semantics.
//!
//! The deliberately versioned JSON records are the CLI's data contract. Local
//! identity claims are audit labels, not authentication against same-user code.
//! Message bytes commit atomically with notification intent; host I/O is separate.

mod delivery;

pub use delivery::Binding;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

/// Maximum UTF-8 message bytes; mail is a bounded handoff, not an artifact store.
pub const MAX_BODY_BYTES: usize = 8192;
/// Maximum headers per read, keeping notification-driven inbox draining bounded.
pub const MAX_PAGE: usize = 100;

/// A stable mailbox failure without message contents or host diagnostic output.
#[derive(Debug)]
pub struct MailError {
    /// Machine-readable recovery identity.
    pub code: &'static str,
    detail: String,
}
impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl Error for MailError {}
impl From<rusqlite::Error> for MailError {
    fn from(_: rusqlite::Error) -> Self {
        failure("MAIL_STORAGE", "mail storage operation failed")
    }
}
impl From<std::io::Error> for MailError {
    fn from(_: std::io::Error) -> Self {
        failure("MAIL_IO", "mail filesystem or process operation failed")
    }
}
fn failure(code: &'static str, detail: &str) -> MailError {
    MailError {
        code,
        detail: detail.into(),
    }
}

/// A bounded identity or message key, safe to include in a command pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key(String);
impl Key {
    /// Returns the validated key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl FromStr for Key {
    type Err = MailError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty()
            || s.len() > 64
            || !s
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        {
            return Err(failure(
                "MAIL_INPUT",
                "keys require 1..64 ASCII letters, digits, dot, underscore or hyphen",
            ));
        }
        Ok(Self(s.into()))
    }
}
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// An immutable authored message, validated before publication.
#[derive(Debug)]
pub struct Draft {
    id: Key,
    from: Key,
    to: Key,
    kind: String,
    body: String,
    reply: Option<Key>,
}
impl Draft {
    /// Validates a bounded body and a supported mail label.
    ///
    /// # Errors
    /// Returns `MAIL_INPUT` for an empty/oversized body or unknown label.
    pub fn new(
        id: Key,
        from: Key,
        to: Key,
        kind: &str,
        body: String,
        reply: Option<Key>,
    ) -> Result<Self, MailError> {
        if body.trim().is_empty()
            || body.len() > MAX_BODY_BYTES
            || !matches!(
                kind,
                "assign" | "ready" | "review" | "blocked" | "question" | "reply" | "note"
            )
            || (kind == "reply" && reply.is_none())
        {
            return Err(failure(
                "MAIL_INPUT",
                "use a supported label and a nonempty body up to 8192 bytes; replies require --in-reply-to",
            ));
        }
        Ok(Self {
            id,
            from,
            to,
            kind: kind.into(),
            body,
            reply,
        })
    }
}

/// A view of handling state; these filters never mutate the mailbox.
#[derive(Clone, Copy, Debug)]
pub enum Folder {
    /// All addressed messages.
    All,
    /// Messages not yet explicitly read.
    Unread,
    /// Messages lacking application receipt, including interrupted reads.
    Pending,
    /// Coordination requests not explicitly completed, even after receipt.
    Outstanding,
}
impl Folder {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Unread => "unread",
            Self::Pending => "pending",
            Self::Outstanding => "outstanding",
        }
    }
}

/// A synchronous mailbox handle; callers supply time at every state boundary.
#[derive(Debug)]
pub struct Mailbox {
    db: Connection,
    home: PathBuf,
    directory: PathBuf,
}
impl Mailbox {
    /// Opens only the dedicated `mail-v1` store beneath the selected home.
    ///
    /// # Errors
    /// Returns an I/O, storage or schema error without modifying other Merl stores.
    pub fn open(home: &Path) -> Result<Self, MailError> {
        if !home.is_absolute() {
            return Err(failure(
                "MAIL_INPUT",
                "--home must be an absolute dedicated mailbox directory",
            ));
        }
        let directory = home.join("mail-v1");
        private_directory(&directory)?;
        let home = home.canonicalize()?;
        let path = home
            .to_str()
            .ok_or_else(|| failure("MAIL_INPUT", "mail home must be UTF-8"))?;
        if path.len() > 512 || path.chars().any(char::is_control) {
            return Err(failure("MAIL_INPUT", "mail home exceeds pointer limits"));
        }
        let file = directory.join("mail.sqlite");
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&file)?;
        let mut db = Connection::open(&file)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != 0 && version != 1 {
            return Err(failure("MAIL_SCHEMA", "unsupported mailbox schema"));
        }
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.commit()?;
        Ok(Self {
            db,
            home,
            directory,
        })
    }

    /// Registers a durable name without guessing its host or session.
    ///
    /// # Errors
    /// Returns a storage error when registration cannot commit.
    pub fn register(&self, agent: &Key) -> Result<Value, MailError> {
        self.db.execute(
            "INSERT OR IGNORE INTO identities(name) VALUES(?1)",
            [agent.as_str()],
        )?;
        Ok(json!({"outcome":"registered","agent":agent.as_str()}))
    }

    /// Publishes a message and notification intent in the same transaction.
    ///
    /// # Errors
    /// Rejects unknown identities, changed retries, or replies to another delivery.
    pub fn send(&mut self, draft: &Draft, now: i64) -> Result<Value, MailError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for agent in [&draft.from, &draft.to] {
            require_identity(&tx, agent)?;
        }
        let existing: Option<(String, String, String, String, Option<String>)> = tx
            .query_row(
                "SELECT sender,recipient,kind,body,reply FROM messages WHERE id=?1",
                [draft.id.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some(old) = existing {
            if old
                != (
                    draft.from.0.clone(),
                    draft.to.0.clone(),
                    draft.kind.clone(),
                    draft.body.clone(),
                    draft.reply.as_ref().map(|k| k.0.clone()),
                )
            {
                return Err(failure(
                    "MAIL_ID_CONFLICT",
                    "message ID already names different immutable content",
                ));
            }
            return Ok(json!({"outcome":"unchanged","id":draft.id.as_str()}));
        }
        if let Some(reply) = &draft.reply {
            let endpoints: Option<(String, String)> = tx
                .query_row(
                    "SELECT sender,recipient FROM messages WHERE id=?1",
                    [reply.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if endpoints != Some((draft.to.0.clone(), draft.from.0.clone())) {
                return Err(failure(
                    "MAIL_REPLY_MISMATCH",
                    "reply must return to the author of a message addressed to this sender",
                ));
            }
        }
        tx.execute("INSERT INTO messages(id,sender,recipient,kind,body,reply,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![draft.id.as_str(),draft.from.as_str(),draft.to.as_str(),draft.kind,draft.body,draft.reply.as_ref().map(Key::as_str),now])?;
        tx.execute(
            "INSERT INTO handling(message) VALUES(?1)",
            [draft.id.as_str()],
        )?;
        tx.execute("INSERT INTO notifications(agent,state,next_at) VALUES(?1,'pending',?2) ON CONFLICT(agent) DO UPDATE SET state='pending',next_at=min(next_at,excluded.next_at)",params![draft.to.as_str(),now])?;
        tx.commit()?;
        Ok(json!({"outcome":"sent","id":draft.id.as_str()}))
    }

    /// Lists bounded headers in commit order without marking them read.
    ///
    /// # Errors
    /// Rejects unknown agents, invalid cursors or page limits outside 1..100.
    pub fn list(
        &self,
        agent: &Key,
        after: Option<&Key>,
        limit: usize,
        folder: Folder,
    ) -> Result<Value, MailError> {
        require_identity(&self.db, agent)?;
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(failure("MAIL_INPUT", "limit must be 1..100"));
        }
        let after = if let Some(id) = after {
            self.db
                .query_row(
                    "SELECT seq FROM messages WHERE id=?1 AND recipient=?2",
                    params![id.as_str(), agent.as_str()],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .ok_or_else(|| failure("MAIL_INPUT", "cursor does not belong to recipient"))?
        } else {
            0
        };
        let mut stmt=self.db.prepare("SELECT m.id,m.sender,m.recipient,m.kind,m.reply,m.created_at,h.read_at,h.received_at,h.completed_at FROM messages m JOIN handling h ON h.message=m.id WHERE m.recipient=?1 AND m.seq>?2 AND (?3='all' OR (?3='unread' AND h.read_at IS NULL) OR (?3='pending' AND h.received_at IS NULL) OR (?3='outstanding' AND m.kind IN ('assign','ready','review','blocked','question') AND h.completed_at IS NULL)) ORDER BY m.seq LIMIT ?4")?;
        let rows = stmt.query_map(
            params![
                agent.as_str(),
                after,
                folder.as_str(),
                i64::try_from(limit + 1).map_err(|_| failure("MAIL_INPUT", "invalid limit"))?
            ],
            header,
        )?;
        let mut messages = rows.collect::<Result<Vec<_>, _>>()?;
        let more = messages.len() > limit;
        messages.truncate(limit);
        let next = if more {
            messages.last().map(|v| v["id"].clone())
        } else {
            None
        };
        Ok(json!({"messages":messages,"next_after":next}))
    }

    /// Reads one delivery; `mark_read` records explicit reading separately from receipt.
    ///
    /// # Errors
    /// Rejects reads by anyone other than the named recipient.
    pub fn show(
        &self,
        agent: &Key,
        id: &Key,
        mark_read: bool,
        now: i64,
    ) -> Result<Value, MailError> {
        require_recipient(&self.db, agent, id)?;
        if mark_read {
            self.db.execute(
                "UPDATE handling SET read_at=coalesce(read_at,?2) WHERE message=?1",
                params![id.as_str(), now],
            )?;
        }
        let mut message=self.db.query_row("SELECT m.id,m.sender,m.recipient,m.kind,m.reply,m.created_at,h.read_at,h.received_at,h.completed_at FROM messages m JOIN handling h ON h.message=m.id WHERE m.id=?1",[id.as_str()],header)?;
        let body: String = self.db.query_row(
            "SELECT body FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        message["body"] = json!(body);
        Ok(json!({"message":message}))
    }

    /// Records application receipt without completing work or generating another wake.
    ///
    /// # Errors
    /// Rejects acknowledgments by anyone other than the recipient.
    pub fn receipt(&self, agent: &Key, id: &Key, now: i64) -> Result<Value, MailError> {
        require_recipient(&self.db, agent, id)?;
        self.db.execute(
            "UPDATE handling SET received_at=coalesce(received_at,?2) WHERE message=?1",
            params![id.as_str(), now],
        )?;
        Ok(json!({"outcome":"received","id":id.as_str()}))
    }

    /// Explicitly completes one request, optionally linking a matching authored reply.
    ///
    /// # Errors
    /// Rejects nonrecipients, nonrequests, and unrelated completion replies.
    pub fn complete(
        &mut self,
        agent: &Key,
        id: &Key,
        reply: Option<&Key>,
        now: i64,
    ) -> Result<Value, MailError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_recipient(&tx, agent, id)?;
        let kind: String = tx.query_row(
            "SELECT kind FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        if !is_request(&kind) {
            return Err(failure(
                "MAIL_INPUT",
                "only an explicit request can be completed",
            ));
        }
        if let Some(reply) = reply {
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages r JOIN messages m ON m.id=r.reply WHERE r.id=?1 AND m.id=?2 AND r.sender=m.recipient AND r.recipient=m.sender)",params![reply.as_str(),id.as_str()],|r|r.get(0))?;
            if !valid {
                return Err(failure(
                    "MAIL_REPLY_MISMATCH",
                    "completion reply belongs to another conversation",
                ));
            }
        }
        let prior: Option<String> = tx.query_row(
            "SELECT completion_reply FROM handling WHERE message=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        let completed: Option<i64> = tx.query_row(
            "SELECT completed_at FROM handling WHERE message=?1",
            [id.as_str()],
            |r| r.get(0),
        )?;
        if completed.is_some() && prior.as_deref() != reply.map(Key::as_str) {
            return Err(failure(
                "MAIL_ID_CONFLICT",
                "completion retry changed its reply",
            ));
        }
        tx.execute("UPDATE handling SET completed_at=coalesce(completed_at,?2),completion_reply=?3 WHERE message=?1",params![id.as_str(),now,reply.map(Key::as_str)])?;
        tx.commit()?;
        Ok(json!({"outcome":"completed","id":id.as_str()}))
    }

    /// Reports unread, unreceipted, outstanding and host-notification facts separately.
    ///
    /// # Errors
    /// Returns an identity or storage error when status is unavailable.
    pub fn status(&self, agent: &Key) -> Result<Value, MailError> {
        require_identity(&self.db, agent)?;
        let counts:(i64,i64,i64)=self.db.query_row("SELECT coalesce(sum(h.read_at IS NULL),0),coalesce(sum(h.received_at IS NULL),0),coalesce(sum(m.kind IN ('assign','ready','review','blocked','question') AND h.completed_at IS NULL),0) FROM messages m JOIN handling h ON h.message=m.id WHERE m.recipient=?1",[agent.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let binding:Value=self.db.query_row("SELECT generation,session,socket FROM identities WHERE name=?1",[agent.as_str()],|r|Ok(json!({"generation":r.get::<_,i64>(0)?,"session":r.get::<_,Option<String>>(1)?,"socket":r.get::<_,Option<String>>(2)?})))?;
        let notification=self.db.query_row("SELECT state,attempts,next_at,last_error,generation FROM notifications WHERE agent=?1",[agent.as_str()],|r|Ok(json!({"state":r.get::<_,String>(0)?,"attempts":r.get::<_,i64>(1)?,"next_attempt_at":r.get::<_,i64>(2)?,"last_error":r.get::<_,Option<String>>(3)?,"binding_generation":r.get::<_,Option<i64>>(4)?}))).optional()?;
        Ok(
            json!({"agent":agent.as_str(),"unread":counts.0,"unreceipted":counts.1,"outstanding":counts.2,"binding":binding,"notification":notification}),
        )
    }
}

fn is_request(kind: &str) -> bool {
    matches!(kind, "assign" | "ready" | "review" | "blocked" | "question")
}
fn require_identity(db: &Connection, agent: &Key) -> Result<(), MailError> {
    let found: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM identities WHERE name=?1)",
        [agent.as_str()],
        |r| r.get(0),
    )?;
    if found {
        Ok(())
    } else {
        Err(failure(
            "MAIL_IDENTITY_MISSING",
            "register the explicit identity first",
        ))
    }
}
fn require_recipient(db: &Connection, agent: &Key, id: &Key) -> Result<(), MailError> {
    let recipient: Option<String> = db
        .query_row(
            "SELECT recipient FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get(0),
        )
        .optional()?;
    match recipient {
        None => Err(failure("MAIL_NOT_FOUND", "message not found")),
        Some(r) if r == agent.as_str() => Ok(()),
        Some(_) => Err(failure(
            "MAIL_NOT_RECIPIENT",
            "only the named recipient may handle this message",
        )),
    }
}
fn header(r: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(
        json!({"id":r.get::<_,String>(0)?,"from":r.get::<_,String>(1)?,"to":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?,"in_reply_to":r.get::<_,Option<String>>(4)?,"created_at":r.get::<_,i64>(5)?,"read_at":r.get::<_,Option<i64>>(6)?,"received_at":r.get::<_,Option<i64>>(7)?,"completed_at":r.get::<_,Option<i64>>(8)?}),
    )
}
fn private_directory(path: &Path) -> Result<(), MailError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
