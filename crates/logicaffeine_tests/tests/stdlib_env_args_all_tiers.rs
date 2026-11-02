//! `args()` (env.lg) is the program's argument vector with the PROGRAM NAME FIRST — `argv` as
//! C, Python and Go define it — on every tier. A run that was given no argument vector at all still
//! has a program name: `logos`. So `item 2 of args()` is always the first real argument and
//! `length of args()` is never 0.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};
use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome};

#[test]
fn a_run_with_no_arguments_still_has_a_program_name() {
    assert_all_tiers(
        "## Main\nShow length of args().\nShow item 1 of args().\n",
        "1\nlogos",
    );
}

#[test]
fn the_program_name_is_first_on_the_rust_tier() {
    assert_compiled_equals_interpreted("## Main\nShow length of args().\n");
}

#[test]
fn the_program_name_is_first_on_the_c_tier() {
    assert_c_output("## Main\nShow length of args().\n", "1");
}

#[test]
fn real_arguments_follow_the_program_name() {
    let argv: Vec<String> = ["prog", "alpha", "beta"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = tw_outcome_with_args(
        "## Main\nShow length of args().\nShow item 2 of args().\nShow item 3 of args().\n",
        &argv,
    );
    assert_eq!(r.error, None);
    assert_eq!(r.output.trim(), "3\nalpha\nbeta");
}

#[test]
fn the_vm_agrees_with_the_tree_walker_on_an_empty_argument_vector() {
    let vm = vm_outcome("## Main\nShow length of args().\n");
    assert_eq!(vm.error, None);
    assert_eq!(vm.output.trim(), "1");
}
