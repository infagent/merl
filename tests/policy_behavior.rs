#[path = "helpers/policy_behavior.rs"]
mod policy_behavior;

use policy_behavior::PolicyScenario;

#[test]
fn each_accepted_object_identifies_the_input_that_produced_it() {
    PolicyScenario::given_two_authorized_command_proposals()
        .when_they_are_accepted_together()
        .then_each_object_resolves_to_its_own_input();
}

#[test]
fn concurrent_preparation_of_the_same_command_applies_it_once() {
    PolicyScenario::given_two_prepared_evaluations_of_the_same_command()
        .when_both_are_committed()
        .then_the_second_is_recorded_as_a_duplicate();
}

#[test]
fn concurrent_preparation_of_one_assertion_under_two_handles_applies_it_once() {
    PolicyScenario::given_two_prepared_evaluations_of_one_assertion()
        .when_both_are_committed()
        .then_the_second_is_recorded_as_a_duplicate();
}

#[test]
fn a_prepared_batch_with_old_and_new_inputs_waits_for_reevaluation() {
    PolicyScenario::given_a_prepared_batch_with_one_competing_input()
        .when_the_competing_input_commits_first()
        .then_the_mixed_batch_conflicts_without_applying_its_new_input();
}

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
        .when_the_command_is_retried_with_the_same_meaning()
        .then_the_retry_creates_no_new_revision()
        .when_the_read_dependency_changes_before_another_commit()
        .then_the_stale_decision_is_rejected_without_partial_state();
}

#[test]
fn accepted_provider_facts_and_inbox_delivery_are_atomic_and_idempotent() {
    PolicyScenario::given_a_subscriber_and_a_trusted_issue_observation()
        .when_a_batch_references_an_unavailable_payload()
        .then_neither_state_nor_inbox_exposes_that_batch()
        .when_the_provider_observation_is_accepted_twice()
        .then_one_revision_and_one_inbox_entry_exist()
        .then_the_provider_fact_has_a_policy_provenance_path();
}

#[test]
fn a_rejected_command_cannot_reuse_its_identity_with_new_content() {
    PolicyScenario::given_an_agent_without_decision_authority()
        .when_the_agent_requests_a_decision()
        .then_the_request_is_rejected_without_a_project_change()
        .when_the_agent_retries_the_same_rejected_command()
        .then_the_retry_records_another_rejection_without_a_project_change()
        .when_the_agent_reuses_the_command_id_for_another_decision()
        .then_the_second_request_conflicts_without_a_project_change()
        .when_an_administrator_performs_a_maintenance_action()
        .then_only_the_administrators_action_changes_accepted_state();
}

#[test]
fn editing_decision_evidence_requires_revalidation_without_withdrawing_the_decision() {
    PolicyScenario::given_a_decision_supported_by_an_issue_comment()
        .when_the_comment_is_edited()
        .then_the_decision_is_active_with_revalidation_pending()
        .when_the_object_projection_is_rebuilt()
        .then_the_decision_is_active_with_revalidation_pending()
        .when_the_edit_confirms_the_same_decision()
        .then_the_decision_has_current_support_again();
}

#[test]
fn erasing_the_only_source_marks_decision_support_unavailable() {
    PolicyScenario::given_a_decision_supported_by_an_issue_comment()
        .when_the_source_bytes_are_erased()
        .then_the_decision_remains_but_support_is_unsupported();
}

#[test]
fn purge_previews_the_derivation_before_erasing_source_and_derived_bytes() {
    PolicyScenario::given_a_decision_supported_by_an_issue_comment()
        .when_the_source_purge_is_previewed()
        .then_the_preview_names_the_affected_run_and_decision()
        .when_the_preview_is_confirmed()
        .then_source_and_compiler_bytes_are_unavailable_with_an_audit_record();
}

#[test]
fn purging_context_evidence_makes_the_later_compilation_incomplete() {
    PolicyScenario::given_a_decision_compiled_with_an_earlier_comment()
        .when_the_earlier_comment_is_purged()
        .then_both_required_sources_report_unavailable_evidence();
}

#[test]
fn editing_context_evidence_reconsiders_an_assertion_citing_another_comment() {
    PolicyScenario::given_a_decision_compiled_with_an_earlier_comment()
        .when_the_earlier_comment_is_edited()
        .then_the_decision_is_active_with_revalidation_pending()
        .when_the_authority_restarts()
        .when_pending_evidence_work_is_listed()
        .then_the_affected_derivation_can_be_resumed()
        .when_the_affected_derivation_is_recompiled_and_confirmed()
        .then_the_context_impact_is_resolved_and_support_is_current()
        .when_pending_evidence_work_is_listed()
        .then_no_evidence_work_remains();
}

#[test]
fn provider_facts_cannot_replace_an_accepted_semantic_object() {
    PolicyScenario::given_a_decision_supported_by_an_issue_comment()
        .when_a_provider_observation_targets_that_decision()
        .then_the_provider_observation_is_rejected_and_the_decision_remains();
}
