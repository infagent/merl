#[path = "helpers/compilation_behavior.rs"]
mod compilation_behavior;

use compilation_behavior::{
    CompilationAuthorizationScenario, CompilationScenario, HistoricalIssue,
};

#[test]
fn on_demand_compilation_requires_the_exact_accepted_authorization() {
    CompilationAuthorizationScenario::given_an_on_demand_source_with_one_accepted_request()
        .when_low_level_preparation_is_attempted()
        .then_only_the_exact_authorized_run_is_prepared()
        .then_eager_work_does_not_need_on_demand_authorization();
}

#[test]
fn late_compilation_keeps_the_authors_world_separate_from_today() {
    CompilationScenario::given_a_note_followed_by_a_later_decision()
        .when_the_note_is_compiled_late()
        .then_the_compiler_sees_only_the_earlier_world()
        .then_the_run_is_recorded_once_for_all_readers();
}

#[test]
fn source_replay_rebuilds_the_recorded_input_without_changing_accepted_history() {
    CompilationScenario::given_a_note_followed_by_a_later_decision()
        .when_the_note_is_compiled_late()
        .when_the_recorded_input_is_rebuilt_and_the_compiler_is_rerun()
        .then_replay_matches_the_recorded_input_and_leaves_the_decision_alone()
        .when_the_source_bytes_are_erased()
        .when_the_recorded_input_is_rebuilt_again()
        .then_replay_reports_missing_evidence();
}

#[test]
fn an_older_selector_version_rebuilds_its_original_context() {
    CompilationScenario::given_many_unrelated_objects_and_a_named_decision()
        .when_an_older_selector_run_is_recorded()
        .when_that_run_is_rebuilt()
        .then_the_original_selection_and_bytes_match()
        .when_that_input_is_replayed_with_a_new_compiler()
        .then_the_new_run_uses_the_verified_bytes_and_original_selector();
}

#[test]
fn required_failures_are_visible_but_optional_cold_notes_do_not_block_coverage() {
    CompilationScenario::given_required_and_optional_notes()
        .when_the_required_note_fails_its_output_budget()
        .then_coverage_reports_the_required_failure_only();
}

#[test]
fn a_pending_retry_replaces_a_prior_failure_in_coverage() {
    CompilationScenario::given_required_and_optional_notes()
        .when_the_required_note_fails_its_output_budget()
        .when_a_retry_is_prepared()
        .then_coverage_reports_pending_instead_of_failed()
        .then_the_retry_has_a_later_durable_attempt_order();
}

#[test]
fn an_evaluation_run_does_not_claim_live_semantic_coverage() {
    CompilationScenario::given_required_and_optional_notes()
        .when_the_required_note_is_compiled_for_evaluation()
        .then_live_coverage_still_has_one_required_gap();
}

#[test]
fn a_historical_issue_never_uses_its_terminal_snapshot_for_earlier_comments() {
    HistoricalIssue::given_a_four_comment_issue()
        .when_the_first_decision_is_accepted_before_the_next_comment()
        .then_each_comment_sees_only_its_causal_history();
}

#[test]
fn a_configured_process_compiler_records_one_typed_assertion() {
    CompilationScenario::given_a_note_for_an_external_compiler()
        .when_the_configured_compiler_runs()
        .then_its_typed_assertion_and_model_provenance_are_retained();
}

#[test]
fn prepared_compiler_work_remains_visible_before_external_execution() {
    CompilationScenario::given_a_note_for_an_external_compiler()
        .when_the_authority_prepares_the_compiler_run()
        .then_pending_work_is_visible_without_a_running_worker();
}

#[test]
fn a_comment_uses_its_own_issue_history() {
    CompilationScenario::given_interleaved_issue_comments()
        .when_the_latest_comment_context_is_built()
        .then_other_issue_comments_are_absent();
}

#[test]
fn a_request_for_more_context_does_not_complete_required_coverage() {
    CompilationScenario::given_a_note_for_an_external_compiler()
        .when_the_compiler_requests_more_context()
        .then_the_run_needs_expansion_and_coverage_remains_open();
}

#[test]
fn a_pending_run_uses_its_saved_context_when_the_original_source_is_unavailable() {
    CompilationScenario::given_a_note_for_an_external_compiler()
        .when_the_run_is_recovered_after_source_bytes_disappear()
        .then_the_original_context_is_still_ready_for_execution();
}

#[test]
fn historical_replay_cannot_assign_a_future_revision_to_an_old_comment() {
    HistoricalIssue::given_a_four_comment_issue()
        .when_a_future_decision_is_accepted_before_the_first_comment_is_bound()
        .then_the_first_comment_rejects_the_future_basis();
}

#[test]
fn hindsight_can_read_current_state_without_claiming_live_coverage() {
    CompilationScenario::given_a_required_note_followed_by_a_later_decision()
        .when_the_note_is_compiled_with_hindsight()
        .then_current_state_is_visible_but_required_coverage_remains_open();
}

#[test]
fn a_large_project_selects_only_the_budgeted_objects() {
    CompilationScenario::given_more_accepted_objects_than_the_context_budget()
        .when_the_latest_comment_context_is_built()
        .then_only_the_budgeted_objects_are_selected();
}

#[test]
fn an_issue_comment_keeps_its_named_decision_in_bounded_context() {
    CompilationScenario::given_many_unrelated_objects_and_a_named_decision()
        .when_the_issue_comment_is_compiled()
        .then_the_named_decision_is_selected_before_unrelated_objects();
}

#[test]
fn an_issue_comment_gets_its_own_decision_without_naming_the_handle() {
    CompilationScenario::given_many_other_issue_objects_and_one_local_decision()
        .when_the_issue_comment_is_compiled()
        .then_its_issue_decision_is_selected();
}

#[test]
fn issue_coverage_excludes_other_threads_and_optional_notes() {
    CompilationScenario::given_required_and_optional_issue_notes()
        .when_issue_coverage_is_inspected()
        .then_only_the_required_issue_note_is_a_gap()
        .when_the_required_issue_note_is_compiled()
        .then_issue_coverage_is_complete_with_an_optional_attachment();
}

#[test]
fn pending_compiler_work_records_every_selection_limit() {
    CompilationScenario::given_a_note_for_an_external_compiler()
        .when_the_authority_prepares_the_compiler_run()
        .then_all_nine_limits_are_retained();
}

#[test]
fn an_edited_comment_keeps_its_author_separate_from_its_editor() {
    CompilationScenario::given_an_edited_issue_with_a_different_editor()
        .when_the_edited_version_is_compiled()
        .then_the_assertion_uses_source_authorship_without_verifying_a_relay();
}
