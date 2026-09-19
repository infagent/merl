#[path = "helpers/github_capture.rs"]
mod github_capture;

use github_capture::GithubCapture;

#[test]
fn edited_issue_versions_keep_their_causal_position_across_pages() {
    GithubCapture::two_pages_with_issue_edit()
        .when_captured()
        .then_has_four_versions_in_order()
        .then_creation_edit_is_one_version()
        .then_the_early_issue_body_is_unavailable()
        .then_edit_supersedes_the_same_issue()
        .then_preserves_stable_actor_ids()
        .then_has_verified_source_digest()
        .then_rejects_exact_replay_across_the_gap();
}

#[test]
fn capture_rejects_a_non_timestamp() {
    GithubCapture::two_pages_with_issue_edit()
        .with_capture_time("yesterday")
        .then_capture_fails_for_timestamp();
}

#[test]
fn same_time_across_sources_is_not_an_exact_causal_cutoff() {
    GithubCapture::two_pages_with_issue_edit()
        .with_issue_edit_tied_to_last_comment()
        .when_captured()
        .then_reports_ambiguous_cutoff();
}

#[test]
fn timezone_offsets_do_not_reorder_observations() {
    GithubCapture::two_pages_with_issue_edit()
        .with_offset_issue_creation_time()
        .when_captured()
        .then_has_four_versions_in_order();
}

#[test]
fn capture_rejects_incomplete_provider_snapshot() {
    GithubCapture::two_pages_with_issue_edit()
        .then_rejects_truncated_labels()
        .then_rejects_truncated_assignees();
}

#[test]
fn captured_edit_metadata_must_match_its_source_version() {
    GithubCapture::two_pages_with_issue_edit()
        .when_captured()
        .then_rejects_mismatched_edit_identity()
        .then_rejects_mismatched_edit_time()
        .then_rejects_invalid_deletion_time();
}
