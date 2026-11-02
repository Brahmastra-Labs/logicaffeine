use super::*;
use crate::intern::Interner;
use std::collections::HashSet;

fn set(syms: &[Symbol]) -> HashSet<Symbol> {
    syms.iter().cloned().collect()
}

#[test]
fn alias_lattice_ops() {
    let mut it = Interner::new();
    let s = it.intern("s");
    let t = it.intern("t");
    assert_eq!(<AliasInfo as AbstractDomain>::top(), AliasInfo::Top);
    assert_eq!(<AliasInfo as AbstractDomain>::bottom(), AliasInfo::Bottom);
    // Unique is "may-alias nobody" = MayAlias(∅)
    assert_eq!(AliasInfo::MayAlias(HashSet::new()), AliasInfo::Unique);
    // join unions the may-alias sets
    assert_eq!(
        AliasInfo::MayAlias(set(&[s])).join(&AliasInfo::MayAlias(set(&[t]))),
        AliasInfo::MayAlias(set(&[s, t]))
    );
    assert_eq!(
        AliasInfo::Unique.join(&AliasInfo::MayAlias(set(&[s]))),
        AliasInfo::MayAlias(set(&[s]))
    );
    assert_eq!(AliasInfo::Top.join(&AliasInfo::Unique), AliasInfo::Top);
    // meet intersects
    assert_eq!(
        AliasInfo::MayAlias(set(&[s, t])).meet(&AliasInfo::MayAlias(set(&[s]))),
        AliasInfo::MayAlias(set(&[s]))
    );
    assert_eq!(
        AliasInfo::Top.meet(&AliasInfo::MayAlias(set(&[s]))),
        AliasInfo::MayAlias(set(&[s]))
    );
    // order: fewer possible aliases is more precise
    assert!(AliasInfo::Unique.leq(&AliasInfo::MayAlias(set(&[s]))));
    assert!(AliasInfo::MayAlias(set(&[s])).leq(&AliasInfo::MayAlias(set(&[s, t]))));
    assert!(AliasInfo::MayAlias(set(&[s])).leq(&AliasInfo::Top));
    assert!(AliasInfo::Bottom.leq(&AliasInfo::Unique));
    assert!(!AliasInfo::Top.leq(&AliasInfo::Unique));
    // widen = join (alias sets are bounded by the program's variables)
    assert_eq!(
        AliasInfo::MayAlias(set(&[s])).widen(&AliasInfo::MayAlias(set(&[t]))),
        AliasInfo::MayAlias(set(&[s])).join(&AliasInfo::MayAlias(set(&[t])))
    );
}

#[test]
fn aliasing_mutation_invalidates_shared_allocation() {
    let mut it = Interner::new();
    let a = it.intern("a");
    let items = it.intern("items");
    let c = it.intern("c"); // unrelated

    let mut g = AliasGraph::new();
    g.link(a, items); // `Let a be items.` — they share one Rc<RefCell>

    // `Push 1 to a.` mutates the shared allocation → invalidate `items` too.
    let inval = g.invalidated_by_mutation(a);
    assert!(inval.contains(&items));
    assert!(inval.contains(&a));
    assert!(!inval.contains(&c));

    // symmetric: mutating `items` invalidates `a`
    assert!(g.invalidated_by_mutation(items).contains(&a));

    // an unrelated variable aliases only itself
    assert_eq!(g.invalidated_by_mutation(c), set(&[c]));

    // rebinding `a` to a fresh allocation breaks the alias
    g.unlink(a);
    assert!(!g.invalidated_by_mutation(items).contains(&a));
}

#[test]
fn alias_transitivity() {
    let mut it = Interner::new();
    let a = it.intern("a");
    let b = it.intern("b");
    let c = it.intern("c");
    let mut g = AliasGraph::new();
    g.link(a, b); // Let b be a
    g.link(b, c); // Let c be b
                  // mutation through a reaches c via b
    let inval = g.invalidated_by_mutation(a);
    assert!(inval.contains(&b) && inval.contains(&c));
}
