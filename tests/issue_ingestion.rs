#[path = "helpers/issue_ingestion.rs"]
mod issue_ingestion;

use issue_ingestion::{IssueHistory, report_format_issue, two_page_github_issue};

#[test]
fn issue_edits_are_captured_once_and_keep_their_source_lineage() {
    let fixture = report_format_issue();
    IssueHistory::new("P1")
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_source_versions_keep_their_order(&fixture)
        .then_issue_is_open_without_duplicate_project_changes(&fixture)
        .then_each_edit_supersedes_the_prior_version()
        .then_edit_provenance_remains_cold()
        .then_original_issue_body_remains_available()
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes(&fixture)
        .when_recapturing_later(&fixture)
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes(&fixture);
}

#[test]
fn an_edit_keeps_its_editor_and_protected_diff() {
    IssueHistory::new("P2")
        .when_importing_an_edit_by_another_person()
        .then_keeps_the_editors_identity_and_diff();
}

#[test]
fn an_older_provider_update_cannot_reopen_a_newer_closed_issue_even_if_polled_later() {
    IssueHistory::new("P3")
        .given_the_provider_closed_the_issue()
        .when_an_older_open_snapshot_arrives_later()
        .then_keeps_the_issue_closed()
        .when_an_unversioned_open_snapshot_arrives_even_later()
        .then_keeps_the_issue_closed();
}

#[test]
fn a_github_issue_import_keeps_provider_identity_and_version_lineage() {
    let fixture = two_page_github_issue();
    IssueHistory::new("P4")
        .when_importing(&fixture)
        .then_has_four_source_versions()
        .then_github_identity_and_edits_survive_import(&fixture)
        .then_provider_facts_remain_available(&fixture)
        .when_provider_snapshot_bytes_are_erased(&fixture)
        .then_provider_facts_remain_available(&fixture);
}

#[test]
fn a_retry_keeps_the_first_capture_time_and_policy() {
    IssueHistory::new("P5")
        .when_a_source_is_recaptured_under_another_policy()
        .then_first_capture_metadata_still_applies();
}
