#[path = "helpers/github_capture.rs"]
mod github_capture;

use github_capture::GithubCapture;

#[test]
fn edited_issue_versions_keep_their_causal_position_across_pages() {
    GithubCapture::given_two_pages_with_issue_edit()
        .when_captured()
        .then_has_four_versions_in_order()
        .then_creation_edit_is_one_version()
        .then_the_early_issue_body_is_unavailable()
        .then_edit_supersedes_the_same_issue()
        .then_preserves_stable_actor_ids()
        .then_has_verified_source_digest()
        .when_exact_replay_is_requested()
        .then_rejects_exact_replay_across_the_gap();
}

#[test]
fn capture_rejects_a_non_timestamp() {
    GithubCapture::given_two_pages_with_issue_edit()
        .given_capture_time("yesterday")
        .when_capture_is_attempted()
        .then_capture_fails_for_timestamp();
}

#[test]
fn same_time_across_sources_is_not_an_exact_causal_cutoff() {
    GithubCapture::given_two_pages_with_issue_edit()
        .given_issue_edit_tied_to_last_comment()
        .when_captured()
        .when_causal_cutoffs_are_checked()
        .then_reports_ambiguous_cutoff();
}

#[test]
fn timezone_offsets_do_not_reorder_observations() {
    GithubCapture::given_two_pages_with_issue_edit()
        .given_offset_issue_creation_time()
        .when_captured()
        .then_has_four_versions_in_order();
}

#[test]
fn capture_rejects_incomplete_provider_snapshot() {
    GithubCapture::given_two_pages_with_issue_edit()
        .when_incomplete_provider_snapshots_are_captured()
        .then_truncated_labels_are_rejected()
        .then_truncated_assignees_are_rejected();
}

#[test]
fn captured_edit_metadata_must_match_its_source_version() {
    GithubCapture::given_two_pages_with_issue_edit()
        .when_captured()
        .when_malformed_edit_metadata_is_validated()
        .then_mismatched_edit_identity_is_rejected()
        .then_mismatched_edit_time_is_rejected()
        .then_invalid_deletion_time_is_rejected();
}

#[test]
fn old_natural_captures_stay_valid_while_new_captures_require_stable_provider_facts() {
    GithubCapture::given_two_pages_with_issue_edit()
        .when_captured()
        .then_uses_the_new_schema()
        .when_read_as_a_legacy_capture()
        .then_remains_valid_without_new_provider_facts()
        .then_does_not_invent_missing_provider_facts()
        .when_legacy_capture_claims_the_new_schema()
        .then_cannot_claim_the_new_schema_without_them();
}
