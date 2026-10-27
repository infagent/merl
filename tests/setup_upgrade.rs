#![cfg(unix)]

#[path = "helpers/setup_upgrade.rs"]
mod setup_upgrade;
use setup_upgrade::Upgrade;

#[test]
fn installer_reuses_saved_choices_and_only_confirms_real_setup_changes() {
    Upgrade::given_current_and_older_saved_setups()
        .when_the_verified_installer_runs_again()
        .then_current_setup_is_quiet_and_real_changes_have_one_specific_confirmation();
}

#[test]
fn legacy_cleanup_preserves_ownership_and_recovers_before_acknowledgment() {
    Upgrade::given_legacy_alias_ownership_and_interruption_cases()
        .when_owned_alias_cleanup_is_planned_interrupted_and_retried()
        .then_only_eligible_aliases_and_records_are_removed_safely();
}
