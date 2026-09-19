//! Conversion of GitHub Issue captures into causally ordered corpus fixtures.

use serde::Deserialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::fixture::{
    ActorRef, Capture, ContentEdit, FIXTURE_SCHEMA, Fixture, HistoryFidelity, MissingBodyReason,
    Observation, ObservationKind, Origin, Partition, Provenance, ProviderSnapshot,
    RedistributionReview, Source, body_digest, source_digest, validate,
};

/// Parse captured GraphQL pages and preserve missing prior edit bodies as gaps.
///
/// # Errors
///
/// Returns an error for malformed pages, timestamps, duplicate comments, or an
/// invalid fixture. The supplied capture time must be RFC 3339.
#[expect(
    clippy::too_many_lines,
    reason = "capture assembly is one auditable source boundary"
)]
pub fn fixture_from_graphql_pages(
    fixture_id: &str,
    captured_at: &str,
    json: &[u8],
) -> Result<Fixture, String> {
    timestamp(captured_at)?;
    let pages: Vec<GraphqlResponse> = serde_json::from_slice(json)
        .map_err(|error| format!("could not parse GitHub response: {error}"))?;
    let first = pages.first().ok_or("GitHub returned no pages")?;
    let repository = &first.data.repository;
    let issue = repository
        .issue
        .as_ref()
        .ok_or("GitHub Issue is not visible")?;

    let mut observations = Vec::new();
    add_versions(
        &mut observations,
        ObservationKind::Issue,
        &issue.id,
        issue.author.as_ref(),
        &issue.body,
        &issue.created_at,
        &issue.updated_at,
        issue.last_edited_at.as_deref(),
        issue.includes_created_edit,
        &issue.user_content_edits.nodes,
    )?;
    if issue.user_content_edits.page_info.has_next_page {
        return Err("Issue edit history exceeds one page; capture would be incomplete".to_owned());
    }

    let mut seen_comments = std::collections::HashSet::new();
    for page in &pages {
        let page_issue = page
            .data
            .repository
            .issue
            .as_ref()
            .ok_or("GitHub pagination lost Issue")?;
        if page_issue.id != issue.id || page.data.repository.id != repository.id {
            return Err("GitHub pagination changed source identity".to_owned());
        }
        if page_issue.labels.page_info.has_next_page {
            return Err("Issue labels snapshot is incomplete".to_owned());
        }
        if page_issue.assignees.page_info.has_next_page {
            return Err("Issue assignees snapshot is incomplete".to_owned());
        }
        for comment in &page_issue.comments.nodes {
            if !seen_comments.insert(&comment.id) {
                return Err(format!("duplicate GitHub comment {}", comment.id));
            }
            add_versions(
                &mut observations,
                ObservationKind::IssueComment,
                &comment.id,
                comment.author.as_ref(),
                &comment.body,
                &comment.created_at,
                &comment.updated_at,
                comment.last_edited_at.as_deref(),
                comment.includes_created_edit,
                &comment.user_content_edits.nodes,
            )?;
            if comment.user_content_edits.page_info.has_next_page {
                return Err(format!(
                    "comment {} edit history exceeds one page",
                    comment.id
                ));
            }
        }
    }

    // Stable sorting keeps provider connection order for ties without treating
    // an identifier as a causal clock. The tie remains ambiguous for replay.
    let mut timed_observations: Vec<_> = observations
        .into_iter()
        .map(|item| timestamp(&item.occurred_at).map(|time| (time, item)))
        .collect::<Result<_, _>>()?;
    timed_observations.sort_by_key(|(time, _)| *time);
    let mut observations: Vec<_> = timed_observations
        .into_iter()
        .map(|(_, item)| item)
        .collect();
    let mut previous = std::collections::HashMap::new();
    let mut prior_time = None;
    for (index, observation) in observations.iter_mut().enumerate() {
        observation.sequence = index as u64 + 1;
        let occurred = timestamp(&observation.occurred_at)?;
        observation.ambiguous_order_with_previous = prior_time == Some(occurred);
        prior_time = Some(occurred);
        observation.supersedes =
            previous.insert(observation.provider_id.clone(), observation.sequence);
    }

    let provider_snapshot = ProviderSnapshot {
        title: issue.title.clone(),
        state: issue.state.clone(),
        closed_at: issue.closed_at.clone(),
        labels: issue
            .labels
            .nodes
            .iter()
            .map(|label| label.name.clone())
            .collect(),
        assignees: issue.assignees.nodes.iter().map(ActorRef::from).collect(),
        milestone: issue
            .milestone
            .as_ref()
            .map(|milestone| milestone.title.clone()),
    };
    let fidelity = if observations.iter().any(|item| item.body.is_none()) {
        HistoryFidelity::DiffOnly
    } else {
        HistoryFidelity::TerminalSnapshotOnly
    };
    let fixture = Fixture {
        schema: FIXTURE_SCHEMA.to_owned(),
        id: fixture_id.to_owned(),
        partition: Partition::Development,
        origin: Origin::Natural,
        source: Source {
            provider: "github".to_owned(),
            repository: Some(repository.name_with_owner.clone()),
            repository_provider_id: Some(repository.id.clone()),
            issue_number: Some(issue.number),
            issue_provider_id: issue.id.clone(),
            url: Some(issue.url.clone()),
        },
        capture: Capture {
            captured_at: captured_at.to_owned(),
            capture_tool_version: format!("corpus/{}", env!("CARGO_PKG_VERSION")),
            source_sha256: source_digest(&provider_snapshot, &observations),
            history_fidelity: fidelity,
            provider_observation_count: observations.len(),
        },
        provenance: Provenance {
            repository_license_at_capture: repository.license_info.as_ref().map_or_else(
                || "NOASSERTION".to_owned(),
                |license| license.spdx_id.clone(),
            ),
            redistribution_review: RedistributionReview::Pending,
        },
        provider_snapshot,
        observations,
        gold_states: Vec::new(),
    };
    validate(&fixture).map_err(|error| format!("captured fixture is invalid: {error}"))?;
    Ok(fixture)
}

#[expect(
    clippy::too_many_arguments,
    reason = "provider content fields remain explicit at the capture boundary"
)]
#[expect(
    clippy::too_many_lines,
    reason = "a source entity and its edits are captured as one unit"
)]
fn add_versions(
    observations: &mut Vec<Observation>,
    kind: ObservationKind,
    entity_id: &str,
    author: Option<&GraphqlActor>,
    terminal_body: &str,
    created_at: &str,
    updated_at: &str,
    last_edited_at: Option<&str>,
    includes_created_edit: bool,
    edits: &[GraphqlEdit],
) -> Result<(), String> {
    let created = timestamp(created_at)?;
    let updated = timestamp(updated_at)?;
    if updated < created {
        return Err(format!("{entity_id} was updated before creation"));
    }
    let last_edit = last_edited_at.map(timestamp).transpose()?;
    if last_edit.is_some_and(|edit_time| edit_time > updated || edit_time < created) {
        return Err(format!(
            "{entity_id} last edit is outside creation/update interval"
        ));
    }
    let mut sorted_edits: Vec<_> = edits
        .iter()
        .map(|edit| timestamp(&edit.edited_at).map(|time| (time, edit)))
        .collect::<Result<_, _>>()?;
    sorted_edits.sort_by_key(|(time, _)| *time);
    let latest_edit_time = sorted_edits.last().map(|(time, _)| *time);
    if latest_edit_time.is_some_and(|edit_time| edit_time > updated || edit_time < created) {
        return Err(format!(
            "{entity_id} edit time is outside creation/update interval"
        ));
    }
    if last_edit
        .zip(latest_edit_time)
        .is_some_and(|(reported, recorded)| reported < recorded)
    {
        return Err(format!(
            "{entity_id} lastEditedAt precedes retained edit history"
        ));
    }
    let created_edit = if includes_created_edit {
        if sorted_edits.first().map(|(time, _)| *time) != Some(created) {
            return Err(format!(
                "{entity_id} claims a creation edit without one at creation"
            ));
        }
        Some(sorted_edits.remove(0).1)
    } else {
        None
    };
    let has_later_edits = !sorted_edits.is_empty() || last_edit.is_some_and(|time| time > created);
    let author = author.map(ActorRef::from);
    observations.push(observation(
        kind,
        entity_id,
        created_edit.map_or_else(|| format!("{entity_id}:initial"), |edit| edit.id.clone()),
        author.clone(),
        created_at,
        created_at,
        if has_later_edits {
            None
        } else {
            Some(terminal_body)
        },
        if has_later_edits {
            Some(MissingBodyReason::PriorVersionUnavailable)
        } else {
            None
        },
        created_edit.map(content_edit),
    ));
    for (index, (_, edit)) in sorted_edits.iter().enumerate() {
        let is_last = index + 1 == sorted_edits.len();
        let terminal_at_this_edit = is_last
            && edit.deleted_at.is_none()
            && last_edit.is_none_or(|reported| Some(reported) == latest_edit_time);
        observations.push(observation(
            kind,
            entity_id,
            edit.id.clone(),
            author.clone(),
            created_at,
            &edit.edited_at,
            if terminal_at_this_edit {
                Some(terminal_body)
            } else {
                None
            },
            if terminal_at_this_edit {
                None
            } else if edit.deleted_at.is_some() {
                Some(MissingBodyReason::DeletedByProvider)
            } else {
                Some(MissingBodyReason::PriorVersionUnavailable)
            },
            Some(content_edit(edit)),
        ));
    }
    if last_edit.is_some_and(|reported| {
        reported > created && latest_edit_time.is_none_or(|recorded| reported > recorded)
    }) && let Some(edited_at) = last_edited_at
    {
        observations.push(observation(
            kind,
            entity_id,
            format!("{entity_id}:terminal@{edited_at}"),
            author,
            created_at,
            edited_at,
            Some(terminal_body),
            None,
            None,
        ));
    }
    // updatedAt may reflect metadata changes; lastEditedAt locates body changes.
    Ok(())
}

fn content_edit(edit: &GraphqlEdit) -> ContentEdit {
    ContentEdit {
        provider_id: edit.id.clone(),
        editor: edit.editor.as_ref().map(ActorRef::from),
        edited_at: edit.edited_at.clone(),
        diff: edit.diff.clone(),
        deleted_at: edit.deleted_at.clone(),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "a captured version records every provenance field explicitly"
)]
fn observation(
    kind: ObservationKind,
    provider_id: &str,
    version_id: String,
    author: Option<ActorRef>,
    created_at: &str,
    occurred_at: &str,
    body: Option<&str>,
    missing_body_reason: Option<MissingBodyReason>,
    edit: Option<ContentEdit>,
) -> Observation {
    Observation {
        sequence: 0,
        kind,
        provider_id: provider_id.to_owned(),
        version_id,
        supersedes: None,
        ambiguous_order_with_previous: false,
        author,
        occurred_at: occurred_at.to_owned(),
        created_at: created_at.to_owned(),
        body: body.map(str::to_owned),
        body_sha256: body.map(body_digest),
        missing_body_reason,
        edit,
    }
}

fn timestamp(value: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| format!("invalid RFC 3339 timestamp {value}: {error}"))
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: GraphqlData,
}
#[derive(Debug, Deserialize)]
struct GraphqlData {
    repository: GraphqlRepository,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlRepository {
    id: String,
    name_with_owner: String,
    license_info: Option<GraphqlLicense>,
    issue: Option<GraphqlIssue>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlLicense {
    spdx_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlIssue {
    id: String,
    number: u64,
    title: String,
    state: String,
    closed_at: Option<String>,
    labels: GraphqlLabels,
    assignees: GraphqlActors,
    milestone: Option<GraphqlMilestone>,
    url: String,
    author: Option<GraphqlActor>,
    body: String,
    created_at: String,
    updated_at: String,
    last_edited_at: Option<String>,
    includes_created_edit: bool,
    user_content_edits: GraphqlEdits,
    comments: GraphqlComments,
}
#[derive(Debug, Deserialize)]
struct GraphqlLabels {
    nodes: Vec<GraphqlLabel>,
    #[serde(rename = "pageInfo")]
    page_info: GraphqlPageInfo,
}
#[derive(Debug, Deserialize)]
struct GraphqlLabel {
    name: String,
}
#[derive(Debug, Deserialize)]
struct GraphqlActors {
    nodes: Vec<GraphqlActor>,
    #[serde(rename = "pageInfo")]
    page_info: GraphqlPageInfo,
}
#[derive(Debug, Deserialize)]
struct GraphqlMilestone {
    title: String,
}
#[derive(Debug, Deserialize)]
struct GraphqlComments {
    nodes: Vec<GraphqlComment>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlComment {
    id: String,
    author: Option<GraphqlActor>,
    body: String,
    created_at: String,
    updated_at: String,
    last_edited_at: Option<String>,
    includes_created_edit: bool,
    user_content_edits: GraphqlEdits,
}
#[derive(Debug, Deserialize)]
struct GraphqlActor {
    id: Option<String>,
    login: String,
}
impl From<&GraphqlActor> for ActorRef {
    fn from(actor: &GraphqlActor) -> Self {
        Self {
            provider_id: actor.id.clone(),
            login: actor.login.clone(),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlEdits {
    nodes: Vec<GraphqlEdit>,
    page_info: GraphqlPageInfo,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlPageInfo {
    has_next_page: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlEdit {
    id: String,
    edited_at: String,
    editor: Option<GraphqlActor>,
    diff: Option<String>,
    deleted_at: Option<String>,
}
