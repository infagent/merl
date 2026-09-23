//! Durable follow-up work for compiler context requests.

use crate::{CompilationResult, CompilationRunStatus, Store, StoreError, valid_record_id};
use merl_core::ProjectId;
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Operator work lists return at most 100 records to bound each polling response.
/// This page size does not change compiler limits or accepted-state semantics.
pub const COMPILATION_WORK_PAGE_SIZE: usize = 100;

/// One immutable context request and the state of its bounded successor.
#[derive(Debug)]
pub struct ExpansionWork {
    /// Run whose response requested more context.
    pub parent_run: String,
    /// Stable run identity reserved for the next round.
    pub child_run: String,
    /// One-based expansion round; the initial compilation is round zero.
    pub round: usize,
    /// Exact structural references requested by the compiler, in response order.
    pub references: Vec<String>,
    /// Pending, running, completed, `needs_context`, blocked, exhausted, or failed.
    pub status: String,
    /// Stable reason when expansion or execution could not finish.
    pub failure_code: Option<String>,
}

pub(super) fn insert_work(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    result: &CompilationResult<'_>,
    status: &CompilationRunStatus,
    requests: &[String],
) -> Result<(), StoreError> {
    if requests.is_empty() || requests.len() > status.limits[4] || !result.assertions.is_empty() {
        return Err(StoreError::InvalidCompilation);
    }
    let prior: Option<i64> = transaction
        .query_row(
            "SELECT round FROM compilation_expansions WHERE project_id=?1 AND child_run=?2",
            params![project.as_str(), result.run_id],
            |r| r.get(0),
        )
        .optional()?;
    let round = prior
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(StoreError::InvalidCompilation)?;
    let digest = Sha256::digest(format!("{}:{}", project, result.run_id).as_bytes());
    let mut child = String::from("cx_");
    for byte in digest {
        write!(child, "{byte:02x}").expect("String write");
    }
    transaction.execute(
        "INSERT INTO compilation_expansions VALUES (?1,?2,?3,?4)",
        params![project.as_str(), result.run_id, child, round],
    )?;
    for (index, request) in requests.iter().enumerate() {
        let reference = request.as_str();
        if !valid_record_id(reference) {
            return Err(StoreError::InvalidCompilation);
        }
        transaction.execute(
            "INSERT INTO compilation_expansion_references VALUES (?1,?2,?3,?4)",
            params![
                project.as_str(),
                result.run_id,
                i64::try_from(index).map_err(|_| StoreError::InvalidCompilation)?,
                reference
            ],
        )?;
    }
    if usize::try_from(round).map_err(|_| StoreError::InvalidCompilation)? > status.limits[5] {
        transaction.execute(
            "INSERT INTO compilation_expansion_failures VALUES (?1,?2,'expansion_round_budget')",
            params![project.as_str(), result.run_id],
        )?;
    }
    Ok(())
}

pub(super) fn validate_successor(
    store: &Store,
    project: &ProjectId,
    record: &crate::CompilationIntent<'_>,
) -> Result<(), StoreError> {
    let Some(parent) = store.expansion_parent(project, record.id)? else {
        return Ok(());
    };
    let prior = store
        .compilation_run_status(project, &parent)?
        .ok_or(StoreError::InvalidCompilation)?;
    let work = store
        .expansion_work(project, &parent)?
        .ok_or(StoreError::InvalidCompilation)?;
    let limits = [
        record.max_input_bytes,
        record.max_output_bytes,
        record.max_output_tokens,
        record.max_assertions,
        record.max_context_requests,
        record.max_expansion_rounds,
        record.max_payload_bytes,
        record.max_source_window,
        record.max_objects,
    ];
    if !prior.needs_context
        || work.round > prior.limits[5]
        || work.failure_code.is_some()
        || prior.source != *record.source
        || prior.mode != record.mode
        || prior.limits != limits
        || prior.compiler_id != record.compiler_id
        || prior.compiler_version != record.compiler_version
        || prior.model_id != record.model_id
        || prior.prompt_digest != record.prompt_digest
        || prior.adapter_config_digest != record.adapter_config_digest
        || prior.interpretation_basis_revision != record.interpretation_basis_revision
        || prior.source_observation_cutoff != record.source_observation_cutoff
        || record.selector_version != "context_expansion_v1"
        || record.renderer_version != "json_v1"
    {
        return Err(StoreError::InvalidCompilation);
    }
    Ok(())
}

// A purge can commit after rendering but before intent insertion. Recheck under
// the writer transaction so expansion cannot copy erased bytes into a new scope.
pub(super) fn validate_retained_context(
    transaction: &Transaction<'_>,
    project: &ProjectId,
    record: &crate::CompilationIntent<'_>,
    replay_origin: Option<&str>,
) -> Result<(), StoreError> {
    let parent_available: Option<bool> = if let Some(original) = replay_origin {
        Some(transaction.query_row(
            "SELECT p.erased=0 FROM compilation_runs r JOIN payloads p
             ON p.project_id=r.project_id AND p.id=r.context_payload_id
             WHERE r.project_id=?1 AND r.id=?2",
            params![project.as_str(), original],
            |r| r.get(0),
        )?)
    } else {
        transaction
            .query_row(
                "SELECT p.erased=0 AND rp.erased=0 FROM compilation_expansions e
         JOIN compilation_runs r ON r.project_id=e.project_id AND r.id=e.parent_run
         JOIN compilation_results c ON c.project_id=r.project_id AND c.run_id=r.id
         JOIN payloads p ON p.project_id=r.project_id AND p.id=r.context_payload_id
         JOIN payloads rp ON rp.project_id=c.project_id AND rp.id=c.response_payload_id
         WHERE e.project_id=?1 AND e.child_run=?2",
                params![project.as_str(), record.id],
                |r| r.get(0),
            )
            .optional()?
    };
    let Some(parent_available) = parent_available else {
        return Ok(());
    };
    if !parent_available {
        return Err(StoreError::InvalidCompilation);
    }
    for source in record.source_window {
        let erased: bool = transaction.query_row(
            "SELECT COALESCE(p.erased,0) FROM source_versions s LEFT JOIN payloads p ON p.project_id=s.project_id AND p.id=s.payload_id WHERE s.project_id=?1 AND s.id=?2",
            params![project.as_str(),source.as_str()],|r|r.get(0))?;
        if erased {
            return Err(StoreError::InvalidCompilation);
        }
    }
    for (object, _) in record.objects {
        let erased: bool = transaction.query_row(
            "SELECT COALESCE(p.erased,0) FROM domain_events e JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id
             LEFT JOIN payloads p ON p.project_id=e.project_id AND p.id=e.payload_id
             WHERE e.project_id=?1 AND e.object_id=?2 AND b.revision<=?3
             ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
            params![project.as_str(),object.as_str(),i64::try_from(record.interpretation_basis_revision.get()).map_err(|_|StoreError::InvalidCompilation)?],|r|r.get(0))?;
        if erased {
            return Err(StoreError::InvalidCompilation);
        }
    }
    Ok(())
}

impl Store {
    /// Returns the earlier run whose exact input a replay retained.
    ///
    /// # Errors
    /// Returns a storage error if the origin cannot be read.
    pub fn compilation_replay_origin(
        &self,
        project: &ProjectId,
        run: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(self.connection.query_row("SELECT original_run FROM compilation_replay_origins WHERE project_id=?1 AND run_id=?2",params![project.as_str(),run],|r|r.get(0)).optional()?)
    }

    /// Reads a semantic object at the original causal basis, excluding control records.
    ///
    /// # Errors
    /// Returns an error for corrupt history, future bases, or storage failures.
    pub fn compiler_object_at_revision(
        &self,
        project: &ProjectId,
        object: &merl_core::ObjectId,
        basis: merl_core::ProjectRevision,
    ) -> Result<Option<(merl_core::ObjectRevision, Option<merl_core::PayloadId>)>, StoreError> {
        let kind: Option<String> = self.connection.query_row(
            "SELECT e.object_kind FROM domain_events e JOIN domain_event_batches b ON b.project_id=e.project_id AND b.id=e.batch_id WHERE e.project_id=?1 AND e.object_id=?2 AND b.revision<=?3 ORDER BY b.revision DESC,e.event_index DESC LIMIT 1",
            params![project.as_str(),object.as_str(),i64::try_from(basis.get()).map_err(|_|StoreError::InvalidCompilation)?], |r|r.get(0)).optional()?;
        if kind.as_deref().is_none_or(|kind| {
            matches!(
                kind,
                "provider_issue"
                    | "source_coverage_requirement"
                    | "source_compilation_request"
                    | "authority_grant"
                    | "candidate_review"
            )
        }) {
            return Ok(None);
        }
        self.object_at_revision(project, object, basis)
    }

    /// Reads committed expansion work without loading protected compiler prose.
    ///
    /// # Errors
    /// Returns a storage error or corrupt-history error for invalid stored data.
    pub fn expansion_work(
        &self,
        project: &ProjectId,
        parent: &str,
    ) -> Result<Option<ExpansionWork>, StoreError> {
        let row: Option<(String, i64, Option<String>)> = self.connection.query_row(
            "SELECT e.child_run,e.round,f.code FROM compilation_expansions e LEFT JOIN compilation_expansion_failures f ON f.project_id=e.project_id AND f.parent_run=e.parent_run WHERE e.project_id=?1 AND e.parent_run=?2",
            params![project.as_str(), parent], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((child_run, round, mut failure_code)) = row else {
            return Ok(None);
        };
        let mut statement = self.connection.prepare("SELECT reference FROM compilation_expansion_references WHERE project_id=?1 AND parent_run=?2 ORDER BY request_index")?;
        let references = statement
            .query_map(params![project.as_str(), parent], |r| r.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        let status = if let Some(code) = &failure_code {
            if matches!(code.as_str(), "expansion_round_budget" | "input_budget") {
                "exhausted"
            } else {
                "blocked"
            }
        } else if let Some(child) = self.compilation_run_status(project, &child_run)? {
            failure_code = child.failure_code;
            if !child.completed {
                "running"
            } else if child.needs_context {
                "needs_context"
            } else if child.succeeded {
                "completed"
            } else {
                "failed"
            }
        } else {
            "pending"
        };
        Ok(Some(ExpansionWork {
            parent_run: parent.into(),
            child_run,
            round: usize::try_from(round).map_err(|_| StoreError::CorruptHistory)?,
            references,
            status: status.into(),
            failure_code,
        }))
    }

    /// Finds the request that reserved a run as its successor.
    ///
    /// # Errors
    /// Returns a storage error if the lineage cannot be read.
    pub fn expansion_parent(
        &self,
        project: &ProjectId,
        child: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(self.connection.query_row("SELECT parent_run FROM compilation_expansions WHERE project_id=?1 AND child_run=?2", params![project.as_str(),child], |r|r.get(0)).optional()?)
    }

    /// Lists at most 100 request identities, ordered by the originating attempt.
    ///
    /// # Errors
    /// Rejects an offset outside SQLite's integer range or a storage failure.
    pub fn expansion_requests(
        &self,
        project: &ProjectId,
        offset: usize,
    ) -> Result<Vec<String>, StoreError> {
        let mut statement = self.connection.prepare("SELECT e.parent_run FROM compilation_expansions e JOIN compilation_runs r ON r.project_id=e.project_id AND r.id=e.parent_run WHERE e.project_id=?1 ORDER BY r.attempt_order LIMIT ?3 OFFSET ?2")?;
        Ok(statement
            .query_map(
                params![
                    project.as_str(),
                    i64::try_from(offset).map_err(|_| StoreError::InvalidCompilation)?,
                    i64::try_from(COMPILATION_WORK_PAGE_SIZE)
                        .map_err(|_| StoreError::InvalidCompilation)?
                ],
                |r| r.get(0),
            )?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Records a terminal expansion failure without altering the compiler response.
    ///
    /// # Errors
    /// Rejects unknown failure codes, missing requests, and already dispatched work.
    pub fn fail_expansion(
        &mut self,
        project: &ProjectId,
        parent: &str,
        code: &str,
    ) -> Result<(), StoreError> {
        if !matches!(
            code,
            "input_budget" | "missing_evidence" | "expansion_reference" | "expansion_loop"
        ) {
            return Err(StoreError::InvalidCompilation);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = transaction.execute("INSERT OR IGNORE INTO compilation_expansion_failures SELECT project_id,parent_run,?3 FROM compilation_expansions e WHERE project_id=?1 AND parent_run=?2 AND NOT EXISTS(SELECT 1 FROM compilation_runs r WHERE r.project_id=e.project_id AND r.id=e.child_run)",params![project.as_str(),parent,code])?;
        if changed == 0 {
            return Err(StoreError::InvalidCompilation);
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Migration must recover retained requests while leaving erased work blocked.
    #[test]
    fn legacy_context_requests_survive_the_schema_upgrade() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(r#"CREATE TABLE compilation_runs(project_id TEXT,id TEXT,attempt_order INTEGER,max_expansion_rounds INTEGER,PRIMARY KEY(project_id,id));
            CREATE TABLE compilation_results(project_id TEXT,run_id TEXT,outcome TEXT,response_payload_id TEXT);
            CREATE TABLE payloads(project_id TEXT,id TEXT,bytes BLOB);
            INSERT INTO compilation_runs VALUES ('P','retained',1,2),('P','erased',2,2);
            INSERT INTO compilation_results VALUES ('P','retained','needs_context','response'),('P','erased','needs_context','gone');
            INSERT INTO payloads VALUES ('P','response',CAST('{"context_required":[{"reference":"D1"}]}' AS BLOB)),('P','gone',NULL);"#).unwrap();
        connection
            .execute_batch(include_str!("../migrations/0023_context_expansions.sql"))
            .unwrap();
        let reference: String = connection.query_row("SELECT reference FROM compilation_expansion_references WHERE parent_run='retained'",[],|r|r.get(0)).unwrap();
        assert_eq!(reference, "D1");
        let failure: String = connection
            .query_row(
                "SELECT code FROM compilation_expansion_failures WHERE parent_run='erased'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(failure, "missing_evidence");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM compilation_expansions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 2);
    }
}
