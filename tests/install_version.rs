#[path = "helpers/install_version.rs"]
mod install_version;

use install_version::Installation;

#[test]
fn an_installed_binary_reports_its_package_version_without_opening_a_mailbox() {
    Installation::given_the_built_binary()
        .when_its_version_is_requested()
        .then_the_package_version_is_printed_successfully();
}
