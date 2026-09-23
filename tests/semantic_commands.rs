#[path = "helpers/semantic_commands.rs"]
mod semantic_commands;

use semantic_commands::Commands;

#[test]
fn structured_commands_preserve_authority_content_and_retry_identity() {
    Commands::given_a_project_with_a_command_actor()
        .when_decisions_and_research_objects_are_submitted_and_retried()
        .then_only_authorized_commands_change_state_and_retries_keep_their_outcome();
}

#[test]
fn accepting_a_task_does_not_schedule_or_start_it() {
    Commands::given_a_project_with_a_command_actor()
        .when_a_task_is_requested_accepted_and_deferred()
        .then_planning_facets_and_deferral_details_survive_rebuild();
}

#[test]
fn supplemental_prose_stays_cold_and_expands_through_command_lineage() {
    Commands::given_a_project_with_a_command_actor()
        .when_a_decision_with_a_note_is_created_and_inspected()
        .then_the_note_is_optional_evidence_of_one_command();
}

#[test]
fn interrupted_commands_resume_without_restoring_erased_content() {
    semantic_commands::RecoveryCases::given_received_commands_without_outcomes()
        .when_the_commands_resume_after_restart_and_dependency_changes()
        .then_recovery_preserves_identity_and_checks_current_dependencies();
}

#[test]
fn supplemental_prose_keeps_a_separate_decision_for_review() {
    Commands::given_a_project_with_a_command_actor()
        .when_a_supplement_is_compiled_with_repeated_and_additional_semantics()
        .then_policy_covers_the_original_action_and_keeps_added_evidence()
        .when_the_additional_decision_is_reviewed()
        .then_review_accepts_the_separate_decision_with_its_own_evidence();
}

#[test]
fn command_previews_and_task_transitions_respect_the_public_contract() {
    Commands::given_a_project_with_a_command_actor()
        .when_invalid_planning_transitions_and_previews_are_submitted()
        .then_previews_are_read_only_and_invalid_transitions_leave_state_unchanged();
}

#[test]
fn a_later_resolution_preserves_the_original_supplement_and_its_erasure_boundary() {
    Commands::given_a_project_with_a_command_actor()
        .when_a_noted_question_is_resolved_and_its_note_is_erased()
        .then_history_keeps_both_commands_and_retry_does_not_restore_the_note();
}

#[test]
fn reviewed_evidence_from_a_project_note_stays_project_scoped() {
    Commands::given_a_project_with_a_command_actor()
        .when_a_project_note_yields_a_reviewed_finding()
        .then_the_finding_has_no_invented_issue_scope();
}
