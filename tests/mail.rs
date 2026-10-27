#[path = "helpers/mail.rs"]
mod mail;

use mail::Mailbox;

#[cfg(unix)]
#[test]
fn a_short_assignment_arrives_inline_without_implicitly_handling_the_work() {
    Mailbox::given_three_named_agents("inline-short")
        .given_a_bound_recipient_and_unavailable_host()
        .when_a_short_assignment_is_delivered()
        .then_its_complete_content_is_available_without_an_inbox_read();
}

#[test]
fn compact_handling_is_explicit_atomic_and_keeps_completion_separate() {
    Mailbox::given_three_named_agents("compact-handling")
        .when_inline_messages_are_handled_together_and_retried()
        .then_only_the_selected_handling_facts_change_atomically();
}

#[test]
fn compact_completion_preserves_reply_retries_and_rejects_invalid_mixed_selections() {
    Mailbox::given_three_named_agents("compact-replies")
        .when_compact_completion_is_retried_with_valid_and_invalid_selections()
        .then_retries_keep_the_original_link_and_failed_batches_change_nothing();
}

#[cfg(unix)]
#[test]
fn inline_notices_bound_escaped_bytes_and_leave_omitted_content_fetchable() {
    Mailbox::given_three_named_agents("inline-overflow")
        .given_a_bound_recipient_and_unavailable_host()
        .when_large_unicode_and_escaped_messages_exceed_one_notice()
        .then_complete_bodies_and_exact_fetch_references_cover_the_bounded_selection();
}

#[cfg(unix)]
#[test]
fn receipt_stops_new_dispatches_but_cannot_retract_host_accepted_duplicates() {
    Mailbox::given_three_named_agents("accepted-duplicates")
        .given_a_bound_recipient_and_unavailable_host()
        .when_a_busy_host_accepts_duplicates_before_receipt()
        .then_accepted_events_can_be_obsolete_without_a_new_dispatch();
}

#[cfg(unix)]
#[test]
fn a_selected_recipient_receipted_during_another_host_call_is_skipped() {
    Mailbox::given_three_named_agents("obsolete-selection")
        .given_a_bound_recipient_and_unavailable_host()
        .when_a_later_recipient_receipts_mail_during_host_io()
        .then_that_obsolete_selection_creates_no_host_event();
}

#[cfg(unix)]
#[test]
fn handling_an_inline_selection_does_not_receipt_mail_that_arrives_later() {
    Mailbox::given_three_named_agents("selected-new-mail")
        .given_a_bound_recipient_and_unavailable_host()
        .when_more_mail_arrives_before_the_inline_selection_is_handled()
        .then_only_consumed_ids_are_receipted_and_later_content_is_delivered();
}

#[cfg(unix)]
#[test]
fn an_oversized_minimal_fetch_reference_remains_pending_without_a_host_call() {
    Mailbox::given_three_named_agents("reference-limit")
        .given_a_bound_recipient_and_unavailable_host()
        .when_even_a_fetch_reference_exceeds_the_notice_budget()
        .then_the_limit_is_reported_without_discarding_or_handling_mail();
}

#[test]
fn receipt_and_reply_do_not_complete_an_assignment() {
    Mailbox::given_three_named_agents("lifecycle")
        .when_an_assignment_is_read_acknowledged_and_replied_to()
        .then_only_the_recipient_can_explicitly_complete_that_assignment();
}

#[cfg(unix)]
#[test]
fn notifications_recover_offline_and_coalesce_without_losing_mail() {
    Mailbox::given_three_named_agents("delivery")
        .given_a_bound_recipient_and_unavailable_host()
        .when_two_messages_arrive_and_dispatch_restarts()
        .then_one_pointer_wakes_the_recipient_and_receipts_stop_retries();
}

#[cfg(unix)]
#[test]
fn stale_session_bindings_never_silently_retarget_pending_mail() {
    Mailbox::given_three_named_agents("binding")
        .given_a_bound_recipient_and_unavailable_host()
        .when_a_session_is_rebound_and_its_host_socket_is_replaced()
        .then_dispatch_waits_for_explicit_rebinding();
}

#[test]
fn concurrent_publication_is_atomic_and_retries_keep_original_content() {
    Mailbox::given_three_named_agents("concurrency")
        .when_concurrent_messages_and_conflicting_retries_are_sent()
        .then_all_messages_are_complete_unique_and_addressed_correctly();
}

#[test]
fn a_read_without_receipt_remains_recoverable_and_received_work_stays_outstanding() {
    Mailbox::given_three_named_agents("interrupted")
        .when_processing_stops_after_reading_and_before_receipt()
        .then_pending_and_outstanding_folders_preserve_the_right_work();
}

#[cfg(unix)]
#[test]
fn the_foreground_watcher_survives_temporary_dispatch_contention() {
    Mailbox::given_three_named_agents("watch-contention")
        .given_a_bound_recipient_and_unavailable_host()
        .when_another_dispatch_temporarily_owns_the_host_boundary()
        .then_the_watcher_waits_and_resumes_without_restarting();
}

#[cfg(unix)]
#[test]
fn a_successful_host_enqueue_allows_time_for_application_receipt() {
    Mailbox::given_three_named_agents("receipt-grace")
        .given_a_bound_recipient_and_unavailable_host()
        .when_the_host_accepts_a_notification_before_the_agent_receipts_it()
        .then_rapid_duplicates_are_coalesced_and_missing_receipts_remain_retryable();
}

#[cfg(unix)]
#[test]
fn new_mail_joins_a_pending_wake_but_a_receipted_batch_does_not_delay_new_work() {
    Mailbox::given_three_named_agents("coalesce-new-mail")
        .given_a_bound_recipient_and_unavailable_host()
        .when_new_messages_arrive_before_and_after_receipt()
        .then_pending_mail_coalesces_and_a_new_batch_wakes_immediately();
}

#[test]
fn mail_uses_the_last_output_format_for_help_and_errors() {
    Mailbox::given_three_named_agents("format")
        .when_human_and_json_output_flags_are_mixed()
        .then_the_last_flag_controls_success_and_error_output();
}

#[cfg(unix)]
#[test]
fn an_older_enqueue_cannot_postpone_a_new_batch_published_during_host_io() {
    Mailbox::given_three_named_agents("concurrent-batch")
        .given_a_bound_recipient_and_unavailable_host()
        .when_a_receipted_batch_is_replaced_while_its_enqueue_is_in_flight()
        .then_the_new_batch_remains_immediately_due();
}
