//! Maintainer tooling for capturing and validating evaluation fixtures.

use std::{
    env, fs,
    path::Path,
    process::{Command, ExitCode},
};

use merl_corpus::{
    fixture::{Fixture, validate},
    github::fixture_from_graphql_pages,
    review::{independent_review_packet, verify_independent_review},
};

const HELP: &str = "\
Capture and validate Merl evaluation fixtures.

Usage:
  merl-corpus validate <fixture>...
  merl-corpus review-packet <fixture> <output>
  merl-corpus review-verify <fixture> <review-packet>
  merl-corpus capture-github <fixture-id> <owner/repository> <issue-number> <captured-at> <output>

The capture command requires a separately installed GitHub CLI (gh).
Authenticate with gh auth login or supply GH_TOKEN. captured-at must be an
RFC 3339 timestamp supplied by the caller so fixture creation is explicit.
";

const ISSUE_QUERY: &str = r"
query($owner: String!, $name: String!, $number: Int!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    id
    nameWithOwner
    licenseInfo { spdxId }
    issue(number: $number) {
      id number title state closedAt url body createdAt updatedAt lastEditedAt includesCreatedEdit
      labels(first: 100) { nodes { id name } pageInfo { hasNextPage } }
      assignees(first: 100) { nodes { id login } pageInfo { hasNextPage } }
      milestone { title }
      author { login ... on Node { id } }
      userContentEdits(first: 100) {
        nodes { id editedAt editor { login ... on Node { id } } diff deletedAt }
        pageInfo { hasNextPage }
      }
      comments(first: 100, after: $endCursor) {
        nodes {
          id body createdAt updatedAt lastEditedAt includesCreatedEdit
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
        "review-packet" => {
            let values: Vec<_> = args.collect();
            let [fixture, output] = values.as_slice() else {
                return Err("review-packet requires a fixture and output path".to_owned());
            };
            let bytes =
                fs::read(fixture).map_err(|error| format!("could not read {fixture}: {error}"))?;
            let packet = independent_review_packet(&bytes)?;
            let encoded = serde_json::to_string_pretty(&packet)
                .map_err(|error| format!("could not encode review packet: {error}"))?;
            fs::write(output, format!("{encoded}\n"))
                .map_err(|error| format!("could not write {output}: {error}"))?;
            println!("wrote gold-blind review packet to {output}");
            Ok(())
        }
        "review-verify" => {
            let values: Vec<_> = args.collect();
            let [fixture, packet] = values.as_slice() else {
                return Err("review-verify requires a fixture and review packet".to_owned());
            };
            let packet_path = packet;
            let fixture_bytes =
                fs::read(fixture).map_err(|error| format!("could not read {fixture}: {error}"))?;
            let packet_bytes = fs::read(packet_path)
                .map_err(|error| format!("could not read {packet_path}: {error}"))?;
            let packet: serde_json::Value = serde_json::from_slice(&packet_bytes)
                .map_err(|error| format!("could not parse {packet_path}: {error}"))?;
            verify_independent_review(&fixture_bytes, &packet)?;
            println!("verified gold-blind review packet {packet_path}");
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
    fixture_from_graphql_pages(fixture_id, captured_at, &output.stdout)
}
