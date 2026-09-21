#[path = "helpers/evaluation_behavior.rs"]
mod evaluation_behavior;

use evaluation_behavior::EvaluationScenario;

#[test]
fn a_benchmark_reader_cannot_see_a_later_correction() {
    EvaluationScenario::given_the_gain_discussion()
        .when_raw_and_recent_readers_revisit_observation(3)
        .then_both_show_the_fixed_gain_decision()
        .then_neither_shows_the_later_sweep_decision();
}

#[test]
fn an_edit_replaces_the_visible_body_without_rewriting_earlier_cutoffs() {
    EvaluationScenario::given_the_edited_export_discussion()
        .when_raw_and_recent_readers_revisit_observation(1)
        .then_both_show_the_original_export_format()
        .when_raw_and_recent_readers_revisit_observation(2)
        .then_both_show_the_edited_export_format()
        .then_neither_shows_the_original_wording();
}

#[test]
fn five_reading_methods_share_one_question_and_count_preparation_per_trial() {
    EvaluationScenario::given_the_gain_discussion()
        .when_five_methods_answer_two_paired_trials_at(4)
        .then_every_reader_received_the_same_question()
        .then_each_pair_shares_one_randomness_setting_and_repeats_differ()
        .then_the_report_has_two_trials_per_method()
        .then_summary_and_merl_preparation_are_counted_once_per_trial()
        .then_correctness_and_provenance_are_reported_with_variance()
        .then_each_method_has_a_break_even_result();
}

#[test]
fn readers_pull_old_sources_and_merl_evidence_only_when_requested() {
    EvaluationScenario::given_the_gain_discussion()
        .when_retrieval_and_merl_expansion_are_requested_at(4)
        .then_only_tool_enabled_readers_expand_details()
        .then_expansion_calls_are_included_in_token_cost();
}

#[test]
fn an_inexact_history_cannot_be_scored_as_a_causal_trial() {
    EvaluationScenario::given_the_gain_discussion_with_inexact_earlier_bodies()
        .when_a_causal_reader_requests_the_history()
        .then_a_causal_reader_refuses_it();
}

#[test]
fn an_available_evidence_reader_marks_a_missing_body_without_using_its_later_version() {
    EvaluationScenario::given_an_edit_with_an_unavailable_first_body()
        .when_a_reader_revisits_the_first_observation()
        .then_the_missing_body_is_visible_as_a_gap()
        .then_the_later_edit_is_not_disclosed();
}

#[test]
fn a_rolling_summary_receives_terminal_capture_text_only_at_capture() {
    EvaluationScenario::given_an_opening_body_known_only_from_terminal_capture()
        .when_five_methods_answer_two_paired_trials_at(1)
        .then_the_early_summaries_do_not_receive_the_opening_body()
        .when_five_methods_answer_two_paired_trials_at(4)
        .then_the_opening_body_is_disclosed_after_the_history();
}

#[test]
fn causal_import_withholds_terminal_only_text_at_an_earlier_cutoff() {
    EvaluationScenario::given_an_opening_body_known_only_from_terminal_capture()
        .when_imported_for_an_earlier_cutoff()
        .then_terminal_only_bytes_are_not_in_the_authority();
}

#[test]
fn terminal_capture_does_not_retroactively_become_historical_compiler_input() {
    EvaluationScenario::given_an_opening_body_known_only_from_terminal_capture()
        .when_imported_for_the_terminal_cutoff()
        .then_terminal_only_bytes_are_not_in_the_authority();
}

#[test]
fn two_questions_share_issue_preparation_within_each_paired_trial() {
    EvaluationScenario::given_the_gain_discussion()
        .when_two_questions_use_the_same_issue_cutoff()
        .then_preparation_is_charged_once_per_trial_and_method();
}

#[test]
fn timestamp_ties_remain_visible_without_claiming_an_upstream_order() {
    EvaluationScenario::given_two_sources_with_unresolved_order()
        .when_a_reader_revisits_the_tied_cutoff()
        .then_both_sources_are_labeled_unordered()
        .when_a_causal_reader_requests_the_history()
        .then_exact_replay_rejects_unresolved_order();
}
