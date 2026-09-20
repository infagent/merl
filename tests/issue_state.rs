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

#[test]
fn an_issue_relation_survives_projection_rebuild() {
    IssueScenario::given_an_imported_issue_with_a_local_decision()
        .when_the_decision_is_linked_to_the_issue()
        .then_the_issue_view_lists_the_relation()
        .when_the_projection_is_rebuilt()
        .then_the_issue_view_lists_the_relation();
}

#[test]
fn an_authorized_relation_command_has_a_durable_policy_origin() {
    IssueScenario::given_an_imported_issue_with_a_local_decision()
        .when_an_authorized_command_links_the_decision()
        .then_the_relation_is_accepted_with_its_input_origin();
}

#[test]
fn an_accepted_replacement_changes_lifecycle_without_changing_evidence_health() {
    IssueScenario::given_an_imported_issue_with_a_local_decision()
        .when_a_later_decision_supersedes_the_original()
        .then_the_original_is_superseded_but_still_supported()
        .when_the_projection_is_rebuilt()
        .then_the_original_is_superseded_but_still_supported();
}
