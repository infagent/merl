//! Atomic informational fanout to one frozen, bounded roster selection.
//!
//! Accepted snapshots and ordinary notes commit together in mail-v1. Callers
//! acquire bodies and candidate IDs before entering this storage boundary.

use crate::{Draft, Key, MAX_BODY_BYTES, MailError, Mailbox, failure, publish, team};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

/// Fanout cap; at most 819,200 body bytes are copied into ordinary notes.
pub const MAX_RECIPIENTS: usize = 100;
/// Bounds selector input before deduplication, independent of roster size.
pub const MAX_ROLE_FILTERS: usize = 16;

/// An explicit all-members selector or canonical exact-role OR selector.
#[derive(Debug)]
pub struct Selection(Option<Vec<String>>);
impl Selection {
    /// Selects every member, including members with no role.
    #[must_use]
    pub fn all() -> Self {
        Self(None)
    }

    /// Validates at most 16 supplied roles, then sorts and deduplicates them.
    ///
    /// # Errors
    /// Returns `TEAM_INPUT` for empty/oversized input or invalid role labels.
    pub fn roles(mut roles: Vec<String>) -> Result<Self, MailError> {
        if roles.is_empty()
            || roles.len() > MAX_ROLE_FILTERS
            || roles.iter().any(|r| {
                r.is_empty() || r.len() > team::MAX_ROLE_BYTES || r.chars().any(char::is_control)
            })
        {
            return Err(failure(
                "TEAM_INPUT",
                "supply 1..16 roles of 1..128 bytes without control characters",
            ));
        }
        roles.sort();
        roles.dedup();
        Ok(Self(Some(roles)))
    }

    fn json(&self) -> Value {
        json!({"all":self.0.is_none(),"roles":self.0.as_deref().unwrap_or(&[])})
    }
}

/// Validated immutable announcement inputs, independent of current membership.
#[derive(Debug)]
pub struct AnnouncementSpec {
    id: Key,
    team: Key,
    author_team: Key,
    author_member: Key,
    body: String,
    selection: Selection,
    include_self: bool,
}
impl AnnouncementSpec {
    /// Validates a note body; the author can belong to another scope.
    ///
    /// # Errors
    /// Returns `MAIL_INPUT` for blank bodies or more than 8192 UTF-8 bytes.
    pub fn new(
        id: Key,
        team: Key,
        author_team: Key,
        author_member: Key,
        body: String,
        selection: Selection,
        include_self: bool,
    ) -> Result<Self, MailError> {
        if body.trim().is_empty() || body.len() > MAX_BODY_BYTES {
            return Err(failure(
                "MAIL_INPUT",
                "announcement body must be nonempty and at most 8192 UTF-8 bytes",
            ));
        }
        Ok(Self {
            id,
            team,
            author_team,
            author_member,
            body,
            selection,
            include_self,
        })
    }
}

fn schema_present(db: &Connection) -> Result<bool, MailError> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='merl_announcement_meta')", [], |r| r.get(0))?;
    if !exists {
        return Ok(false);
    }
    let version: Option<i64> = db
        .query_row(
            "SELECT version FROM merl_announcement_meta WHERE id=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if version != Some(1) {
        return Err(failure("TEAM_SCHEMA", "unsupported announcement schema"));
    }
    Ok(true)
}

fn accepted(db: &Connection, id: &Key) -> Result<Option<Value>, MailError> {
    if !schema_present(db)? {
        return Ok(None);
    }
    let mut result: Option<Value> = db.query_row(
        "SELECT team,author_team,author_member,author_key,selector,include_self,created_at FROM merl_announcements WHERE id=?1",
        [id.as_str()], |r| {
            let team: String = r.get(0)?;
            let author_team: String = r.get(1)?;
            let author_member: String = r.get(2)?;
            let raw: String = r.get(4)?;
            let selector: Value = serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok(json!({"id":id.as_str(),"team":team,"author":{"team":author_team,"member":author_member,"qualified":format!("{author_team}/{author_member}"),"mail_key":r.get::<_,String>(3)?},"selector":selector,"include_self":r.get::<_,bool>(5)?,"created_at":r.get::<_,i64>(6)?}))
        }).optional()?;
    if let Some(data) = &mut result {
        let team = data["team"]
            .as_str()
            .ok_or_else(|| failure("MAIL_STORAGE", "invalid announcement metadata"))?;
        let mut stmt = db.prepare("SELECT member,mail_key,message FROM merl_announcement_recipients WHERE announcement=?1 ORDER BY member LIMIT 101")?;
        let recipients = stmt.query_map([id.as_str()], |r| {
            let member: String = r.get(0)?;
            Ok(json!({"member":member,"qualified":format!("{team}/{member}"),"mail_key":r.get::<_,String>(1)?,"message_id":r.get::<_,String>(2)?}))
        })?.collect::<Result<Vec<_>,_>>()?;
        if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
            return Err(failure(
                "MAIL_STORAGE",
                "invalid announcement recipient snapshot",
            ));
        }
        data["recipient_count"] = json!(recipients.len());
        data["recipients"] = json!(recipients);
    }
    Ok(result)
}

impl Mailbox {
    /// Reads one accepted snapshot without body, handling facts or roster expansion.
    ///
    /// This also lets callers skip candidate acquisition on a committed retry.
    /// No table or directory is created by this method.
    ///
    /// # Errors
    /// Returns schema/storage errors; an unknown ID returns `None`.
    pub fn announcement(&self, id: &Key) -> Result<Option<Value>, MailError> {
        accepted(&self.db, id)
    }

    /// Publishes every selected ordinary note and its accepted snapshot atomically.
    ///
    /// Supply at most 100 candidate message IDs obtained before this call. A new
    /// announcement needs one per selected recipient; unused candidates reserve
    /// nothing. Exact retries ignore candidates and current roster/bindings.
    ///
    /// # Errors
    /// Returns `TEAM_CONFLICT` for changed accepted inputs, `TEAM_SELECTION` for
    /// zero or more than 100 selected members, `TEAM_MESSAGE_CONFLICT` for an
    /// existing child ID, or input/team/schema/storage errors. Pre-commit failure
    /// rolls back every note and notification change. After an uncertain commit
    /// failure, inspect/retry the announcement ID to determine acceptance.
    pub fn announce(
        &mut self,
        spec: &AnnouncementSpec,
        candidates: &[Key],
        now: i64,
    ) -> Result<Value, MailError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(mut old) = accepted(&tx, &spec.id)? {
            let body: String = tx.query_row(
                "SELECT body FROM merl_announcements WHERE id=?1",
                [spec.id.as_str()],
                |r| r.get(0),
            )?;
            if old["team"] != spec.team.as_str()
                || old["author"]["team"] != spec.author_team.as_str()
                || old["author"]["member"] != spec.author_member.as_str()
                || body != spec.body
                || old["selector"] != spec.selection.json()
                || old["include_self"] != spec.include_self
            {
                return Err(failure(
                    "TEAM_CONFLICT",
                    "announcement ID names different immutable inputs",
                ));
            }
            old["outcome"] = json!("unchanged");
            return Ok(old);
        }
        if candidates.len() > MAX_RECIPIENTS {
            return Err(failure("TEAM_INPUT", "at most 100 candidate IDs"));
        }
        team::require_team(&tx, &spec.team)?;
        team::require_team(&tx, &spec.author_team)?;
        let author: String = tx
            .query_row(
                "SELECT mail_key FROM merl_team_members WHERE team=?1 AND member=?2",
                params![spec.author_team.as_str(), spec.author_member.as_str()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| failure("TEAM_MEMBER_MISSING", "author has not joined its scope"))?;
        let roles = json!(spec.selection.0.as_deref().unwrap_or(&[])).to_string();
        // Filter in SQL before fetching the overflow sentinel. Never load an
        // unbounded roster and then truncate it into a successful publication.
        let recipients = {
            let mut stmt = tx.prepare("SELECT member,mail_key FROM merl_team_members WHERE team=?1 AND (?2 OR role IN (SELECT value FROM json_each(?3))) AND (?4 OR mail_key<>?5) ORDER BY member LIMIT 101")?;
            stmt.query_map(
                params![
                    spec.team.as_str(),
                    spec.selection.0.is_none(),
                    roles,
                    spec.include_self,
                    author
                ],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?
        };
        if recipients.is_empty() || recipients.len() > MAX_RECIPIENTS {
            return Err(failure(
                "TEAM_SELECTION",
                "select between 1 and 100 recipients; no announcement accepted",
            ));
        }
        if candidates.len() < recipients.len() {
            return Err(failure("TEAM_INPUT", "insufficient candidate message IDs"));
        }
        tx.execute_batch(include_str!("announcement-schema.sql"))?;
        tx.execute("INSERT INTO merl_announcements(id,team,author_team,author_member,author_key,body,selector,include_self,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![spec.id.as_str(),spec.team.as_str(),spec.author_team.as_str(),spec.author_member.as_str(),author,spec.body,spec.selection.json().to_string(),spec.include_self,now])?;
        for ((member, recipient), candidate) in recipients.iter().zip(candidates) {
            let draft = Draft::new(
                candidate.clone(),
                author.parse()?,
                recipient.parse()?,
                "note",
                spec.body.clone(),
                None,
            )?;
            publish(&tx, &draft, now, true)?;
            tx.execute("INSERT INTO merl_announcement_recipients(announcement,member,mail_key,message) VALUES(?1,?2,?3,?4)", params![spec.id.as_str(),member,recipient,candidate.as_str()])?;
        }
        let mut result = accepted(&tx, &spec.id)?
            .ok_or_else(|| failure("MAIL_STORAGE", "accepted snapshot missing"))?;
        tx.commit()?;
        result["outcome"] = json!("published");
        Ok(result)
    }
}
