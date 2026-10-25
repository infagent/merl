//! Serialized dispatch and explicit host binding; no model calls while idle.

use super::{Key, MailError, Mailbox, failure, require_identity};
use rusqlite::params;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

// A minute allows asynchronous receipt without rapid empty model turns while
// bounding recovery latency. New work after a receipted batch bypasses this wait.
const RECEIPT_GRACE_MILLIS: i64 = 60_000;
// The crash lease exceeds the ten-second host-command deadline, so a crashed
// dispatcher does not immediately race its possibly accepted enqueue.
const DISPATCH_LEASE_MILLIS: i64 = 30_000;
const HOST_TIMEOUT_SECS: u64 = 10;

/// An exact Codex session UUID and a verified local host socket.
#[derive(Debug)]
pub struct Binding {
    session: String,
    socket: PathBuf,
    fingerprint: String,
}
impl Binding {
    /// Validates a UUID and fingerprints the existing Unix socket.
    ///
    /// # Errors
    /// Returns `MAIL_INPUT` or `MAIL_HOST_UNAVAILABLE`; never starts a host.
    pub fn new(session: &str, socket: &Path) -> Result<Self, MailError> {
        if session.len() != 36
            || !session.bytes().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_hexdigit()
                }
            })
        {
            return Err(failure(
                "MAIL_INPUT",
                "session must be an exact UUID, not a name",
            ));
        }
        let text = socket
            .to_str()
            .ok_or_else(|| failure("MAIL_INPUT", "socket path must be UTF-8"))?;
        if !socket.is_absolute() || text.len() > 512 || text.chars().any(char::is_control) {
            return Err(failure(
                "MAIL_INPUT",
                "socket must be an absolute bounded local path",
            ));
        }
        Ok(Self {
            session: session.to_ascii_lowercase(),
            socket: socket.into(),
            fingerprint: fingerprint(socket)?,
        })
    }
}

impl Mailbox {
    /// Explicitly replaces one binding under its current generation guard.
    ///
    /// # Errors
    /// Rejects stale generation, missing identity, or concurrent host dispatch.
    pub fn bind(
        &self,
        agent: &Key,
        binding: &Binding,
        expected: i64,
        now: i64,
    ) -> Result<Value, MailError> {
        let _guard = self.dispatch_guard()?;
        require_identity(&self.db, agent)?;
        let changed=self.db.execute("UPDATE identities SET generation=generation+1,session=?2,socket=?3,socket_identity=?4 WHERE name=?1 AND generation=?5 AND generation<9223372036854775807",params![agent.as_str(),binding.session,binding.socket.to_str(),binding.fingerprint,expected])?;
        if changed != 1 {
            return Err(failure(
                "MAIL_BINDING_CONFLICT",
                "inspect current generation before explicitly binding again",
            ));
        }
        self.db.execute(
            "UPDATE notifications SET next_at=?2,state='pending' WHERE agent=?1",
            params![agent.as_str(), now],
        )?;
        Ok(
            json!({"outcome":"bound","agent":agent.as_str(),"generation":expected+1,"session":binding.session}),
        )
    }

    /// Explicitly disables host notification without discarding durable messages.
    ///
    /// # Errors
    /// Rejects stale generation, unknown identity, or concurrent dispatch.
    pub fn unbind(&self, agent: &Key, expected: i64) -> Result<Value, MailError> {
        let _guard = self.dispatch_guard()?;
        require_identity(&self.db, agent)?;
        let changed=self.db.execute("UPDATE identities SET generation=generation+1,session=NULL,socket=NULL,socket_identity=NULL WHERE name=?1 AND generation=?2 AND generation<9223372036854775807",params![agent.as_str(),expected])?;
        if changed != 1 {
            return Err(failure(
                "MAIL_BINDING_CONFLICT",
                "binding generation changed",
            ));
        }
        Ok(json!({"outcome":"unbound","generation":expected+1}))
    }

    /// Holds the scoped foreground-watch lock until the returned handle is dropped.
    ///
    /// # Errors
    /// Returns `MAIL_BUSY` if another watcher owns this mailbox.
    pub fn watch_guard(&self) -> Result<File, MailError> {
        lock(&self.directory.join("watch.lock"))
    }

    fn dispatch_guard(&self) -> Result<File, MailError> {
        lock(&self.directory.join("dispatch.lock"))
    }

    /// Attempts at most 100 due recipients, with one bounded pointer per recipient.
    ///
    /// Binding changes serialize with dispatch; host I/O holds no SQLite transaction.
    /// A crash after enqueue may cause a duplicate notification after the lease expires.
    ///
    /// # Errors
    /// Returns a lock/storage error. Host failures remain durable notification facts.
    pub fn dispatch(&self, codex: &Path, merl: &Path, now: i64) -> Result<Value, MailError> {
        let _guard = self.dispatch_guard()?;
        if !codex.is_absolute() || !merl.is_absolute() {
            return Err(failure(
                "MAIL_INPUT",
                "host and Merl executables must use absolute paths",
            ));
        }
        let agents = {
            let mut s=self.db.prepare("SELECT n.agent FROM notifications n WHERE n.next_at<=?1 AND EXISTS(SELECT 1 FROM messages m JOIN handling h ON h.message=m.id WHERE m.recipient=n.agent AND h.received_at IS NULL) ORDER BY n.agent LIMIT 100")?;
            s.query_map([now], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut results = Vec::new();
        for agent in agents {
            results.push(self.notify(&agent, codex, merl, now)?);
        }
        Ok(json!({"notifications":results}))
    }

    fn notify(&self, agent: &str, codex: &Path, merl: &Path, now: i64) -> Result<Value, MailError> {
        let binding = self.db.query_row(
            "SELECT generation,session,socket,socket_identity FROM identities WHERE name=?1",
            [agent],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )?;
        let prior: i64 = self.db.query_row(
            "SELECT attempts FROM notifications WHERE agent=?1",
            [agent],
            |r| r.get(0),
        )?;
        let attempts = prior.saturating_add(1);
        // This persisted lease bounds crash recovery without holding a database lock
        // while the host command runs. Enqueue itself has no idempotency guarantee.
        self.db.execute("UPDATE notifications SET state='dispatching',next_at=?2,attempts=?3,generation=?4 WHERE agent=?1",params![agent,now.saturating_add(DISPATCH_LEASE_MILLIS),attempts,binding.0])?;
        let state = if let (Some(session), Some(socket), Some(recorded)) =
            (&binding.1, &binding.2, &binding.3)
        {
            match fingerprint(Path::new(socket)) {
                Ok(current) if current == *recorded => {
                    let pointer = format!(
                        "New Merl mail. Run {} mail list --home {} --agent {} --pending --json; drain all pages, read each ID, then record mail receipt. Receipt does not complete work. Duplicate notifications are safe.",
                        quote(
                            merl.to_str()
                                .ok_or_else(|| failure("MAIL_INPUT", "executable must be UTF-8"))?
                        ),
                        quote(
                            self.home
                                .to_str()
                                .ok_or_else(|| failure("MAIL_INPUT", "home must be UTF-8"))?
                        ),
                        agent
                    );
                    if pointer.len() > 2048 {
                        "pointer_limit"
                    } else if enqueue(codex, socket, session, &pointer)? {
                        "enqueued"
                    } else {
                        "failed"
                    }
                }
                _ => "stale_binding",
            }
        } else {
            "offline"
        };
        let shift = u32::try_from(attempts.clamp(0, 6))
            .map_err(|_| failure("MAIL_STORAGE", "invalid attempt counter"))?;
        // Enqueue is asynchronous: allow a normal agent turn to record receipt
        // before another wake. Failed host calls retain faster bounded backoff.
        let delay = if state == "enqueued" {
            RECEIPT_GRACE_MILLIS
        } else {
            (1000_i64 << shift).min(60_000)
        };
        let error = if state == "enqueued" {
            None
        } else {
            Some(state)
        };
        self.db.execute(
            "UPDATE notifications SET state=?2,next_at=?3,last_error=?4 WHERE agent=?1",
            params![agent, state, now.saturating_add(delay), error],
        )?;
        Ok(
            json!({"agent":agent,"state":state,"attempts":attempts,"binding_generation":binding.0,"next_attempt_at":now.saturating_add(delay)}),
        )
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
fn lock(path: &Path) -> Result<File, MailError> {
    let mut o = fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let f = o.open(path)?;
    f.try_lock().map_err(|_| {
        failure(
            "MAIL_BUSY",
            "another dispatcher or binding change owns this mailbox; retry later",
        )
    })?;
    Ok(f)
}
fn enqueue(codex: &Path, socket: &str, session: &str, pointer: &str) -> Result<bool, MailError> {
    let child = Command::new(codex)
        .args([
            "queue",
            "--remote",
            &format!("unix://{socket}"),
            "--thread",
            session,
            "--message",
            pointer,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Ok(false);
    };
    let deadline = Instant::now() + Duration::from_secs(HOST_TIMEOUT_SECS);
    loop {
        let Ok(status) = child.try_wait() else {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        };
        if let Some(exit) = status {
            return Ok(exit.success());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(unix)]
fn fingerprint(socket: &Path) -> Result<String, MailError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let m = fs::metadata(socket).map_err(|_| {
        failure(
            "MAIL_HOST_UNAVAILABLE",
            "host socket unavailable; start the host explicitly then bind",
        )
    })?;
    if !m.file_type().is_socket() {
        return Err(failure(
            "MAIL_HOST_UNAVAILABLE",
            "configured path is not a Unix host socket",
        ));
    }
    Ok(format!(
        "{}:{}:{}:{}",
        m.dev(),
        m.ino(),
        m.ctime(),
        m.ctime_nsec()
    ))
}
#[cfg(not(unix))]
fn fingerprint(_: &Path) -> Result<String, MailError> {
    Err(failure(
        "MAIL_HOST_UNAVAILABLE",
        "this host adapter requires Unix sockets",
    ))
}
