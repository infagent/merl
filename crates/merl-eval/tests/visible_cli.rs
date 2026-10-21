//! Only synthetic package bytes cross the evaluator CLI in these checks.
#[path = "helpers/visible_cli.rs"]
mod visible_cli;
use visible_cli::Scenario;

#[test]
fn verify_lists_all_units_without_running_readers() {
    Scenario::given_a_synthetic_package()
        .when_the_package_is_verified()
        .then_all_units_are_verified_without_scoring();
}

#[cfg(target_os = "linux")]
#[test]
fn an_executed_prefix_keeps_exclusions_and_missing_units_out_of_success_counts() {
    Scenario::given_a_synthetic_package()
        .when_synthetic_readers_execute_a_prepared_prefix()
        .then_exclusions_failures_missing_units_and_shared_costs_remain_explicit();
}
