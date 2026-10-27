#[path = "helpers/announcements.rs"]
mod announcements;

use announcements::Announcements;

#[test]
fn informational_notices_freeze_recipients_and_leave_work_individually_owned() {
    Announcements::given_two_scopes_and_a_roleless_member()
        .when_notices_are_filtered_retried_and_receipted()
        .then_retries_keep_the_snapshot_and_receipts_only_handle_one_note();
}

#[test]
fn a_later_child_collision_rolls_back_the_whole_notice_without_adopting_mail() {
    Announcements::given_a_roster("rollback", 2)
        .when_a_later_candidate_collides_and_the_notice_is_retried()
        .then_rollback_preserves_foreign_mail_and_commit_preserves_the_snapshot();
}

#[test]
fn concurrent_identical_announcements_accept_one_snapshot() {
    Announcements::given_a_roster("concurrent", 2)
        .when_identical_callers_publish_concurrently()
        .then_both_observe_one_committed_set_of_notes();
}

#[test]
fn selection_and_input_limits_reject_without_accepting_partial_notices() {
    Announcements::given_a_roster("limits", 100)
        .when_maximum_notices_and_invalid_selections_are_submitted()
        .then_only_bounded_complete_snapshots_exist_and_detail_reads_are_pure();
}
