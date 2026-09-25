#[path = "helpers/help_contract.rs"]
mod help_contract;

use help_contract::HelpScenario;

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
