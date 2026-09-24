#[path = "helpers/assertion_policy.rs"]
mod assertion_policy;

use assertion_policy::AssertionScenario;

#[test]
fn compiler_relations_keep_independent_policy_and_reviewed_provenance() {
    AssertionScenario::given_a_decision_and_a_grounded_relation()
        .when_the_relation_is_applied_reviewed_and_rebuilt()
        .then_the_relation_retains_its_compiler_basis_and_focused_view();
}

#[test]
fn invalid_relations_do_not_discard_an_independent_decision() {
    AssertionScenario::given_a_decision_and_invalid_relations()
        .when_the_run_is_inspected_and_applied()
        .then_only_the_relations_are_rejected();
}

#[test]
fn accepted_relations_expand_to_evidence_and_follow_purge_lineage() {
    AssertionScenario::given_a_decision_and_a_grounded_relation()
        .when_the_relation_is_applied_reviewed_and_rebuilt()
        .when_the_accepted_relation_is_expanded_and_its_source_purge_previewed()
        .then_expansion_and_purge_name_the_exact_relation_origin();
}

#[test]
fn relation_only_reviews_require_current_authority_endpoints_and_evidence() {
    assertion_policy::RelationReviewCases::given_relation_only_candidates()
        .when_reviews_run_across_dependency_changes()
        .then_only_current_authorized_reviews_accept_edges();
}

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

#[test]
fn a_reviewer_can_find_and_accept_a_candidate_after_restart() {
    AssertionScenario::given_a_candidate_for_review()
        .when_the_candidate_is_inspected_and_accepted()
        .then_review_keeps_provenance_and_accepts_once();
}

#[test]
fn rejection_keeps_the_interpretation_and_an_erasable_reason() {
    AssertionScenario::given_a_candidate_for_review()
        .when_the_candidate_is_rejected_and_its_reason_is_erased()
        .then_rejection_survives_without_retaining_reason_text();
}

#[test]
fn correction_preserves_the_original_interpretation() {
    AssertionScenario::given_a_candidate_for_review()
        .when_the_candidate_is_corrected()
        .then_the_accepted_object_expands_to_both_interpretations();
}

#[test]
fn review_cannot_bypass_authority_or_changed_evidence() {
    AssertionScenario::given_a_candidate_for_review()
        .when_review_is_attempted_without_permission_then_after_an_edit()
        .then_unauthorized_review_is_rejected_and_stale_review_conflicts();
}

#[test]
fn prepared_reviews_cannot_outlive_their_authority_evidence_or_candidate() {
    assertion_policy::ReviewCases::given_prepared_review_cases()
        .when_dependencies_change_before_commit()
        .then_each_changed_dependency_records_a_conflict();
}

#[test]
fn corrected_evidence_remains_expandable_after_another_object_update() {
    AssertionScenario::given_a_candidate_for_review()
        .when_the_candidate_is_corrected_then_the_object_changes()
        .then_history_retains_the_correction_and_original_evidence();
}

#[test]
fn review_previews_and_retries_preserve_their_public_contract() {
    AssertionScenario::given_a_candidate_for_review()
        .when_review_is_previewed_then_retried_after_revocation_and_rebuild()
        .then_preview_is_read_only_and_retry_preserves_the_original_outcome();
}

#[test]
fn interrupted_review_retries_reuse_matching_reasons_without_overwriting_or_restoring_them() {
    assertion_policy::ReviewRecoveryCases::given_review_reasons_without_requests()
        .when_the_same_commands_are_retried_after_restart()
        .then_matching_reasons_recover_and_conflicting_or_erased_reasons_stay_unchanged();
}
