#[path = "helpers/github_capture.rs"]
mod github_capture;

use github_capture::GithubCapture;

#[test]
fn edited_issue_versions_keep_their_causal_position_across_pages() {
    GithubCapture::two_pages_with_issue_edit()
        .when_captured()
        .then_has_four_versions_in_order()
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
