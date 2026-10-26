//! Persistent shared scope and roster, with atomic ownership of mail identities.
//!
//! Team tables are additive to mail-v1. Joining creates the membership and a
//! fresh identity in one transaction; idempotent registration is never evidence
//! that an existing identity belongs to this membership. References are inert.

use crate::{Key, MAX_BODY_BYTES, MAX_PAGE, MailError, Mailbox, failure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::{Value, json};

/// Maximum role bytes, allowing ordinary responsibility labels without enums.
pub const MAX_ROLE_BYTES: usize = 128;
/// Maximum pointers per brief; detailed project material stays outside Merl.
pub const MAX_REFS: usize = 16;
/// Maximum bytes per inert pointer, bounding storage and roster detail reads.
pub const MAX_REF_BYTES: usize = 2048;

/// Validated shared scope with optional descriptive coordinator metadata.
#[derive(Debug)]
pub struct TeamSpec {
    name: Key,
    coordinator: Option<Key>,
    project: Option<Key>,
    scope: String,
    refs: Vec<String>,
}
impl TeamSpec {
    /// Validates bounded scope and inert guidance references.
    ///
    /// # Errors
    /// Returns `TEAM_INPUT` when a brief or reference exceeds its limits.
    pub fn new(
        name: Key,
        coordinator: Option<Key>,
        scope: String,
        refs: Vec<String>,
    ) -> Result<Self, MailError> {
        brief(&scope)?;
        references(&refs)?;
        Ok(Self {
            name,
            coordinator,
            project: None,
            scope,
            refs,
        })
    }

    /// Records an explicit navigation reference, checked for existence on creation.
    #[must_use]
    pub fn with_project(mut self, project: Key) -> Self {
        self.project = Some(project);
        self
    }
}

/// Validated member guidance; a role is an optional free-form label.
#[derive(Debug)]
pub struct MemberSpec {
    member: Key,
    role: Option<String>,
    description: String,
    refs: Vec<String>,
}
impl MemberSpec {
    /// Validates saved starting guidance, optional role and inert pointers.
    ///
    /// # Errors
    /// Returns `TEAM_INPUT` for oversized or empty guidance, roles or references.
    pub fn new(
        member: Key,
        role: Option<String>,
        description: String,
        refs: Vec<String>,
    ) -> Result<Self, MailError> {
        brief(&description)?;
        references(&refs)?;
        if role.as_ref().is_some_and(|r| {
            r.is_empty() || r.len() > MAX_ROLE_BYTES || r.chars().any(char::is_control)
        }) {
            return Err(failure(
                "TEAM_INPUT",
                "role must be 1..128 bytes without control characters",
            ));
        }
        Ok(Self {
            member,
            role,
            description,
            refs,
        })
    }
}

fn brief(value: &str) -> Result<(), MailError> {
    if value.is_empty() || value.len() > MAX_BODY_BYTES {
        return Err(failure(
            "TEAM_INPUT",
            "guidance must be 1..8192 UTF-8 bytes",
        ));
    }
    Ok(())
}
fn references(refs: &[String]) -> Result<(), MailError> {
    if refs.len() > MAX_REFS
        || refs
            .iter()
            .any(|r| r.is_empty() || r.len() > MAX_REF_BYTES || r.chars().any(char::is_control))
    {
        return Err(failure(
            "TEAM_INPUT",
            "at most 16 references of 1..2048 bytes without control characters",
        ));
    }
    Ok(())
}

fn schema_present(db: &Connection) -> Result<bool, MailError> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='merl_team_meta')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    let version: Option<i64> = db
        .query_row("SELECT version FROM merl_team_meta WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    if version != Some(1) {
        return Err(failure("TEAM_SCHEMA", "unsupported team schema"));
    }
    Ok(true)
}
fn require_team(db: &Connection, team: &Key) -> Result<(), MailError> {
    if !schema_present(db)?
        || !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM merl_teams WHERE name=?1)",
            [team.as_str()],
            |r| r.get::<_, bool>(0),
        )?
    {
        return Err(failure("TEAM_NOT_FOUND", "team does not exist"));
    }
    Ok(())
}
fn refs_json(raw: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(raw).map_err(|_| rusqlite::Error::InvalidQuery)
}

impl Mailbox {
    /// Pages scope headers, optionally matching one explicit project reference.
    ///
    /// No role, inherited authority, recursive scope traversal or guidance read
    /// participates in this discovery.
    ///
    /// # Errors
    /// Returns `TEAM_INPUT` for invalid limits/cursors, or schema/storage errors.
    /// A supplied project must name an existing scope.
    pub fn teams(
        &self,
        project: Option<&Key>,
        after: Option<&Key>,
        limit: usize,
    ) -> Result<Value, MailError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(failure("TEAM_INPUT", "limit must be 1..100"));
        }
        if let Some(project) = project {
            require_team(&self.db, project)?;
        }
        if !schema_present(&self.db)? {
            return Ok(json!({"teams":[],"next_after":null}));
        }
        if let Some(after) = after {
            let valid: bool = self.db.query_row("SELECT EXISTS(SELECT 1 FROM merl_teams WHERE name=?1 AND (?2 IS NULL OR project=?2))", params![after.as_str(),project.map(Key::as_str)], |r| r.get(0))?;
            if !valid {
                return Err(failure(
                    "TEAM_INPUT",
                    "cursor does not belong to selected scope list",
                ));
            }
        }
        let mut statement = self.db.prepare("SELECT name,project,coordinator,generation FROM merl_teams WHERE name>?1 AND (?2 IS NULL OR project=?2) ORDER BY name LIMIT ?3")?;
        let count =
            i64::try_from(limit + 1).map_err(|_| failure("TEAM_INPUT", "invalid page size"))?;
        let mut teams = statement.query_map(params![after.map_or("",Key::as_str),project.map(Key::as_str),count], |r| Ok(json!({"team":r.get::<_,String>(0)?,"project":r.get::<_,Option<String>>(1)?,"coordinator":r.get::<_,Option<String>>(2)?,"generation":r.get::<_,i64>(3)?})))?.collect::<Result<Vec<_>,_>>()?;
        let next_after = if teams.len() > limit {
            teams.truncate(limit);
            teams.last().map(|t| t["team"].clone())
        } else {
            None
        };
        Ok(json!({"teams":teams,"next_after":next_after}))
    }

    /// Looks up a qualified display name without loading member guidance.
    ///
    /// # Errors
    /// Returns a team-schema or storage error; legacy identities return `None`.
    pub fn qualified_member(&self, mail_key: &Key) -> Result<Option<String>, MailError> {
        if !schema_present(&self.db)? {
            return Ok(None);
        }
        Ok(self
            .db
            .query_row(
                "SELECT team,member FROM merl_team_members WHERE mail_key=?1",
                [mail_key.as_str()],
                |r| {
                    Ok(format!(
                        "{}/{}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?
                    ))
                },
            )
            .optional()?)
    }

    /// Creates shared scope, or recognizes an exact immutable creation retry.
    ///
    /// # Errors
    /// Returns `TEAM_CONFLICT` for changed inputs, `TEAM_NOT_FOUND` for a missing
    /// project reference, and schema/storage failures.
    pub fn create_team(&mut self, spec: &TeamSpec) -> Result<Value, MailError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema_present(&tx)?;
        tx.execute_batch(include_str!("team-schema.sql"))?;
        let old: Option<(Option<String>, String, String, Option<String>)> = tx
            .query_row(
                "SELECT coordinator,scope,refs,project FROM merl_teams WHERE name=?1",
                [spec.name.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let refs = json!(spec.refs).to_string();
        let outcome = if let Some(old) = old {
            if old
                != (
                    spec.coordinator.as_ref().map(ToString::to_string),
                    spec.scope.clone(),
                    refs,
                    spec.project.as_ref().map(ToString::to_string),
                )
            {
                return Err(failure(
                    "TEAM_CONFLICT",
                    "team exists with different saved scope or guidance",
                ));
            }
            "unchanged"
        } else {
            if let Some(project) = &spec.project {
                require_team(&tx, project)?;
            }
            tx.execute(
                "INSERT INTO merl_teams(name,coordinator,scope,refs,project) VALUES(?1,?2,?3,?4,?5)",
                params![
                    spec.name.as_str(),
                    spec.coordinator.as_ref().map(Key::as_str),
                    spec.scope,
                    refs,
                    spec.project.as_ref().map(Key::as_str)
                ],
            )?;
            "created"
        };
        tx.commit()?;
        let mut result = self.show_team(&spec.name)?;
        result["outcome"] = json!(outcome);
        Ok(result)
    }

    /// Atomically joins a member and creates its fresh mail identity.
    ///
    /// The caller supplies a fresh opaque key, making ID generation controllable.
    /// An exact membership retry uses the committed key, ignoring the new candidate.
    ///
    /// # Errors
    /// Returns `TEAM_IDENTITY_CONFLICT` for a pre-existing candidate identity,
    /// `TEAM_CONFLICT` for changed guidance, or a missing-team/schema/storage error.
    pub fn join_team(
        &mut self,
        team: &Key,
        spec: &MemberSpec,
        candidate: &Key,
    ) -> Result<Value, MailError> {
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_team(&tx, team)?;
        let old: Option<(Option<String>, String, String, String)> = tx.query_row("SELECT role,description,refs,mail_key FROM merl_team_members WHERE team=?1 AND member=?2", params![team.as_str(),spec.member.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let refs = json!(spec.refs).to_string();
        let outcome = if let Some((role, description, old_refs, key)) = old {
            if (role, description, old_refs) != (spec.role.clone(), spec.description.clone(), refs)
            {
                return Err(failure(
                    "TEAM_CONFLICT",
                    "member exists with different saved guidance or role",
                ));
            }
            crate::require_identity(&tx, &key.parse()?)?;
            "unchanged"
        } else {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE name=?1)",
                [candidate.as_str()],
                |r| r.get(0),
            )?;
            if exists {
                return Err(failure(
                    "TEAM_IDENTITY_CONFLICT",
                    "candidate identity already exists; no identity was adopted",
                ));
            }
            tx.execute(
                "INSERT INTO identities(name) VALUES(?1)",
                [candidate.as_str()],
            )?;
            tx.execute("INSERT INTO merl_team_members(team,member,mail_key,role,description,refs) VALUES(?1,?2,?3,?4,?5,?6)", params![team.as_str(),spec.member.as_str(),candidate.as_str(),spec.role,spec.description,refs])?;
            "joined"
        };
        tx.commit()?;
        let mut result = self.team_member(team, &spec.member)?;
        result["outcome"] = json!(outcome);
        Ok(result)
    }

    /// Reads the shared brief without loading referenced documents.
    ///
    /// # Errors
    /// Returns a missing-team, unsupported-schema or storage error.
    pub fn show_team(&self, team: &Key) -> Result<Value, MailError> {
        require_team(&self.db, team)?;
        Ok(self.db.query_row("SELECT coordinator,scope,refs,generation,project FROM merl_teams WHERE name=?1", [team.as_str()], |r| Ok(json!({"team":team.as_str(),"coordinator":r.get::<_,Option<String>>(0)?,"scope":r.get::<_,String>(1)?,"guidance_refs":refs_json(&r.get::<_,String>(2)?)?,"generation":r.get::<_,i64>(3)?,"project":r.get::<_,Option<String>>(4)?})))?)
    }

    /// Reads one member's durable starting guidance and stable identity.
    ///
    /// # Errors
    /// Returns `TEAM_MEMBER_MISSING`, or a missing-team/schema/storage error.
    pub fn team_member(&self, team: &Key, member: &Key) -> Result<Value, MailError> {
        require_team(&self.db, team)?;
        self.db.query_row("SELECT mail_key,role,description,refs,generation FROM merl_team_members WHERE team=?1 AND member=?2", params![team.as_str(),member.as_str()], |r| Ok(json!({"team":team.as_str(),"member":member.as_str(),"qualified":format!("{team}/{member}"),"mail_key":r.get::<_,String>(0)?,"role":r.get::<_,Option<String>>(1)?,"description":r.get::<_,String>(2)?,"guidance_refs":refs_json(&r.get::<_,String>(3)?)?,"generation":r.get::<_,i64>(4)?}))).optional()?.ok_or_else(|| failure("TEAM_MEMBER_MISSING", "member has not joined this team"))
    }

    /// Pages roster headers in member-name order, without descriptions or pointers.
    ///
    /// # Errors
    /// Returns `TEAM_INPUT` for invalid limits/cursors or a missing-team/storage error.
    pub fn team_members(
        &self,
        team: &Key,
        after: Option<&Key>,
        limit: usize,
    ) -> Result<Value, MailError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(failure("TEAM_INPUT", "limit must be 1..100"));
        }
        require_team(&self.db, team)?;
        if let Some(after) = after {
            self.team_member(team, after)
                .map_err(|_| failure("TEAM_INPUT", "cursor must name a member of this team"))?;
        }
        let mut statement = self.db.prepare("SELECT member,role,generation FROM merl_team_members WHERE team=?1 AND member>?2 ORDER BY member LIMIT ?3")?;
        let count =
            i64::try_from(limit + 1).map_err(|_| failure("TEAM_INPUT", "invalid page size"))?;
        let mut members = statement.query_map(params![team.as_str(),after.map_or("", Key::as_str),count], |r| Ok(json!({"member":r.get::<_,String>(0)?,"role":r.get::<_,Option<String>>(1)?,"generation":r.get::<_,i64>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
        let next_after = if members.len() > limit {
            members.truncate(limit);
            members.last().map(|m| m["member"].clone())
        } else {
            None
        };
        Ok(json!({"team":team.as_str(),"members":members,"next_after":next_after}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Case(PathBuf);
    impl Case {
        fn new(name: &str) -> Self {
            let home =
                std::env::temp_dir().join(format!("merl-team-unit-{}-{name}", std::process::id()));
            assert!(!home.exists());
            Self(home)
        }
        fn store(&self) -> Mailbox {
            Mailbox::open(&self.0).unwrap()
        }
    }
    impl Drop for Case {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn scope() -> TeamSpec {
        TeamSpec::new(
            "alpha".parse().unwrap(),
            Some("planner".parse().unwrap()),
            "Synthetic scope".into(),
            vec![],
        )
        .unwrap()
    }
    fn member() -> MemberSpec {
        MemberSpec::new(
            "dev".parse().unwrap(),
            None,
            "Saved synthetic guidance".into(),
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn saved_guidance_and_labels_are_bounded_in_bytes() {
        let key = || "dev".parse().unwrap();
        let pointers = vec!["x".repeat(MAX_REF_BYTES); MAX_REFS];
        assert!(
            MemberSpec::new(
                key(),
                Some("é".repeat(64)),
                "é".repeat(4096),
                pointers.clone()
            )
            .is_ok()
        );
        for (role, description, refs) in [
            (None, String::new(), vec![]),
            (None, "é".repeat(4097), vec![]),
            (Some("é".repeat(65)), "brief".into(), vec![]),
            (Some("review\nrole".into()), "brief".into(), vec![]),
            (None, "brief".into(), vec!["pointer".into(); MAX_REFS + 1]),
            (None, "brief".into(), vec!["x".repeat(MAX_REF_BYTES + 1)]),
        ] {
            assert_eq!(
                MemberSpec::new(key(), role, description, refs)
                    .unwrap_err()
                    .code,
                "TEAM_INPUT"
            );
        }
        assert!(TeamSpec::new(key(), Some(key()), "é".repeat(4096), pointers).is_ok());
        assert_eq!(
            TeamSpec::new(key(), Some(key()), "é".repeat(4097), vec![])
                .unwrap_err()
                .code,
            "TEAM_INPUT"
        );
    }

    #[test]
    fn a_collision_cannot_adopt_even_an_unused_unbound_identity() {
        let case = Case::new("collision");
        let mut store = case.store();
        let team = "alpha".parse().unwrap();
        let candidate = "foreign".parse().unwrap();
        store.register(&candidate).unwrap();
        store.create_team(&scope()).unwrap();
        let before = store.status(&candidate).unwrap();
        assert_eq!(
            store
                .join_team(&team, &member(), &candidate)
                .unwrap_err()
                .code,
            "TEAM_IDENTITY_CONFLICT"
        );
        assert_eq!(store.status(&candidate).unwrap(), before);
        assert_eq!(
            store
                .team_member(&team, &"dev".parse().unwrap())
                .unwrap_err()
                .code,
            "TEAM_MEMBER_MISSING"
        );
        let fresh = "fresh".parse().unwrap();
        assert_eq!(
            store.join_team(&team, &member(), &fresh).unwrap()["mail_key"],
            "fresh"
        );
    }

    #[test]
    fn failed_member_publication_rolls_back_identity_and_lost_acknowledgment_retries() {
        let case = Case::new("transaction");
        let mut store = case.store();
        let team = "alpha".parse().unwrap();
        let candidate = "chosen".parse().unwrap();
        store.create_team(&scope()).unwrap();
        // Force failure after identity insertion, at the other half of the
        // transaction. Assertions use public roster/mail boundaries.
        store.db.execute_batch("CREATE TRIGGER fail_member BEFORE INSERT ON merl_team_members BEGIN SELECT RAISE(ABORT,'synthetic publication failure'); END;").unwrap();
        assert!(store.join_team(&team, &member(), &candidate).is_err());
        assert_eq!(
            store.status(&candidate).unwrap_err().code,
            "MAIL_IDENTITY_MISSING"
        );
        assert_eq!(
            store
                .team_member(&team, &"dev".parse().unwrap())
                .unwrap_err()
                .code,
            "TEAM_MEMBER_MISSING"
        );
        store.db.execute_batch("DROP TRIGGER fail_member;").unwrap();
        // Ignore the committed acknowledgment, then restart with a new candidate.
        store.join_team(&team, &member(), &candidate).unwrap();
        drop(store);
        let mut restarted = case.store();
        let unused = "different-candidate".parse().unwrap();
        let retry = restarted.join_team(&team, &member(), &unused).unwrap();
        assert_eq!(retry["outcome"], "unchanged");
        assert_eq!(retry["mail_key"], "chosen");
        assert_eq!(
            restarted.status(&unused).unwrap_err().code,
            "MAIL_IDENTITY_MISSING"
        );
        assert_eq!(
            restarted.team_members(&team, None, 100).unwrap()["members"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
