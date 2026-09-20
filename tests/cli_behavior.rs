#[path = "helpers/cli_behavior.rs"]
mod cli_behavior;

use cli_behavior::CliScenario;

#[test]
fn cli_help_is_incremental_and_machine_readable() {
    CliScenario::given_the_merl_cli()
        .when_help_is_requested_at_each_depth()
        .then_help_is_incremental_and_machine_readable()
        .when_a_project_is_initialized()
        .then_initialization_has_a_versioned_result()
        .when_an_unknown_command_is_used()
        .then_the_error_has_versioned_json();
}

#[test]
fn the_last_format_option_also_controls_errors() {
    CliScenario::given_the_merl_cli()
        .when_human_format_is_last()
        .then_errors_use_human_format()
        .when_json_format_is_last()
        .then_errors_use_json_format();
}
