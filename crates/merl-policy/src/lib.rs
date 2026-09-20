//! Deterministic first-release authority rules over typed, provenance-backed inputs.

use std::{collections::HashSet, error::Error, fmt};

use merl_core::{
    ActorId, BatchId, DomainEvent, DomainEventBatch, PayloadId, PolicyDisposition,
    PolicyEvaluation, PolicyEvaluationId, PolicyInput, PolicyInputDecision, PolicyInputId,
    PolicyRead, PolicyVersion, PolicyWrite, ProjectId, ProjectRevision, ProviderObservation,
    ReasonCode,
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
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "{error}"),
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

/// Evaluates typed inputs against the current accepted state without mutating it.
///
/// One evaluation can assign different dispositions to several assertions.
/// The store checks its recorded reads and writes again at commit time.
///
/// # Errors
/// Rejects missing or mismatched source provenance, duplicate proposal targets,
/// and unavailable accepted-state metadata.
#[expect(
    clippy::too_many_arguments,
    reason = "evaluation identity and authority are explicit at this boundary"
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
    let mut targets = HashSet::new();
    let mut provider = None;
    for proposal in proposals {
        let input = proposal.input();
        let digest = input_digest(proposal, actor);
        let (disposition, reason) =
            if let Some((accepted_digest, _)) = store.accepted_policy_input(project, &input)? {
                if accepted_digest != digest {
                    return Err(PolicyError::Store(StoreError::PolicyInputConflict));
                }
                (PolicyDisposition::Duplicate, "already_accepted")
            } else {
                disposition_for(store, project, actor, rules, proposal)?
            };
        inputs.push(PolicyInputDecision {
            input,
            input_digest: digest,
            disposition,
            reason: ReasonCode::try_from(reason).map_err(|_| PolicyError::InvalidProposal)?,
        });
        if disposition == PolicyDisposition::Accepted {
            let DomainEvent::PutObject { object, .. } = proposal.event();
            if !targets.insert(object.as_str().to_owned()) {
                return Err(PolicyError::InvalidProposal);
            }
            let existing = store.object(project, object)?;
            let revision = existing.as_ref().map(|value| value.revision);
            reads.push(PolicyRead::Object {
                id: object.clone(),
                revision,
            });
            writes.push(PolicyWrite {
                object: object.clone(),
                expected_revision: revision,
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
                ..
            } = event;
            if object != &observation.issue
                || kind.as_str() != "provider_issue"
                || payload.as_ref() != Some(&observation.snapshot_payload)
            {
                return Err(PolicyError::InvalidProposal);
            }
            Ok((PolicyDisposition::Accepted, "trusted_provider_fact"))
        }
        Proposal::ObservedAssertion {
            run, index, event, ..
        } => {
            let status = store
                .compilation_run_status(project, run.as_str())?
                .ok_or(PolicyError::InvalidProposal)?;
            if !status.succeeded || status.needs_context {
                return Err(PolicyError::InvalidProposal);
            }
            let assertion = store
                .observed_assertions(project, run.as_str())?
                .into_iter()
                .nth(*index as usize)
                .ok_or(PolicyError::InvalidProposal)?;
            let source = store
                .source_version(project, &assertion.source)?
                .ok_or(PolicyError::InvalidProposal)?;
            let DomainEvent::PutObject {
                object,
                kind,
                payload,
                ..
            } = event;
            if assertion.subject != object.as_str()
                || assertion.predicate != kind.as_str()
                || assertion.value != payload.as_ref().map_or("none", PayloadId::as_str)
                || source.source_author.as_ref().map(ActorId::as_str)
                    != assertion.asserted_by.as_deref()
            {
                return Err(PolicyError::InvalidProposal);
            }
            if store.accepted_assertion(project, run, *index)? {
                return Ok((PolicyDisposition::Duplicate, "assertion_already_accepted"));
            }
            if assertion.act == "request"
                && source
                    .source_author
                    .as_ref()
                    .is_some_and(|author| rules.decision_authors.contains(author))
            {
                Ok((PolicyDisposition::Accepted, "authorized_direct_author"))
            } else {
                // Quoted attribution is model output, not an authenticated grant.
                Ok((PolicyDisposition::Candidate, "direct_authority_unverified"))
            }
        }
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
    if let Proposal::ObservedAssertion { run, index, .. } = proposal {
        part(run.as_str().as_bytes());
        part(&index.to_be_bytes());
    }
    let DomainEvent::PutObject {
        object,
        kind,
        payload,
        ..
    } = proposal.event();
    part(object.as_str().as_bytes());
    part(kind.as_str().as_bytes());
    part(payload.as_ref().map_or("", PayloadId::as_str).as_bytes());
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
