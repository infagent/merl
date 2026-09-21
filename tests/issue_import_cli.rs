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

#[test]
fn an_operator_can_preview_and_confirm_a_source_purge() {
    CliIssueHistory::new()
        .given_two_projects_with_the_same_issue()
        .when_a_source_purge_is_previewed()
        .then_the_preview_names_protected_bytes()
        .when_the_preview_is_confirmed()
        .then_source_content_is_unavailable_and_the_audit_is_complete()
        .then_the_other_projects_copy_remains_available();
}

#[test]
fn purge_scrubs_the_active_store_when_no_other_project_retains_the_bytes() {
    CliIssueHistory::new()
        .given_a_new_project()
        .when_importing_an_issue()
        .when_a_source_purge_is_previewed()
        .when_the_preview_is_confirmed()
        .then_active_store_no_longer_contains_the_source_body();
}
