use super::Repl;
use crate as logicaffeine_kernel;
use crate::prelude::StandardLibrary;
use crate::{Context, Term, Universe};

fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn g(name: &str) -> Term {
    Term::Global(name.into())
}

#[test]
fn the_cached_prelude_is_indistinguishable_from_a_fresh_one() {
    fn census(ctx: &Context) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (n, t) in ctx.iter_inductives() {
            out.push(format!("inductive {n} : {t}"));
            for (c, ct) in ctx.get_constructors(n) {
                out.push(format!("  ctor {c} : {ct}"));
            }
        }
        for (n, t) in ctx.iter_declarations() {
            out.push(format!("declaration {n} : {t}"));
        }
        for (n, t, b) in ctx.iter_definitions() {
            out.push(format!("definition {n} : {t} := {b}"));
        }
        out.sort();
        out
    }

    let mut fresh = Context::new();
    StandardLibrary::register(&mut fresh);
    let cached = logicaffeine_kernel::interface::standard_context();

    let a = census(&fresh);
    let b = census(&cached);
    assert!(!a.is_empty(), "the prelude registers something");
    let only_fresh: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect();
    let only_cached: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect();
    assert!(
        only_fresh.is_empty() && only_cached.is_empty(),
        "a cached prelude that differs from a registered one is a second standard library. \
         Only in the fresh one: {only_fresh:#?}. Only in the cached one: {only_cached:#?}"
    );
    assert_eq!(
        fresh.refused_declarations(),
        cached.refused_declarations(),
        "the refusal ledger is part of what a context IS"
    );

    // MUTATION IS PRIVATE. The clone is a value, not a view.
    let mut mine = logicaffeine_kernel::interface::standard_context();
    mine.add_inductive("Interloper", sort(0));
    mine.add_constructor("Only", "Interloper", g("Interloper"));
    let next = logicaffeine_kernel::interface::standard_context();
    assert!(
        next.get_global("Interloper").is_none(),
        "writing to one clone must not reach the cache — copy-on-write is the whole reason this \
         is safe, and it is checked here rather than assumed"
    );
    assert!(
        mine.get_global("Interloper").is_some(),
        "…and the writer keeps its own write"
    );

    // …and the cache is a CACHE: the same contents, every time it is asked.
    assert_eq!(
        census(&logicaffeine_kernel::interface::standard_context()),
        b,
        "the second hand-out must equal the first"
    );
}

#[test]
fn two_repls_built_from_the_cached_prelude_are_independent() {
    let mut a = Repl::new();
    let mut b = Repl::new();

    // Both start from the same prelude, and it is actually THERE.
    for repl in [&a, &b] {
        assert!(
            repl.context().get_global("Nat").is_some(),
            "the prelude is loaded"
        );
        assert!(repl.context().get_global("Succ").is_some());
    }

    a.execute("Definition onlyInA : Nat := Zero.")
        .expect("a definition in A");
    a.execute("Inductive Hue := Red : Hue | Green : Hue.")
        .expect("an inductive in A");
    a.execute("Definition Space.member : Nat := Zero.")
        .expect("a namespace in A");
    a.execute("Protected Definition Space.hidden : Nat := Zero.")
        .expect("a protected mark in A");
    a.execute("Namespace Space.").expect("A enters a namespace");
    a.execute("Export Space (member).").expect("an export in A");
    a.execute("Section S.").expect("A opens a kernel scope");

    for absent in [
        "onlyInA",
        "Hue",
        "Red",
        "Hue_rec",
        "Space.member",
        "Space.hidden",
    ] {
        assert!(
            b.context().get_global(absent).is_none() && !b.context().is_inductive(absent),
            "`{absent}` was declared in A and must not be visible in B — a shared prelude must \
             share nothing a write can reach"
        );
    }
    assert_eq!(b.context().scope_depth(), 0, "A's open scope is not B's");
    assert!(
        b.execute("Definition onlyInA : Nat := (Succ Zero).")
            .is_ok(),
        "…and the key is genuinely free in B"
    );
    assert_eq!(
        b.execute("Eval onlyInA.").unwrap().trim(),
        "(Succ Zero)",
        "B's declaration is B's"
    );
    assert_eq!(
        a.execute("Eval _root_.onlyInA.").unwrap().trim(),
        "Zero",
        "…and A's is still A's, unchanged by anything B did"
    );

    // A THIRD REPL made after all that mutation still gets the pristine prelude, which is the
    // failure mode a cache actually has: the cached value drifting because somebody wrote to it.
    let c = Repl::new();
    for absent in ["onlyInA", "Hue", "Space.member"] {
        assert!(
            c.context().get_global(absent).is_none(),
            "a REPL built AFTER two others mutated theirs must still see the pristine prelude, \
             and `{absent}` leaked into it"
        );
    }
    assert!(
        c.context().get_global("Nat").is_some(),
        "…and the prelude is still all there"
    );
    assert_eq!(c.context().scope_depth(), 0);
}

#[test]
fn repl_and_standard_context_share_the_registered_prelude() {
    let shared = crate::interface::standard_context();
    let repl = Repl::new();
    // Arc-backed immutable entries must be the same allocation, not merely equal terms.
    assert!(
        std::ptr::eq(
            shared.get_global("Nat").expect("standard Nat"),
            repl.context().get_global("Nat").expect("REPL Nat"),
        ),
        "Repl::new must reuse the standard_context prelude cache"
    );
}

#[test]
fn standard_context_and_repl_mutations_are_independent() {
    let mut shared = crate::interface::standard_context();
    let mut repl = Repl::new();
    shared.add_inductive("SharedOnly", sort(0));
    shared.add_constructor("SharedCtor", "SharedOnly", g("SharedOnly"));
    repl.execute("Definition replOnly : Nat := Zero.")
        .expect("REPL definition");
    for name in ["SharedOnly", "SharedCtor"] {
        assert!(repl.context().get_global(name).is_none());
        assert!(crate::interface::standard_context()
            .get_global(name)
            .is_none());
        assert!(Repl::new().context().get_global(name).is_none());
        assert!(shared.get_global(name).is_some());
    }
    assert!(repl.context().get_global("replOnly").is_some());
    assert!(shared.get_global("replOnly").is_none());
    assert!(crate::interface::standard_context()
        .get_global("replOnly")
        .is_none());
    assert!(Repl::new().context().get_global("replOnly").is_none());
}
