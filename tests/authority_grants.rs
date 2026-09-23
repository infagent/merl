#[path = "helpers/authority_grants.rs"]
mod authority_grants;

use authority_grants::AuthorityScenario;

#[test]
fn administrators_can_manage_durable_semantic_grants() {
    AuthorityScenario::given_two_projects_and_an_administrator()
        .when_both_semantic_permissions_are_granted()
        .when_grants_are_read_after_restart()
        .then_grants_and_audit_are_visible_only_in_the_target_project()
        .when_both_permissions_are_revoked()
        .when_grants_are_read_after_restart()
        .then_revocation_changes_the_digest_and_preserves_history();
}

#[test]
fn a_non_administrator_cannot_grant_or_revoke_authority() {
    AuthorityScenario::given_two_projects_and_an_administrator()
        .when_both_semantic_permissions_are_granted()
        .when_an_outsider_attempts_grant_and_revoke()
        .then_rejections_leave_grants_and_revision_unchanged();
}

#[test]
fn retrying_a_grant_does_not_reapply_it_after_revocation() {
    AuthorityScenario::given_two_projects_and_an_administrator()
        .when_both_semantic_permissions_are_granted()
        .when_both_permissions_are_revoked()
        .when_the_original_grant_is_retried()
        .then_the_original_result_returns_without_restoring_authority()
        .when_the_request_id_is_reused_for_another_actor()
        .then_the_retry_conflicts();
}

#[test]
fn provider_and_administrative_policy_share_the_same_durable_grants() {
    AuthorityScenario::given_two_projects_and_an_administrator()
        .when_both_semantic_permissions_are_granted()
        .when_provider_capture_and_an_administrative_change_are_evaluated()
        .then_both_boundaries_record_the_same_configuration()
        .when_the_provider_capture_is_retried_after_the_grant_change()
        .then_the_capture_retry_preserves_accepted_history();
}

#[test]
fn authority_help_and_human_results_explain_the_local_contract() {
    AuthorityScenario::given_two_projects_and_an_administrator()
        .when_authority_help_and_human_grant_results_are_read()
        .then_help_and_results_name_the_permission_outcome_and_revision();
}
