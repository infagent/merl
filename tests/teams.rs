#[path = "helpers/project_coordination.rs"]
mod project_coordination;
#[path = "helpers/teams.rs"]
mod teams;

use project_coordination::Project;
use teams::Teams;

#[test]
fn one_project_manager_coordinates_two_epics_without_joining_either() {
    Project::given_a_synthetic_project()
        .when_a_manager_discovers_and_coordinates_two_epic_planners()
        .then_one_saved_manager_identity_receives_both_outcomes();
}

#[test]
fn teammates_discover_saved_guidance_and_complete_their_own_handoffs() {
    Teams::given_a_synthetic_home("handoffs")
        .given_a_team_with_four_members()
        .when_members_discover_and_exchange_work()
        .then_guidance_is_durable_and_each_request_has_one_owner();
}

#[test]
fn identically_named_members_keep_separate_mail() {
    Teams::given_a_synthetic_home("isolation")
        .given_identical_names_in_two_teams_and_legacy_mail()
        .when_names_and_conversations_are_crossed()
        .then_scopes_stay_distinct_and_message_ids_stay_global();
}

#[cfg(unix)]
#[test]
fn a_fresh_session_loads_guidance_without_replacing_delivery() {
    Teams::given_a_synthetic_home("restart")
        .given_a_team_with_four_members()
        .when_a_member_restarts_and_explicitly_rebinds()
        .then_identity_and_guidance_survive_without_implicit_rebinding();
}

#[test]
fn discovery_and_retries_preserve_saved_inputs() {
    Teams::given_a_synthetic_home("inputs")
        .when_help_discovery_and_conflicting_inputs_are_requested()
        .then_reads_are_pure_and_changed_inputs_never_overwrite_guidance();
}
