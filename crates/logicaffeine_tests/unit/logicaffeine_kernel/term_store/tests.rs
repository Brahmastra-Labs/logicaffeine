use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn let_in(n: &str, ty: Term, v: Term, b: Term) -> Term {
    Term::Let {
        name: n.to_string(),
        ty: Box::new(ty),
        value: Box::new(v),
        body: Box::new(b),
    }
}
fn fix(n: &str, b: Term) -> Term {
    Term::Fix {
        name: n.to_string(),
        body: Box::new(b),
    }
}
fn nat() -> Term {
    g("Nat")
}

fn exact_free_name_cases() -> Vec<(Term, bool)> {
    vec![
        (g("Nat"), true),
        (var("x"), false),
        (lam("x", nat(), var("x")), true),
        (lam("x", var("x"), var("x")), false),
        (pi("x", nat(), var("x")), true),
        (pi("x", var("x"), nat()), false),
        (lam("x", nat(), lam("x", nat(), var("x"))), true),
        (lam("x", nat(), lam("x", var("outside"), var("x"))), false),
        (let_in("x", nat(), g("Zero"), var("x")), true),
        (let_in("x", var("outside"), g("Zero"), var("x")), false),
        (let_in("x", nat(), var("x"), var("x")), false),
        (fix("f", lam("x", nat(), app(var("f"), var("x")))), true),
        (
            fix("f", lam("x", var("outside"), app(var("f"), var("x")))),
            false,
        ),
        (
            Term::MutualFix {
                defs: vec![("f".into(), var("g")), ("g".into(), var("f"))],
                index: 0,
            },
            true,
        ),
        (
            Term::MutualFix {
                defs: vec![("f".into(), var("g")), ("g".into(), var("outside"))],
                index: 0,
            },
            false,
        ),
        (
            Term::Match {
                discriminant: Box::new(g("Zero")),
                motive: Box::new(var("outside")),
                cases: vec![],
            },
            false,
        ),
    ]
}

#[test]
fn exact_free_name_emptiness_includes_annotations_values_and_mutual_scopes() {
    let mut store = TermStore::new();
    for (term, expected) in exact_free_name_cases() {
        let id = store.intern(&term);
        assert_eq!(store.free_names_are_empty(id), expected, "{term:?}");
        assert_eq!(store.free_vars(id).is_empty(), expected, "{term:?}");
    }
}

/// Every `Term` shape, each one with a binder that shadows `x` and one that does not, so a
/// scope rule that is wrong in either direction shows up somewhere below.
fn corpus() -> Vec<Term> {
    vec![
        var("x"),
        var("y"),
        g("Nat"),
        Term::Sort(Universe::Type(0)),
        Term::Hole,
        Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(3))),
        Term::Const {
            name: "c".into(),
            levels: vec![Universe::Type(1)],
        },
        app(var("x"), var("x")),
        app(var("x"), var("y")),
        lam("x", var("x"), var("x")),
        lam("y", var("x"), var("x")),
        pi("x", var("x"), var("x")),
        pi("y", var("x"), app(var("x"), var("x"))),
        let_in("x", var("x"), var("x"), var("x")),
        let_in("y", var("x"), var("x"), app(var("x"), var("x"))),
        fix("x", app(var("x"), var("x"))),
        fix("y", app(var("x"), var("x"))),
        Term::MutualFix {
            defs: vec![("x".into(), var("x")), ("z".into(), var("x"))],
            index: 0,
        },
        Term::MutualFix {
            defs: vec![
                ("p".into(), var("x")),
                ("q".into(), app(var("x"), var("x"))),
            ],
            index: 1,
        },
        Term::Match {
            discriminant: Box::new(var("x")),
            motive: Box::new(lam("x", nat(), var("x"))),
            cases: vec![var("x"), app(var("x"), var("x"))],
        },
        // The corpus's own shapes.
        (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc)),
        let_in(
            "b",
            nat(),
            (0..6).fold(var("u"), |acc, _| app(g("Succ"), acc)),
            app(
                app(app(app(g("quad"), var("b")), var("b")), var("b")),
                var("b"),
            ),
        ),
    ]
}

/// The DOOR and this file measure the same tree. `depth_exceeds(t, n)` is `term_depth(t) > n`
/// at EVERY limit, not merely at the one the door happens to use — a door whose measure is
/// off by one admits a term the walker cannot descend.
#[test]
fn term_depth_is_exactly_the_doors_own_measure_at_every_limit() {
    for t in corpus() {
        let d = term_depth(&t);
        for limit in 0..=(d + 3) {
            assert_eq!(
                crate::type_checker::depth_exceeds(&t, limit),
                d > limit,
                "depth_exceeds({t}, {limit}) must be `term_depth = {d} > {limit}`"
            );
        }
    }
}

/// The store counts the same tree in RUNGS, one more than the door counts it in EDGES. Both
/// numbers are load-bearing — the door sizes a stack from one, the store annotates a node
/// with the other — so the relation between them is pinned rather than remembered.
#[test]
fn the_stores_height_is_the_doors_depth_plus_one() {
    let mut store = TermStore::new();
    for t in corpus() {
        let id = store.intern(&t);
        assert_eq!(
            store.depth(id) as usize,
            term_depth(&t) + 1,
            "a leaf is 1 rung and 0 edges: {t}"
        );
    }
}

/// A spine far past the native stack's ceiling: both measures still answer, because both
/// walk with an explicit stack.
///
/// EVERY measurement is taken BEFORE the first assertion, and the term is forgotten in
/// between. `Term`'s derived `Drop` is native recursion, so a failing assertion here would
/// unwind, drop the term and ABORT — reporting a stack overflow instead of the number that
/// was wrong. That is not hypothetical: it is how this test first failed.
#[test]
fn both_depth_measures_survive_a_term_no_native_walk_could() {
    const RUNGS: usize = 200_000;
    let deep = (0..RUNGS).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    let measured = term_depth(&deep);
    let past = crate::type_checker::depth_exceeds(&deep, RUNGS - 1);
    let at = crate::type_checker::depth_exceeds(&deep, RUNGS);
    // Interning is iterative too, so the store agrees about a term nothing may recurse over.
    let mut store = TermStore::new();
    let id = store.intern(&deep);
    let height = store.depth(id) as usize;
    std::mem::forget(deep);

    assert_eq!(
        measured, RUNGS,
        "a `Succ` spine of {RUNGS} rungs is {RUNGS} edges deep"
    );
    assert!(past, "one rung below is exceeded");
    assert!(!at, "its own depth is not");
    assert_eq!(
        height,
        RUNGS + 1,
        "the store counts the same tree in rungs, not edges"
    );
}

/// The `Term`-side occurrence gate and the STORE's hash-consed one are two independent
/// implementations of one specification. They must never disagree.
#[test]
fn the_term_side_occurrence_gate_agrees_with_the_stores() {
    let mut store = TermStore::new();
    for t in corpus() {
        for name in ["x", "y", "z", "u", "b", "absent"] {
            let id = store.intern(&t);
            let sym = store.symbol(name);
            assert_eq!(
                occurrence_count_in_term(&t, name),
                store.occurrence_count(id, sym),
                "occurrences of `{name}` in {t}"
            );
            assert_eq!(
                occurrences_in_term(&t, name),
                store.occurrences(id, sym),
                "the classified gate for `{name}` in {t}"
            );
            assert_eq!(
                occurs_free_in_term(&t, name),
                store.occurs_free_sym(id, sym),
                "free occurrence of `{name}` in {t}"
            );
        }
    }
}

/// The NEGATIVE the gate exists for: a binder HIDES the name, and it hides it in its body
/// only. Getting this wrong in the permissive direction makes ζ duplicate; getting it wrong
/// in the restrictive direction makes ζ capture.
#[test]
fn a_binder_hides_its_name_in_its_body_and_nowhere_else() {
    // The annotation is OUTSIDE the binder…
    assert_eq!(
        occurrences_in_term(&lam("x", var("x"), var("x")), "x"),
        Occurrences::Once
    );
    assert_eq!(
        occurrences_in_term(&pi("x", var("x"), var("x")), "x"),
        Occurrences::Once
    );
    // …and so are a `Let`'s type and its bound VALUE; only the body is inside.
    assert_eq!(
        occurrences_in_term(&let_in("x", var("x"), var("x"), var("x")), "x"),
        Occurrences::Many,
        "the annotation and the value are outside the binding, the body is inside"
    );
    // A `Fix` binds over its body, which is its only child.
    assert_eq!(
        occurrences_in_term(&fix("x", var("x")), "x"),
        Occurrences::Zero
    );
    // A mutual block binds every name over every body.
    assert_eq!(
        occurrences_in_term(
            &Term::MutualFix {
                defs: vec![("x".into(), var("x")), ("z".into(), var("x"))],
                index: 0
            },
            "x"
        ),
        Occurrences::Zero
    );
    // And a name a different binder captured is NOT this one's.
    assert_eq!(
        occurrences_in_term(&lam("x", nat(), var("x")), "y"),
        Occurrences::Zero
    );
}

/// The classifier is the CHEAP form of the count, so it has to agree with it at the two
/// boundaries it exists to distinguish, one occurrence either side.
#[test]
fn the_occurrence_classifier_matches_the_count_one_either_side_of_each_boundary() {
    for n in 0u32..5 {
        let body = (0..n).fold(g("end"), |acc, _| app(acc, var("x")));
        assert_eq!(occurrence_count_in_term(&body, "x"), n);
        assert_eq!(
            occurrences_in_term(&body, "x"),
            match n {
                0 => Occurrences::Zero,
                1 => Occurrences::Once,
                _ => Occurrences::Many,
            }
        );
        assert_eq!(occurs_free_in_term(&body, "x"), n > 0);
    }
}

/// Counting is iterative, so it answers about a term no recursive counter could walk —
/// which is the whole reason the gate lives here and not beside `body_stats`.
#[test]
fn occurrence_counting_survives_a_term_no_native_walk_could() {
    let deep = (0..200_000).fold(var("x"), |acc, _| app(g("Succ"), acc));
    let count = occurrence_count_in_term(&deep, "x");
    let classified = occurrences_in_term(&deep, "x");
    let absent = occurs_free_in_term(&deep, "absent");
    std::mem::forget(deep);

    assert_eq!(count, 1);
    assert_eq!(classified, Occurrences::Once);
    assert!(!absent);
}

// -- the kernel's own stack -----------------------------------------------------------

/// The sizing is monotone, never below the floor, and never wraps.
#[test]
fn a_sized_stack_is_monotone_floored_and_saturating() {
    assert_eq!(stack_bytes_for_depth(0), BASE_STACK_BYTES);
    assert!(stack_bytes_for_depth(1) > stack_bytes_for_depth(0));
    assert!(stack_bytes_for_depth(1600) > stack_bytes_for_depth(1599));
    assert_eq!(
        stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH),
        BASE_STACK_BYTES + crate::type_checker::MAX_TERM_DEPTH * BYTES_PER_RUNG
    );
    assert_eq!(stack_bytes_for_depth(usize::MAX), usize::MAX);
}

/// The deepest term the DOOR admits must be one the sizing can actually pay for. If
/// `MAX_TERM_DEPTH` ever rises past what a thread stack can hold, this is where it is
/// noticed — not in a corpse on the test runner.
#[test]
fn the_door_never_admits_a_term_the_sizing_cannot_pay_for() {
    const ONE_GIB: usize = 1 << 30;
    assert!(
        stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH) <= ONE_GIB,
        "a stack reservation for the deepest admitted term must stay inside one GiB; \
             MAX_TERM_DEPTH is {} and that asks for {} bytes",
        crate::type_checker::MAX_TERM_DEPTH,
        stack_bytes_for_depth(crate::type_checker::MAX_TERM_DEPTH)
    );
}

/// A shallow term runs RIGHT HERE — no thread, no handoff — and the nested call sees the
/// flag and is told to carry on. That is what keeps the switch off the hot path.
#[test]
fn a_shallow_term_runs_inline_and_a_nested_call_is_waved_through() {
    let shallow = app(g("f"), g("a"));
    let here = std::thread::current().id();
    let inner = deepen_stack(&shallow, || {
        // Nested: the flag is set, so the answer is "carry on".
        assert!(
            deepen_stack(&shallow, || unreachable!("a nested call must never run f")).is_none()
        );
        std::thread::current().id()
    });
    assert_eq!(inner, Some(here), "a shallow term must not cost a thread");
    // The flag is cleared on the way out, so the next outermost call measures again.
    assert!(deepen_stack(&shallow, || 1u8).is_some());
}

/// A term past the inline band moves to a stack of its own, and a nested call on THAT
/// thread is waved through in exactly the same way.
#[test]
fn a_deep_term_moves_to_a_stack_of_its_own() {
    let deep = (0..INLINE_DEPTH + 1).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    let here = std::thread::current().id();
    let (there, bytes) = deepen_stack(&deep, || {
        assert!(deepen_stack(&deep, || unreachable!()).is_none());
        (std::thread::current().id(), kernel_stack_bytes())
    })
    .expect("an outermost call always runs");
    assert_ne!(
        there, here,
        "a term past the inline band gets its own stack"
    );
    assert_eq!(bytes, stack_bytes_for_depth(INLINE_DEPTH + 1));
    // And the calling thread is back to reporting the floor.
    assert_eq!(kernel_stack_bytes(), BASE_STACK_BYTES);
}

/// One rung either side of the inline boundary, so the band is a decision and not an
/// accident.
#[test]
fn the_inline_boundary_decides_one_rung_either_side() {
    let here = std::thread::current().id();
    let at = (0..INLINE_DEPTH).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    let past = (0..INLINE_DEPTH + 1).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    assert_eq!(term_depth(&at), INLINE_DEPTH);
    assert_eq!(term_depth(&past), INLINE_DEPTH + 1);
    assert_eq!(
        deepen_stack(&at, || std::thread::current().id()),
        Some(here)
    );
    assert_ne!(
        deepen_stack(&past, || std::thread::current().id()),
        Some(here)
    );
}

/// A PANIC crosses the switch with its payload intact. A kernel that swallowed a panic —
/// or turned it into a different one — would make every `#[should_panic]` test in the suite
/// lie about which assertion fired.
#[test]
fn a_panic_crosses_the_switch_with_its_message_intact() {
    let deep = (0..INLINE_DEPTH + 1).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(|| {
        deepen_stack(&deep, || panic!("the exact words"));
    });
    std::panic::set_hook(previous);
    let payload = caught.expect_err("the panic must reach the caller");
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("the exact words"),
        "the payload must arrive unchanged"
    );
    // And the flag was restored, so the kernel is usable again afterwards.
    assert_eq!(kernel_stack_bytes(), BASE_STACK_BYTES);
}

// -- the occurrence-gated ζ step ------------------------------------------------------

/// Drive [`zeta_or_share`] to a fixpoint exactly as `normalize_checked_at` drives
/// `reduce_step`: apply, compare, stop when nothing changed.
fn zeta_fixpoint(start: &Term, step: &dyn Fn(&Term) -> Term) -> Term {
    let mut current = start.clone();
    for _ in 0..10_000 {
        let next = match &current {
            Term::Let {
                name,
                ty,
                value,
                body,
            } => zeta_or_share(name, ty, value, body, step),
            other => step(other),
        };
        if next == current {
            return current;
        }
        current = next;
    }
    panic!("the ζ fixpoint did not settle");
}

/// A toy reducer for the tests below: `drop e` erases to `gone`, and nothing else moves. It
/// stands in for the real reducer's one relevant property — reduction can ERASE occurrences.
fn toy_step(t: &Term) -> Term {
    if let Term::App(f, _) = t {
        if matches!(f.as_ref(), Term::Global(n) if n == "drop") {
            return g("gone");
        }
    }
    let mut kids: Vec<Term> = Vec::new();
    term_for_each_child(t, &mut |c| kids.push(toy_step(c)));
    if kids.is_empty() {
        return t.clone();
    }
    rebuild_term(t, &kids)
}

/// Rebuild `t` with new children, in `term_for_each_child` order. Wildcard-free.
fn rebuild_term(t: &Term, kids: &[Term]) -> Term {
    match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => t.clone(),
        Term::Pi { param, .. } => Term::Pi {
            param: param.clone(),
            param_type: Box::new(kids[0].clone()),
            body_type: Box::new(kids[1].clone()),
        },
        Term::Lambda { param, .. } => Term::Lambda {
            param: param.clone(),
            param_type: Box::new(kids[0].clone()),
            body: Box::new(kids[1].clone()),
        },
        Term::App(_, _) => Term::App(Box::new(kids[0].clone()), Box::new(kids[1].clone())),
        Term::Match { .. } => Term::Match {
            discriminant: Box::new(kids[0].clone()),
            motive: Box::new(kids[1].clone()),
            cases: kids[2..].to_vec(),
        },
        Term::Fix { name, .. } => Term::Fix {
            name: name.clone(),
            body: Box::new(kids[0].clone()),
        },
        Term::MutualFix { defs, index } => Term::MutualFix {
            defs: defs
                .iter()
                .zip(kids)
                .map(|((n, _), b)| (n.clone(), b.clone()))
                .collect(),
            index: *index,
        },
        Term::Let { name, .. } => Term::Let {
            name: name.clone(),
            ty: Box::new(kids[0].clone()),
            value: Box::new(kids[1].clone()),
            body: Box::new(kids[2].clone()),
        },
        Term::Annotated { annotation, .. } => Term::annotated(annotation.clone(), kids[0].clone()),
    }
}

fn tree_nodes(t: &Term) -> usize {
    let mut n = 1;
    term_for_each_child(t, &mut |c| n += tree_nodes(c));
    n
}

/// A binding used ONCE, or not at all, ζ-expands exactly as the kernel does today. This is
/// the shape of every ζ case already in the suite, so the gate must be invisible to them.
#[test]
fn a_binding_used_at_most_once_still_zeta_expands() {
    let once = let_in("b", nat(), app(g("Succ"), g("Zero")), app(g("f"), var("b")));
    assert_eq!(
        zeta_fixpoint(&once, &toy_step),
        app(g("f"), app(g("Succ"), g("Zero"))),
        "one occurrence: ζ cannot duplicate anything, so it fires"
    );
    let never = let_in("b", nat(), app(g("Succ"), g("Zero")), g("k"));
    assert_eq!(
        zeta_fixpoint(&never, &toy_step),
        g("k"),
        "zero occurrences: the value is dropped"
    );
}

/// THE CORPUS SHAPE. A binding used four times is KEPT, and the normal form is 92 nodes
/// against the 329 eager ζ produces and the 97 the case allows.
#[test]
fn a_binding_used_more_than_once_is_kept_as_the_sharing_node() {
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let shared = let_in(
        "b",
        nat(),
        big.clone(),
        app(
            app(app(app(g("quad"), var("b")), var("b")), var("b")),
            var("b"),
        ),
    );
    let normal = zeta_fixpoint(&shared, &toy_step);
    assert!(
        matches!(normal, Term::Let { .. }),
        "the binding survives: {normal}"
    );
    assert_eq!(tree_nodes(&big), 81);
    assert_eq!(
        tree_nodes(&normal),
        92,
        "one `Let`, its annotation, the value ONCE, and the head"
    );
    assert!(tree_nodes(&normal) <= 81 + 16, "the corpus ceiling is 97");

    // …and it MEANS the same thing: ζ on demand recovers the expanded term exactly.
    let expanded = zeta_expand_head_term(&normal).expect("the survivor is `Let`-headed");
    assert_eq!(
        tree_nodes(&expanded),
        4 * 81 + 5,
        "which is the 329 the corpus measures"
    );
}

/// THE RE-EVALUATION, and the reason the gate is read twice. A body with two uses of `b`
/// before reduction and one after must still expand — otherwise
/// `a_dependent_let_body_types_only_because_zeta_runs_before_inference` goes red.
#[test]
fn the_gate_is_re_read_after_reduction_erases_an_occurrence() {
    let body = app(app(g("pair"), app(g("drop"), var("b"))), var("b"));
    assert_eq!(
        occurrences_in_term(&body, "b"),
        Occurrences::Many,
        "two occurrences BEFORE reduction, so the first reading holds the binding back"
    );
    assert_eq!(
        occurrences_in_term(&toy_step(&body), "b"),
        Occurrences::Once,
        "and one after, so the second reading releases it"
    );
    let shared = let_in("b", nat(), app(g("Succ"), g("Zero")), body);
    assert_eq!(
        zeta_fixpoint(&shared, &toy_step),
        app(app(g("pair"), g("gone")), app(g("Succ"), g("Zero"))),
        "the binding is expanded once reduction has erased the second use"
    );
}

/// The gated rule and the STORE's gated normaliser are two implementations of one
/// specification, so on a term neither of them reduces they must produce the same shape.
#[test]
fn the_term_side_zeta_gate_agrees_with_the_stores() {
    let cases = vec![
        // Kept.
        let_in(
            "b",
            nat(),
            app(g("Succ"), var("u")),
            app(app(g("p"), var("b")), var("b")),
        ),
        // Expanded — one use.
        let_in("b", nat(), app(g("Succ"), var("u")), app(g("p"), var("b"))),
        // Expanded — no use.
        let_in("b", nat(), app(g("Succ"), var("u")), g("p")),
        // Nested: an inner binding kept inside an outer one that expands.
        let_in(
            "o",
            nat(),
            g("v"),
            let_in("i", nat(), var("o"), app(app(g("p"), var("i")), var("i"))),
        ),
    ];
    let identity: &dyn Fn(&Term) -> Term = &|t: &Term| t.clone();
    for t in cases {
        let by_term = zeta_fixpoint(&t, identity);
        let mut store = TermStore::new();
        let id = store.intern(&t);
        let StoreNormalForm::Reached(nf) = store.beta_zeta_normalize(id, 1 << 16) else {
            panic!("the store normalises {t}")
        };
        assert_eq!(
            tree_nodes(&by_term),
            tree_nodes(&store.extract(nf)),
            "the two gates disagree about the SIZE of the normal form of {t}: \
                 `{by_term}` against `{}`",
            store.extract(nf)
        );
    }
}

// -- the conversion cache -------------------------------------------------------------

/// Conversion is symmetric, so the cache must answer in either order from one entry.
#[test]
fn the_conversion_cache_is_symmetric_in_its_key() {
    let mut store = TermStore::new();
    let a = store.intern(&app(g("f"), g("x")));
    let b = store.intern(&app(g("f"), g("y")));
    let mut cache = DefEqCache::new();
    assert_eq!(
        cache.get(a, b),
        None,
        "an unasked pair is unknown, not false"
    );
    cache.insert(a, b, true);
    assert_eq!(cache.get(a, b), Some(true));
    assert_eq!(
        cache.get(b, a),
        Some(true),
        "the same entry answers both orders"
    );
    assert_eq!(cache.len(), 1, "and it is ONE entry");
}

/// A NEGATIVE is remembered as a negative — the expensive half, and the one a cache that
/// only stored successes would re-decide every time.
#[test]
fn the_conversion_cache_remembers_a_refusal_as_a_refusal() {
    let mut store = TermStore::new();
    let a = store.intern(&g("Nat"));
    let b = store.intern(&g("Bool"));
    let mut cache = DefEqCache::new();
    cache.insert(a, b, false);
    assert_eq!(
        cache.get(a, b),
        Some(false),
        "not `None`, which would mean `ask again`"
    );
    assert_eq!(cache.get(b, a), Some(false));
}

/// Distinct pairs never alias, including the pairs that share a member and the reflexive
/// pair. An aliasing cache would certify one conversion with another's answer.
#[test]
fn the_conversion_cache_never_aliases_two_pairs() {
    let mut store = TermStore::new();
    let ids: Vec<TermId> = ["a", "b", "c", "d"]
        .iter()
        .map(|n| store.intern(&g(n)))
        .collect();
    let mut cache = DefEqCache::new();
    // Every unordered pair including the reflexive ones, each with its own verdict.
    let mut expected: Vec<((TermId, TermId), bool)> = Vec::new();
    for (i, &x) in ids.iter().enumerate() {
        for &y in ids.iter().skip(i) {
            let verdict = (x.index() + y.index()) % 2 == 0;
            cache.insert(x, y, verdict);
            expected.push(((x, y), verdict));
        }
    }
    assert_eq!(cache.len(), 10, "four ids make ten unordered pairs");
    for ((x, y), verdict) in expected {
        assert_eq!(cache.get(x, y), Some(verdict), "pair ({x:?}, {y:?})");
        assert_eq!(cache.get(y, x), Some(verdict), "and in the other order");
    }
}

/// The counters are what a wiring decision is measured with: a hit is a normalisation that
/// did not happen.
#[test]
fn the_conversion_cache_counts_what_it_saved() {
    let mut store = TermStore::new();
    let a = store.intern(&g("a"));
    let b = store.intern(&g("b"));
    let mut cache = DefEqCache::new();
    assert_eq!((cache.hits(), cache.misses()), (0, 0));
    assert_eq!(cache.get(a, b), None);
    assert_eq!((cache.hits(), cache.misses()), (0, 1));
    cache.insert(a, b, true);
    for _ in 0..7 {
        assert_eq!(cache.get(a, b), Some(true));
    }
    assert_eq!(
        (cache.hits(), cache.misses()),
        (7, 1),
        "seven normalisations not performed"
    );
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(
        cache.get(a, b),
        None,
        "a cleared cache has forgotten, not inverted"
    );
    assert_eq!((cache.hits(), cache.misses()), (0, 1));
}

/// The trap the doc comment names, made a test: told both answers for one pair, the cache
/// FAILS LOUDLY instead of quietly certifying the stale one. That is the failure mode of a
/// cache that outlived a `Context::pop_scope`.
#[test]
#[should_panic(expected = "outlived the context it decided under")]
fn a_conversion_cache_told_both_answers_for_one_pair_fails_loudly() {
    let mut store = TermStore::new();
    let a = store.intern(&g("a"));
    let b = store.intern(&g("b"));
    let mut cache = DefEqCache::new();
    cache.insert(a, b, true);
    cache.insert(b, a, false);
}

/// The reflexive shortcut hash-consing buys is the CALLER's to take, and it is only sound on
/// a float-free term. This pins both halves so nobody folds it into the cache.
#[test]
fn the_reflexive_shortcut_belongs_to_the_caller_and_only_off_the_floats() {
    let mut store = TermStore::new();
    let plain = store.intern(&app(g("f"), g("x")));
    assert!(
        store.same(plain, plain) && store.float_free(plain),
        "the sound shortcut"
    );

    let nan = store.intern(&Term::Lit(Literal::Float(f64::NAN)));
    assert!(store.same(nan, nan), "identity is reflexive");
    assert!(
        !store.float_free(nan),
        "the conservative guard still refuses every float"
    );
    assert_eq!(
        store.extract(nan),
        store.extract(nan),
        "bit-identity `Literal` equality is reflexive at `NaN`"
    );
    let pos = store.intern(&Term::Lit(Literal::Float(0.0)));
    let neg = store.intern(&Term::Lit(Literal::Float(-0.0)));
    assert!(
        !store.same(pos, neg) && store.extract(pos) != store.extract(neg),
        "and `±0.0` are distinct under both identity and `Term` equality"
    );
}

/// The whole point: a walk that overflows a 2 MiB stack COMPLETES on a sized one. The
/// probe is the recursion itself, so this is the mechanism under test and not a proxy.
#[test]
fn a_recursion_that_overflows_two_mebibytes_completes_on_a_sized_stack() {
    // ~16 KiB per frame — `infer_type`'s measured frame — so 2 MiB holds about 128.
    fn burn(n: usize) -> usize {
        let pad = [0u8; 16 * 1024];
        if n == 0 {
            return std::hint::black_box(&pad).len();
        }
        std::hint::black_box(&pad);
        burn(n - 1)
    }
    const RUNGS: usize = 900;
    let deep = (0..RUNGS).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    assert!(
        stack_bytes_for_depth(RUNGS) > RUNGS * 16 * 1024,
        "the sizing must cover a frame per rung"
    );
    let answer = deepen_stack(&deep, || burn(RUNGS)).expect("an outermost call always runs");
    assert_eq!(answer, 16 * 1024);
}
