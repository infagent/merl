#![cfg(unix)]

#[path = "helpers/qualified_lanes.rs"]
mod qualified_lanes;
use qualified_lanes::Subscriptions;

#[test]
fn named_subscriptions_target_qualified_members_without_adopting_legacy_names() {
    Subscriptions::given_identically_named_members_and_a_legacy_identity("recipients")
        .when_new_subscriptions_select_members_and_legacy_names()
        .then_each_subscription_reaches_only_its_explicit_recipient();
}

#[test]
fn member_names_are_optional_display_metadata_not_a_resume_or_binding_dependency() {
    Subscriptions::given_identically_named_members_and_a_legacy_identity("restart")
        .when_a_member_rebinds_and_optional_name_metadata_is_unavailable()
        .then_saved_delivery_and_json_survive_name_lookup_failures();
}

#[test]
fn team_joins_and_rejected_retargeting_cannot_change_a_frozen_legacy_batch() {
    Subscriptions::given_a_legacy_subscription_with_frozen_activity()
        .when_members_join_and_a_different_recipient_is_requested()
        .then_frozen_mail_and_individual_handling_keep_their_original_owner();
}
