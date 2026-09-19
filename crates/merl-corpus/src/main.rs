//! Maintainer tooling for capturing and validating evaluation fixtures.

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

use merl_corpus::corpus::{
    Capture, ContentEdit, FIXTURE_SCHEMA, Fixture, HistoryFidelity, Observation, ObservationKind,
    Origin, Partition, Provenance, RedistributionReview, Source, source_digest, validate,
};
use serde::Deserialize;

const HELP: &str = "\
Capture and validate Merl evaluation fixtures.

Usage:
  merl-corpus validate <fixture>...
  merl-corpus capture-github <fixture-id> <owner/repository> <issue-number> <captured-at> <output>

The capture command requires an authenticated GitHub CLI. captured-at must be
an RFC 3339 timestamp supplied by the caller so fixture creation is explicit.
";

const ISSUE_QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    id
    nameWithOwner
    url
    licenseInfo { spdxId }
    issue(number: $number) {
      id
      number
      title
      state
      closedAt
      labels(first: 100) { nodes { name } }
      assignees(first: 100) { nodes { login } }
      milestone { title }
      url
      author { login }
      body
      createdAt
      updatedAt
      userContentEdits(first: 100) {
        nodes {
          id
          editedAt
          editor { login }
          diff
          deletedAt
        }
      }
      comments(first: 100, after: $endCursor) {
        nodes {
          id
          author { login }
          body
          createdAt
          updatedAt
          userContentEdits(first: 100) {
            nodes {
              id
              editedAt
              editor { login }
              diff
              deletedAt
            }
          }
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("merl-corpus: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print!("{HELP}");
        return Ok(());
    };

    match command.as_str() {
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        "validate" => {
            let paths: Vec<_> = args.collect();
            if paths.is_empty() {
                return Err("validate requires at least one fixture path".to_owned());
            }
            for path in paths {
                validate_path(Path::new(&path))?;
                println!("valid {path}");
            }
            Ok(())
        }
        "capture-github" => {
            let values: Vec<_> = args.collect();
            let [fixture_id, repository, issue_number, captured_at, output] = values.as_slice()
            else {
                return Err(
                    "capture-github requires five arguments; run merl-corpus help".to_owned(),
                );
            };
            let issue_number = issue_number
                .parse::<u64>()
                .map_err(|error| format!("invalid Issue number {issue_number}: {error}"))?;
            let fixture = capture_github(fixture_id, repository, issue_number, captured_at)?;
            let encoded = serde_json::to_string_pretty(&fixture)
                .map_err(|error| format!("could not encode fixture: {error}"))?;
            fs::write(output, format!("{encoded}\n"))
                .map_err(|error| format!("could not write {output}: {error}"))?;
            println!("captured {fixture_id} in {output}");
            Ok(())
        }
        other => Err(format!("unknown command {other}; run merl-corpus help")),
    }
}

fn validate_path(path: &Path) -> Result<(), String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let fixture: Fixture = serde_json::from_str(&source)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    validate(&fixture).map_err(|error| format!("{}: {error}", path.display()))
}

fn capture_github(
    fixture_id: &str,
    repository: &str,
    issue_number: u64,
    captured_at: &str,
) -> Result<Fixture, String> {
    let (owner, name) = repository
        .split_once('/')
        .ok_or_else(|| format!("repository must be owner/name, found {repository}"))?;
    let output = Command::new("gh")
        .args([
            "api",
            "graphql",
            "--paginate",
            "--slurp",
            "-f",
            &format!("query={ISSUE_QUERY}"),
            "-F",
            &format!("owner={owner}"),
            "-F",
            &format!("name={name}"),
            "-F",
            &format!("number={issue_number}"),
        ])
        .output()
        .map_err(|error| format!("could not start GitHub CLI: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "GitHub capture failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let pages: Vec<GraphqlResponse> = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not parse GitHub response: {error}"))?;
    fixture_from_pages(fixture_id, captured_at, &pages)
}

fn fixture_from_pages(
    fixture_id: &str,
    captured_at: &str,
    pages: &[GraphqlResponse],
) -> Result<Fixture, String> {
    let first = pages
        .first()
        .ok_or_else(|| "GitHub returned no pages".to_owned())?;
    let repository = &first.data.repository;
    let issue = repository
        .issue
        .as_ref()
        .ok_or_else(|| "GitHub Issue does not exist or is not visible".to_owned())?;

    let mut observations = vec![Observation {
        sequence: 1,
        kind: ObservationKind::Issue,
        provider_id: issue.id.clone(),
        author: issue.author.as_ref().map(|actor| actor.login.clone()),
        created_at: issue.created_at.clone(),
        updated_at: issue.updated_at.clone(),
        body: issue.body.clone(),
        edits: edits(&issue.user_content_edits.nodes),
    }];

    for page in pages {
        let page_issue = page
            .data
            .repository
            .issue
            .as_ref()
            .ok_or_else(|| "GitHub pagination lost the requested Issue".to_owned())?;
        for comment in &page_issue.comments.nodes {
            let sequence = u64::try_from(observations.len())
                .map_err(|error| format!("too many observations: {error}"))?
                + 1;
            observations.push(Observation {
                sequence,
                kind: ObservationKind::IssueComment,
                provider_id: comment.id.clone(),
                author: comment.author.as_ref().map(|actor| actor.login.clone()),
                created_at: comment.created_at.clone(),
                updated_at: comment.updated_at.clone(),
                body: comment.body.clone(),
                edits: edits(&comment.user_content_edits.nodes),
            });
        }
    }

    let provider_snapshot = merl_corpus::corpus::ProviderSnapshot {
        title: issue.title.clone(),
        state: issue.state.clone(),
        closed_at: issue.closed_at.clone(),
        labels: issue
            .labels
            .nodes
            .iter()
            .map(|label| label.name.clone())
            .collect(),
        assignees: issue
            .assignees
            .nodes
            .iter()
            .map(|actor| actor.login.clone())
            .collect(),
        milestone: issue
            .milestone
            .as_ref()
            .map(|milestone| milestone.title.clone()),
    };
    let provider_events = Vec::new();
    let source_sha256 = source_digest(&provider_snapshot, &observations, &provider_events);
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
            capture_tool_version: format!("merl-corpus/{}", env!("CARGO_PKG_VERSION")),
            source_sha256,
            history_fidelity: HistoryFidelity::DiffOnly,
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
        provider_events,
        gold_states: Vec::new(),
    };
    validate(&fixture).map_err(|error| format!("captured fixture is invalid: {error}"))?;
    Ok(fixture)
}

fn edits(source: &[GraphqlEdit]) -> Vec<ContentEdit> {
    source
        .iter()
        .map(|edit| ContentEdit {
            provider_id: edit.id.clone(),
            editor: edit.editor.as_ref().map(|actor| actor.login.clone()),
            edited_at: edit.edited_at.clone(),
            diff: edit.diff.clone(),
            deleted_at: edit.deleted_at.clone(),
        })
        .collect()
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
    user_content_edits: GraphqlEdits,
    comments: GraphqlComments,
}

#[derive(Debug, Deserialize)]
struct GraphqlLabels {
    nodes: Vec<GraphqlLabel>,
}

#[derive(Debug, Deserialize)]
struct GraphqlLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlActors {
    nodes: Vec<GraphqlActor>,
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
    user_content_edits: GraphqlEdits,
}

#[derive(Debug, Deserialize)]
struct GraphqlActor {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlEdits {
    nodes: Vec<GraphqlEdit>,
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
