use crate::{
    GitHub, Lane, LaneError, MAX_REQUESTS, MailSink, Query, Store, failure, input, protocol,
    store::{load_lane, next_ticket, scheduled},
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

// Retry a transient failure gently, and follow GitHub's increasing waits for
// repeated secondary limits. Server deadlines remain uncapped lower bounds.
const RETRY_MILLIS: i64 = 5_000;
const RATE_RETRY_MILLIS: i64 = 60_000;
const MAX_RATE_RETRY_MILLIS: i64 = 3_600_000;

#[derive(Debug)]
struct Job {
    lane: Lane,
    number: i64,
    query: Query,
}

impl Store {
    /// Performs a pass on a fixed logical clock, useful for controlled embeddings.
    ///
    /// Use `scan` with a changing clock for real I/O, so response latency cannot
    /// shorten provider retry deadlines or the next polling interval.
    ///
    /// # Errors
    /// Returns the same explicit source, delivery and storage failures as `scan`.
    pub fn scan_at(
        &mut self,
        github: &mut impl GitHub,
        mail: &mut impl MailSink,
        now: i64,
        max_requests: usize,
    ) -> Result<Value, LaneError> {
        self.scan(github, mail, &|| Ok(now), max_requests)
    }
    /// Performs bounded fair source work and retries immutable pending mail.
    ///
    /// A home-scoped lock excludes overlapping scanners. Config/pause changes can
    /// commit during I/O; their generation fences every subsequent checkpoint.
    /// Server deadlines also apply to later passes, independently of mail delivery.
    ///
    /// # Errors
    /// Returns visible storage/provider/rate/delivery failures. Earlier transactions
    /// remain durable, and no failed/incomplete page advances its checkpoint.
    pub fn scan(
        &mut self,
        github: &mut impl GitHub,
        mail: &mut impl MailSink,
        clock: &impl Fn() -> Result<i64, LaneError>,
        max_requests: usize,
    ) -> Result<Value, LaneError> {
        if !(1..=MAX_REQUESTS).contains(&max_requests) {
            return Err(input("max requests must be 1..16"));
        }
        let now = clock()?;
        let _guard = self.scan_guard()?;
        let mut delivered = self.flush(mail, now)?;
        let mut requests = 0;
        for _ in 0..max_requests {
            let now = clock()?;
            let retry: i64 =
                self.db
                    .query_row("SELECT retry_at FROM control WHERE id=1", [], |r| r.get(0))?;
            if retry > now {
                break;
            }
            let Some(job) = self.job(now)? else {
                break;
            };
            requests += 1;
            let reply = github.fetch(&job.query, clock);
            let now = clock()?;
            let result = match reply {
                Ok(reply) => {
                    if reply.not_before > now {
                        self.db.execute(
                            "UPDATE control SET retry_at=max(retry_at,?1) WHERE id=1",
                            [reply.not_before],
                        )?;
                    }
                    let applied = self.apply(&job, &reply.document, now);
                    if applied.is_ok() {
                        self.db
                            .execute("UPDATE control SET failures=0 WHERE id=1", [])?;
                    }
                    applied.map(|_| ())
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result
                && self.record_failure(&job, &error, now)?
            {
                return Err(error);
            }
        }
        let now = clock()?;
        delivered += self.flush(mail, now)?;
        let retry: i64 = self
            .db
            .query_row("SELECT retry_at FROM control WHERE id=1", [], |r| r.get(0))?;
        Ok(
            json!({"outcome":if retry>now{"waiting"}else if requests==0&&delivered==0{"idle"}else{"scanned"},"requests":requests,"published_batches":delivered,"provider_retry_at":retry}),
        )
    }

    fn job(&self, now: i64) -> Result<Option<Job>, LaneError> {
        let row=self.db.query_row("SELECT name,number FROM (SELECT name,0 AS number,ticket FROM lanes WHERE paused=0 AND next_scan<=?1 UNION ALL SELECT p.lane,p.number,p.ticket FROM prs p JOIN lanes l ON l.name=p.lane WHERE l.paused=0 AND p.in_scope=1 AND p.tracking=1 AND p.due<=?1) ORDER BY ticket LIMIT 1",[now],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).optional()?;
        let Some((name, number)) = row else {
            return Ok(None);
        };
        let lane = load_lane(&self.db, &name)?.ok_or_else(protocol)?;
        let query = if number == 0 {
            let after = self.db.query_row(
                "SELECT discovery_after FROM lanes WHERE name=?1",
                [&name],
                |r| r.get::<_, Option<String>>(0),
            )?;
            Query::Discover {
                repository: lane.configuration.repository.clone(),
                labels: lane.configuration.labels.clone(),
                after,
            }
        } else {
            let (after, threads_after, resync) = self.db.query_row(
                "SELECT timeline_cursor,threads_after,resync FROM prs WHERE lane=?1 AND number=?2",
                params![name, number],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, bool>(2)?)),
            )?;
            Query::Observe {
                repository: lane.configuration.repository.clone(),
                number,
                after,
                threads_after,
                resync,
            }
        };
        Ok(Some(Job {
            lane,
            number,
            query,
        }))
    }

    fn apply(&mut self, job: &Job, document: &Value, now: i64) -> Result<bool, LaneError> {
        if document
            .get("errors")
            .is_some_and(|value| value.as_array().is_none_or(|errors| !errors.is_empty()))
        {
            return Err(failure(
                "LANE_GITHUB",
                "partial GraphQL data cannot advance lane progress",
            ));
        }
        let repository = document["data"]
            .get("repository")
            .filter(|v| !v.is_null())
            .ok_or_else(|| {
                failure(
                    "LANE_SOURCE_UNAVAILABLE",
                    "repository absent or inaccessible; progress retained",
                )
            })?;
        let parsed = if job.number == 0 {
            let page = Page::new(&repository["pullRequests"])?;
            Parsed::Discovery(
                page.nodes
                    .iter()
                    .map(Snapshot::new)
                    .collect::<Result<Vec<_>, _>>()?,
                page,
            )
        } else {
            let value = repository
                .get("pullRequest")
                .filter(|v| !v.is_null())
                .ok_or_else(|| {
                    failure(
                        "LANE_SOURCE_UNAVAILABLE",
                        "PR absent or inaccessible; scope is not inferred from absence",
                    )
                })?;
            Parsed::Observation(Snapshot::new(value)?, Page::new(&value["reviewThreads"])?)
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !current(&tx, &job.lane)? {
            return Ok(false);
        }
        match parsed {
            Parsed::Discovery(snapshots, page) => {
                apply_discovery(&tx, &job.lane, &snapshots, &page, now)?;
            }
            Parsed::Observation(snapshot, threads) => {
                apply_observation(&tx, job, &snapshot, &threads, now)?;
            }
        }
        tx.commit()?;
        Ok(true)
    }

    fn record_failure(
        &mut self,
        job: &Job,
        error: &LaneError,
        now: i64,
    ) -> Result<bool, LaneError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let failures: i64 = tx.query_row(
            "UPDATE control SET failures=min(failures+1,11) WHERE id=1 RETURNING failures",
            [],
            |r| r.get(0),
        )?;
        let shift = u32::try_from((failures - 1).clamp(0, 6)).map_err(|_| protocol())?;
        let (base, cap) = if error.code == "LANE_RATE_LIMITED" {
            (RATE_RETRY_MILLIS, MAX_RATE_RETRY_MILLIS)
        } else {
            (RETRY_MILLIS, 60_000)
        };
        let retry = now
            .saturating_add((base << shift).min(cap))
            .max(error.retry_at.unwrap_or(0));
        // A configuration fence invalidates content, not a shared credential's
        // cooldown. Preserve resource facts even for stale content responses.
        tx.execute(
            "UPDATE control SET retry_at=max(retry_at,?1) WHERE id=1",
            [retry],
        )?;
        if !current(&tx, &job.lane)? {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE lanes SET last_error=?2 WHERE name=?1",
            params![job.lane.name, error.code],
        )?;
        let ticket = next_ticket(&tx)?;
        if job.number == 0 {
            tx.execute(
                "UPDATE lanes SET next_scan=?2,ticket=?3 WHERE name=?1",
                params![job.lane.name, retry, ticket],
            )?;
            if error.code == "LANE_CURSOR_GAP" {
                tx.execute(
                    "UPDATE lanes SET discovery_after=NULL,gaps=gaps+1 WHERE name=?1",
                    [&job.lane.name],
                )?;
            }
        } else {
            tx.execute(
                "UPDATE prs SET due=?3,ticket=?4,last_error=?5 WHERE lane=?1 AND number=?2",
                params![job.lane.name, job.number, retry, ticket, error.code],
            )?;
            if error.code == "LANE_CURSOR_GAP" {
                tx.execute("UPDATE prs SET resync=1,initializing=1,timeline_cursor=NULL,threads_after=NULL,gaps=gaps+1 WHERE lane=?1 AND number=?2",params![job.lane.name,job.number])?;
                tx.execute(
                    "DELETE FROM threads WHERE lane=?1 AND number=?2",
                    params![job.lane.name, job.number],
                )?;
                let (head, state) = tx.query_row(
                    "SELECT head,state FROM prs WHERE lane=?1 AND number=?2",
                    params![job.lane.name, job.number],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?;
                stage(
                    &tx,
                    &job.lane,
                    job.number,
                    &head,
                    &state,
                    ["cursor-gap".to_owned()].into(),
                )?;
            }
        }
        tx.commit()?;
        Ok(true)
    }
}

enum Parsed {
    Discovery(Vec<Snapshot>, Page),
    Observation(Snapshot, Page),
}
struct Page {
    nodes: Vec<Value>,
    after: Option<String>,
    more: bool,
}
impl Page {
    fn new(value: &Value) -> Result<Self, LaneError> {
        let edges = value["edges"]
            .as_array()
            .filter(|v| v.len() <= 50)
            .ok_or_else(protocol)?;
        let more = value["pageInfo"]["hasNextPage"]
            .as_bool()
            .ok_or_else(protocol)?;
        let after = cursor(&value["pageInfo"]["endCursor"])?;
        if more && (edges.is_empty() || after.is_none()) {
            return Err(protocol());
        }
        if let Some(last) = edges.last()
            && cursor(&last["cursor"])? != after
        {
            return Err(protocol());
        }
        Ok(Self {
            nodes: edges.iter().map(|e| e["node"].clone()).collect(),
            after,
            more,
        })
    }
}
struct Snapshot {
    number: i64,
    head: String,
    state: String,
    draft: bool,
    labels: Vec<String>,
    labels_more: bool,
    timeline: Page,
}
impl Snapshot {
    fn new(value: &Value) -> Result<Self, LaneError> {
        let number = value["number"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(protocol)?;
        let head = bounded(&value["headRefOid"], 40)?;
        if head.len() != 40 || !head.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(protocol());
        }
        let state = bounded(&value["state"], 16)?;
        if !matches!(state.as_str(), "OPEN" | "CLOSED" | "MERGED") {
            return Err(protocol());
        }
        let labels = value["labels"]["nodes"]
            .as_array()
            .filter(|v| v.len() <= 100)
            .ok_or_else(protocol)?
            .iter()
            .map(|l| bounded(&l["name"], 256))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            number,
            head,
            state,
            draft: value["isDraft"].as_bool().ok_or_else(protocol)?,
            labels,
            labels_more: value["labels"]["pageInfo"]["hasNextPage"]
                .as_bool()
                .ok_or_else(protocol)?,
            timeline: Page::new(&value["timelineItems"])?,
        })
    }
    fn matches(&self, lane: &Lane) -> Result<bool, LaneError> {
        let matches = lane.configuration.labels.as_slice().iter().all(|required| {
            self.labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case(required))
        });
        if !matches && self.labels_more {
            return Err(failure(
                "LANE_INCOMPLETE",
                "truncated label metadata cannot establish a scope exit; progress retained",
            ));
        }
        Ok(matches)
    }
}

fn current(db: &rusqlite::Connection, lane: &Lane) -> Result<bool, LaneError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM lanes WHERE name=?1 AND generation=?2 AND paused=0)",
        params![lane.name, lane.generation],
        |r| r.get(0),
    )?)
}

fn apply_discovery(
    db: &rusqlite::Connection,
    lane: &Lane,
    snapshots: &[Snapshot],
    page: &Page,
    now: i64,
) -> Result<(), LaneError> {
    for snapshot in snapshots {
        if snapshot.state != "OPEN" || !snapshot.matches(lane)? {
            continue;
        }
        let existing: Option<bool> = db
            .query_row(
                "SELECT in_scope=1 AND tracking=1 FROM prs WHERE lane=?1 AND number=?2",
                params![lane.name, snapshot.number],
                |r| r.get(0),
            )
            .optional()?;
        if existing == Some(true) {
            continue;
        }
        let ticket = next_ticket(db)?;
        db.execute("INSERT INTO prs(lane,number,head,state,draft,in_scope,initializing,timeline_cursor,due,ticket) VALUES(?1,?2,?3,?4,?5,1,1,?6,?7,?8) ON CONFLICT(lane,number) DO UPDATE SET head=excluded.head,state=excluded.state,draft=excluded.draft,in_scope=1,tracking=1,initializing=1,timeline_cursor=excluded.timeline_cursor,threads_after=NULL,due=excluded.due,ticket=excluded.ticket,last_error=NULL,resync=0,gaps=gaps+1",params![lane.name,snapshot.number,snapshot.head,snapshot.state,snapshot.draft,snapshot.timeline.after,now,ticket])?;
        db.execute(
            "DELETE FROM threads WHERE lane=?1 AND number=?2",
            params![lane.name, snapshot.number],
        )?;
        let mut reasons: BTreeSet<String> = ["discovered".to_owned()].into();
        if existing.is_some() {
            reasons.insert("reentry-gap".to_owned());
        }
        stage(
            db,
            lane,
            snapshot.number,
            &snapshot.head,
            &snapshot.state,
            reasons,
        )?;
    }
    let old: Option<String> = db.query_row(
        "SELECT discovery_after FROM lanes WHERE name=?1",
        [&lane.name],
        |r| r.get(0),
    )?;
    if page.more && old == page.after {
        return Err(protocol());
    }
    let ticket = next_ticket(db)?;
    db.execute("UPDATE lanes SET discovery_after=?2,next_scan=?3,ticket=?4,last_scan_at=CASE WHEN ?5 THEN last_scan_at ELSE ?6 END,last_error=NULL WHERE name=?1",params![lane.name,if page.more{page.after.clone()}else{None},if page.more{now}else{scheduled(now)},ticket,page.more,now])?;
    Ok(())
}

fn apply_observation(
    db: &rusqlite::Connection,
    job: &Job,
    snapshot: &Snapshot,
    threads: &Page,
    now: i64,
) -> Result<(), LaneError> {
    if snapshot.number != job.number {
        return Err(protocol());
    }
    let lane = &job.lane;
    let (head,state,draft,initializing,old_cursor,old_threads):(String,String,bool,bool,Option<String>,Option<String>)=db.query_row("SELECT head,state,draft,initializing,timeline_cursor,threads_after FROM prs WHERE lane=?1 AND number=?2",params![lane.name,job.number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    let scope = snapshot.matches(lane)?;
    let incomplete = snapshot.timeline.more || threads.more;
    let tracking = scope && (snapshot.state == "OPEN" || incomplete);
    let mut reasons = BTreeSet::new();
    if head != snapshot.head {
        reasons.insert("head".to_owned());
    }
    if state != snapshot.state || draft != snapshot.draft {
        reasons.insert("lifecycle".to_owned());
    }
    if !scope {
        reasons.insert("left-scope".to_owned());
        if incomplete {
            reasons.insert("scope-exit-gap".to_owned());
        }
    }
    let resync = matches!(job.query, Query::Observe { resync: true, .. });
    if !resync {
        if snapshot.timeline.more && snapshot.timeline.after == old_cursor {
            return Err(protocol());
        }
        for node in &snapshot.timeline.nodes {
            reasons.insert(event_kind(node)?);
        }
    }
    if threads.more && threads.after == old_threads {
        return Err(protocol());
    }
    for node in &threads.nodes {
        let id = bounded(&node["id"], 256)?;
        let signature = thread_signature(node)?;
        let previous: Option<String> = db
            .query_row(
                "SELECT signature FROM threads WHERE lane=?1 AND number=?2 AND id=?3",
                params![lane.name, job.number, id],
                |r| r.get(0),
            )
            .optional()?;
        if !initializing && previous.as_deref() != Some(&signature) {
            reasons.insert("review-comments".to_owned());
        }
        db.execute("INSERT INTO threads(lane,number,id,signature) VALUES(?1,?2,?3,?4) ON CONFLICT(lane,number,id) DO UPDATE SET signature=excluded.signature",params![lane.name,job.number,id,signature])?;
    }
    if initializing {
        reasons.insert("discovered".to_owned());
    }
    let initializing = initializing && threads.more && tracking;
    // GitHub returns a null endCursor on an empty completed connection. The
    // consumed prefix still exists; clearing it would replay history next poll.
    let timeline_cursor = snapshot.timeline.after.clone().or(old_cursor);
    let ticket = next_ticket(db)?;
    db.execute("UPDATE prs SET head=?3,state=?4,draft=?5,in_scope=?6,initializing=?7,timeline_cursor=?8,threads_after=?9,due=?10,ticket=?11,last_success=CASE WHEN ?12 THEN last_success ELSE ?13 END,last_error=NULL,resync=0,tracking=?14,gaps=gaps+?15 WHERE lane=?1 AND number=?2",params![lane.name,job.number,snapshot.head,snapshot.state,snapshot.draft,scope,initializing,timeline_cursor,if threads.more{threads.after.clone()}else{None},if incomplete{now}else{scheduled(now)},ticket,incomplete,now,tracking,i64::from(!scope&&incomplete)])?;
    if !reasons.is_empty() {
        stage(
            db,
            lane,
            job.number,
            &snapshot.head,
            &snapshot.state,
            reasons,
        )?;
    }
    Ok(())
}

fn thread_signature(node: &Value) -> Result<String, LaneError> {
    let count = node["comments"]["totalCount"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or_else(protocol)?;
    let resolved = node["isResolved"].as_bool().ok_or_else(protocol)?;
    let nodes = node["comments"]["nodes"]
        .as_array()
        .filter(|v| v.len() <= 1)
        .ok_or_else(protocol)?;
    if count > 0 && nodes.is_empty() {
        return Err(protocol());
    }
    let last = nodes
        .first()
        .map(|last| -> Result<Value, LaneError> {
            Ok(json!({"id":bounded(&last["id"],256)?,"updatedAt":bounded(&last["updatedAt"],64)?}))
        })
        .transpose()?;
    Ok(json!({"count":count,"resolved":resolved,"last":last}).to_string())
}

fn event_kind(node: &Value) -> Result<String, LaneError> {
    let kind = bounded(&node["__typename"], 64)?;
    if !matches!(
        kind.as_str(),
        "PullRequestCommit"
            | "IssueComment"
            | "PullRequestReview"
            | "PullRequestReviewThread"
            | "HeadRefForcePushedEvent"
            | "MergedEvent"
            | "ClosedEvent"
            | "ReopenedEvent"
            | "ConvertToDraftEvent"
            | "ReadyForReviewEvent"
            | "LabeledEvent"
            | "UnlabeledEvent"
            | "ReviewDismissedEvent"
            | "ReviewRequestedEvent"
            | "ReviewRequestRemovedEvent"
    ) {
        return Err(protocol());
    }
    Ok(kind)
}

fn cursor(value: &Value) -> Result<Option<String>, LaneError> {
    if value.is_null() {
        Ok(None)
    } else {
        bounded(value, 1024).map(Some)
    }
}
fn bounded(value: &Value, max: usize) -> Result<String, LaneError> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(protocol)
}
fn stage(
    db: &rusqlite::Connection,
    lane: &Lane,
    number: i64,
    head: &str,
    state: &str,
    mut reasons: BTreeSet<String>,
) -> Result<(), LaneError> {
    let old: Option<String> = db
        .query_row(
            "SELECT reasons FROM pending WHERE lane=?1 AND number=?2 AND generation=?3",
            params![lane.name, number, lane.generation],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        reasons.extend(serde_json::from_str::<BTreeSet<String>>(&old).map_err(|_| protocol())?);
    }
    db.execute("INSERT INTO pending(lane,number,generation,head,state,reasons) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(lane,number,generation) DO UPDATE SET head=excluded.head,state=excluded.state,reasons=excluded.reasons",params![lane.name,number,lane.generation,head,state,json!(reasons).to_string()])?;
    Ok(())
}
