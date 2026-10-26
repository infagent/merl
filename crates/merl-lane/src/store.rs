use crate::{
    Configuration, Labels, Lane, LaneError, Name, POLL_MILLIS, Repository, failure, input,
};
use merl_mail::Key;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::Duration,
};

// A bounded local registry keeps status and fair scheduling small. PR histories
// and immutable mail are retained; this is not a retention or purge policy.
const MAX_LANES: i64 = 16;

/// Durable configuration/checkpoints beneath a separate `lane-v1` runtime namespace.
#[derive(Debug)]
pub struct Store {
    pub(super) db: Connection,
    pub(super) home: PathBuf,
    directory: PathBuf,
}

impl Store {
    /// Opens lane metadata without modifying the existing `mail-v1` schema.
    ///
    /// # Errors
    /// Rejects relative/unbounded homes, unknown schemas and storage failures.
    pub fn open(home: &Path) -> Result<Self, LaneError> {
        if !home.is_absolute() {
            return Err(input(
                "--home must be an absolute dedicated mailbox directory",
            ));
        }
        let directory = home.join("lane-v1");
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        let home = home.canonicalize()?;
        let text = home.to_str().ok_or_else(|| input("home must be UTF-8"))?;
        if text.len() > 512 || text.chars().any(char::is_control) {
            return Err(input("home exceeds runtime pointer limits"));
        }
        let directory = home.join("lane-v1");
        let path = directory.join("lanes.sqlite");
        options().open(&path)?;
        let mut db = Connection::open(path)?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.execute_batch("PRAGMA foreign_keys=ON;")?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != 0 && version != 1 {
            return Err(failure("LANE_SCHEMA", "unsupported lane schema"));
        }
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.commit()?;
        Ok(Self {
            db,
            home,
            directory,
        })
    }

    /// Returns the canonical stable runtime directory for configured subprocesses.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Creates a subscription or resumes it under an explicit generation guard.
    ///
    /// An exact active creation retry is unchanged. Repository and recipient are
    /// immutable; binding a fresh session is a separate mailbox operation.
    ///
    /// # Errors
    /// Rejects changed retries, stale generations, missing configuration or registry limits.
    pub fn start(
        &mut self,
        name: &Name,
        configuration: Option<Configuration>,
        expected: Option<i64>,
        now: i64,
    ) -> Result<Value, LaneError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = load_lane(&tx, name.as_str())?;
        if let Some(old) = existing {
            let paused: bool = tx.query_row(
                "SELECT paused FROM lanes WHERE name=?1",
                [name.as_str()],
                |r| r.get(0),
            )?;
            if let Some(config) = configuration {
                if !paused && expected.is_none() && same(&old.configuration, &config) {
                    return Ok(json!({"outcome":"unchanged","generation":old.generation}));
                }
                return Err(conflict());
            }
            if expected != Some(old.generation) || old.generation == i64::MAX {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE lanes SET paused=0,generation=generation+1,next_scan=?2 WHERE name=?1",
                params![name.as_str(), now],
            )?;
            tx.execute(
                "UPDATE prs SET due=?2 WHERE lane=?1 AND in_scope=1 AND tracking=1",
                params![name.as_str(), now],
            )?;
            tx.commit()?;
            return Ok(json!({"outcome":"started","generation":old.generation+1}));
        }
        if expected.is_some_and(|g| g != 0) {
            return Err(conflict());
        }
        let config =
            configuration.ok_or_else(|| input("new lanes require --repo, --agent and --label"))?;
        let count: i64 = tx.query_row("SELECT count(*) FROM lanes", [], |r| r.get(0))?;
        if count >= MAX_LANES {
            return Err(failure("LANE_LIMIT", "at most 16 named lanes per home"));
        }
        let ticket = next_ticket(&tx)?;
        tx.execute("INSERT INTO lanes(name,repo,agent,labels,generation,next_scan,ticket) VALUES(?1,?2,?3,?4,1,?5,?6)", params![name.as_str(),config.repository.as_str(),config.agent.as_str(),json!(config.labels.as_slice()).to_string(),now,ticket])?;
        tx.commit()?;
        Ok(json!({"outcome":"started","generation":1}))
    }

    /// Replaces only the label selector and schedules a fresh membership sweep.
    ///
    /// Existing PR cursors and already staged/frozen delivery are preserved.
    ///
    /// # Errors
    /// Rejects absent lanes or a stale generation.
    pub fn update(
        &mut self,
        name: &Name,
        labels: &Labels,
        expected: i64,
        now: i64,
    ) -> Result<Value, LaneError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute("UPDATE lanes SET labels=?2,generation=generation+1,discovery_after=NULL,next_scan=?3,last_scan_at=NULL WHERE name=?1 AND generation=?4 AND generation<9223372036854775807",params![name.as_str(),json!(labels.as_slice()).to_string(),now,expected])?;
        if changed != 1 {
            return Err(conflict());
        }
        tx.execute(
            "UPDATE prs SET due=?2 WHERE lane=?1 AND in_scope=1 AND tracking=1",
            params![name.as_str(), now],
        )?;
        tx.commit()?;
        Ok(json!({"outcome":"updated","generation":expected+1}))
    }

    /// Pauses scanning and unclaimed publication without deleting progress or mail.
    ///
    /// A publication already in flight may finish; immutable published mail still
    /// follows normal receipt/completion rules and may wake its bound recipient.
    ///
    /// # Errors
    /// Rejects absent lanes and stale generations.
    pub fn stop(&self, name: &Name, expected: i64) -> Result<Value, LaneError> {
        let changed = self.db.execute("UPDATE lanes SET paused=1,generation=generation+1 WHERE name=?1 AND generation=?2 AND generation<9223372036854775807",params![name.as_str(),expected])?;
        if changed != 1 {
            return Err(conflict());
        }
        Ok(json!({"outcome":"stopped","generation":expected+1}))
    }

    /// Lists bounded lane health independently of host enqueue and review completion.
    ///
    /// `last_scan_at` is the last complete discovery sweep, not a claim that every
    /// nested PR connection is current. Use `queue` for per-PR progress.
    ///
    /// # Errors
    /// Returns storage errors or `LANE_NOT_FOUND` for a missing selected lane.
    pub fn status(&self, name: Option<&Name>) -> Result<Value, LaneError> {
        let mut query = self.db.prepare("SELECT name,repo,agent,labels,generation,paused,discovery_after,next_scan,last_scan_at,last_error,gaps FROM lanes WHERE (?1 IS NULL OR name=?1) ORDER BY name LIMIT 16")?;
        let lanes = query.query_map([name.map(Name::as_str)], |r| {
            let name: String = r.get(0)?;
            let labels:Value=serde_json::from_str(&r.get::<_,String>(3)?).map_err(|error|rusqlite::Error::FromSqlConversionFailure(3,rusqlite::types::Type::Text,Box::new(error)))?;
            Ok(json!({"name":name,"repository":r.get::<_,String>(1)?,"agent":r.get::<_,String>(2)?,"labels":labels,"generation":r.get::<_,i64>(4)?,"paused":r.get::<_,bool>(5)?,"discovery_after":r.get::<_,Option<String>>(6)?,"next_scan_at":r.get::<_,i64>(7)?,"last_scan_at":r.get::<_,Option<i64>>(8)?,"last_error":r.get::<_,Option<String>>(9)?,"gaps":r.get::<_,i64>(10)?}))
        })?.collect::<Result<Vec<_>,_>>()?;
        if name.is_some() && lanes.is_empty() {
            return Err(failure("LANE_NOT_FOUND", "named lane not found"));
        }
        let mut records = Vec::new();
        for mut lane in lanes {
            let name = lane["name"]
                .as_str()
                .ok_or_else(|| failure("LANE_STORAGE", "invalid lane record"))?;
            let counts: (i64,i64,i64,i64) = self.db.query_row("SELECT (SELECT count(*) FROM prs WHERE lane=?1 AND in_scope=1 AND tracking=1),(SELECT count(*) FROM prs WHERE lane=?1 AND initializing=1 AND in_scope=1),(SELECT count(*) FROM prs WHERE lane=?1 AND last_error IS NOT NULL),(SELECT count(*) FROM outbox WHERE lane=?1 AND state!='sent')",[name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
            let staged: i64 =
                self.db
                    .query_row("SELECT count(*) FROM pending WHERE lane=?1", [name], |r| {
                        r.get(0)
                    })?;
            let failed: i64 = self.db.query_row(
                "SELECT count(*) FROM outbox WHERE lane=?1 AND last_error IS NOT NULL",
                [name],
                |r| r.get(0),
            )?;
            lane["active_prs"] = json!(counts.0);
            lane["initializing_prs"] = json!(counts.1);
            lane["failed_prs"] = json!(counts.2);
            lane["pending_batches"] = json!(counts.3);
            lane["staged_prs"] = json!(staged);
            lane["failed_batches"] = json!(failed);
            records.push(lane);
        }
        let retry: i64 = self
            .db
            .query_row("SELECT retry_at FROM control WHERE id=1", [], |r| r.get(0))?;
        Ok(json!({"lanes":records,"provider_retry_at":retry}))
    }

    /// Pages the retained PR queue, including explicit scope exits and terminal state.
    ///
    /// # Errors
    /// Rejects unknown lanes, invalid limits and storage failures.
    pub fn queue(&self, name: &Name, after: i64, limit: usize) -> Result<Value, LaneError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(input("queue limit must be 1..100 and after nonnegative"));
        }
        if load_lane(&self.db, name.as_str())?.is_none() {
            return Err(failure("LANE_NOT_FOUND", "named lane not found"));
        }
        let mut query=self.db.prepare("SELECT number,head,state,draft,in_scope,initializing,timeline_cursor,threads_after,last_success,last_error,gaps,tracking FROM prs WHERE lane=?1 AND number>?2 ORDER BY number LIMIT ?3")?;
        let rows=query.query_map(params![name.as_str(),after,i64::try_from(limit+1).map_err(|_| input("invalid queue limit"))?],|r|Ok(json!({"number":r.get::<_,i64>(0)?,"head":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"draft":r.get::<_,bool>(3)?,"in_scope":r.get::<_,bool>(4)?,"initializing":r.get::<_,bool>(5)?,"timeline_cursor":r.get::<_,Option<String>>(6)?,"threads_after":r.get::<_,Option<String>>(7)?,"last_success_at":r.get::<_,Option<i64>>(8)?,"last_error":r.get::<_,Option<String>>(9)?,"gaps":r.get::<_,i64>(10)?,"tracking":r.get::<_,bool>(11)?})))?;
        let mut prs = rows.collect::<Result<Vec<_>, _>>()?;
        let more = prs.len() > limit;
        prs.truncate(limit);
        let next = if more {
            prs.last().map(|v| v["number"].clone())
        } else {
            None
        };
        Ok(json!({"prs":prs,"next_after":next}))
    }

    pub(super) fn scan_guard(&self) -> Result<File, LaneError> {
        let file = options().open(self.directory.join("scan.lock"))?;
        file.try_lock().map_err(|_| {
            failure(
                "LANE_BUSY",
                "another lane scan owns this runtime; retry later",
            )
        })?;
        Ok(file)
    }
}

fn options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}
fn same(a: &Configuration, b: &Configuration) -> bool {
    a.repository.as_str() == b.repository.as_str() && a.agent == b.agent && a.labels == b.labels
}
fn conflict() -> LaneError {
    failure(
        "LANE_CONFLICT",
        "lane generation/configuration changed; inspect status before updating or resuming",
    )
}
pub(super) fn next_ticket(db: &Connection) -> Result<i64, LaneError> {
    db.query_row("UPDATE control SET ticket=ticket+1 WHERE id=1 AND ticket<9223372036854775807 RETURNING ticket",[],|r|r.get(0)).map_err(LaneError::from)
}
pub(super) fn load_lane(db: &Connection, name: &str) -> Result<Option<Lane>, LaneError> {
    let row = db
        .query_row(
            "SELECT repo,agent,labels,generation FROM lanes WHERE name=?1",
            [name],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    row.map(|(repo, agent, labels, generation)| {
        Ok(Lane {
            name: name.into(),
            configuration: Configuration {
                repository: repo.parse::<Repository>()?,
                agent: agent
                    .parse::<Key>()
                    .map_err(|_| failure("LANE_STORAGE", "invalid stored recipient"))?,
                labels: Labels::new(
                    serde_json::from_str(&labels)
                        .map_err(|_| failure("LANE_STORAGE", "invalid stored labels"))?,
                )?,
            },
            generation,
        })
    })
    .transpose()
}

pub(super) fn scheduled(now: i64) -> i64 {
    now.saturating_add(POLL_MILLIS)
}
