#[path = "helpers/visible_package.rs"]
mod visible_package;

use visible_package::IntegrityCases;
use visible_package::PackageCase;

#[test]
fn approved_units_keep_exact_prompts_without_reopening_historical_approval_flags() {
    PackageCase::given_a_synthetic_approved_package()
        .when_the_package_is_verified()
        .then_three_units_share_two_prefixes_and_keep_their_prompts()
        .then_reader_stages_exclude_oracles_and_audit_records();
}

#[test]
fn rebound_package_tampering_cannot_change_the_approved_unit_contract() {
    IntegrityCases::given_independently_tampered_synthetic_packages()
        .when_the_packages_are_verified()
        .then_every_tampered_contract_is_rejected();
}

#[cfg(target_os = "linux")]
#[test]
fn semantic_workers_keep_the_source_pool_and_grader_state_out_of_reader_contexts() {
    PackageCase::given_a_synthetic_approved_package()
        .when_isolated_semantic_workers_read_and_grade()
        .then_three_units_share_two_prefixes_and_keep_their_prompts();
}

#[test]
fn partial_execution_keeps_missing_and_failed_units_out_of_success_counts() {
    PackageCase::given_a_synthetic_approved_package()
        .when_one_prefix_has_a_grading_failure()
        .then_all_scoring_slots_and_shared_costs_remain_accounted_for();
}

#[cfg(target_os = "linux")]
#[test]
fn legacy_grades_other_cutoffs_and_missing_reasons_cannot_become_semantic_successes() {
    PackageCase::given_a_synthetic_approved_package()
        .when_incompatible_grades_are_submitted()
        .then_no_incompatible_grade_receives_credit();
}

#[test]
fn a_failed_shared_summary_does_not_erase_costs_or_block_independent_methods() {
    PackageCase::given_a_synthetic_approved_package()
        .when_a_shared_summary_fails()
        .then_only_dependent_units_fail_and_known_preparation_cost_survives();
}

#[test]
fn a_prefix_boundary_keeps_timestamp_ambiguity_without_disclosing_the_next_body() {
    PackageCase::given_a_synthetic_approved_package()
        .when_the_cutoff_splits_a_timestamp_tie()
        .then_the_boundary_remains_ambiguous_and_cannot_claim_exact_replay();
}

#[test]
fn failed_grading_keeps_read_and_shared_preparation_costs_in_method_totals() {
    visible_package::CostCase::given_two_units_sharing_preparation()
        .when_the_first_units_answers_cannot_be_graded()
        .then_method_totals_include_all_reported_reads_and_preparation();
}
