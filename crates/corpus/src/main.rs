//! Maintainer tooling for capturing and validating evaluation fixtures.

use std::{
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};

use corpus::{
    fixture::{Fixture, validate},
    github::fixture_from_graphql_pages,
};

const HELP: &str = "\
Capture and validate Merl evaluation fixtures.

Usage:
  corpus validate <fixture>...
  corpus capture-github <fixture-id> <owner/repository> <issue-number> <captured-at> <output>

The capture command requires an authenticated GitHub CLI. captured-at must be
an RFC 3339 timestamp supplied by the caller so fixture creation is explicit.
";

const ISSUE_QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    id
    nameWithOwner
    licenseInfo { spdxId }
    issue(number: $number) {
      id number title state closedAt url body createdAt updatedAt lastEditedAt
      labels(first: 100) { nodes { name } }
      assignees(first: 100) { nodes { id login } }
      milestone { title }
      author { login ... on Node { id } }
      userContentEdits(first: 100) {
        nodes { id editedAt editor { login ... on Node { id } } diff deletedAt }
        pageInfo { hasNextPage }
      }
      comments(first: 100, after: $endCursor) {
        nodes {
          id body createdAt updatedAt lastEditedAt
          author { login ... on Node { id } }
          userContentEdits(first: 100) {
            nodes { id editedAt editor { login ... on Node { id } } diff deletedAt }
            pageInfo { hasNextPage }
          }
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
}
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corpus: {error}");
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
                return Err("capture-github requires five arguments; run corpus help".to_owned());
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
        other => Err(format!("unknown command {other}; run corpus help")),
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
    fixture_from_graphql_pages(fixture_id, captured_at, &output.stdout)
}
