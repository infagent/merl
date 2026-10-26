#[path = "helpers/product.rs"]
mod product;

use product::Product;

#[test]
fn merl_exposes_one_messaging_product_with_complete_incremental_help() {
    Product::given_the_messaging_product()
        .when_the_public_help_tree_is_read()
        .then_messaging_and_lane_commands_are_discoverable_with_matching_help();
}

#[test]
fn retired_commands_cannot_open_or_change_old_runtime_data() {
    Product::given_an_opaque_old_runtime_file()
        .when_retired_product_commands_target_that_file()
        .then_commands_are_unavailable_and_the_old_bytes_are_unchanged();
}
