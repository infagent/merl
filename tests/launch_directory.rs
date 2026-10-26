#[path = "helpers/launch_directory.rs"]
mod launch_directory;

use launch_directory::Dispatcher;

#[cfg(unix)]
#[test]
fn removing_a_development_checkout_does_not_break_host_delivery() {
    Dispatcher::given_mail_and_a_removable_launch_directory("host")
        .when_the_launch_directory_is_removed_during_host_delivery()
        .then_the_host_uses_the_stable_mailbox_directory_and_delivery_succeeds();
}

#[cfg(unix)]
#[test]
fn removing_a_development_checkout_does_not_break_github_polling() {
    Dispatcher::given_mail_and_a_removable_launch_directory("github")
        .given_a_reviewer_lane_and_synthetic_github()
        .when_the_launch_directory_is_removed_during_source_polling()
        .then_github_uses_the_stable_mailbox_directory_and_polling_succeeds();
}
