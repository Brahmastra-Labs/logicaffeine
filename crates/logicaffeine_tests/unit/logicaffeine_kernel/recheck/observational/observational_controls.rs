//! The re-checker's observational rules, guard by guard: each seam the main kernel's controls
//! name stays stuck here too, beside a control that steps.
use super::*;
use crate::interface::TermParser;

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

/// One step of the closed term `text`, as this kernel's rule takes it.
fn step(ctx: &Context, text: &str) -> Option<Db> {
    let db = to_db(&term(text), &mut Vec::new()).expect("a closed term");
    reduce(ctx, &Vec::<Db>::new(), &db)
}

#[test]
fn each_guard_leaves_its_seam_stuck_beside_a_control_that_steps() {
    let ctx = crate::observational::tests::seam_context();
    for (seam, stuck, control) in crate::observational::tests::SEAMS {
        assert!(step(&ctx, stuck).is_none(), "{seam}: `{stuck}` is stuck");
        assert!(
            step(&ctx, control).is_some(),
            "{seam}: the control `{control}` steps"
        );
    }
}
