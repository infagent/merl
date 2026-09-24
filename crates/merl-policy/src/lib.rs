//! Deterministic first-release authority rules over typed, provenance-backed inputs.

mod assertions;
mod authority;
mod candidates;
mod commands;
mod relations;
mod revalidation;
pub use candidates::{
    candidate_dependencies_current, candidate_review_evaluation, prepare_candidate_review,
    preview_candidate_review, review_candidate,
};
pub use commands::{
    MAX_COMMAND_TEXT_BYTES, SemanticCommand, prepare_semantic_command, preview_semantic_command,
    semantic_command_evaluation, submit_semantic_command,
};
pub use revalidation::{
    prepare_revalidation_review, resolve_revalidation, revalidation_evaluation,
};

pub use assertions::{apply_assertions, prepare_assertions};

pub use authority::{AuthorityChange, change_authority};

use std::{collections::HashSet, error::Error, fmt};

use merl_core::{
    ActorId, BatchId, DomainEvent, DomainEventBatch, PayloadId, PolicyDisposition,
    PolicyEvaluation, PolicyEvaluationId, PolicyEventOrigin, PolicyInput, PolicyInputDecision,
    PolicyInputId, PolicyRead, PolicyVersion, PolicyWrite, ProjectId, ProjectRevision,
    ProviderObservation, ReasonCode,
};
use merl_store::{Store, StoreError};
use sha2::{Digest, Sha256};

/// An immutable semantic proposal submitted to first-release policy.
#[derive(Clone, Debug)]
pub enum Proposal {
    /// A compiler assertion whose author and span are resolved from stored provenance.
    ObservedAssertion {
        /// Stable identity of this policy input.
        id: PolicyInputId,
        /// Compiler run that emitted it.
        run: merl_core::CompilationRunId,
        /// Position within the recorded result.
        index: u32,
        /// Proposed change, checked against the stored assertion.
        event: DomainEvent,
    },
    /// A compiler relation with endpoint evidence loaded from its recorded run.
    ObservedRelation {
        /// Stable identity of this policy input.
        id: PolicyInputId,
        /// Compiler origin.
        run: merl_core::CompilationRunId,
        /// Position in the relation list.
        index: u32,
        /// Proposed edge, checked against the stored relation.
        event: DomainEvent,
    },
    /// A structured action by the authenticated evaluation actor.
    Command {
        /// Stable client retry identity.
        id: PolicyInputId,
        /// Proposed structural change.
        event: DomainEvent,
    },
    /// Provider-owned Issue facts from an attached source namespace.
    ProviderObservation {
        /// Typed fact captured by ingestion.
        observation: ProviderObservation,
        /// Provider mirror update for the same Issue and protected snapshot.
        event: DomainEvent,
    },
    /// A maintenance action whose actor must have administrative authority.
    AdministrativeAction {
        /// Stable retry identity.
        id: PolicyInputId,
        /// Proposed structural change.
        event: DomainEvent,
    },
}

impl Proposal {
    fn input(&self) -> PolicyInput {
        match self {
            Self::ObservedAssertion { id, run, index, .. } => PolicyInput::ObservedAssertion {
                id: id.clone(),
                run: run.clone(),
                index: *index,
            },
            Self::ObservedRelation { id, run, index, .. } => PolicyInput::ObservedRelation {
                id: id.clone(),
                run: run.clone(),
                index: *index,
            },
            Self::Command { id, .. } => PolicyInput::Command(id.clone()),
            Self::ProviderObservation { observation, .. } => {
                PolicyInput::ProviderObservation(observation.id.clone())
            }
            Self::AdministrativeAction { id, .. } => PolicyInput::AdministrativeAction(id.clone()),
        }
    }

    fn event(&self) -> &DomainEvent {
        match self {
            Self::ObservedAssertion { event, .. }
            | Self::ObservedRelation { event, .. }
            | Self::Command { event, .. }
            | Self::ProviderObservation { event, .. }
            | Self::AdministrativeAction { event, .. } => event,
        }
    }
}

/// Explicit first-release grants; local OS processes are trusted at the filesystem boundary.
#[derive(Clone, Debug)]
pub struct PolicyRules {
    /// Version persisted with each evaluation.
    pub version: PolicyVersion,
    /// Source authors allowed to make explicit project decisions.
    pub decision_authors: Vec<ActorId>,
    /// Authenticated actors allowed to submit semantic commands.
    pub command_actors: Vec<ActorId>,
    /// Authenticated actors allowed to perform maintenance actions.
    pub administrators: Vec<ActorId>,
}

impl PolicyRules {
    /// Loads the effective grants shared by public application entry points.
    ///
    /// # Errors
    /// Returns an error when the project or its stored grants cannot be read.
    pub fn from_store(store: &Store, project: &ProjectId) -> Result<Self, StoreError> {
        let grants = store.authority_grants(project)?;
        Self::from_grants(grants)
    }

    fn from_grants(grants: merl_store::AuthorityGrants) -> Result<Self, StoreError> {
        Ok(Self {
            version: PolicyVersion::try_from("authority_v7")
                .map_err(|_| StoreError::CorruptHistory)?,
            decision_authors: grants.decision_authors,
            command_actors: grants.command_actors,
            administrators: grants.administrators,
        })
    }

    /// Hashes the exact grants used to decide an evaluation.
    #[must_use]
    pub fn configuration_digest(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        for (kind, actors) in [
            ("decision_author", &self.decision_authors),
            ("command_actor", &self.command_actors),
            ("administrator", &self.administrators),
        ] {
            hash.update((kind.len() as u64).to_be_bytes());
            hash.update(kind.as_bytes());
            let mut names: Vec<&str> = actors.iter().map(ActorId::as_str).collect();
            names.sort_unstable();
            names.dedup();
            hash.update((names.len() as u64).to_be_bytes());
            for name in names {
                hash.update((name.len() as u64).to_be_bytes());
                hash.update(name.as_bytes());
            }
        }
        hash.finalize().into()
    }
}

/// A policy result prepared outside the final accepted-state transaction.
#[derive(Clone, Debug)]
pub struct PreparedPolicy {
    /// Auditable decision and any proposed accepted events.
    pub evaluation: PolicyEvaluation,
    /// Provider fact to persist with a provider-owned mirror event, if present.
    pub provider: Option<ProviderObservation>,
}

impl PreparedPolicy {
    /// Checks dependencies and commits the decision, accepted batch, and inbox atomically.
    ///
    /// # Errors
    /// A stale read, write conflict, reused input identity, or storage failure leaves no batch.
    pub fn commit(&self, store: &mut Store) -> Result<Option<ProjectRevision>, StoreError> {
        store.commit_policy_evaluation(&self.evaluation, self.provider.as_ref())
    }
}

/// A proposal could not be evaluated without inventing authority or provenance.
#[derive(Debug)]
pub enum PolicyError {
    /// Accepted-state or captured-source lookup failed.
    Store(StoreError),
    /// An input lacks the claimed immutable provenance or proposes a different fact.
    InvalidProposal,
    /// A run is unfinished or requires explicit promotion before causal application.
    RunIneligible,
    /// A review lacks a completed hindsight attempt linked to its impact.
    RevalidationRunIneligible,
    /// No recorded assertion candidate has this identity in the project.
    CandidateMissing,
    /// Structured command arguments violate the public content or date contract.
    InvalidCommand(&'static str),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommand(message) => formatter.write_str(message),
            Self::CandidateMissing => formatter.write_str("candidate does not exist"),
            Self::Store(error) => write!(formatter, "{error}"),
            Self::RevalidationRunIneligible => formatter
                .write_str("revalidation requires a completed hindsight run linked to the impact"),
            Self::RunIneligible => formatter.write_str(
                "assertion application requires a completed live run without context requests",
            ),
            Self::InvalidProposal => {
                formatter.write_str("policy proposal does not match its source")
            }
        }
    }
}

impl Error for PolicyError {}

impl From<StoreError> for PolicyError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Applies application work with durable grants, preserving outcomes on identical retries.
///
/// A retry returns its recorded evaluation even if grants or the clock have changed.
/// A caller seeking reevaluation must use a new evaluation ID. The input content and
/// actor still have to match, including rejected requests.
///
/// # Errors
/// Returns an identity conflict for changed retry content, or an evaluation or commit failure.
pub fn apply_current(
    store: &mut Store,
    project: &ProjectId,
    actor: &ActorId,
    evaluation_id: PolicyEvaluationId,
    batch_id: BatchId,
    occurred_at_millis: i64,
    proposals: &[Proposal],
) -> Result<merl_store::RecordedPolicyEvaluation, PolicyError> {
    if let Some(record) = recorded_outcome(store, project, actor, &evaluation_id, proposals)? {
        return Ok(record);
    }
    let prepared = evaluate_current(
        store,
        project,
        actor,
        evaluation_id,
        batch_id,
        occurred_at_millis,
        proposals,
    )?;
    prepared.commit(store)?;
    store
        .policy_evaluation(project, &prepared.evaluation.id)?
        .ok_or(PolicyError::Store(StoreError::CorruptHistory))
}

fn recorded_outcome(
    store: &Store,
    project: &ProjectId,
    actor: &ActorId,
    evaluation: &PolicyEvaluationId,
    proposals: &[Proposal],
) -> Result<Option<merl_store::RecordedPolicyEvaluation>, PolicyError> {
    let record = store.policy_evaluation(project, evaluation)?;
    if let Some(record) = &record
        && (record.actor != *actor
            || record.inputs.len() != proposals.len()
            || record
                .inputs
                .iter()
                .zip(proposals)
                .any(|(input, proposal)| {
                    input.input != proposal.input()
                        || input.input_digest != input_digest(proposal, actor)
                }))
    {
        return Err(StoreError::PolicyInputConflict.into());
    }
    Ok(record)
}

/// Evaluates application work using the project's durable authority grants.
///
/// A grant-collection dependency prevents work prepared before a revocation from
/// committing under stale authority. Callers supply proposals, never actor lists.
///
/// # Errors
/// Returns provenance, grant lookup, or policy validation failures.
pub fn evaluate_current(
    store: &Store,
    project: &ProjectId,
    actor: &ActorId,
    evaluation_id: PolicyEvaluationId,
    batch_id: BatchId,
    occurred_at_millis: i64,
    proposals: &[Proposal],
) -> Result<PreparedPolicy, PolicyError> {
    let grants = store.authority_grants(project)?;
    let revision = grants.revision;
    let rules = PolicyRules::from_grants(grants)?;
    let mut prepared = evaluate(
        store,
        project,
        actor,
        evaluation_id,
        batch_id,
        occurred_at_millis,
        &rules,
        proposals,
    )?;
    prepared.evaluation.reads.push(PolicyRead::KindCollection {
        kind: merl_core::ObjectKind::try_from("authority_grant")
            .map_err(|_| PolicyError::InvalidProposal)?,
        latest_project_revision: revision,
    });
    Ok(prepared)
}

/// Evaluates typed inputs against the current accepted state without mutating it.
///
/// This explicit-rule seam supports controlled policy tests. Application entry points
/// use [`evaluate_current`] to load and guard durable grants.
///
/// One evaluation can assign different dispositions to several assertions.
/// The store checks its recorded reads and writes again at commit time.
/// Accepted input IDs are deduplicated. A rejected or candidate input with the
/// same ID and content may receive a new evaluation under later policy rules.
///
/// # Errors
/// Rejects missing or mismatched source provenance, duplicate proposal targets,
/// and unavailable accepted-state metadata.
#[expect(
    clippy::too_many_arguments,
    reason = "evaluation identity and authority are explicit at this boundary"
)]
#[expect(
    clippy::too_many_lines,
    reason = "each proposal contributes one checked policy input and optional event"
)]
pub fn evaluate(
    store: &Store,
    project: &ProjectId,
    actor: &ActorId,
    evaluation_id: PolicyEvaluationId,
    batch_id: BatchId,
    occurred_at_millis: i64,
    rules: &PolicyRules,
    proposals: &[Proposal],
) -> Result<PreparedPolicy, PolicyError> {
    if proposals.is_empty() {
        return Err(PolicyError::InvalidProposal);
    }
    let basis = store.project_revision(project)?;
    let mut inputs = Vec::with_capacity(proposals.len());
    let mut events = Vec::new();
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut event_origins = Vec::new();
    let mut targets = HashSet::new();
    let mut provider = None;
    let decisions = proposals
        .iter()
        .map(|proposal| {
            let input = proposal.input();
            let digest = input_digest(proposal, actor);
            if let Some((accepted_digest, _)) = store.accepted_policy_input(project, &input)? {
                if accepted_digest != digest {
                    return Err(PolicyError::Store(StoreError::PolicyInputConflict));
                }
                Ok((PolicyDisposition::Duplicate, "already_accepted"))
            } else {
                disposition_for(store, project, actor, rules, proposal)
            }
        })
        .collect::<Result<Vec<_>, PolicyError>>()?;
    let mut assertion_targets = std::collections::HashMap::new();
    for (proposal, (disposition, _)) in proposals.iter().zip(&decisions) {
        if *disposition == PolicyDisposition::Accepted
            && let Proposal::ObservedAssertion {
                event: DomainEvent::PutObject { object, .. },
                ..
            } = proposal
        {
            *assertion_targets.entry(object).or_insert(0_usize) += 1;
        }
    }
    for (position, proposal) in proposals.iter().enumerate() {
        let input = proposal.input();
        let digest = input_digest(proposal, actor);
        let (mut disposition, mut reason) = decisions[position];
        if disposition == PolicyDisposition::Accepted
            && let Proposal::ObservedAssertion {
                event: DomainEvent::PutObject { object, .. },
                ..
            } = proposal
            && assertion_targets
                .get(object)
                .is_some_and(|count| *count > 1)
        {
            disposition = PolicyDisposition::Conflict;
            reason = "competing_assertion_targets";
        }
        let input_index = u32::try_from(inputs.len()).map_err(|_| PolicyError::InvalidProposal)?;
        inputs.push(PolicyInputDecision {
            input,
            input_digest: digest,
            disposition,
            reason: ReasonCode::try_from(reason).map_err(|_| PolicyError::InvalidProposal)?,
        });
        if disposition == PolicyDisposition::Accepted {
            let event_id = match proposal.event() {
                DomainEvent::PutObject { id, object, .. } => {
                    if !targets.insert(format!("object:{object}")) {
                        return Err(PolicyError::InvalidProposal);
                    }
                    let revision = store.object(project, object)?.map(|value| value.revision);
                    reads.push(PolicyRead::Object {
                        id: object.clone(),
                        revision,
                    });
                    writes.push(PolicyWrite::Object {
                        object: object.clone(),
                        expected_revision: revision,
                    });
                    id
                }
                DomainEvent::ResolveSupport { id, object, .. } => {
                    if !targets.insert(format!("support:{object}")) {
                        return Err(PolicyError::InvalidProposal);
                    }
                    let current = store
                        .object(project, object)?
                        .ok_or(PolicyError::InvalidProposal)?;
                    reads.push(PolicyRead::Object {
                        id: object.clone(),
                        revision: Some(current.revision),
                    });
                    // The impact guard serializes support reviews; there is no semantic write.
                    id
                }
                DomainEvent::PutRelation { id, relation } => {
                    if relation.project != *project
                        || !targets.insert(format!("relation:{}", relation.id))
                    {
                        return Err(PolicyError::InvalidProposal);
                    }
                    for endpoint in [&relation.subject, &relation.object] {
                        let current = store
                            .object(project, endpoint)?
                            .ok_or(PolicyError::InvalidProposal)?;
                        reads.push(PolicyRead::Object {
                            id: endpoint.clone(),
                            revision: Some(current.revision),
                        });
                    }
                    let revision = store
                        .relation(project, &relation.id)?
                        .map(|value| value.revision);
                    writes.push(PolicyWrite::Relation {
                        relation: relation.id.clone(),
                        expected_revision: revision,
                    });
                    id
                }
            };
            event_origins.push(PolicyEventOrigin {
                input_index,
                event: event_id.clone(),
            });
            events.push(proposal.event().clone());
            if let Proposal::ProviderObservation { observation, .. } = proposal
                && provider.replace(observation.clone()).is_some()
            {
                return Err(PolicyError::InvalidProposal);
            }
        }
    }
    let batch = (!events.is_empty()).then(|| DomainEventBatch {
        id: batch_id,
        project: project.clone(),
        actor: actor.clone(),
        occurred_at_millis,
        events,
    });
    Ok(PreparedPolicy {
        evaluation: PolicyEvaluation {
            id: evaluation_id,
            project: project.clone(),
            actor: actor.clone(),
            version: rules.version.clone(),
            configuration_digest: rules.configuration_digest(),
            basis_project_revision: basis,
            inputs,
            reads,
            writes,
            event_origins,
            batch,
        },
        provider,
    })
}

fn disposition_for(
    store: &Store,
    project: &ProjectId,
    actor: &ActorId,
    rules: &PolicyRules,
    proposal: &Proposal,
) -> Result<(PolicyDisposition, &'static str), PolicyError> {
    if !matches!(
        proposal,
        Proposal::ProviderObservation { .. } | Proposal::ObservedAssertion { .. }
    ) && let DomainEvent::PutObject { object, kind, .. } = proposal.event()
        && (kind.as_str() == "provider_issue"
            || store
                .object(project, object)?
                .is_some_and(|current| current.kind.as_str() == "provider_issue"))
    {
        return Err(PolicyError::InvalidProposal);
    }
    if !matches!(proposal, Proposal::ObservedAssertion { .. })
        && let DomainEvent::PutObject { object, kind, .. } = proposal.event()
        && (kind.as_str() == "authority_grant"
            || store
                .object(project, object)?
                .is_some_and(|current| current.kind.as_str() == "authority_grant"))
        && (!matches!(proposal, Proposal::AdministrativeAction { .. })
            || kind.as_str() != "authority_grant")
    {
        return Ok((PolicyDisposition::Rejected, "administrator_required"));
    }
    match proposal {
        Proposal::Command { .. } => Ok(if rules.command_actors.contains(actor) {
            (PolicyDisposition::Accepted, "authorized_command")
        } else {
            (PolicyDisposition::Rejected, "command_actor_not_authorized")
        }),
        Proposal::AdministrativeAction { .. } => Ok(if rules.administrators.contains(actor) {
            (PolicyDisposition::Accepted, "authorized_administrator")
        } else {
            (PolicyDisposition::Rejected, "administrator_required")
        }),
        Proposal::ProviderObservation { observation, event } => {
            if !store.source_binding_exists(project, &observation.binding)? {
                return Err(PolicyError::InvalidProposal);
            }
            let DomainEvent::PutObject {
                object,
                kind,
                payload,
                issue_scope,
                lifecycle,
                ..
            } = event
            else {
                return Err(PolicyError::InvalidProposal);
            };
            if object != &observation.issue
                || kind.as_str() != "provider_issue"
                || payload.as_ref() != Some(&observation.snapshot_payload)
                || issue_scope.is_some()
                || *lifecycle != merl_core::ObjectLifecycle::Active
                || store
                    .object(project, object)?
                    .is_some_and(|current| current.kind.as_str() != "provider_issue")
            {
                return Err(PolicyError::InvalidProposal);
            }
            Ok((PolicyDisposition::Accepted, "trusted_provider_fact"))
        }
        Proposal::ObservedRelation {
            run, index, event, ..
        } => relations::disposition(store, project, run, *index, event),
        Proposal::ObservedAssertion {
            run, index, event, ..
        } => assertions::disposition(store, project, rules, run, *index, event),
    }
}

fn input_digest(proposal: &Proposal, actor: &ActorId) -> [u8; 32] {
    let mut hash = Sha256::new();
    let mut part = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    };
    let input = proposal.input();
    part(input.kind().as_bytes());
    part(input.id().as_str().as_bytes());
    if matches!(
        proposal,
        Proposal::Command { .. } | Proposal::AdministrativeAction { .. }
    ) {
        part(actor.as_str().as_bytes());
    }
    if let Proposal::ObservedAssertion { run, index, .. }
    | Proposal::ObservedRelation { run, index, .. } = proposal
    {
        part(run.as_str().as_bytes());
        part(&index.to_be_bytes());
    }
    match proposal.event() {
        DomainEvent::PutObject {
            object,
            kind,
            payload,
            issue_scope,
            lifecycle,
            ..
        } => {
            part(object.as_str().as_bytes());
            part(kind.as_str().as_bytes());
            part(payload.as_ref().map_or("", PayloadId::as_str).as_bytes());
            if let Some(scope) = issue_scope {
                part(b"issue_scope_v1");
                part(scope.as_bytes());
            }
            part(lifecycle.as_str().as_bytes());
        }
        DomainEvent::ResolveSupport { object, review, .. } => {
            part(b"resolve_support_v1");
            part(object.as_str().as_bytes());
            part(review.as_str().as_bytes());
        }
        DomainEvent::PutRelation { relation, .. } => {
            part(b"put_relation_v1");
            part(relation.id.as_str().as_bytes());
            part(relation.subject.as_str().as_bytes());
            part(relation.kind.as_str().as_bytes());
            part(relation.object.as_str().as_bytes());
        }
    }
    if let Proposal::ProviderObservation { observation, .. } = proposal {
        part(observation.binding.as_str().as_bytes());
        part(observation.issue.as_str().as_bytes());
        part(observation.state.as_str().as_bytes());
        part(
            &observation
                .upstream_updated_at_millis
                .unwrap_or(0)
                .to_be_bytes(),
        );
        part(&observation.closed_at_millis.unwrap_or(0).to_be_bytes());
        part(&observation.observed_at_millis.to_be_bytes());
        part(&[u8::from(observation.label_provider_ids.is_some())]);
        for label in observation
            .label_provider_ids
            .as_deref()
            .unwrap_or_default()
        {
            part(label.as_bytes());
        }
        part(&[u8::from(observation.assignee_provider_ids.is_some())]);
        for assignee in observation
            .assignee_provider_ids
            .as_deref()
            .unwrap_or_default()
        {
            part(assignee.as_bytes());
        }
        part(observation.snapshot_payload.as_str().as_bytes());
    }
    hash.finalize().into()
}
