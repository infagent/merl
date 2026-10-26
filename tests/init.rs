#[path = "helpers/init.rs"]
mod init;

use init::SetupCase;

#[test]
fn setup_installs_matching_skills_and_defaults_while_uninstall_preserves_mail() {
    SetupCase::given_supported_local_hosts()
        .when_setup_is_applied_used_and_uninstalled()
        .then_skills_match_the_binary_defaults_work_and_mail_survives();
}

#[test]
fn preview_and_show_do_not_create_even_setup_directories_or_locks() {
    SetupCase::given_an_unconfigured_user()
        .when_preview_and_show_are_requested_as_json()
        .then_the_user_files_are_unchanged();
}

#[test]
fn an_explicit_mailbox_bypasses_broken_saved_settings() {
    SetupCase::given_broken_user_settings()
        .when_explicit_and_default_mailbox_commands_and_help_are_used()
        .then_only_the_explicit_mailbox_opens_and_help_stays_pure();
}

#[test]
fn borrowed_skills_and_local_edits_survive_rerun_and_uninstall() {
    SetupCase::given_a_borrowed_matching_skill_and_existing_mail()
        .when_setup_is_retried_after_a_local_edit_and_removed()
        .then_borrowed_and_modified_content_and_all_mail_are_preserved();
}

#[test]
fn invalid_managed_destinations_are_rejected_before_any_setup_mutation() {
    SetupCase::given_overlapping_and_symlinked_destinations()
        .when_each_invalid_setup_plan_is_applied()
        .then_every_conflict_preserves_the_original_files();
}

#[test]
fn claude_skill_setup_does_not_require_or_fabricate_a_codex_dispatcher() {
    SetupCase::given_only_a_local_claude_host()
        .when_claude_setup_requests_foreground_instructions()
        .then_claude_is_configured_and_missing_codex_is_reported_truthfully();
}

#[test]
fn interrupted_setup_recovers_ownership_and_upgrades_only_unchanged_owned_skills() {
    SetupCase::given_a_borrowed_matching_skill_and_existing_mail()
        .when_publication_is_interrupted_then_recovered_and_upgraded()
        .then_recovery_is_visible_and_uninstall_preserves_borrowed_skill_and_mail();
}

#[test]
fn interactive_setup_reads_host_choices_from_the_terminal_and_json_never_prompts() {
    SetupCase::given_supported_local_hosts()
        .when_json_omits_choices_then_terminal_setup_selects_claude()
        .then_only_the_terminal_selected_host_is_installed();
}

#[test]
fn custom_settings_can_share_a_parent_with_mail_and_uninstall_keeps_an_empty_home() {
    SetupCase::given_an_unconfigured_user()
        .when_custom_settings_and_an_empty_mail_home_are_created_then_removed()
        .then_custom_setup_is_removed_but_the_empty_mail_home_is_retained();
}

#[test]
fn a_host_can_move_after_scoped_uninstall_without_adopting_its_old_borrowed_skill() {
    SetupCase::given_a_borrowed_matching_skill_and_existing_mail()
        .when_a_host_is_uninstalled_then_installed_at_a_new_destination()
        .then_only_the_new_owned_skill_is_removed_and_the_borrowed_skill_remains();
}
