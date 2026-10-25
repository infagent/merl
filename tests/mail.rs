#[path = "helpers/mail.rs"]
mod mail;

use mail::Mailbox;

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
