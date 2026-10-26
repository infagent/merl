use merl_lane::{Configuration, GitHub, Labels, LaneError, MailSink, Query, Reply, Store};
use merl_mail::{Folder, Key, Mailbox};
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub struct Integrity {
    home: PathBuf,
    attempts: Vec<(String, String)>,
    messages: Vec<Value>,
    states: Vec<Value>,
    errors: Vec<String>,
}

struct Source {
    head: String,
    activity: bool,
    change: Option<(PathBuf, bool)>,
    rate_limit: bool,
}
impl Source {
    fn original() -> Self {
        Self {
            head: "a".repeat(40),
            activity: false,
            change: None,
            rate_limit: false,
        }
    }
    fn snapshot(&self) -> Value {
        json!({"number":1,"headRefOid":self.head,"state":"OPEN","isDraft":false,"updatedAt":"2026-01-01T00:00:00Z","labels":{"nodes":[{"name":"review-ready"}],"pageInfo":{"hasNextPage":false}},"timelineItems":Self::timeline(self.activity),"reviewThreads":{"edges":[],"pageInfo":{"endCursor":null,"hasNextPage":false}}})
    }
    fn timeline(include: bool) -> Value {
        json!({"edges":if include{vec![json!({"cursor":"t:1","node":{"__typename":"IssueComment"}})]}else{Vec::new()},"pageInfo":{"endCursor":if include{Some("t:1")}else{None},"hasNextPage":false}})
    }
}
impl GitHub for Source {
    fn fetch(
        &mut self,
        query: &Query,
        _clock: &impl Fn() -> Result<i64, LaneError>,
    ) -> Result<Reply, LaneError> {
        if let Some((home, pause)) = self.change.take() {
            let mut changed = Store::open(&home)?;
            if pause {
                changed.stop(&"review".parse()?, 1)?;
            } else {
                changed.update(
                    &"review".parse()?,
                    &Labels::new(vec!["other".into()])?,
                    1,
                    1000,
                )?;
            }
        }
        if self.rate_limit {
            let mut error: LaneError = io::Error::from(io::ErrorKind::WouldBlock).into();
            error.code = "LANE_RATE_LIMITED";
            error.retry_at = Some(601_000);
            return Err(error);
        }
        let mut snapshot = self.snapshot();
        let repository = match query {
            Query::Discover { .. } => {
                json!({"pullRequests":{"edges":[{"cursor":"d:1","node":snapshot}],"pageInfo":{"endCursor":"d:1","hasNextPage":false}}})
            }
            Query::Observe { after, resync, .. } => {
                snapshot["timelineItems"] =
                    Self::timeline(self.activity && (*resync || after.as_deref() != Some("t:1")));
                json!({"pullRequest":snapshot})
            }
        };
        Ok(Reply {
            document: json!({"data":{"repository":repository}}),
            not_before: 0,
        })
    }
}

struct Ambiguous<'a> {
    mail: &'a mut Mailbox,
    attempts: &'a mut Vec<(String, String)>,
    interrupt: bool,
}
impl MailSink for Ambiguous<'_> {
    fn publish(
        &mut self,
        id: &Key,
        recipient: &Key,
        body: &str,
        now: i64,
    ) -> Result<(), LaneError> {
        self.attempts.push((id.as_str().into(), body.into()));
        self.mail.publish(id, recipient, body, now)?;
        if self.interrupt {
            self.interrupt = false;
            return Err(io::Error::from(io::ErrorKind::Interrupted).into());
        }
        Ok(())
    }
}

impl Integrity {
    pub fn given_a_lane_with_a_mail_publication_boundary(name: &str) -> Self {
        let home =
            std::env::temp_dir().join(format!("merl-lane-integrity-{name}-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        Self {
            home,
            attempts: Vec::new(),
            messages: Vec::new(),
            states: Vec::new(),
            errors: Vec::new(),
        }
    }
    fn configure(home: &Path) -> (Store, Mailbox) {
        let mail = Mailbox::open(home).unwrap();
        mail.register(&"reviewer".parse().unwrap()).unwrap();
        let mut store = Store::open(home).unwrap();
        store
            .start(
                &"review".parse().unwrap(),
                Some(Configuration {
                    repository: "example/widgets".parse().unwrap(),
                    agent: "reviewer".parse().unwrap(),
                    labels: Labels::new(vec!["review-ready".into()]).unwrap(),
                }),
                None,
                0,
            )
            .unwrap();
        (store, mail)
    }
    pub fn when_publication_succeeds_but_its_acknowledgment_is_lost_before_new_activity(
        mut self,
    ) -> Self {
        let (mut store, mut mail) = Self::configure(&self.home);
        let mut source = Source::original();
        store.scan_at(&mut source, &mut mail, 1000, 1).unwrap();
        let mut sink = Ambiguous {
            mail: &mut mail,
            attempts: &mut self.attempts,
            interrupt: true,
        };
        self.errors.push(
            store
                .scan_at(&mut source, &mut sink, 1000, 1)
                .unwrap_err()
                .code
                .into(),
        );
        drop(store);
        let mut store = Store::open(&self.home).unwrap();
        source.head = "b".repeat(40);
        source.activity = true;
        let mut sink = Ambiguous {
            mail: &mut mail,
            attempts: &mut self.attempts,
            interrupt: false,
        };
        store.scan_at(&mut source, &mut sink, 62_000, 16).unwrap();
        let key = "reviewer".parse().unwrap();
        let headers = mail.list(&key, None, 100, Folder::All).unwrap();
        self.messages = headers["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                mail.show(
                    &key,
                    &m["id"].as_str().unwrap().parse().unwrap(),
                    false,
                    62_000,
                )
                .unwrap()["message"]
                    .clone()
            })
            .collect();
        self
    }
    pub fn then_restart_retries_the_frozen_batch_and_new_activity_has_a_separate_id(self) {
        assert_eq!(self.errors, ["LANE_IO"]);
        assert_eq!(self.attempts.len(), 3);
        assert_eq!(
            self.attempts[0], self.attempts[1],
            "ambiguous retry keeps exact ID and bytes"
        );
        assert_ne!(self.attempts[1].0, self.attempts[2].0);
        assert!(
            self.attempts[0].1.contains(&"a".repeat(40))
                && !self.attempts[0].1.contains(&"b".repeat(40))
        );
        assert!(self.attempts[2].1.contains(&"b".repeat(40)));
        assert_eq!(
            self.messages.len(),
            2,
            "publication retry is unchanged, not a duplicate review"
        );
    }
    pub fn when_the_lane_is_paused_or_updated_while_its_source_reply_is_in_flight(
        mut self,
    ) -> Self {
        for pause in [true, false] {
            let home = self.home.join(if pause { "pause" } else { "update" });
            let (mut store, mut mail) = Self::configure(&home);
            let mut source = Source::original();
            source.change = Some((home, pause));
            store.scan_at(&mut source, &mut mail, 1000, 1).unwrap();
            self.states.push(store.status(None).unwrap());
            self.states
                .push(store.queue(&"review".parse().unwrap(), 0, 100).unwrap());
            self.states.push(
                mail.list(&"reviewer".parse().unwrap(), None, 100, Folder::All)
                    .unwrap(),
            );
        }
        self
    }
    pub fn then_the_old_reply_cannot_commit_progress_or_delivery(self) {
        for group in self.states.as_chunks::<3>().0 {
            assert_eq!(group[0]["lanes"][0]["generation"], 2);
            assert!(group[0]["lanes"][0]["last_scan_at"].is_null());
            assert!(group[0]["lanes"][0]["discovery_after"].is_null());
            assert!(group[1]["prs"].as_array().unwrap().is_empty());
            assert!(group[2]["messages"].as_array().unwrap().is_empty());
        }
    }

    pub fn when_configuration_changes_while_the_provider_returns_a_rate_limit(mut self) -> Self {
        let (mut store, mut mail) = Self::configure(&self.home);
        let mut source = Source::original();
        source.change = Some((self.home.clone(), false));
        source.rate_limit = true;
        store.scan_at(&mut source, &mut mail, 1000, 1).unwrap();
        self.states.push(store.status(None).unwrap());
        self.errors.push(
            store
                .scan_at(&mut source, &mut mail, 601_001, 1)
                .unwrap_err()
                .code
                .into(),
        );
        self.states.push(store.status(None).unwrap());
        self
    }

    pub fn then_only_content_progress_is_fenced_and_the_server_cooldown_is_preserved(self) {
        assert_eq!(self.states[0]["lanes"][0]["generation"], 2);
        assert!(self.states[0]["lanes"][0]["last_scan_at"].is_null());
        assert_eq!(self.states[0]["provider_retry_at"], 601_000);
        assert_eq!(self.errors, ["LANE_RATE_LIMITED"]);
        assert!(
            self.states[1]["provider_retry_at"].as_i64().unwrap() >= 721_001,
            "repeated secondary limits increase the retry delay"
        );
    }
}
impl Drop for Integrity {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.home).unwrap();
    }
}
