//! Candidate review commands expose original evidence beside each explicit reviewer action.

use crate::{CliError, assertion_json, invalid_input};
use merl_core::{ActorId, ObjectId, ObjectKind, PayloadId, PolicyInputId, ProjectId};
use merl_store::{Candidate, CandidateReview, PayloadRead, ReviewAction, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

// Review notes are short audit explanations. Cap them at 8 KiB so a single
// command cannot turn bounded candidate inspection into a document download.
const MAX_REASON_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy)]
pub(super) struct Options<'a> {
    pub id: Option<&'a str>,
    pub actor: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub kind: Option<&'a str>,
    pub value: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub after: Option<&'a str>,
    pub offset: Option<&'a str>,
    pub dry_run: bool,
    pub json_output: bool,
}
fn required<T: for<'a> TryFrom<&'a str>>(value: Option<&str>) -> Result<T, CliError> {
    T::try_from(value.ok_or_else(|| invalid_input("missing required review argument"))?)
        .map_err(|_| invalid_input("invalid structural review argument"))
}
fn candidate(
    store: &Store,
    project: &ProjectId,
    id: &PolicyInputId,
) -> Result<Candidate, CliError> {
    store
        .candidate(project, id)?
        .ok_or_else(|| merl_policy::PolicyError::CandidateMissing.into())
}
fn status(store: &Store, project: &ProjectId, candidate: &Candidate) -> Result<String, CliError> {
    Ok(
        if let Some(action) = store.candidate_resolution(project, candidate)? {
            match action.as_str() {
                "accept" => "accepted",
                "reject" => "rejected",
                _ => "corrected",
            }
            .into()
        } else if !candidate.relation
            && store.accepted_assertion(project, &candidate.run, candidate.index)?
        {
            "accepted".into()
        } else {
            "pending".into()
        },
    )
}
pub(super) fn execute(
    store: &mut Store,
    project: &ProjectId,
    operation: &str,
    tail: &[&str],
    options: Options<'_>,
    now: i64,
) -> Result<String, CliError> {
    let value = match (operation, tail) {
        ("list", []) => {
            let (entries, more) = store.candidate_page(project, options.after, 100)?;
            let rows=entries.iter().map(|entry| Ok(json!({"id":entry.id.as_str(),"status":status(store,project,entry)?,"run":entry.run.as_str(),"index":entry.index,"reason":entry.reason.as_str()}))).collect::<Result<Vec<_>,CliError>>()?;
            json!({"schema":"merl.candidates/v1","project":project.as_str(),"candidates":rows,"next_after":if more {entries.last().map(|e|e.id.as_str())}else{None}})
        }
        ("show", [id]) => {
            let candidate = candidate(store, project, &required(Some(id))?)?;
            let offset = options
                .offset
                .unwrap_or("0")
                .parse::<usize>()
                .map_err(|_| invalid_input("invalid offset"))?;
            let (reviews, more) = store.candidate_reviews(project, &candidate.id, offset)?;
            let rows = reviews
                .iter()
                .map(|review| review_json(store, project, review, false))
                .collect::<Result<Vec<_>, _>>()?;
            if candidate.relation {
                let relation = crate::assertions::relation_json(
                    store,
                    project,
                    &candidate.run,
                    candidate.index,
                    false,
                )?;
                return Ok(render(
                    &json!({"schema":"merl.candidate/v1","project":project.as_str(),"id":candidate.id.as_str(),"status":status(store,project,&candidate)?,
                    "original_evaluation":candidate.evaluation.as_str(),"basis_project_revision":candidate.basis.get(),"reason":candidate.reason.as_str(),
                    "relation":relation,"proposal":relation,"reviews":rows,"next_offset":if more {Some(offset+100)}else{None},
                    "dependency_state":if merl_policy::candidate_dependencies_current(store,project,&candidate)? {"current"}else{"changed"}}),
                    options.json_output,
                ));
            }
            let assertion = assertion_json(store, project, &candidate.run, candidate.index, false)?;
            json!({"schema":"merl.candidate/v1","project":project.as_str(),"id":candidate.id.as_str(),"status":status(store,project,&candidate)?,
                "original_evaluation":candidate.evaluation.as_str(),"basis_project_revision":candidate.basis.get(),"reason":candidate.reason.as_str(),
                "assertion":assertion,"proposal":{"subject":assertion["subject"],"kind":assertion["predicate"],"value":assertion["value"]},
                "dependency_state":if merl_policy::candidate_dependencies_current(store,project,&candidate)? {"current"}else{"changed"},
                "reviews":rows,"next_offset":if more {Some(offset+100)}else{None}})
        }
        ("accept" | "reject" | "correct", [candidate_id]) => {
            let review = parse_review(store, project, operation, candidate_id, options)?;
            if let Some(prior) = store.candidate_review(project, &review.id)?
                && prior != review
            {
                return Err(merl_store::StoreError::PolicyInputConflict.into());
            }
            if options.dry_run {
                let prepared = merl_policy::preview_candidate_review(store, project, &review, now)?;
                let decision = &prepared.inputs[0];
                json!({"schema":"merl.candidate-review-preview/v1","candidate":review.candidate.as_str(),"action":operation,"outcome":decision.disposition.as_str(),"reason":decision.reason.as_str(),"basis_project_revision":prepared.basis_project_revision.get()})
            } else {
                if let (Some(payload), Some(text)) = (&review.reason, options.reason)
                    && store.candidate_review(project, &review.id)?.is_none()
                {
                    store.put_review_reason(project, payload, text.as_bytes())?;
                }
                let record = merl_policy::review_candidate(store, project, &review, now)?;
                let decision = &record.inputs[0];
                json!({"schema":"merl.candidate-review/v1","project":project.as_str(),"candidate":review.candidate.as_str(),"request":review.id.as_str(),"action":operation,
                    "actor":review.actor.as_str(),"outcome":decision.disposition.as_str(),"reason":decision.reason.as_str(),"evaluation":record.id.as_str(),
                    "revision":record.committed_revision.map(merl_core::ProjectRevision::get),"basis_project_revision":record.basis_project_revision.get(),"policy_version":record.version.as_str(),
                    "configuration_digest":crate::digest_text(&record.configuration_digest),"conflict":record.conflict.map(|c|json!({"reason":c.reason_code,"target":c.target_id,"expected_revision":c.expected_revision,"actual_revision":c.actual_revision}))})
            }
        }
        _ => {
            return Err(invalid_input(
                "unknown candidate command; run `merl help candidate`",
            ));
        }
    };
    Ok(render(&value, options.json_output))
}

fn parse_review(
    store: &Store,
    project: &ProjectId,
    operation: &str,
    candidate_id: &str,
    options: Options<'_>,
) -> Result<CandidateReview, CliError> {
    let id: PolicyInputId = required(options.id)?;
    let actor: ActorId = required(options.actor)?;
    let candidate_id: PolicyInputId = required(Some(candidate_id))?;
    candidate(store, project, &candidate_id)?;
    let action = match operation {
        "accept" => ReviewAction::Accept,
        "reject" => ReviewAction::Reject,
        _ => ReviewAction::Correct {
            object: required::<ObjectId>(options.subject)?,
            kind: required::<ObjectKind>(options.kind)?,
            payload: match options.value {
                Some("none") => None,
                value => Some(required::<PayloadId>(value)?),
            },
        },
    };
    if operation != "correct"
        && (options.subject.is_some() || options.kind.is_some() || options.value.is_some())
    {
        return Err(invalid_input(
            "replacement fields require candidate correct",
        ));
    }
    if operation != "accept" && options.reason.is_none_or(|reason| reason.trim().is_empty()) {
        return Err(invalid_input("rejection and correction require a reason"));
    }
    if options
        .reason
        .is_some_and(|reason| reason.len() > MAX_REASON_BYTES)
    {
        return Err(invalid_input("review reason exceeds 8192 bytes"));
    }
    let reason = options
        .reason
        .map(|text| {
            // Scope the content identity to this request so erasing one note
            // does not erase an equal note submitted by another reviewer.
            let digest = Sha256::digest(format!("{project}/{id}/{text}").as_bytes());
            PayloadId::try_from(
                format!(
                    "review_reason_{}",
                    crate::digest_text(&digest.into()).trim_start_matches("sha256:")
                )
                .as_str(),
            )
            .map_err(|_| invalid_input("invalid reason identity"))
        })
        .transpose()?;
    let review = CandidateReview {
        id,
        actor,
        candidate: candidate_id,
        action,
        reason,
    };
    Ok(review)
}

pub(super) fn review_json(
    store: &Store,
    project: &ProjectId,
    review: &CandidateReview,
    expand_source: bool,
) -> Result<Value, CliError> {
    let candidate = candidate(store, project, &review.candidate)?;
    let record = store.policy_evaluation(
        project,
        &merl_policy::candidate_review_evaluation(project, &review.id)?,
    )?;
    let reason = review
        .reason
        .as_ref()
        .map(|id| store.read_payload(project, id))
        .transpose()?;
    let proposal = match &review.action {
        ReviewAction::Correct {
            object,
            kind,
            payload,
        } => {
            json!({"subject":object.as_str(),"kind":kind.as_str(),"value":payload.as_ref().map_or("none",PayloadId::as_str)})
        }
        _ => Value::Null,
    };
    let evidence = if candidate.relation {
        crate::assertions::relation_json(
            store,
            project,
            &candidate.run,
            candidate.index,
            expand_source,
        )?
    } else {
        assertion_json(
            store,
            project,
            &candidate.run,
            candidate.index,
            expand_source,
        )?
    };
    let mut value = json!({"request":review.id.as_str(),"candidate":review.candidate.as_str(),"actor":review.actor.as_str(),"action":review.action.as_str(),
        "original_evaluation":candidate.evaluation.as_str(),"proposal":proposal,
        "evaluation":record.as_ref().map(|r|r.id.as_str()),"outcome":record.as_ref().and_then(|r|r.inputs.first()).map(|i|i.disposition.as_str()),
        "reason_payload":review.reason.as_ref().map(PayloadId::as_str),"reason_available":matches!(reason,Some(PayloadRead::Available(_))),
        "reason_text":match reason {Some(PayloadRead::Available(bytes))=>Some(String::from_utf8_lossy(&bytes).into_owned()),_=>None}});
    value[if candidate.relation {
        "relation"
    } else {
        "assertion"
    }] = evidence;
    Ok(value)
}
fn render(value: &Value, json_output: bool) -> String {
    if json_output {
        return format!("{value}\n");
    }
    let mut lines = Vec::new();
    if let Some(entries) = value["candidates"].as_array() {
        for entry in entries {
            lines.push(format!(
                "{}: {} ({}[{}], {})",
                text(entry, "id"),
                text(entry, "status"),
                text(entry, "run"),
                entry["index"],
                text(entry, "reason")
            ));
        }
        if entries.is_empty() {
            lines.push("No candidates".into());
        }
        if !value["next_after"].is_null() {
            lines.push(format!(
                "Continue with --after {}",
                text(value, "next_after")
            ));
        }
    } else if !value["relation"].is_null() {
        lines.push(format!("{}: {}", text(value, "id"), text(value, "status")));
        let relation = &value["relation"];
        lines.push(format!(
            "{} {} {} ({}[{}])",
            text(relation, "subject"),
            text(relation, "predicate"),
            text(relation, "object"),
            text(relation, "run"),
            relation["index"]
        ));
        lines.push(format!("Dependencies: {}", text(value, "dependency_state")));
    } else if !value["assertion"].is_null() {
        lines.push(format!("{}: {}", text(value, "id"), text(value, "status")));
        lines.push(format!(
            "Policy {} at revision {}: {}; dependencies {}",
            text(value, "original_evaluation"),
            value["basis_project_revision"],
            text(value, "reason"),
            text(value, "dependency_state")
        ));
        lines.extend(assertion_lines(&value["assertion"]));
        for review in value["reviews"].as_array().into_iter().flatten() {
            lines.push(format!(
                "Review {}: {} by {} -> {} (evaluation {})",
                text(review, "request"),
                text(review, "action"),
                text(review, "actor"),
                text(review, "outcome"),
                text(review, "evaluation")
            ));
            if !review["proposal"].is_null() {
                lines.push(format!("Replacement: {}", review["proposal"]));
            }
            if !review["reason_payload"].is_null() {
                lines.push(format!(
                    "Reason {}: {}",
                    text(review, "reason_payload"),
                    review["reason_text"].as_str().unwrap_or("unavailable")
                ));
            }
        }
        if !value["next_offset"].is_null() {
            lines.push(format!("Continue with --offset {}", value["next_offset"]));
        }
    } else {
        lines.push(format!(
            "{} {}: {} ({})",
            text(value, "action"),
            text(value, "candidate"),
            text(value, "outcome"),
            text(value, "reason")
        ));
        if value["evaluation"].is_null() {
            lines.push("Preview; no request or state committed".into());
        } else {
            lines.push(format!(
                "Evaluation {}; revision {}; actor {}; policy {}",
                text(value, "evaluation"),
                value["revision"],
                text(value, "actor"),
                text(value, "policy_version")
            ));
        }
        if !value["conflict"].is_null() {
            lines.push(format!("Conflict: {}", value["conflict"]));
        }
    }
    format!("{}\n", lines.join("\n"))
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("pending")
}
fn assertion_lines(assertion: &Value) -> Vec<String> {
    vec![
        format!(
            "Original: {} {} {}; {}[{}] at {} bytes {}..{}",
            text(assertion, "subject"),
            text(assertion, "predicate"),
            text(assertion, "value"),
            text(assertion, "run"),
            assertion["index"],
            text(assertion, "source_version"),
            assertion["span"]["start"],
            assertion["span"]["end"]
        ),
        format!(
            "Axes: {}, {}, {}; confidence {}; source author {}; attribution {}",
            text(assertion, "act"),
            text(assertion, "epistemic_basis"),
            text(assertion, "polarity"),
            assertion["confidence_millis"],
            assertion["asserted_by"],
            assertion["attributed_to"]
        ),
    ]
}

pub(super) fn help(operation: Option<&str>, json_output: bool) -> Result<String, CliError> {
    let (usage, description, related) = match operation {
        None => (
            "merl candidate <list|show|accept|reject|correct>",
            "Inspect and resolve compiler interpretations under current command authority.",
            vec![
                "candidate list",
                "candidate show",
                "candidate accept",
                "candidate reject",
                "candidate correct",
            ],
        ),
        Some("list") => (
            "merl candidate list --project <id> --database <path> [--after <candidate>] [--json]",
            "List up to 100 pending and resolved candidates. Continue with next_after.",
            vec!["candidate show"],
        ),
        Some("show") => (
            "merl candidate show <candidate> --project <id> --database <path> [--offset <n>] [--json]",
            "Inspect the original assertion or relation, dependency state, and up to 100 review attempts.",
            vec!["candidate accept", "candidate reject", "candidate correct"],
        ),
        Some("accept") => (
            "merl candidate accept <candidate> --project <id> --database <path> --actor <id> --id <request> [--reason <text>] [--dry-run] [--json]",
            "Accept the recorded proposal under current command_actor authority. Changed dependencies conflict.",
            vec!["candidate show"],
        ),
        Some("reject") => (
            "merl candidate reject <candidate> --project <id> --database <path> --actor <id> --id <request> --reason <text> [--dry-run] [--json]",
            "Close the candidate with an erasable reason while preserving its compiler input.",
            vec!["candidate show"],
        ),
        Some("correct") => (
            "merl candidate correct <candidate> --project <id> --database <path> --actor <id> --id <request> --subject <object> --kind <kind> --value <payload|none> --reason <text> [--dry-run] [--json]",
            "Submit a replacement object interpretation linked to the original assertion. Relation candidates support accept or reject.",
            vec!["candidate show", "show"],
        ),
        _ => return Err(invalid_input("unknown candidate help topic")),
    };
    let example = match operation {
        None | Some("list") => "merl candidate list --project P1 --database project.sqlite --json",
        Some("show") => "merl candidate show C81 --project P1 --database project.sqlite --json",
        Some("accept") => {
            "merl candidate accept C81 --project P1 --database project.sqlite --actor reviewer --id review-1 --json"
        }
        Some("reject") => {
            "merl candidate reject C81 --project P1 --database project.sqlite --actor reviewer --id review-1 --reason 'Unsupported interpretation' --json"
        }
        _ => {
            "merl candidate correct C81 --project P1 --database project.sqlite --actor reviewer --id review-1 --subject Q1 --kind question --value none --reason 'Still an open question' --json"
        }
    };
    let trust = usage
        .contains("--actor ")
        .then_some(crate::security::ACTOR_CLAIM);
    let result = json!({"trust_boundary":trust,"schema":"merl.help/v1","command":operation.map_or("candidate".into(),|op|format!("candidate {op}")),"usage":usage,"description":description,
        "related":related,"outcomes":["accepted","rejected","conflict"],"errors":["CANDIDATE_NOT_FOUND","POLICY_INPUT_CONFLICT","INVALID_INPUT","POLICY_ERROR"],
        "examples":[example]});
    if json_output {
        Ok(format!("{result}\n"))
    } else {
        Ok(format!(
            "{usage}\n\n{description}\n{}\nRelated: {}\n",
            trust.unwrap_or_default(),
            related.join(", ")
        ))
    }
}

pub(super) fn evidence_history(
    store: &Store,
    project: &ProjectId,
    entries: &[merl_store::ObjectHistoryEntry],
) -> Result<Vec<Value>, CliError> {
    let mut result = Vec::new();
    for entry in entries {
        match &entry.input {
            Some(merl_core::PolicyInput::ObservedAssertion {run,index,..}) => result.push(json!({"event":entry.event.as_str(),"assertion":assertion_json(store,project,run,*index,true)?})),
            Some(merl_core::PolicyInput::Command(id)) => {
                if let Some(review)=store.candidate_review(project,id)? && review.action!=ReviewAction::Reject {
                    let detail=review_json(store,project,&review,true)?;
                    result.push(json!({"event":entry.event.as_str(),"assertion":detail["assertion"],"review":detail}));
                }
            },
            _=>{},
        }
    }
    Ok(result)
}
