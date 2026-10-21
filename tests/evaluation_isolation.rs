#[cfg(target_os = "linux")]
#[path = "helpers/evaluation_isolation.rs"]
mod evaluation_isolation;

#[cfg(target_os = "linux")]
use evaluation_isolation::IsolationCase;

#[cfg(target_os = "linux")]
#[test]
fn reader_processes_see_only_their_unit_and_cannot_retrieve_host_evidence() {
    IsolationCase::given_two_units_and_private_host_evidence()
        .when_each_unit_runs_in_a_fresh_reader()
        .then_only_the_selected_files_are_visible()
        .then_host_files_and_services_are_unreachable()
        .then_later_readers_cannot_inherit_another_units_state();
}
