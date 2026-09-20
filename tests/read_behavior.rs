#[path = "helpers/read_behavior.rs"]
mod read_behavior;

use read_behavior::ReadScenario;

#[test]
fn an_issue_view_keeps_unprocessed_sources_visible_without_loading_their_text() {
    ReadScenario::given_an_imported_issue()
        .when_the_issue_view_is_requested()
        .then_the_view_separates_revision_from_coverage_and_keeps_source_text_cold();
}

#[test]
fn a_new_decision_reaches_the_agent_as_a_compact_delta_once() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_the_agent_polls()
        .then_one_entry_names_the_changed_decision_without_source_prose()
        .when_the_agent_acknowledges_it_twice()
        .then_the_cursor_stays_at_the_decision_revision();
}

#[test]
fn an_agent_expands_provenance_only_when_it_asks_for_it() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_the_decision_is_expanded()
        .then_its_command_and_history_are_visible();
}

#[test]
fn erased_source_bytes_have_an_explicit_unavailable_result() {
    ReadScenario::given_an_imported_issue()
        .when_a_source_is_expanded()
        .then_the_captured_body_is_returned()
        .when_its_bytes_are_erased_and_the_source_is_expanded_again()
        .then_the_source_is_unavailable();
}

#[test]
fn roles_get_the_same_accepted_state_in_a_useful_order() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_researcher_engineer_and_pm_views_are_requested()
        .then_each_view_has_its_own_order_and_the_same_revision();
}

#[test]
fn the_project_delta_contains_only_the_new_batch() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_the_project_delta_is_requested_since_zero()
        .then_the_delta_names_the_accepted_batch_and_refs();
}

#[test]
fn a_late_subscription_starts_after_existing_history() {
    ReadScenario::given_an_imported_issue()
        .when_an_agent_subscribes_and_polls()
        .then_the_agent_starts_after_existing_history();
}

#[test]
fn a_compiled_decision_expands_to_the_exact_sentence_that_supported_it() {
    read_behavior::AssertionScenario::given_a_decision_from_a_captured_comment()
        .when_the_decision_source_is_requested()
        .then_the_assertion_and_its_exact_source_span_are_shown();
}

#[test]
fn a_new_comment_changes_the_inbox_without_resending_the_thread() {
    read_behavior::AssertionScenario::given_a_decision_from_a_captured_comment()
        .when_the_subscriber_polls()
        .then_only_the_accepted_reference_is_delivered();
}

#[test]
fn a_large_batch_stays_bounded_and_advertises_the_omitted_refs() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_a_large_batch_is_accepted_and_the_delta_is_requested()
        .then_the_delta_is_bounded_and_marked_truncated();
}

#[test]
fn an_agent_can_read_every_change_in_a_large_batch_before_acknowledging() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_a_large_batch_is_accepted_and_every_page_is_read()
        .then_all_references_are_observable_and_the_batch_can_be_acknowledged();
}

#[test]
fn role_selection_finds_important_work_beyond_the_first_hundred_ids() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_many_low_priority_objects_and_one_task_are_viewed_as_pm()
        .then_the_task_is_in_the_bounded_pm_view();
}

#[test]
fn engineers_with_different_task_focus_get_different_first_objects() {
    read_behavior::InboxScenario::given_an_agent_subscribed_before_a_decision()
        .when_two_engineers_focus_on_different_tasks()
        .then_each_engineer_sees_their_task_first();
}

#[test]
fn an_object_keeps_its_source_evidence_after_a_later_command() {
    read_behavior::AssertionScenario::given_a_decision_from_a_captured_comment()
        .when_a_command_changes_the_decision_and_its_source_is_requested()
        .then_the_original_assertion_span_is_still_expandable();
}
