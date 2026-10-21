//! Synthetic HTTP traffic only; workers must retain bubblewrap isolation.
#![cfg(target_os = "linux")]

#[path = "helpers/mock_provider.rs"]
mod mock_provider;
#[path = "helpers/restricted_transport.rs"]
mod restricted_transport;
use restricted_transport::Scenario;

#[test]
fn a_reader_uses_only_its_configured_transport_without_receiving_credentials() {
    Scenario::given_a_mock_provider_and_isolated_worker("success")
        .when_the_transport_runs()
        .then_the_provider_receives_only_the_pinned_request()
        .then_the_response_and_reported_cost_have_provenance();
}

#[test]
fn a_failed_worker_keeps_the_cost_of_its_completed_provider_call() {
    Scenario::given_a_mock_provider_and_isolated_worker("bad_final")
        .when_the_transport_runs()
        .then_the_provider_receives_only_the_pinned_request()
        .then_failure_keeps_reported_usage();
}

#[test]
fn transport_rejects_capability_changes_and_retains_provider_failures() {
    for mode in [
        "override_url",
        "redirect",
        "provider_failure",
        "missing_usage",
        "unavailable",
        "missing_credentials",
        "credential_echo",
        "call_limit",
        "bad_usage",
        "role_swap",
        "adapter_tamper",
    ] {
        Scenario::given_a_mock_provider_and_isolated_worker(mode)
            .when_the_transport_runs()
            .then_the_boundary_fails_closed();
    }
}

#[test]
fn failed_remote_reads_and_shared_preparation_keep_their_known_costs() {
    for mode in ["bad_final", "empty_summary"] {
        Scenario::given_a_mock_provider_and_isolated_worker(mode)
            .when_reader_trials_fail_after_provider_usage()
            .then_each_failed_read_and_shared_preparation_is_charged();
    }
}

#[test]
fn compiler_transport_keeps_its_native_response_and_honors_the_run_output_cap() {
    Scenario::given_a_mock_provider_and_isolated_worker("preparer")
        .when_the_transport_runs()
        .then_the_provider_receives_only_the_pinned_request()
        .then_the_response_and_reported_cost_have_provenance();
}

#[test]
fn a_reader_rejects_model_configuration_that_differs_from_its_transport() {
    Scenario::given_a_mock_provider_and_isolated_worker("success")
        .when_the_reader_requests_another_model()
        .then_no_provider_call_or_cost_is_attributed();
}
