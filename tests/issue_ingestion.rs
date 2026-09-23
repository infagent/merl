#[path = "helpers/issue_ingestion.rs"]
mod issue_ingestion;

use issue_ingestion::IssueHistory;

#[test]
fn issue_edits_are_captured_once_and_keep_their_source_lineage() {
    IssueHistory::new("P1")
        .given_a_report_format_issue()
        .when_the_issue_is_imported()
        .then_has_four_source_versions()
        .then_source_versions_keep_their_order()
        .then_issue_is_open_without_duplicate_project_changes()
        .then_provider_fact_has_a_policy_decision()
        .then_each_edit_supersedes_the_prior_version()
        .then_edit_provenance_remains_cold()
        .then_original_issue_body_remains_available()
        .when_the_issue_is_imported()
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes()
        .when_recapturing_later()
        .then_has_four_source_versions()
        .then_issue_is_open_without_duplicate_project_changes();
}

#[test]
fn an_edit_keeps_its_editor_and_protected_diff() {
    IssueHistory::new("P2")
        .given_an_edit_by_another_person()
        .when_the_issue_is_imported()
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
    IssueHistory::new("P4")
        .given_a_two_page_github_issue()
        .when_the_issue_is_imported()
        .then_has_four_source_versions()
        .then_github_identity_and_edits_survive_import()
        .then_provider_facts_remain_available()
        .when_provider_snapshot_bytes_are_erased()
        .then_provider_facts_remain_available();
}

#[test]
fn a_retry_keeps_the_first_capture_time_and_policy() {
    IssueHistory::new("P5")
        .given_a_captured_source()
        .when_the_source_is_recaptured_under_another_policy()
        .then_first_capture_metadata_still_applies();
}

#[test]
fn a_retry_cannot_change_a_known_entity_author() {
    IssueHistory::new("P7")
        .given_a_source_authored_by_alice_and_edited_by_bob()
        .when_the_source_is_retried_with_author(Some("bob"))
        .then_the_retry_reports_an_author_conflict()
        .then_the_source_provenance_is_unchanged();
}

#[test]
fn a_retry_accepts_the_same_author_or_a_missing_author_id() {
    IssueHistory::new("P8")
        .given_a_source_authored_by_alice_and_edited_by_bob()
        .when_the_source_is_retried_with_author(Some("alice"))
        .then_the_retry_is_a_no_op()
        .then_the_source_provenance_is_unchanged()
        .when_the_source_is_retried_with_author(None)
        .then_the_retry_is_a_no_op()
        .then_the_source_provenance_is_unchanged();
}

#[test]
fn a_retry_cannot_fill_in_an_unknown_entity_author() {
    IssueHistory::new("P9")
        .given_a_captured_source()
        .when_the_source_is_retried_with_author(Some("alice"))
        .then_the_retry_is_a_no_op()
        .then_the_source_provenance_is_unchanged();
}

#[test]
fn github_source_update_times_remain_queryable_without_backdating_them() {
    IssueHistory::new("P6")
        .given_a_github_issue_with_an_edit()
        .when_the_issue_is_imported()
        .then_current_issue_and_comment_updates_are_queryable()
        .then_the_earlier_issue_version_has_no_future_update_time();
}
