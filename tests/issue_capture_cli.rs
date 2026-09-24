#[path = "helpers/issue_capture_cli.rs"]
mod issue_capture_cli;

use issue_capture_cli::Capture;

#[test]
fn refresh_captures_one_comment_and_dispatches_it_once() {
    Capture::given_a_new_project()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .then_reuses_the_binding()
        .when_the_issue_is_captured()
        .then_changes_nothing();
}

#[test]
fn edits_and_observed_deletions_preserve_the_captured_body() {
    Capture::given_a_new_project()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(2)
        .when_the_comment_is_edited()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .when_the_comment_disappears()
        .when_the_issue_is_captured()
        .then_records_one_deletion()
        .when_the_original_comment_is_read()
        .then_keeps_the_original_body()
        .when_the_issue_is_captured()
        .then_changes_nothing();
}

#[test]
fn provider_facts_share_the_project_delta_and_inbox() {
    Capture::given_a_new_project()
        .when_the_issue_is_captured()
        .when_the_provider_closes_the_issue()
        .when_the_issue_is_captured()
        .then_changes_only_provider_state()
        .when_changes_are_read()
        .then_delta_and_inbox_reach_the_same_revision();
}

#[test]
fn refresh_reuses_the_binding_capture_policy() {
    Capture::given_a_new_project()
        .given_optional_capture_policy()
        .when_the_issue_is_captured()
        .then_leaves_optional_sources_cold()
        .when_capture_policy_flags_are_omitted()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .then_leaves_optional_sources_cold();
}

#[test]
fn incomplete_provider_pages_do_not_imply_deletions() {
    Capture::given_a_new_project()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .when_the_provider_page_is_truncated()
        .when_the_issue_is_captured()
        .then_reports_incomplete_without_changes();
}

#[test]
fn compiler_failures_are_visible_and_not_dispatched_again() {
    Capture::given_a_new_project()
        .given_a_failing_compiler()
        .when_the_issue_is_captured()
        .then_reports_failed_compilation()
        .when_the_issue_is_captured()
        .then_reports_the_same_failure_without_dispatch();
}

#[test]
fn a_new_comment_after_a_deletion_still_gets_causal_compiler_work() {
    Capture::given_a_new_project()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .when_the_comment_disappears()
        .when_the_issue_is_captured()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .when_the_compiler_inputs_are_read()
        .then_context_records_the_deletion_and_stays_within_its_cutoff();
}

#[test]
fn a_deleted_comment_marks_its_accepted_decision_for_revalidation() {
    Capture::given_a_project_with_a_supported_comment()
        .when_the_comment_disappears()
        .when_the_issue_is_captured()
        .then_records_one_deletion()
        .when_the_decision_is_read()
        .then_requires_evidence_revalidation();
}

#[test]
fn overlapping_captures_do_not_dispatch_the_same_intent() {
    Capture::given_a_new_project()
        .given_a_provider_that_starts_an_overlapping_capture()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .then_the_overlapping_capture_is_busy();
}

#[test]
fn human_output_distinguishes_capture_outcomes() {
    Capture::given_a_new_project()
        .when_the_issue_is_captured_for_humans()
        .then_reports_to_humans("captured")
        .when_the_issue_is_captured_for_humans()
        .then_reports_to_humans("unchanged")
        .when_the_provider_page_is_truncated()
        .when_the_issue_is_captured_for_humans()
        .then_reports_to_humans("incomplete")
        .when_the_provider_is_unavailable()
        .when_the_issue_is_captured_for_humans()
        .then_reports_to_humans("failed");
}

#[test]
fn a_missing_compiler_does_not_establish_an_unusable_binding() {
    Capture::given_a_new_project()
        .when_capture_is_attempted_without_a_compiler()
        .then_requires_compiler_configuration()
        .when_optional_capture_is_selected()
        .when_the_issue_is_captured()
        .then_leaves_optional_sources_cold();
}

#[test]
fn an_older_provider_response_cannot_replace_a_captured_edit() {
    Capture::given_a_new_project()
        .when_a_comment_arrives()
        .when_the_issue_is_captured()
        .when_the_comment_is_edited()
        .when_the_issue_is_captured()
        .when_an_older_comment_snapshot_arrives()
        .then_rejects_the_stale_body_without_a_new_observation();
}

#[test]
fn optional_eager_sources_receive_compiler_work_once() {
    Capture::given_a_new_project()
        .given_optional_eager_policy()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1)
        .then_optional_work_leaves_required_coverage_complete()
        .when_the_issue_is_captured()
        .then_changes_nothing();
}

#[test]
fn optional_eager_failures_do_not_create_required_coverage_gaps() {
    Capture::given_a_new_project()
        .given_optional_eager_policy()
        .given_a_failing_compiler()
        .when_the_issue_is_captured()
        .then_reports_failed_compilation()
        .then_optional_work_leaves_required_coverage_complete()
        .when_the_issue_is_captured()
        .then_reports_the_same_failure_without_dispatch()
        .then_optional_work_leaves_required_coverage_complete();
}

#[test]
fn callers_cannot_set_merls_observation_time() {
    Capture::given_a_new_project()
        .when_a_future_observation_time_is_supplied()
        .then_rejects_the_observation_time_override()
        .when_the_issue_is_captured()
        .then_captures_and_compiles(1);
}

#[test]
fn binding_policy_changes_apply_only_to_future_captures() {
    issue_capture_cli::PolicyCases::given_cold_bindings_for_each_policy()
        .when_an_administrator_changes_policy_and_captures_new_activity()
        .then_each_version_keeps_its_policy_and_only_eager_work_runs();
}

#[test]
fn binding_policy_changes_require_authority_and_preserve_retry_results() {
    issue_capture_cli::PolicyCases::given_a_binding_with_an_administrator()
        .when_policy_changes_are_rejected_accepted_retried_and_contested()
        .then_only_authorized_current_changes_reach_the_audit_stream();
}

#[test]
fn prepared_binding_changes_guard_their_binding_and_authority() {
    issue_capture_cli::PolicyCases::given_bindings_with_competing_administrative_work()
        .when_prepared_changes_commit_after_other_work()
        .then_only_unrelated_binding_changes_can_commit();
}

#[test]
fn binding_policy_commands_explain_their_contract_and_reject_changed_retries() {
    issue_capture_cli::PolicyCases::given_a_binding_with_an_administrator()
        .when_help_human_results_and_retries_are_read()
        .then_help_and_results_preserve_policy_and_retry_identity();
}
