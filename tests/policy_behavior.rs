#[path = "helpers/policy_behavior.rs"]
mod policy_behavior;

use policy_behavior::PolicyScenario;

#[test]
fn a_relay_does_not_borrow_the_quoted_humans_authority() {
    PolicyScenario::given_an_authorized_human_and_an_agent_relay()
        .when_both_propose_the_same_decision()
        .then_only_the_direct_human_assertion_is_accepted()
        .then_each_disposition_has_a_provenance_path()
        .when_the_original_assertion_is_replayed_under_a_new_handle()
        .then_no_second_decision_is_created();
}

#[test]
fn unrelated_work_does_not_stale_an_evaluation_but_a_changed_dependency_does() {
    PolicyScenario::given_a_prepared_decision_with_a_read_dependency()
        .when_an_unrelated_object_changes()
        .then_the_decision_can_commit()
        .when_the_read_dependency_changes_before_another_commit()
        .then_the_stale_decision_is_rejected_without_partial_state();
}

#[test]
fn accepted_provider_facts_and_inbox_delivery_are_atomic_and_idempotent() {
    PolicyScenario::given_a_subscriber_and_a_trusted_issue_observation()
        .when_the_provider_observation_is_accepted_twice()
        .then_one_revision_and_one_inbox_entry_exist()
        .then_the_provider_fact_has_a_policy_provenance_path();
}
