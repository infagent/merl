#[path = "helpers/context_expansion.rs"]
mod context_expansion;

use context_expansion::Expansion;

#[test]
fn requested_context_survives_restart_and_completes_with_causal_evidence() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_it_requests_the_decision()
        .when_the_work_is_inspected_after_restart()
        .then_the_request_is_pending_and_coverage_is_open();
}

#[test]
fn bounded_rounds_resume_saved_input_and_expose_their_full_lineage() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_it_requests_the_decision()
        .when_two_rounds_resume_after_restart()
        .then_only_requested_historical_context_is_added()
        .then_final_assertions_expand_through_both_rounds();
}

#[test]
fn unavailable_references_loops_and_limits_keep_required_coverage_open() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_invalid_or_exhausted_expansions_are_attempted()
        .then_each_failure_is_durable_without_partial_assertions();
}

#[test]
fn an_explicit_source_reference_adds_only_that_earlier_project_source() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_a_later_note_requests_another_threads_source()
        .then_only_the_named_source_is_added();
}

#[test]
fn the_cli_resumes_one_round_with_the_original_compiler_configuration() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_the_cli_resumes_and_retries_process_work()
        .then_the_original_budget_and_completed_successor_are_reused();
}

#[test]
fn erasure_before_intent_commit_cannot_be_undone_by_expansion() {
    Expansion::given_a_compiler_missing_an_earlier_decision()
        .when_it_requests_the_decision()
        .when_erasure_intervenes_before_the_expanded_intent_commits()
        .then_erased_content_has_no_new_retained_copy();
}
