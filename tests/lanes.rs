#[path = "helpers/lanes.rs"]
mod lanes;

use lanes::Lane;

#[cfg(unix)]
#[test]
fn a_reviewer_lane_follows_many_prs_and_discovers_work_after_an_earlier_pr_closes() {
    Lane::given_a_repository_with_existing_review_work("many-prs")
        .when_a_lane_observes_new_heads_comments_and_later_prs_across_restarts()
        .then_current_work_is_notified_without_replaying_old_discussions_or_unchanged_polls();
}

#[cfg(unix)]
#[test]
fn pausing_and_changing_a_lane_keeps_subscription_and_mail_handling_separate() {
    Lane::given_a_repository_with_existing_review_work("pause-selector")
        .when_polling_is_paused_resumed_and_its_required_labels_change()
        .then_generations_scope_changes_and_published_review_work_remain_explicit();
}

#[cfg(unix)]
#[test]
fn incomplete_pages_and_graphql_rate_limits_do_not_claim_a_current_scan() {
    Lane::given_a_repository_with_existing_review_work("retry-pages")
        .when_a_paged_scan_is_interrupted_by_a_long_server_rate_limit()
        .then_checkpoints_are_retained_and_retry_waits_for_the_server_deadline();
}

#[cfg(unix)]
#[test]
fn rejected_cursors_and_incomplete_scope_evidence_remain_visible() {
    Lane::given_a_repository_with_existing_review_work("cursor-scope")
        .when_a_cursor_is_rejected_and_later_scope_metadata_is_truncated()
        .then_resynchronization_records_a_gap_and_truncation_cannot_retire_the_pr();
}

#[cfg(unix)]
#[test]
fn empty_completed_activity_pages_keep_the_cursor_and_do_not_replay_history() {
    Lane::given_a_repository_with_existing_review_work("empty-page")
        .when_an_unchanged_lane_is_polled_after_each_due_deadline()
        .then_empty_pages_preserve_progress_and_publish_no_new_review_requests();
}

#[cfg(unix)]
#[test]
fn relative_rate_limit_delays_start_when_the_http_response_arrives() {
    Lane::given_a_repository_with_existing_review_work("response-time")
        .when_a_rate_response_arrives_after_time_passes_during_io()
        .then_network_time_does_not_shorten_the_server_retry_delay();
}

#[cfg(unix)]
#[test]
fn a_transient_github_failure_retains_the_previous_success_and_recovers_without_noise() {
    Lane::given_a_repository_with_existing_review_work("outage")
        .when_github_fails_after_a_successful_scan_and_then_recovers()
        .then_the_outage_is_explicit_and_recovery_keeps_existing_progress_and_mail();
}
