#[path = "helpers/setup_summary.rs"]
mod summary;

use summary::SummaryCase;

#[test]
fn init_shows_the_same_readable_summary_as_status_and_cancel_changes_nothing() {
    SummaryCase::given_a_fresh_user_with_local_hosts()
        .when_terminal_setup_is_cancelled_then_applied_and_status_is_read()
        .then_proposed_and_current_summaries_are_clear_and_cancellation_is_pure();
}

#[test]
fn status_reports_setup_facts_and_full_json_without_changing_files() {
    SummaryCase::given_absent_configured_modified_partial_and_invalid_setups()
        .when_each_setup_is_inspected_in_summary_and_full_json_modes()
        .then_status_is_read_only_truthful_and_all_json_aliases_agree();
}

#[test]
fn setup_previews_distinguish_unchanged_borrowed_and_owned_removals() {
    SummaryCase::given_an_owned_and_a_borrowed_skill()
        .when_setup_and_uninstall_are_previewed_then_uninstalled()
        .then_the_shared_summary_describes_changes_and_retained_mail();
}
