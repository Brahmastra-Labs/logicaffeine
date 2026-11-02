//! The Math Context panel lists the environment a run built. Every run starts from a fresh kernel,
//! and every fresh kernel from one cached standard library, so running the same program again lists
//! the same entries in the same places: the panel holds still under a reader working down it.
use super::*;
use crate::ui::examples::MATH_NAT;

fn names(entries: &[ContextEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn every_run_of_one_program_lists_one_context() {
    let first = run_math(MATH_NAT).context.expect("a Math document builds a context");
    assert!(first.inductives.len() > 3, "the natural-numbers example lists too few inductives to reorder");
    assert!(first.definitions.len() > 3, "the natural-numbers example lists too few definitions to reorder");
    for run in 2..=8 {
        let now = run_math(MATH_NAT).context.expect("a Math document builds a context");
        assert_eq!(names(&now.inductives), names(&first.inductives), "run {run} listed the inductive types in another order");
        assert_eq!(names(&now.definitions), names(&first.definitions), "run {run} listed the definitions in another order");
        assert!(now == first, "run {run} built a Context panel that differs from the first run's");
    }
}

#[test]
fn a_fresh_kernel_lists_its_prelude_one_way() {
    let first = MathContext::prelude();
    for run in 2..=8 {
        let now = MathContext::prelude();
        assert_eq!(names(&now.inductives), names(&first.inductives), "fresh kernel {run} listed the prelude's inductive types in another order");
        assert_eq!(names(&now.definitions), names(&first.definitions), "fresh kernel {run} listed the prelude's definitions in another order");
    }
}
