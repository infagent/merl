#[path = "helpers/help_contract.rs"]
mod help_contract;

use help_contract::HelpScenario;

#[cfg(unix)]
#[path = "helpers/help_errors.rs"]
mod help_errors;

#[cfg(unix)]
use help_errors::ErrorScenario;

#[test]
fn every_discoverable_command_explains_its_contract_in_both_formats() {
    HelpScenario::given_the_merl_cli()
        .when_the_help_tree_is_read()
        .then_each_command_has_complete_matching_help();
}

#[test]
fn help_distinguishes_compiler_work_from_reads_and_reports_reachable_errors() {
    HelpScenario::given_the_merl_cli()
        .when_public_failures_and_their_help_are_read()
        .then_help_matches_the_command_error_boundary();
}

#[test]
fn help_preserves_the_outcome_names_emitted_by_commands() {
    HelpScenario::given_the_merl_cli()
        .when_a_project_is_created_with_its_help()
        .then_help_names_the_emitted_outcome();
}

#[test]
fn error_help_respects_payload_reads_and_recorded_conflict_outcomes() {
    HelpScenario::given_the_merl_cli()
        .when_the_help_tree_is_read()
        .then_errors_stay_with_the_commands_that_expose_them();
}

#[cfg(unix)]
#[test]
fn changed_compiler_retry_configuration_has_a_documented_identity_conflict() {
    ErrorScenario::given_a_recorded_compilation()
        .when_the_same_request_retries_with_changed_compiler_configuration()
        .then_help_names_the_retry_conflict();
}

#[cfg(unix)]
#[test]
fn metadata_help_does_not_claim_to_open_erased_policy_reasons() {
    ErrorScenario::given_a_binding_with_an_erased_policy_reason()
        .when_binding_metadata_and_help_are_read()
        .then_metadata_remains_readable_without_payload_errors();
}

#[cfg(unix)]
#[test]
fn compiler_failures_and_erased_input_keep_their_distinct_help_errors() {
    ErrorScenario::given_a_recorded_compilation()
        .when_compilers_fail_and_recorded_input_is_erased()
        .then_help_names_each_failure_at_its_public_boundary();
}
