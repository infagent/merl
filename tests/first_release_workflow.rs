#[cfg(unix)]
#[path = "helpers/first_release_workflow.rs"]
mod first_release_workflow;

#[cfg(unix)]
#[test]
fn an_issue_reaches_reviewed_state_and_delivers_only_new_accepted_work() {
    first_release_workflow::Workflow::given_an_issue_and_no_project_database()
        .when_the_project_and_authorities_are_created()
        .when_the_issue_is_captured_and_its_assertion_is_applied()
        .then_the_interpretation_waits_for_review()
        .when_the_candidate_is_accepted()
        .then_the_decision_has_exact_source_provenance()
        .when_the_reader_catches_up()
        .when_a_later_comment_is_captured_and_accepted()
        .then_the_reader_receives_one_new_revision()
        .when_the_comment_is_edited_and_its_support_is_revalidated()
        .then_review_restores_support_without_replacing_the_decision()
        .when_history_is_replayed_and_projections_are_rebuilt()
        .then_accepted_history_and_views_are_unchanged()
        .when_a_structured_decision_includes_a_note()
        .then_the_command_is_accepted_without_a_compiler()
        .when_the_note_is_compiled_and_its_added_finding_is_reviewed()
        .then_the_note_adds_evidence_without_recreating_the_decision()
        .when_the_note_is_purged_and_the_reader_catches_up()
        .then_protected_text_is_unavailable_with_an_audit_receipt()
        .then_the_issue_and_reader_are_caught_up();
}
