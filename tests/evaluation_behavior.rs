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
