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

#[test]
fn an_operator_can_rebuild_the_accepted_projection_without_recompiling_sources() {
    CliIssueHistory::new()
        .given_a_new_project()
        .when_importing_an_issue()
        .when_the_project_projection_is_rebuilt()
        .then_the_accepted_revision_and_issue_state_are_unchanged();
}

#[test]
fn an_operator_can_verify_a_recorded_compiler_input_from_causal_sources() {
    CliIssueHistory::new()
        .given_an_issue_with_a_recorded_compiler_run()
        .when_the_recorded_source_is_replayed()
        .then_the_input_digest_matches_without_an_accepted_change();
}

#[cfg(unix)]
#[test]
fn an_operator_can_rerun_a_compiler_without_accepting_its_new_assertions() {
    CliIssueHistory::new()
        .given_an_issue_with_a_recorded_compiler_run()
        .given_a_deterministic_compiler_process()
        .when_the_recorded_source_is_recompiled()
        .then_a_new_replay_run_exists_without_an_accepted_change();
}

#[test]
fn a_purged_source_cannot_be_claimed_as_an_exact_replay() {
    CliIssueHistory::new()
        .given_an_issue_with_a_recorded_compiler_run()
        .when_a_source_purge_is_previewed()
        .when_the_preview_is_confirmed()
        .when_the_purged_source_is_replayed()
        .then_replay_reports_missing_evidence();
}
