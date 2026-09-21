#[path = "helpers/evaluation_behavior.rs"]
mod evaluation_behavior;

use evaluation_behavior::EvaluationScenario;

#[test]
fn a_benchmark_reader_cannot_see_a_later_correction() {
    EvaluationScenario::given_the_gain_discussion()
        .when_raw_and_recent_contexts_are_prepared_at(3)
        .then_both_show_the_fixed_gain_decision()
        .then_neither_shows_the_later_sweep_decision();
}

#[test]
fn an_edit_replaces_the_visible_body_without_rewriting_earlier_cutoffs() {
    EvaluationScenario::given_the_edited_export_discussion()
        .when_raw_and_recent_contexts_are_prepared_at(1)
        .then_both_show_the_original_export_format()
        .when_raw_and_recent_contexts_are_prepared_at(2)
        .then_both_show_the_edited_export_format()
        .then_neither_shows_the_original_wording();
}
