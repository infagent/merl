#[path = "helpers/lane_integrity.rs"]
mod lane_integrity;

use lane_integrity::Integrity;

#[test]
fn an_ambiguous_mail_publication_retries_identical_bytes_and_preserves_later_activity() {
    Integrity::given_a_lane_with_a_mail_publication_boundary("outbox")
        .when_publication_succeeds_but_its_acknowledgment_is_lost_before_new_activity()
        .then_restart_retries_the_frozen_batch_and_new_activity_has_a_separate_id();
}

#[test]
fn a_pause_or_selector_change_during_source_io_fences_the_old_scan() {
    Integrity::given_a_lane_with_a_mail_publication_boundary("generation")
        .when_the_lane_is_paused_or_updated_while_its_source_reply_is_in_flight()
        .then_the_old_reply_cannot_commit_progress_or_delivery();
}

#[test]
fn changing_a_selector_during_io_cannot_shorten_a_server_rate_limit() {
    Integrity::given_a_lane_with_a_mail_publication_boundary("rate-generation")
        .when_configuration_changes_while_the_provider_returns_a_rate_limit()
        .then_only_content_progress_is_fenced_and_the_server_cooldown_is_preserved();
}
