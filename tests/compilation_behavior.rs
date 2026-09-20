#[path = "helpers/compilation_behavior.rs"]
mod compilation_behavior;

use compilation_behavior::{CompilationScenario, HistoricalIssue};

#[test]
fn late_compilation_keeps_the_authors_world_separate_from_today() {
    CompilationScenario::given_a_note_followed_by_a_later_decision()
        .when_the_note_is_compiled_on_demand()
        .then_the_compiler_sees_only_the_earlier_world()
        .then_the_run_is_recorded_once_for_all_readers();
}

#[test]
fn required_failures_are_visible_but_optional_cold_notes_do_not_block_coverage() {
    CompilationScenario::given_required_and_optional_notes()
        .when_the_required_note_fails_its_output_budget()
        .then_coverage_reports_the_required_failure_only();
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
