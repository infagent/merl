#[path = "helpers/kernel_behavior.rs"]
mod kernel_behavior;

use kernel_behavior::{ProjectIsolationScenario, ProjectScenario};

#[test]
fn accepted_history_rebuilds_and_a_protected_payload_can_disappear() {
    ProjectScenario::given_a_new_project("P1")
        .given_protected_statement("D1", b"Keep the baseline gain fixed")
        .when_an_accepted_batch_puts_the_decision("B1", "E1", "D1")
        .then_revision_is(1)
        .then_decision_is_at_revision("D1", 1)
        .when_projection_is_rebuilt()
        .then_decision_is_at_revision("D1", 1)
        .when_the_statement_is_erased()
        .then_history_still_names_the_decision()
        .then_statement_is_unavailable();
}

#[test]
fn identical_statements_in_two_projects_have_separate_erasure_fates() {
    ProjectIsolationScenario::given_two_projects_with_the_same_statement()
        .when_one_project_erases_its_copy()
        .then_only_that_projects_copy_is_unavailable();
}

#[test]
fn a_bad_event_cannot_leave_half_an_accepted_batch() {
    ProjectScenario::given_a_new_project("P1")
        .when_a_batch_references_a_missing_payload()
        .then_no_part_of_the_batch_is_accepted();
}
