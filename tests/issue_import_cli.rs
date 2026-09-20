#[path = "helpers/issue_import_cli.rs"]
mod issue_import_cli;

use issue_import_cli::CliIssueHistory;

#[test]
fn an_issue_fixture_import_is_offline_and_retryable() {
    CliIssueHistory::new()
        .given_a_new_project()
        .when_importing_an_issue()
        .then_four_versions_are_captured()
        .when_importing_the_same_issue_again()
        .then_nothing_new_is_captured();
}
