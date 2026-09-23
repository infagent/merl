#[path = "helpers/assertion_policy.rs"]
mod assertion_policy;

use assertion_policy::AssertionScenario;

#[test]
fn a_compound_comment_gets_independent_outcomes_and_one_accepted_batch() {
    AssertionScenario::given_a_compiled_compound_comment()
        .when_the_run_is_inspected_and_applied()
        .then_each_assertion_has_its_own_outcome_and_exact_provenance()
        .when_the_application_is_retried_after_restart()
        .then_the_original_outcome_returns_without_another_batch();
}

#[test]
fn decision_authority_has_a_bounded_semantic_scope() {
    AssertionScenario::given_the_permission_matrix()
        .when_each_run_is_applied()
        .then_only_direct_explicit_decisions_are_accepted();
}

#[test]
fn non_live_or_unfinished_runs_cannot_change_accepted_state() {
    AssertionScenario::given_ineligible_runs()
        .when_each_run_is_applied()
        .then_each_run_is_explained_without_accepted_changes();
}

#[test]
fn retries_preserve_outcomes_while_new_attempts_use_current_grants() {
    AssertionScenario::given_a_decision_without_an_author_grant()
        .when_it_is_evaluated_before_and_after_grant_changes()
        .then_retries_are_stable_and_new_attempts_respect_authority();
}

#[test]
fn revocation_between_preparation_and_commit_returns_a_recorded_conflict() {
    AssertionScenario::given_a_compiled_compound_comment()
        .when_authority_is_revoked_before_the_prepared_application_commits()
        .then_the_attempt_conflicts_without_partial_semantic_state();
}

#[test]
fn unavailable_compiler_evidence_cannot_become_current_support() {
    AssertionScenario::given_a_compiled_compound_comment()
        .when_the_compiler_context_is_erased_before_application()
        .then_inspection_reports_unavailable_and_application_accepts_nothing();
}

#[test]
fn competing_assertions_do_not_choose_an_object_update_by_position() {
    AssertionScenario::given_competing_assertions()
        .when_each_run_is_applied()
        .then_competing_assertions_create_no_accepted_event();
}

#[test]
fn editing_evidence_between_preparation_and_commit_conflicts() {
    AssertionScenario::given_a_compiled_compound_comment()
        .when_evidence_changes_after_preparation()
        .then_changed_evidence_prevents_the_whole_batch();
}

#[test]
fn preview_explains_policy_without_accepting_state() {
    AssertionScenario::given_a_compiled_compound_comment()
        .when_the_application_is_previewed_and_help_is_requested()
        .then_the_preview_explains_outcomes_without_committing();
}
