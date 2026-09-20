#[path = "helpers/issue_state.rs"]
mod issue_state;

use issue_state::IssueScenario;

#[test]
fn provider_facts_and_issue_semantics_survive_projection_rebuild() {
    IssueScenario::given_an_imported_issue_with_a_local_decision()
        .when_the_issue_state_is_read()
        .then_provider_facts_and_decision_are_separate()
        .when_the_projection_is_rebuilt()
        .then_provider_facts_and_decision_are_separate();
}

#[test]
fn a_semantic_command_cannot_replace_a_provider_fact() {
    IssueScenario::given_an_imported_issue_with_a_local_decision()
        .when_a_command_tries_to_replace_the_provider_mirror()
        .then_the_command_is_rejected_and_provider_state_is_unchanged();
}
