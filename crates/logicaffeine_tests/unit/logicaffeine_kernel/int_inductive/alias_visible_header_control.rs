use super::*;
use crate::Universe;
#[test]
fn a_constructor_collision_cannot_change_the_visible_alias_header_under_a_cached_body() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    assert!(accelerates_integer_alias(&ctx, "add"));
    ctx.add_inductive("Other", Term::Sort(Universe::Type(0)));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "add", |c| {
        c.add_constructor("add", "Other", g("Other"));
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("add", "Other", g("Other"));
    // A constructor never shares its name with a live definition: the definition is displaced,
    // so typing and reduction agree that `add` is the constructor.
    assert_eq!(ctx.get_global("add"), Some(&g("Other")));
    assert_eq!(ctx.get_definition_body("add"), None);
    assert!(
        !accelerates_integer_alias(&ctx, "add"),
        "the visible declaration header, not only the definition table, controls reduction"
    );
}
