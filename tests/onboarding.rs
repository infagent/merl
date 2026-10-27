#[path = "helpers/onboarding.rs"]
mod onboarding;

use onboarding::Onboarding;

#[test]
fn the_usage_primer_needs_no_user_setup_or_runtime_access() {
    Onboarding::given_an_unconfigured_user()
        .when_usage_and_help_are_requested_without_a_user_environment()
        .then_the_primer_and_json_are_pure_and_discoverable();
}

#[test]
fn canonical_setup_retains_legacy_bytes_roots_and_ownership() {
    Onboarding::given_completed_legacy_setups()
        .when_setup_is_upgraded_retried_and_uninstalled()
        .then_canonical_setup_works_and_only_owned_files_are_removed();
}

#[test]
fn incomplete_or_conflicting_legacy_setup_is_preserved_before_publication() {
    Onboarding::given_incomplete_and_conflicting_legacy_setups()
        .when_upgrade_is_attempted_and_incomplete_setup_is_safely_removed()
        .then_conflicts_write_nothing_and_v1_recovery_stays_available();
}

#[test]
fn interrupted_v2_publication_and_removal_resume_the_same_host_associations() {
    Onboarding::given_one_completed_legacy_setup()
        .when_publication_and_removal_are_interrupted_then_retried()
        .then_ownership_and_unselected_host_state_survive_recovery();
}
