//! The Math Context panel lists the kernel's environment by name. The kernel keeps its inductive
//! types and definitions in hash maps, whose order differs from one process to the next, so a
//! panel listed in the maps' order is one order on the server that renders the page and another in
//! the browser, and another again on the next visit. By name, it is one order everywhere.
use super::*;
use crate::ui::examples::MATH_NAT;

fn names(entries: &[ContextEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut sorted: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    sorted.sort();
    sorted
}

#[test]
fn a_fresh_kernel_lists_its_prelude_by_name() {
    let prelude = MathContext::prelude();
    assert!(prelude.inductives.len() > 3 && prelude.definitions.len() > 3, "the prelude lists too few entries to order");
    assert_eq!(names(&prelude.inductives), sorted(&names(&prelude.inductives)), "the prelude's inductive types are not listed by name");
    assert_eq!(names(&prelude.definitions), sorted(&names(&prelude.definitions)), "the prelude's definitions are not listed by name");
}

#[test]
fn a_run_lists_what_it_built_by_name() {
    let context = run_math(MATH_NAT).context.expect("a Math document builds a context");
    assert_eq!(names(&context.inductives), sorted(&names(&context.inductives)), "the run's inductive types are not listed by name");
    assert_eq!(names(&context.definitions), sorted(&names(&context.definitions)), "the run's definitions are not listed by name");
}
