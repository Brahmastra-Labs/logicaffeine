//! The equation compiler — the behaviour the corpus does not ask about.
//!
//! `interface/match_compile.rs` turns a surface clause matrix into a tree of the kernel's
//! positional `Match` nodes. The corpus fixes three points on that surface (a nested pattern, an
//! overlapping clause set, a catch-all). Everything else the compiler decides is pinned here, in
//! positive/negative pairs, because a positive-only test is satisfiable by a weak implementation:
//!
//! * a clause lands in the slot of the constructor it NAMES — and the wrong answer that source
//!   order would give is asserted to be wrong, not merely absent;
//! * overlapping clauses resolve TOP-TO-BOTTOM — and the second, shadowed body is asserted never
//!   to be reachable;
//! * a catch-all fans out over EVERY remaining constructor — and the census says which clauses
//!   nothing can reach;
//! * a constructor from another inductive, an unknown name, or the wrong field count is REFUSED
//!   by name rather than mis-slotted;
//! * a clause set that misses a case is REFUSED, because a LOGOS definition is total;
//! * the kernel's own arity rule is untouched: a hand-written `Match` with the wrong number of
//!   cases is still `WrongNumberOfCases` at every node of a tree, not just the outermost;
//! * both kernels agree about every compiled definition.

use logicaffeine_kernel::interface::{
    compile_clauses, compile_clauses_reporting, parse_command_in_ctx, Clause, Command, Pattern,
    Surface,
};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    auto_bind_implicits, bind_self_recursion, defeq_for_test, double_check, fill_match_motives,
    infer_type, is_subtype, normalize, surface_elaborate_against, BigInt, Context, DoubleCheck,
    Literal, Term, Universe,
};

// ==== PER-FILE HELPERS (house style: every kernel test declares its own) ====

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn nat() -> Term {
    g("Nat")
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn peano(n: usize) -> Term {
    (0..n).fold(g("Zero"), |acc, _| app(g("Succ"), acc))
}
/// The prelude, registered ONCE per process and handed out by copy-on-write clone.
///
/// `StandardLibrary::register` costs 314 ms on this box and this file asks for it about fifty
/// times; `Context::clone` costs a microsecond, because every table is `Arc`-backed and every
/// mutating accessor goes through `Arc::make_mut`. The two are pinned equal by
/// `the_cached_prelude_is_indistinguishable_from_a_fresh_one`, which is what makes this a
/// speed-up rather than a second standard library.
fn std_ctx() -> Context {
    logicaffeine_kernel::interface::standard_context()
}

/// A three-constructor enumeration, registered in a declared order a source order can disagree
/// with. Equal arity throughout — which is exactly when a positional case vector goes silently
/// wrong, and why `Nat` (whose constructors differ in arity) never showed the bug.
fn with_hue(ctx: &mut Context) {
    ctx.add_inductive("Hue", sort(0));
    for c in ["Ruby", "Jade", "Slate"] {
        ctx.add_constructor(c, "Hue", g("Hue"));
    }
}

/// `Repl::execute`'s `Command::Definition` arm, verbatim, through the CONTEXT-AWARE door.
///
/// The REPL itself still reads its commands from text alone, so this is the pipeline a definition
/// takes once `parse_command_in_ctx` is wired in: auto-bind implicits, bind self-recursion, fill
/// match motives, elaborate against the declared type, infer, compare the declared type against
/// the inferred one, register.
fn define(ctx: &mut Context, src: &str) -> Result<(), String> {
    let surface = Surface::new();
    let cmd = parse_command_in_ctx(ctx, &surface, src).map_err(|e| format!("parse: {e}"))?;
    let Command::Definition {
        name,
        ty,
        body,
        implicit_count,
        ..
    } = cmd
    else {
        return Err("not a definition".to_string());
    };
    let (ty, body, _implicit_count) = match ty {
        Some(t) => {
            let (t, b, k) = auto_bind_implicits(ctx, &t, &body, implicit_count);
            (Some(t), b, k)
        }
        None => (None, body, implicit_count),
    };
    let body = bind_self_recursion(&name, &body);
    let body = fill_match_motives(ctx, &body, ty.as_ref()).map_err(|e| format!("motive: {e}"))?;
    let body = surface_elaborate_against(ctx, &body, ty.as_ref())
        .map_err(|e| format!("elaborate: {e}"))?;
    let inferred = infer_type(ctx, &body).map_err(|e| format!("infer: {e}"))?;
    let ty = match ty {
        Some(declared) => {
            if !is_subtype(ctx, &inferred, &declared) {
                return Err(format!("declared {declared}, inferred {inferred}"));
            }
            declared
        }
        None => inferred,
    };
    assert_eq!(
        double_check(ctx, &body),
        DoubleCheck::Agreed,
        "the second kernel must agree about every compiled definition — `{name}`"
    );
    ctx.add_definition(name, ty, body);
    Ok(())
}

/// What a defined function computes at an argument, as a normal form.
fn eval1(ctx: &Context, f: &str, arg: Term) -> Term {
    normalize(ctx, &app(g(f), arg))
}

// =============================================================================
// NESTED PATTERNS — the tree, and the case it must NOT invent
// =============================================================================

/// The corpus's `classify`, written EXHAUSTIVELY: a nested pattern compiles to the two-level
/// positional tree, and it computes at every rung — one below the nesting, on it, and above it.
#[test]
fn a_nested_constructor_pattern_compiles_to_a_tree_that_computes() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("a nested constructor pattern must compile");

    for (input, want) in [(0usize, 0usize), (1, 1), (2, 0), (5, 3)] {
        let got = eval1(&ctx, "classify", peano(input));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "classify {input} must be {want}, got {got}"
        );
    }
    // The boundary, stated as a refusal: the nesting must not collapse `Succ Zero` into
    // `Succ (Succ _)`. `classify 1` is the rung that tells those two apart.
    assert!(
        !defeq_for_test(&ctx, &eval1(&ctx, "classify", peano(1)), &peano(0)),
        "classify 1 must not fall through to the `Succ (Succ k)` clause's shape"
    );
}

/// The negative that makes the test above mean something: the SAME definition with the middle
/// rung missing is REFUSED, by name. A LOGOS definition is a total function; Lean refuses this
/// too (`missing cases: Nat.succ Nat.zero`), and the alternative — inventing a value for the
/// uncovered case — is a silent wrong answer for every input that reaches it.
#[test]
fn a_nested_pattern_set_that_misses_a_case_is_refused_and_names_it() {
    let mut ctx = std_ctx();
    let err = define(
        &mut ctx,
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ (Succ k): Yield k.",
    )
    .expect_err("a clause set that covers no `Succ Zero` must be refused");
    assert!(
        err.contains("not exhaustive"),
        "the refusal must say the definition is not exhaustive: {err}"
    );
    assert!(
        err.contains("Succ Zero"),
        "the refusal must NAME the missing case, the way Lean does: {err}"
    );
}

/// A clause body may itself be a `Consider`, and the binder it matches on must be in scope WITH
/// ITS TYPE — otherwise the inner discriminant has no inductive to split on. The hand-nested
/// spelling and the pattern-nested spelling must denote the same function.
#[test]
fn a_clause_body_may_be_a_consider_on_the_binder_it_just_bound() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To deep (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ m: Consider m: When Zero: Yield (Succ Zero). When Succ k: Yield k.",
    )
    .expect("a nested `Consider` must see its binder's type");
    define(
        &mut ctx,
        "## To flat (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("the pattern-nested spelling must compile");

    for input in 0..6usize {
        let a = eval1(&ctx, "deep", peano(input));
        let b = eval1(&ctx, "flat", peano(input));
        assert!(
            defeq_for_test(&ctx, &a, &b),
            "the two spellings must denote one function; at {input} they gave {a} and {b}"
        );
    }
}

// =============================================================================
// FIRST MATCH — overlapping clauses
// =============================================================================

/// Overlapping clauses resolve top-to-bottom. The second `When Zero:` is SHADOWED — the compiler
/// must keep the first body, and the census must say the second is unreachable rather than the
/// compiler silently keeping whichever one it happened to visit last.
#[test]
fn overlapping_clauses_keep_the_first_body_and_the_rest_are_reported_dead() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To pick (a: Nat) -> Nat: Consider a: When Zero: Yield (Succ Zero). \
         When Zero: Yield (Succ (Succ Zero)). When Succ k: Yield (Succ (Succ (Succ Zero))).",
    )
    .expect("overlapping clauses must be accepted and split");

    let at_zero = eval1(&ctx, "pick", peano(0));
    assert!(
        defeq_for_test(&ctx, &at_zero, &peano(1)),
        "first match wins: pick Zero must be 1, got {at_zero}"
    );
    assert!(
        !defeq_for_test(&ctx, &at_zero, &peano(2)),
        "the SHADOWED second clause must be unreachable — keeping the last clause per \
         constructor is a well-typed term that denotes a different function"
    );
    assert!(
        defeq_for_test(&ctx, &eval1(&ctx, "pick", peano(9)), &peano(3)),
        "the `Succ` clause still applies"
    );

    // The census, straight from the compiler, over the same matrix.
    let cctx = std_ctx().extend("a", nat());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Zero".into(),
                args: vec![],
            }],
            body: lit(1),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Zero".into(),
                args: vec![],
            }],
            body: lit(2),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Succ".into(),
                args: vec![Pattern::Var("k".into())],
            }],
            body: lit(3),
        },
    ];
    let (_, report) = compile_clauses_reporting(&cctx, &[var("a")], &nat(), &clauses)
        .expect("the matrix compiles");
    assert_eq!(
        report.unreachable_clauses,
        vec![1],
        "exactly the shadowed clause is reported dead"
    );
}

/// A genuine TWO-COLUMN first-match matrix — `pick 0 _ = 1 | _ 0 = 2 | _ _ = 3` — is what the
/// corpus's hand-built disjoint tree denotes. The compiler must produce that function, and the
/// two wrong readings (keep the first clause per constructor of the FIRST column only, or keep
/// the last) each get a different answer on at least one of the four inputs.
#[test]
fn a_two_column_matrix_splits_into_the_disjoint_tree() {
    let ctx = std_ctx();
    let work = ctx.extend("a", nat()).extend("b", nat());
    let zero = || Pattern::Ctor {
        name: "Zero".into(),
        args: vec![],
    };
    let clauses = vec![
        Clause {
            patterns: vec![zero(), Pattern::Wildcard],
            body: lit(1),
        },
        Clause {
            patterns: vec![Pattern::Wildcard, zero()],
            body: lit(2),
        },
        Clause {
            patterns: vec![Pattern::Wildcard, Pattern::Wildcard],
            body: lit(3),
        },
    ];
    let compiled = compile_clauses(&work, &[var("a"), var("b")], &nat(), &clauses)
        .expect("a two-column matrix must compile");
    let pick = lam("a", nat(), lam("b", nat(), compiled));
    infer_type(&ctx, &pick).expect("the compiled tree must type-check");

    for (a, b, want) in [(0usize, 0usize, 1u64), (0, 5, 1), (5, 0, 2), (5, 5, 3)] {
        let got = normalize(&ctx, &apps(pick.clone(), &[peano(a), peano(b)]));
        assert!(
            defeq_for_test(&ctx, &got, &lit(want)),
            "first-match semantics: pick {a} {b} = {want}, got {got}"
        );
    }
}

// =============================================================================
// REGISTRATION ORDER — the silent wrong answer
// =============================================================================

/// `Bool` is registered `true` then `false`. Writing the clauses the other way round must not
/// compute the other function — and the wrong answer is asserted to be wrong, which is the whole
/// point: the function that source order produces type-checks perfectly.
#[test]
fn a_clause_lands_in_the_slot_of_the_constructor_it_names() {
    let mut ctx = std_ctx();
    assert_eq!(
        ctx.get_constructors("Bool")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["true", "false"],
        "the registration order this test is about"
    );
    define(
        &mut ctx,
        "## To flip (b: Bool) -> Bool: Consider b: When false: Yield true. When true: Yield false.",
    )
    .expect("clauses in either order must be accepted");

    assert_eq!(
        eval1(&ctx, "flip", g("true")),
        g("false"),
        "flip true = false"
    );
    assert_eq!(
        eval1(&ctx, "flip", g("false")),
        g("true"),
        "flip false = true"
    );
}

/// The same fact on a three-constructor enumeration, where there are six source orders and only
/// one of them coincides with the declaration. Every spelling must denote ONE function.
#[test]
fn every_source_order_of_an_enumeration_denotes_one_function() {
    let orders: [[&str; 3]; 3] = [
        ["Ruby", "Jade", "Slate"],
        ["Slate", "Ruby", "Jade"],
        ["Jade", "Slate", "Ruby"],
    ];
    let answer = |c: &str| match c {
        "Ruby" => "true",
        _ => "false",
    };
    for (i, order) in orders.iter().enumerate() {
        let mut ctx = std_ctx();
        with_hue(&mut ctx);
        let clauses: String = order
            .iter()
            .map(|c| format!("When {c}: Yield {}. ", answer(c)))
            .collect();
        let name = format!("isRuby{i}");
        define(
            &mut ctx,
            &format!("## To {name} (h: Hue) -> Bool: Consider h: {clauses}"),
        )
        .unwrap_or_else(|e| panic!("order {order:?} must compile: {e}"));
        for c in ["Ruby", "Jade", "Slate"] {
            assert_eq!(
                eval1(&ctx, &name, g(c)),
                g(answer(c)),
                "written in order {order:?}, {name} {c} must still be {}",
                answer(c)
            );
        }
    }
}

// =============================================================================
// THE CATCH-ALL
// =============================================================================

/// `When _:` fans out over every constructor with no clause of its own, and the body is
/// replicated per constructor. Three constructors, two clauses, and the two the catch-all covers
/// must both answer.
#[test]
fn a_catch_all_fans_out_over_every_remaining_constructor() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    define(
        &mut ctx,
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    )
    .expect("a catch-all must be accepted");
    for (c, want) in [("Ruby", "true"), ("Jade", "false"), ("Slate", "false")] {
        assert_eq!(eval1(&ctx, "isRuby", g(c)), g(want), "isRuby {c} = {want}");
    }
}

/// A catch-all in the MIDDLE shadows everything after it, and a catch-all that covers nothing is
/// dead — reported, not refused, because first-match makes it harmless and Lean's reading of the
/// meaning is unambiguous.
#[test]
fn a_catch_all_that_covers_nothing_is_dead_rather_than_an_error() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    define(
        &mut ctx,
        "## To tag (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When Jade: Yield false. \
         When Slate: Yield false. When _: Yield true.",
    )
    .expect("a redundant catch-all must not refuse the definition");
    for (c, want) in [("Ruby", "true"), ("Jade", "false"), ("Slate", "false")] {
        assert_eq!(
            eval1(&ctx, "tag", g(c)),
            g(want),
            "the dead catch-all must not change what {c} answers"
        );
    }

    let mut hctx = std_ctx();
    with_hue(&mut hctx);
    let work = hctx.extend("h", g("Hue"));
    let named = |c: &str| Pattern::Ctor {
        name: c.to_string(),
        args: vec![],
    };
    let clauses = vec![
        Clause {
            patterns: vec![named("Ruby")],
            body: g("true"),
        },
        Clause {
            patterns: vec![named("Jade")],
            body: g("false"),
        },
        Clause {
            patterns: vec![named("Slate")],
            body: g("false"),
        },
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: g("true"),
        },
    ];
    let (_, report) = compile_clauses_reporting(&work, &[var("h")], &g("Bool"), &clauses)
        .expect("the matrix compiles");
    assert_eq!(
        report.unreachable_clauses,
        vec![3],
        "the catch-all covers nothing"
    );

    // …and a catch-all EARLIER shadows the clauses after it, which is the same fact read the
    // other way round.
    let shadowing = vec![
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: g("true"),
        },
        Clause {
            patterns: vec![named("Jade")],
            body: g("false"),
        },
    ];
    let (term, report) = compile_clauses_reporting(&work, &[var("h")], &g("Bool"), &shadowing)
        .expect("the matrix compiles");
    assert_eq!(
        report.unreachable_clauses,
        vec![1],
        "the clause after a catch-all is dead"
    );
    let f = lam("h", g("Hue"), term);
    for c in ["Ruby", "Jade", "Slate"] {
        assert_eq!(
            normalize(&hctx, &app(f.clone(), g(c))),
            g("true"),
            "the catch-all answers for every constructor, {c} included"
        );
    }
}

// =============================================================================
// REFUSALS — a mis-slotted clause used to be a silent wrong answer
// =============================================================================

#[test]
fn a_clause_naming_a_constructor_of_another_inductive_is_refused() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let err = define(
        &mut ctx,
        "## To odd (n: Nat) -> Bool: Consider n: When Zero: Yield true. When Ruby: Yield false.",
    )
    .expect_err("a `Hue` constructor cannot match a `Nat`");
    assert!(
        err.contains("Ruby") && err.contains("Hue") && err.contains("Nat"),
        "the refusal must name the constructor, its real inductive and the one being matched: {err}"
    );
}

#[test]
fn a_clause_naming_an_unknown_constructor_is_refused_with_the_real_list() {
    let mut ctx = std_ctx();
    let err = define(
        &mut ctx,
        "## To odd (n: Nat) -> Bool: Consider n: When Zero: Yield true. When Zorp k: Yield false.",
    )
    .expect_err("`Zorp` is not a constructor of anything");
    assert!(
        err.contains("Zorp") && err.contains("Zero") && err.contains("Succ"),
        "the refusal must list the constructors that DO exist: {err}"
    );
}

#[test]
fn a_constructor_pattern_with_the_wrong_field_count_is_refused() {
    // Too few: `Succ` takes one field.
    let mut ctx = std_ctx();
    let err = define(
        &mut ctx,
        "## To bad (n: Nat) -> Bool: Consider n: When Zero: Yield true. When Succ: Yield false.",
    )
    .expect_err("`Succ` with no field must be refused");
    assert!(
        err.contains("Succ") && err.contains("1 field"),
        "the refusal must say how many fields the constructor takes: {err}"
    );

    // Too many: `Zero` takes none.
    let mut ctx2 = std_ctx();
    let err2 = define(
        &mut ctx2,
        "## To bad2 (n: Nat) -> Bool: Consider n: When Zero k: Yield true. When Succ k: Yield false.",
    )
    .expect_err("`Zero` with a field must be refused");
    assert!(
        err2.contains("Zero"),
        "the refusal must name the constructor: {err2}"
    );
}

#[test]
fn a_flat_clause_set_that_misses_a_constructor_is_refused_by_name() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let err = define(
        &mut ctx,
        "## To partial (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When Jade: Yield false.",
    )
    .expect_err("a missing constructor must be refused");
    assert!(
        err.contains("not exhaustive") && err.contains("Slate"),
        "the refusal must name the constructor nothing covers: {err}"
    );
}

// =============================================================================
// THE KERNEL IS NOT RELAXED
// =============================================================================

/// The compiler is a SURFACE pass. Whatever it accepts, a hand-written `Match` with the wrong
/// case count is still refused — at EVERY node of a tree, not only the outermost one.
#[test]
fn the_kernels_case_arity_rule_is_untouched_at_every_node() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);

    // Three cases for two constructors.
    assert!(
        infer_type(
            &ctx,
            &lam(
                "n",
                nat(),
                match_(var("n"), nat(), vec![lit(1), lit(2), lit(3)])
            )
        )
        .is_err(),
        "three cases for a two-constructor inductive must still be WrongNumberOfCases"
    );
    // Two cases for three constructors.
    assert!(
        infer_type(
            &ctx,
            &lam(
                "h",
                g("Hue"),
                match_(var("h"), g("Bool"), vec![g("true"), g("false")])
            )
        )
        .is_err(),
        "two cases for a three-constructor inductive must still be refused"
    );
    // One case at an INNER node.
    let inner_short = lam(
        "n",
        nat(),
        match_(
            var("n"),
            nat(),
            vec![
                lit(10),
                lam(
                    "m",
                    nat(),
                    match_(var("m"), nat(), vec![lam("k", nat(), var("k"))]),
                ),
            ],
        ),
    );
    assert!(
        infer_type(&ctx, &inner_short).is_err(),
        "an inner match with one case for a two-constructor inductive must be refused"
    );

    // And the compiler never emits such a thing: every node it produces has one case per
    // constructor, so the term it hands the kernel type-checks.
    let work = ctx.extend("n", nat());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Zero".into(),
                args: vec![],
            }],
            body: lit(0),
        },
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: lit(1),
        },
    ];
    let compiled = compile_clauses(&work, &[var("n")], &nat(), &clauses).expect("compiles");
    infer_type(&ctx, &lam("n", nat(), compiled)).expect("the compiled tree type-checks");
}

// =============================================================================
// POLYMORPHISM, AND WHAT A BARE NAME MEANS
// =============================================================================

/// A parametric inductive: the constructor's fields carry the discriminant's own type argument,
/// so `TCons`'s head field is a `Nat` and its tail a `TList Nat` — and the compiled tree must
/// COMPUTE, not merely type-check.
#[test]
fn a_parametric_inductive_compiles_with_its_type_argument_instantiated() {
    let ctx = std_ctx();
    let tlist_nat = app(g("TList"), nat());
    let work = ctx.extend("xs", tlist_nat.clone());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TNil".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TCons".into(),
                args: vec![Pattern::Var("h".into()), Pattern::Wildcard],
            }],
            body: var("h"),
        },
    ];
    let compiled =
        compile_clauses(&work, &[var("xs")], &nat(), &clauses).expect("a TList match compiles");
    let head_or_zero = lam("xs", tlist_nat, compiled);
    infer_type(&ctx, &head_or_zero).expect("the parametric tree must type-check");
    assert_eq!(
        double_check(&ctx, &head_or_zero),
        DoubleCheck::Agreed,
        "both kernels must agree about the parametric tree"
    );

    let nil = app(g("TNil"), nat());
    let cons = |h: Term, t: Term| apps(g("TCons"), &[nat(), h, t]);
    assert!(defeq_for_test(
        &ctx,
        &normalize(&ctx, &app(head_or_zero.clone(), nil.clone())),
        &peano(0)
    ));
    assert!(defeq_for_test(
        &ctx,
        &normalize(&ctx, &app(head_or_zero, cons(peano(4), nil))),
        &peano(4)
    ));
}

/// A bare name in a pattern is a CONSTRUCTOR when one of that name is in scope and a VARIABLE
/// otherwise — Lean's rule. The two readings are told apart here by what the function computes:
/// a variable head binds the whole value, a constructor head selects one case.
#[test]
fn a_bare_pattern_name_is_a_constructor_when_one_exists_and_a_variable_otherwise() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To ident (n: Nat) -> Nat: Consider n: When whatever: Yield whatever.",
    )
    .expect("a bare non-constructor name is a variable binding the whole value");
    for k in 0..4usize {
        let got = eval1(&ctx, "ident", peano(k));
        assert!(
            defeq_for_test(&ctx, &got, &peano(k)),
            "ident {k} = {k}, got {got}"
        );
    }

    // The neighbouring reading: `Zero` IS a constructor, so the same shape selects a case and
    // the other constructor needs its own clause — a variable head would have covered both.
    let mut ctx2 = std_ctx();
    let err = define(
        &mut ctx2,
        "## To only (n: Nat) -> Nat: Consider n: When Zero: Yield Zero.",
    )
    .expect_err("`Zero` is a constructor, so the `Succ` case is still owed");
    assert!(
        err.contains("not exhaustive") && err.contains("Succ"),
        "the refusal must name the constructor left uncovered: {err}"
    );
}

/// `Pattern::render` is what every diagnostic above reads out of, so the spelling it produces is
/// pinned: nesting is parenthesised, a nullary constructor stands alone, `_` stays `_`.
#[test]
fn a_pattern_reads_back_the_way_it_was_written() {
    let p = Pattern::Ctor {
        name: "Succ".into(),
        args: vec![Pattern::Ctor {
            name: "Succ".into(),
            args: vec![Pattern::Var("k".into())],
        }],
    };
    assert_eq!(p.render(), "Succ (Succ k)");
    assert_eq!(p.binders(), vec!["k".to_string()]);
    assert_eq!(Pattern::Wildcard.render(), "_");
    assert_eq!(
        Pattern::Ctor {
            name: "Zero".into(),
            args: vec![]
        }
        .render(),
        "Zero"
    );
    assert!(
        Pattern::Wildcard.binders().is_empty(),
        "a wildcard binds nothing"
    );
}

// =============================================================================
// THE CONTEXT-FREE DOOR STILL BEHAVES AS IT DID
// =============================================================================

/// The text-only door is unchanged for the shape it always handled, and REFUSES the three shapes
/// it cannot express instead of guessing — a parser with no constructor list must not pick a slot.
#[test]
fn the_text_only_door_keeps_its_old_reading_and_refuses_what_it_cannot_express() {
    use logicaffeine_kernel::interface::literate_parser::parse_definition;

    let (cmd, _) = parse_definition(
        "## To pred (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. When Succ k: Yield k.",
    )
    .expect("the flat reading still parses");
    let Command::Definition { body, .. } = cmd else {
        panic!("a definition")
    };
    // Exactly today's shape: the parameter λ, then one case per clause, in source order, with
    // each clause's binders re-abstracted as λs whose types the kernel fills in bidirectionally.
    let Term::Lambda {
        param, body: inner, ..
    } = &body
    else {
        panic!("a λ, got {body}")
    };
    assert_eq!(param, "n", "the definition's parameter");
    let Term::Match { cases, .. } = inner.as_ref() else {
        panic!("a match, got {inner}")
    };
    assert_eq!(cases.len(), 2, "one case per clause, as before");
    assert!(
        matches!(cases[1], Term::Lambda { .. }),
        "the `Succ` clause's binder is a λ"
    );

    for (src, what) in [
        (
            "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
             When Succ (Succ k): Yield k.",
            "nested",
        ),
        (
            "## To isZero (n: Nat) -> Bool: Consider n: When Zero: Yield true. When _: Yield false.",
            "catch-all",
        ),
    ] {
        let err = parse_definition(src)
            .err()
            .unwrap_or_else(|| panic!("the text-only door must refuse a {what} pattern"));
        let msg = format!("{err}");
        assert!(
            msg.contains("kernel context"),
            "the refusal must say where the compiler lives: {msg}"
        );
    }
}

/// A name bound twice in one clause is REFUSED. The wrong behaviour is not a crash: the compiler
/// binds each field by substituting the clause's binder, so the second occurrence would find
/// nothing left to replace and the body would read the FIRST field for both — a well-typed term
/// denoting a different function. Lean refuses it too.
#[test]
fn a_clause_that_binds_one_name_twice_is_refused() {
    let ctx = std_ctx();
    let tlist_nat = app(g("TList"), nat());
    let work = ctx.extend("xs", tlist_nat);
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TNil".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TCons".into(),
                args: vec![Pattern::Var("h".into()), Pattern::Var("h".into())],
            }],
            body: var("h"),
        },
    ];
    let err = compile_clauses(&work, &[var("xs")], &nat(), &clauses)
        .expect_err("a repeated binder must be refused");
    let msg = format!("{err}");
    assert!(msg.contains("more than once") && msg.contains('h'), "{msg}");

    // …and the same name in DIFFERENT clauses is fine, because each clause is its own scope.
    let fine = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TNil".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TCons".into(),
                args: vec![Pattern::Var("h".into()), Pattern::Wildcard],
            }],
            body: var("h"),
        },
    ];
    compile_clauses(&work, &[var("xs")], &nat(), &fine)
        .expect("one binder per field is fine, and `h` may be reused by another clause");

    // Across COLUMNS of one clause it is still one scope, so a repeat there is refused too.
    let two_col = ctx.extend("a", nat()).extend("b", nat());
    let across = vec![Clause {
        patterns: vec![Pattern::Var("n".into()), Pattern::Var("n".into())],
        body: var("n"),
    }];
    assert!(
        compile_clauses(&two_col, &[var("a"), var("b")], &nat(), &across).is_err(),
        "a name repeated across two columns of one clause is still bound twice"
    );
}

// =============================================================================
// THE LITERATE PROOF SURFACE — every global it emits must EXIST
// =============================================================================

/// The pit-of-success lock for the whole `Proof:` surface: a derivation the parser builds may
/// only name constants the kernel actually declares.
///
/// It exists because two did not. `parse_tactic_sequence` combined a bullet's tactics with
/// `tact_seq`, and `parse_single_tactic` rendered `intro` as `try_intro` — and NEITHER is
/// declared in the prelude or reduced in `reduction.rs`. Nothing in the suite noticed, because
/// a bullet with one tactic never reaches either. So `- simp. auto.` and `- intro. assumption.`
/// were spellings the surface advertised and the kernel could not read; the whole proof failed
/// with `UnboundVariable`, naming a constant no user wrote.
///
/// A vocabulary check cannot catch this — `tact_seq` was never in a vocabulary. What catches it
/// is walking the term the parser ACTUALLY emitted and demanding every name in it be resolvable.
#[test]
fn every_global_a_literate_proof_emits_is_declared() {
    use logicaffeine_kernel::interface::literate_parser::parse_theorem;

    fn globals(t: &Term, out: &mut Vec<String>) {
        match t {
            Term::Global(n) => out.push(n.clone()),
            Term::Const { name, .. } => out.push(name.clone()),
            Term::App(f, a) => {
                globals(f, out);
                globals(a, out);
            }
            Term::Lambda {
                param_type, body, ..
            } => {
                globals(param_type, out);
                globals(body, out);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                globals(param_type, out);
                globals(body_type, out);
            }
            Term::Fix { body, .. } => globals(body, out),
            Term::Let {
                ty, value, body, ..
            } => {
                globals(ty, out);
                globals(value, out);
                globals(body, out);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                globals(discriminant, out);
                globals(motive, out);
                for c in cases {
                    globals(c, out);
                }
            }
            _ => {}
        }
    }

    let ctx = std_ctx();
    // Every shape of the `Proof:` surface, including the two nobody had exercised: a bullet
    // with a SEQUENCE of tactics, and `intro` inside a bullet.
    let sources = [
        "## Theorem: A\n    Statement: x equals x.\n    Proof: refl.",
        "## Theorem: B\n    Statement: x equals x.\n    Proof: intro. assumption.",
        "## Theorem: C\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.",
        "## Theorem: D\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - simp. auto.\n    - refl.",
        "## Theorem: E\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - intro. assumption.",
    ];

    let mut missing: Vec<String> = Vec::new();
    for src in sources {
        let (cmd, _) = parse_theorem(src).unwrap_or_else(|e| panic!("{src}\n  did not parse: {e}"));
        let Command::Definition { name, body, .. } = cmd else {
            panic!("a theorem must parse as a definition");
        };
        let mut names = Vec::new();
        globals(&body, &mut names);
        names.sort();
        names.dedup();
        for n in names {
            // `_` is the parser's "type unknown here" placeholder, not a reference.
            if n != "_" && ctx.get_global(&n).is_none() {
                missing.push(format!(
                    "`{name}` emits `{n}`, which the kernel does not declare"
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "the literate proof surface may not emit a constant the kernel cannot resolve — a proof \
         that names one fails with `UnboundVariable` pointing at a name the AUTHOR never wrote: \
         {missing:#?}"
    );
}

/// A bullet's tactic SEQUENCE is the script mechanism that already exists, not a second one.
///
/// `try_script <names> <goal>` is how `Proof: intro. assumption.` is represented, and
/// `eval_script_names` is the one evaluator for it. A bullet case is the same thing scoped to
/// one constructor, so it reifies the same way — one representation, one evaluator, no drift.
#[test]
fn a_bullet_with_several_tactics_reifies_as_the_script_that_exists() {
    use logicaffeine_kernel::interface::literate_parser::parse_theorem;

    let (cmd, _) = parse_theorem(
        "## Theorem: Seq\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - simp. auto.\n    - refl.",
    )
    .expect("a multi-tactic bullet must parse");
    let Command::Definition { body, .. } = cmd else {
        panic!("not a definition")
    };
    let rendered = format!("{body}");
    assert!(
        rendered.contains("try_script"),
        "a bullet's tactic sequence must reify as `try_script`: {rendered}"
    );
    assert!(
        !rendered.contains("tact_seq"),
        "`tact_seq` is not a thing the kernel has: {rendered}"
    );
    assert!(
        rendered.contains("\"simp\"") && rendered.contains("\"auto\""),
        "both steps must appear in the script's name list, in order: {rendered}"
    );

    // The NEGATIVE: a single tactic is NOT wrapped — it stays the one-shot application, which is
    // what every green proof in the suite already reifies as.
    let (cmd, _) = parse_theorem(
        "## Theorem: One\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.",
    )
    .expect("a single-tactic bullet must parse");
    let Command::Definition { body, .. } = cmd else {
        panic!("not a definition")
    };
    let rendered = format!("{body}");
    assert!(
        !rendered.contains("try_script"),
        "one tactic is not a script — wrapping it would change every existing proof object: \
         {rendered}"
    );
    assert!(
        rendered.contains("try_refl"),
        "a single bullet tactic stays the direct application: {rendered}"
    );
}

/// `intro` inside a bullet is a SCRIPT STEP, never a one-shot tactic. It was rendered as
/// `try_intro <goal>` — a constant that does not exist — so every `- intro. …` bullet was an
/// `UnboundVariable` waiting to happen.
#[test]
fn intro_inside_a_bullet_is_a_script_step_and_not_a_phantom_tactic() {
    use logicaffeine_kernel::interface::literate_parser::parse_theorem;

    let (cmd, _) = parse_theorem(
        "## Theorem: Intro\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - intro. assumption.",
    )
    .expect("`intro` in a bullet must parse");
    let Command::Definition { body, .. } = cmd else {
        panic!("not a definition")
    };
    let rendered = format!("{body}");
    assert!(
        !rendered.contains("try_intro"),
        "`try_intro` is not declared anywhere in the kernel: {rendered}"
    );
    assert!(
        rendered.contains("\"intro\"") && rendered.contains("\"assumption\""),
        "`intro` is the script step `eval_script_names` reads: {rendered}"
    );

    // `intro` may name the variable it introduces, as it does in Lean. The name is a comment to
    // the reader — the script evaluator introduces the antecedent either way — but it must not
    // derail the parse.
    let (cmd, _) = parse_theorem(
        "## Theorem: IntroNamed\n    Statement: forall h : Nat, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - intro k. assumption.",
    )
    .expect("`intro k.` in a bullet must parse");
    let Command::Definition { body, .. } = cmd else {
        panic!("not a definition")
    };
    assert!(
        format!("{body}").contains("\"assumption\""),
        "the step AFTER a named `intro` must still be collected"
    );
}

/// The one-shot tactic table and the script-step vocabulary are ONE list.
///
/// They were two, hand-maintained, and identical by luck. A tactic added to the table but not to
/// the script list would be writable as `Proof: <t>.` and silently unreadable as a script step —
/// the parse would stop at it and the remainder would surface as trailing input, blaming the
/// author's next line.
#[test]
fn the_script_vocabulary_is_the_tactic_table() {
    use logicaffeine_kernel::interface::literate_parser::parse_theorem;

    for (kw, _) in logicaffeine_kernel::interface::literate_parser::ONE_SHOT_TACTICS {
        let src =
            format!("## Theorem: V_{kw}\n    Statement: x equals x.\n    Proof: intro. {kw}.");
        let parsed = parse_theorem(&src);
        assert!(
            parsed.is_ok(),
            "`{kw}` is a one-shot tactic, so it must also be a script step: {parsed:?}"
        );
        let (cmd, consumed) = parsed.unwrap();
        assert!(
            src[consumed.min(src.len())..].trim().is_empty(),
            "`{kw}` as a script step must consume the whole proof — a step the vocabulary misses \
             stops the parse and reports the REMAINDER ({:?}) as trailing input",
            &src[consumed.min(src.len())..]
        );
        let Command::Definition { body, .. } = cmd else {
            panic!("not a definition")
        };
        assert!(
            format!("{body}").contains(&format!("\"{kw}\"")),
            "`{kw}` must reach the script's name list"
        );
    }
}

// =============================================================================
// THE CORPUS SOURCES, THROUGH THE DOOR THAT HAS A CONTEXT
// =============================================================================

/// T04-009 and T04-010, spelled exactly as the corpus spells them, compiled through
/// [`parse_command_in_ctx`] — and then RUN.
///
/// The corpus drives these through `Repl::execute`, which still reads its command from text
/// alone (`parse_command_in`). An equation set cannot be read from text alone: the constructor
/// list, its registration ORDER and its arities live in the kernel context, and without them a
/// parser can only push one case per clause in source order — which computes a different
/// function for every equal-arity inductive, and cannot express a nested pattern or a catch-all
/// at all. So the capability is pinned HERE, at the door that has a context, and the remaining
/// gap is one line of wiring rather than a missing compiler.
#[test]
fn the_corpus_equation_sources_compile_and_compute_through_the_context_door() {
    // ---- T04-009 · overlapping equations, first match wins ----
    //
    // The corpus spells the bodies as the NUMERALS `1`/`2`/`3`. A literate numeral is
    // `Literal::Int` (the reflection layer's `SLit` needs it to be, or every `ring`/`lia`
    // theorem loses its literals), and `elab_surface`'s `Term::Lit` arm can inject a literal
    // into a single-field record but cannot change a numeral's KIND — so `Yield 1` against a
    // declared `-> Nat` is `expected Nat, found Int` at the kernel, before the compiler is even
    // reached. That is an elaborator gap (Lean's `OfNat` at the expected type), recorded in the
    // report; the COMPILER's obligation is the splitting, pinned here in Peano spelling so a
    // numeral defect cannot mask it.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a: When Zero: Yield (Succ Zero). \
         When Zero: Yield (Succ (Succ Zero)). When Succ k: Yield (Succ (Succ (Succ Zero))).",
    )
    .expect("overlapping equations must be accepted and split");
    for (a, want) in [(0usize, 1usize), (5, 3)] {
        let got = normalize(&ctx, &apps(g("pick"), &[peano(a), peano(0)]));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "first match wins: pick {a} _ must be {want}, got {got}"
        );
    }
    assert!(
        !defeq_for_test(
            &ctx,
            &normalize(&ctx, &apps(g("pick"), &[peano(0), peano(0)])),
            &peano(2)
        ),
        "the SHADOWED second `When Zero:` must be unreachable — keeping the LAST clause per \
         constructor is a well-typed term that denotes a different function"
    );

    // ---- T04-010(b) · the clause lands in the slot of the constructor it NAMES ----
    let mut ctx = std_ctx();
    assert_eq!(
        ctx.get_constructors("Bool")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        vec!["true", "false"],
        "`Bool`'s registration order is what a positional case vector is zipped against"
    );
    define(
        &mut ctx,
        "## To flip (b: Bool) -> Bool: Consider b: When false: Yield true. When true: Yield false.",
    )
    .expect("the scrambled clause order must be accepted");
    assert_eq!(
        eval1(&ctx, "flip", g("true")),
        g("false"),
        "`flip true` is `false`. Source order would put the body `true` in slot 0 and answer \
         `true` — a silent wrong answer for every equal-arity inductive"
    );
    assert_eq!(
        eval1(&ctx, "flip", g("false")),
        g("true"),
        "`flip false` is `true`"
    );

    // ---- T04-010(a) · the catch-all fans out over the REMAINING constructors ----
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    define(
        &mut ctx,
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    )
    .expect("a `When _:` catch-all must be accepted and fanned out");
    for (c, want) in [("Ruby", "true"), ("Jade", "false"), ("Slate", "false")] {
        assert_eq!(
            eval1(&ctx, "isRuby", g(c)),
            g(want),
            "the expansion must agree on EVERY constructor, not only the enumerated one"
        );
    }

    // ---- T04-008 · the nested pattern, EXHAUSTIVE (Lean refuses the corpus's two-clause
    // spelling: `missing cases: Nat.succ Nat.zero`) ----
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("a nested constructor pattern must compile to the positional tree");
    for (input, want) in [(0usize, 0usize), (1, 1), (5, 3)] {
        let got = eval1(&ctx, "classify", peano(input));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "classify {input} = {want}, got {got}"
        );
    }
}

// =============================================================================
// LITERATE INDUCTION — what it closes today, and what it must still refuse
// =============================================================================

/// Literate `induction` over an ENUMERATION closes every one of its bullet cases.
///
/// The campaign's standing claim is that literate `induction` "can close NO goal of ANY type".
/// That was true of the motive-with-no-`SLam` version; it is not true now. For an inductive
/// whose constructors are all NON-recursive, `build_case_expected` hands bullet *i* the OPEN
/// goal `P(Cᵢ)` — an ordinary proposition an ordinary tactic closes — and `DElim` verifies each
/// case conclusion against the constructor it belongs to before conceding `∀h:Hue, P h`.
///
/// What still fails is exactly the RECURSIVE constructor, and only that: for `Nat`'s `Succ`,
/// `build_induction_step_formula` hands the bullet the CLOSED `∀k:Nat. P k → P (Succ k)`, and no
/// tactic in the vocabulary dispatches on a `Forall`-headed goal. Recording the boundary here is
/// what makes the remaining gap one sentence instead of a mystery.
#[test]
fn literate_induction_closes_every_case_of_an_enumeration() {
    use logicaffeine_kernel::interface::Repl;

    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .expect("the literate inductive must register");
    repl.execute(
        "## Theorem: HueRefl\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    )
    .expect("three bullets over a three-constructor enumeration must certify");

    // The stored derivation re-checks, and the SECOND kernel agrees about it.
    let ctx = repl.context();
    let body = ctx
        .get_definition_body("HueRefl")
        .cloned()
        .expect("a certified theorem is stored");
    infer_type(ctx, &body).expect("the stored derivation must re-check");
    assert_eq!(
        double_check(ctx, &body),
        DoubleCheck::Agreed,
        "both kernels must agree about a certified induction"
    );
}

/// The negatives that make the test above mean something. A tactic that cannot close a case must
/// sink the whole elimination — an induction that certifies a FALSE statement because one bullet
/// was not checked is the worst failure this surface can have.
#[test]
fn literate_induction_refuses_a_case_no_bullet_closes() {
    use logicaffeine_kernel::interface::Repl;

    let hue = "A Hue is either Ruby or Jade or Slate.";

    // FALSE: `h equals Ruby` holds only at `Ruby`. The `Jade` and `Slate` bullets cannot close,
    // and `DElim` must refuse rather than concede the ∀.
    let mut repl = Repl::new();
    repl.execute(hue).unwrap();
    let r = repl.execute(
        "## Theorem: AllRuby\n    Statement: forall h : Hue, h equals Ruby.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    );
    assert!(
        r.is_err(),
        "`∀h:Hue, h = Ruby` is FALSE — an induction whose bullets cannot close their cases must \
         not certify it: {r:?}"
    );
    assert!(
        repl.context().get_definition_body("AllRuby").is_none(),
        "a refused theorem must leave nothing behind"
    );

    // Too FEW bullets: three constructors, two cases.
    let mut repl = Repl::new();
    repl.execute(hue).unwrap();
    assert!(
        repl.execute(
            "## Theorem: Short\n    Statement: forall h : Hue, h equals h.\n    \
             Proof: induction h.\n    - refl.\n    - refl."
        )
        .is_err(),
        "two bullets cannot cover three constructors"
    );

    // Too MANY bullets: the extra case has no constructor to be checked against.
    let mut repl = Repl::new();
    repl.execute(hue).unwrap();
    assert!(
        repl.execute(
            "## Theorem: Long\n    Statement: forall h : Hue, h equals h.\n    \
             Proof: induction h.\n    - refl.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "four bullets cannot be checked against three constructors"
    );

    // A bullet whose tactic is not in the vocabulary at all.
    let mut repl = Repl::new();
    repl.execute(hue).unwrap();
    assert!(
        repl.execute(
            "## Theorem: Bogus\n    Statement: forall h : Hue, h equals h.\n    \
             Proof: induction h.\n    - sorcery.\n    - refl.\n    - refl."
        )
        .is_err(),
        "an unknown tactic name must be refused, never treated as a closed case"
    );
}

/// The induction variable decides the inductive, and the bullet count is checked against THAT
/// inductive's constructors — not against `Nat`'s two, which is what the old hard-coded
/// `SName "Nat"` made every proof mean.
#[test]
fn the_induction_variable_chooses_the_inductive_it_eliminates() {
    use logicaffeine_kernel::interface::Repl;

    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    // Two binders, two inductives, one statement: which one is eliminated is decided by the NAME
    // written after `induction`, and the bullet count follows it.
    repl.execute(
        "## Theorem: Pair\n    Statement: forall h : Hue, forall b : Bool, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    )
    .expect("inducting on the `Hue` binder needs THREE bullets");

    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    assert!(
        repl.execute(
            "## Theorem: PairWrong\n    Statement: forall h : Hue, forall b : Bool, h equals h.\n    \
             Proof: induction b.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "inducting on the `Bool` binder takes TWO bullets — three must be refused, which is the \
         only way a hard-coded inductive becomes observable"
    );
}

/// `induction <v>` must name a variable the STATEMENT binds.
///
/// It used to default to `Nat` whenever the name was not found — including when the statement
/// quantified over something else entirely. The motive is then built over `Nat` from a body in
/// which the author's binder has become an eigenvariable, so the proposition the REPL gate
/// CERTIFIES is `∀_:Nat, P(h#eig)` while the statement said `∀h:Hue, P h`. Nothing exploits that
/// today only because no tactic can close `Nat`'s `Succ` case; the divergence is armed and
/// waiting for the first ∀-aware bullet step. A misspelt variable is a typo, and a typo must be
/// an error, not a different theorem.
#[test]
fn inducting_on_a_variable_the_statement_does_not_bind_is_refused() {
    use logicaffeine_kernel::interface::Repl;

    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    let r = repl.execute(
        "## Theorem: Typo\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction q.\n    - refl.\n    - refl.\n    - refl.",
    );
    assert!(
        r.is_err(),
        "`induction q` on a statement that binds `h` must be refused, not silently eliminated \
         over `Nat`: {r:?}"
    );
    let msg = format!("{r:?}");
    assert!(
        msg.contains('h') && msg.contains("Hue"),
        "the refusal must say what the statement DOES bind: {msg}"
    );

    // The boundary, one rung either side. A statement with NO prenex binder keeps the `Nat`
    // reading — a free variable on this surface IS universally quantified, and `Nat` is the
    // quantifier it gets. The bullet count is what makes the reading observable: `Nat` has two
    // constructors, so two bullets certify (each case has its field and hypothesis introduced,
    // as Lean's `| succ k ih =>` does, and `x = x` holds in both) and three are refused.
    let mut repl = Repl::new();
    repl.execute(
        "## Theorem: Free\n    Statement: x equals x.\n    Proof: induction n.\n    \
         - refl.\n    - refl.",
    )
    .expect("the Nat reading still applies: two bullets, one per `Nat` constructor");
    let mut repl = Repl::new();
    assert!(
        repl.execute(
            "## Theorem: FreeThree\n    Statement: x equals x.\n    Proof: induction n.\n    \
             - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "three bullets over the `Nat` reading must be refused — `Nat` has two constructors"
    );
    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    repl.execute(
        "## Theorem: Named\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    )
    .expect("naming the binder the statement DOES bind still certifies");
}

/// Soundness lock for the elimination itself: a literate induction over an inductive whose
/// constructors carry FIELDS must not certify a false goal, whether the field is plain (`Tag`'s
/// `Val n`) or RECURSIVE (`Chain`'s `Link rest`).
///
/// Each bullet runs with its constructor's fields and induction hypotheses already introduced —
/// Lean's `| link rest ih => …` — against the case goal `build_case_expected` computes, and
/// `DElim` re-verifies every case conclusion before conceding `∀t, P t`. So a TRUE statement is
/// certified with Lean's own proof (`refl` in every case of `∀c, c = c`) and a false one is
/// refused by every tactic in the vocabulary, the introduced hypotheses notwithstanding.
#[test]
fn literate_induction_over_a_field_carrying_constructor_refuses_a_false_goal() {
    use logicaffeine_kernel::interface::Repl;

    let setup = [
        "A Tag is either Zed or a Val with n: Nat.",
        "## To num (t: Tag) -> Nat: Consider t: When Zed: Yield Zero. When Val n: Yield n.",
    ];
    // `∀t:Tag, num t = Zero` is FALSE: `num (Val (Succ Zero))` is `Succ Zero`.
    for tactic in [
        "refl",
        "simp",
        "auto",
        "cc",
        "decide",
        "lia",
        "omega",
        "ring",
        "assumption",
    ] {
        let mut repl = Repl::new();
        for s in setup {
            repl.execute(s).expect("the setup must register");
        }
        let r = repl.execute(&format!(
            "## Theorem: False_{tactic}\n    Statement: forall t : Tag, (num t) equals Zero.\n    \
             Proof: induction {}.\n    - {tactic}.\n    - {tactic}.",
            "t"
        ));
        assert!(
            r.is_err(),
            "`∀t:Tag, num t = Zero` is false — `{tactic}` must not certify it: {r:?}"
        );
    }

    // A RECURSIVE field gets its field AND its induction hypothesis, introduced before the
    // bullet runs — Lean's `| link rest ih => rfl`. `Link`'s case goal is then `Link x = Link x`,
    // which is TRUE, so `refl` certifies it exactly as `rfl` does in Lean.
    let mut repl = Repl::new();
    repl.execute("A Chain is either Stop or a Link with rest: Chain.")
        .unwrap();
    repl.execute(
        "## Theorem: ChainRefl\n    Statement: forall c : Chain, c equals c.\n    \
         Proof: induction c.\n    - refl.\n    - refl.",
    )
    .expect("`∀c:Chain, c = c` by induction with `refl` in each case is Lean's own proof");

    // …and the soundness half over the RECURSIVE constructor: `∀c:Chain, c = Stop` is false at
    // `Link x`, so the introduced case goal `Link x = Stop` must defeat every one-shot tactic,
    // the induction hypothesis `x = Stop` notwithstanding.
    for tactic in ["refl", "simp", "auto", "cc", "decide", "assumption"] {
        let mut repl = Repl::new();
        repl.execute("A Chain is either Stop or a Link with rest: Chain.")
            .unwrap();
        let r = repl.execute(&format!(
            "## Theorem: ChainFalse_{tactic}\n    Statement: forall c : Chain, c equals Stop.\n    \
             Proof: induction c.\n    - {tactic}.\n    - {tactic}."
        ));
        assert!(
            r.is_err(),
            "`∀c:Chain, c = Stop` is false at `Link Stop` — `{tactic}` must not certify it: {r:?}"
        );
    }
}

/// A bullet-count mistake must be reported AS a bullet-count mistake.
///
/// The count is a fact about the INDUCTIVE, so only a door that has the context can check it.
/// `DElim` checks it too — but by the time it does, the inductive, its constructor names and the
/// number the author wrote have all been left behind, and what a reader gets is "the tactic did
/// not establish the stated goal", which is true of a dozen unrelated mistakes. This is the axis
/// on which a proof assistant is actually used, and it is one where we can simply be better.
#[test]
fn a_bullet_count_mistake_names_the_inductive_its_constructors_and_the_count() {
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let surface = Surface::new();

    let r = parse_command_in_ctx(
        &ctx,
        &surface,
        "## Theorem: Short\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.",
    );
    let msg = format!("{r:?}");
    assert!(
        r.is_err(),
        "two bullets cannot cover three constructors: {msg}"
    );
    for needle in ["Hue", "Ruby", "Jade", "Slate", "3", "2"] {
        assert!(
            msg.contains(needle),
            "the refusal must name `{needle}` — the inductive, what its constructors ARE, and \
             both counts: {msg}"
        );
    }

    // The boundary, one rung either side: the right count is accepted, one more is refused.
    parse_command_in_ctx(
        &ctx,
        &surface,
        "## Theorem: Right\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    )
    .expect("three bullets for three constructors");
    assert!(
        parse_command_in_ctx(
            &ctx,
            &surface,
            "## Theorem: Long\n    Statement: forall h : Hue, h equals h.\n    \
             Proof: induction h.\n    - refl.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "four bullets for three constructors"
    );

    // …and eliminating over something that is not an inductive at all is refused by name, rather
    // than producing an elimination with zero cases that can never conclude.
    let mut opaque = std_ctx();
    opaque.add_declaration("Widget", sort(0));
    assert!(
        format!(
            "{:?}",
            parse_command_in_ctx(
                &opaque,
                &surface,
                "## Theorem: NotInductive\n    Statement: forall w : Widget, w equals w.\n    \
                 Proof: induction w.\n    - refl."
            )
        )
        .contains("Widget"),
        "an opaque carrier has no constructors to induct over, and the refusal must say so"
    );

    // The text-only door is UNCHANGED: with no context there is no constructor list, so the
    // count cannot be checked there and must not be guessed.
    use logicaffeine_kernel::interface::literate_parser::parse_theorem;
    assert!(
        parse_theorem(
            "## Theorem: Short\n    Statement: forall h : Hue, h equals h.\n    \
             Proof: induction h.\n    - refl.\n    - refl."
        )
        .is_ok(),
        "the context-free door still parses what text alone can express"
    );
}

// =============================================================================
// WHAT THE COMPILED TREE HAS TO SURVIVE AFTERWARDS
// =============================================================================

/// A compiled equation set is not finished when it type-checks: it then has to pass the
/// TERMINATION guard, which reads the tree's shape rather than the equations that produced it.
/// `| n+2 => …` is every `halve`/`fib`/`pairSum` in every library, and its compiled form nests a
/// match inside a case binder of another match — precisely the shape the Giménez guard is
/// narrowest about. Both the flat recursion and the two-deep one are pinned here, WITH their
/// values, because "the definition was admitted" and "the definition computes" are different
/// claims and only the second is the capability.
#[test]
fn a_recursive_equation_set_is_admitted_and_computes() {
    // Flat: one constructor deep, recursion on the tail field.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To len (xs: TList of Nat) -> Nat: Consider xs: When TNil: Yield Zero. \
         When TCons h t: Yield (Succ (len t)).",
    )
    .expect("a recursive clause set must be admitted");
    let nil = app(g("TNil"), nat());
    let cons = |h: Term, t: Term| apps(g("TCons"), &[nat(), h, t]);
    for (list, want) in [
        (nil.clone(), 0usize),
        (cons(peano(7), nil.clone()), 1),
        (cons(peano(7), cons(peano(8), nil.clone())), 2),
    ] {
        let got = normalize(&ctx, &app(g("len"), list));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "len must be {want}, got {got}"
        );
    }

    // Two constructors deep, recursion on the INNER field — `| n+2 => …`.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To halve (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield Zero. When Succ (Succ k): Yield (Succ (halve k)).",
    )
    .expect("`| n+2 => …` must be admitted once the compiler writes the tree");
    for (input, want) in [(0usize, 0usize), (1, 0), (2, 1), (3, 1), (7, 3), (8, 4)] {
        let got = normalize(&ctx, &app(g("halve"), peano(input)));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "halve {input} = {want}, got {got}"
        );
    }
}

/// A clause binder SHADOWS an outer name of the same spelling inside that clause's body, and
/// only there. This is the substitution's job and it is easy to get backwards: bind the field to
/// the outer value and `pred (n+1)` silently returns `n+1`.
#[test]
fn a_clause_binder_shadows_the_parameter_it_shares_a_name_with() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To pred (n: Nat) -> Nat: Consider n: When Zero: Yield n. When Succ n: Yield n.",
    )
    .expect("a clause binder may reuse the parameter's name");
    // `Zero`'s body sees the PARAMETER (which is `Zero` there); `Succ n`'s body sees the FIELD.
    for (input, want) in [(0usize, 0usize), (1, 0), (2, 1), (5, 4)] {
        let got = normalize(&ctx, &app(g("pred"), peano(input)));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "pred {input} = {want}, got {got}"
        );
    }
    assert!(
        !defeq_for_test(&ctx, &normalize(&ctx, &app(g("pred"), peano(5))), &peano(5)),
        "the `Succ n` body must read the FIELD, not the whole value it shadows"
    );
}

/// First match wins even when the catch-all comes FIRST, and a clause it shadows is reported dead
/// rather than quietly preferred. Preferring the more SPECIFIC clause would be a defensible
/// language and is not Lean's; picking whichever the compiler visited last is not a language at
/// all.
#[test]
fn a_leading_catch_all_shadows_every_clause_after_it() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To f (n: Nat) -> Nat: Consider n: When _: Yield Zero. When Zero: Yield (Succ Zero).",
    )
    .expect("a leading catch-all is legal, and everything after it is dead");
    for k in [0usize, 1, 6] {
        let got = normalize(&ctx, &app(g("f"), peano(k)));
        assert!(
            defeq_for_test(&ctx, &got, &peano(0)),
            "the FIRST clause wins at every input, including the one the later clause names: \
             f {k} gave {got}"
        );
    }
    // The census says which clause nothing can reach.
    let work = ctx.extend("n", nat());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Zero".into(),
                args: vec![],
            }],
            body: app(g("Succ"), g("Zero")),
        },
    ];
    let (_, report) = compile_clauses_reporting(&work, &[var("n")], &nat(), &clauses)
        .expect("the clause set is total from its first clause onwards");
    assert_eq!(
        report.unreachable_clauses,
        vec![1],
        "the shadowed clause must be REPORTED dead, not silently dropped"
    );
}

/// Four constructors deep, with a catch-all for everything shallower. The tree is one layer per
/// constructor the pattern inspected, and the catch-all has to be replicated into each of those
/// layers with the right arity — which is where a fan-out that re-abstracts wrongly shows up.
#[test]
fn a_four_deep_pattern_nests_four_layers_and_the_catch_all_fills_every_one() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To drop4 (n: Nat) -> Nat: Consider n: When Succ (Succ (Succ (Succ k))): Yield k. \
         When _: Yield Zero.",
    )
    .expect("a four-deep pattern with a catch-all must compile");
    for (input, want) in [(0usize, 0usize), (1, 0), (3, 0), (4, 0), (5, 1), (9, 5)] {
        let got = normalize(&ctx, &app(g("drop4"), peano(input)));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "drop4 {input} = {want}, got {got}"
        );
    }
    assert!(
        !defeq_for_test(
            &ctx,
            &normalize(&ctx, &app(g("drop4"), peano(3))),
            &peano(1)
        ),
        "input 3 is SHALLOWER than the pattern and must reach the catch-all, not fall through \
         into a partly-matched branch"
    );
}

// =============================================================================
// THE SHAPES A NAIVE COMPILER GETS WRONG
// =============================================================================

/// A DEPENDENT constructor: its second field's type mentions its first field. Each case is a
/// λ-telescope over FRESH binders, so every later field type has to be re-closed over the binder
/// that actually stands for the earlier field — `λ(#fld0:Bool). λ(#fld1:Carrier #fld0). …`. A
/// compiler that emits the constructor's own parameter names instead produces a case whose second
/// binder mentions a name nothing binds; one that drops the dependency produces a case the kernel
/// accepts at the WRONG type.
#[test]
fn a_dependent_constructors_later_field_type_is_closed_over_the_binder_before_it() {
    let mut ctx = std_ctx();
    // Carrier : Bool → Type 0 := λb. match b with | true => Nat | false => Bool
    ctx.add_definition(
        "Carrier".to_string(),
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(g("Bool")),
            body_type: Box::new(sort(0)),
        },
        lam(
            "b",
            g("Bool"),
            match_(
                var("b"),
                lam("_", g("Bool"), sort(0)),
                vec![nat(), g("Bool")],
            ),
        ),
    );
    ctx.add_inductive("Dep", sort(0));
    ctx.add_constructor(
        "mk",
        "Dep",
        Term::Pi {
            param: "b".into(),
            param_type: Box::new(g("Bool")),
            body_type: Box::new(Term::Pi {
                param: "v".into(),
                param_type: Box::new(app(g("Carrier"), var("b"))),
                body_type: Box::new(g("Dep")),
            }),
        },
    );

    let work = ctx.extend("d", g("Dep"));
    let clauses = vec![Clause {
        patterns: vec![Pattern::Ctor {
            name: "mk".into(),
            args: vec![Pattern::Var("b".into()), Pattern::Wildcard],
        }],
        body: var("b"),
    }];
    let compiled = compile_clauses(&work, &[var("d")], &g("Bool"), &clauses)
        .expect("a dependent constructor pattern must compile");
    let tag = lam("d", g("Dep"), compiled);
    infer_type(&ctx, &tag).expect("the dependent case telescope must type-check");
    assert_eq!(
        double_check(&ctx, &tag),
        DoubleCheck::Agreed,
        "both kernels must agree about a dependent case telescope"
    );
    assert_eq!(
        normalize(
            &ctx,
            &app(tag.clone(), apps(g("mk"), &[g("true"), peano(0)]))
        ),
        g("true"),
        "the dependent match must COMPUTE, not merely type-check"
    );
    assert_eq!(
        normalize(&ctx, &app(tag, apps(g("mk"), &[g("false"), g("false")]))),
        g("false")
    );

    // The scope types the PARSER puts a clause's binders in are the same telescope walk, so the
    // two cannot drift: `pattern_binder_types` must report `v : Carrier b`, not `Carrier` applied
    // to the constructor's own dead parameter name.
    let binders = logicaffeine_kernel::interface::match_compile::pattern_binder_types(
        &ctx,
        &g("Dep"),
        &Pattern::Ctor {
            name: "mk".into(),
            args: vec![Pattern::Var("b".into()), Pattern::Var("v".into())],
        },
    )
    .expect("the binder telescope must be computable");
    assert_eq!(binders.len(), 2);
    assert_eq!(binders[0].0, "b");
    assert_eq!(binders[1].0, "v");
    assert_eq!(
        format!("{}", binders[1].1),
        "(Carrier b)",
        "the later field's type must name the binder the PATTERN introduced"
    );
}

/// A nested pattern on a PARAMETRIC inductive: the inner sub-pattern is matched at the type the
/// outer constructor's field gives it, with the discriminant's own type argument substituted in.
/// `TList (TList Nat)` inspected two deep has to reach a `Nat`, and the shallower shapes have to
/// reach the catch-all rather than a partly-matched branch.
#[test]
fn a_nested_pattern_on_a_parametric_inductive_instantiates_at_each_layer() {
    let ctx = std_ctx();
    let nested_list = app(g("TList"), app(g("TList"), nat()));
    let work = ctx.extend("xss", nested_list.clone());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "TCons".into(),
                args: vec![
                    Pattern::Ctor {
                        name: "TCons".into(),
                        args: vec![Pattern::Var("h".into()), Pattern::Wildcard],
                    },
                    Pattern::Wildcard,
                ],
            }],
            body: var("h"),
        },
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: g("Zero"),
        },
    ];
    let compiled = compile_clauses(&work, &[var("xss")], &nat(), &clauses)
        .expect("a two-layer parametric pattern must compile");
    let first_first = lam("xss", nested_list, compiled);
    infer_type(&ctx, &first_first).expect("the parametric tree must type-check");
    assert_eq!(double_check(&ctx, &first_first), DoubleCheck::Agreed);

    let nil_n = app(g("TNil"), nat());
    let nil_l = app(g("TNil"), app(g("TList"), nat()));
    let cons_n = |h: Term, t: Term| apps(g("TCons"), &[nat(), h, t]);
    let cons_l = |h: Term, t: Term| apps(g("TCons"), &[app(g("TList"), nat()), h, t]);

    let got = normalize(
        &ctx,
        &app(
            first_first.clone(),
            cons_l(cons_n(peano(7), nil_n.clone()), nil_l.clone()),
        ),
    );
    assert!(
        defeq_for_test(&ctx, &got, &peano(7)),
        "[[7]] must reach the inner head, got {got}"
    );
    assert!(
        defeq_for_test(
            &ctx,
            &normalize(
                &ctx,
                &app(first_first.clone(), cons_l(nil_n, nil_l.clone()))
            ),
            &peano(0)
        ),
        "[[]] is one layer short of the pattern and must reach the catch-all"
    );
    assert!(
        defeq_for_test(&ctx, &normalize(&ctx, &app(first_first, nil_l)), &peano(0)),
        "[] is two layers short and must reach the catch-all"
    );
}

/// An INDEXED discriminant is refused by name rather than approximated.
///
/// The kernel types an indexed match by a different rule: the motive abstracts the INDICES as
/// well as the scrutinee, and coverage is over the constructors the indices still allow, not over
/// all of them. A clause list carries no motive and cannot say which indices it means, so
/// compiling one into a plain positional match would emit a tree whose case count is right by
/// luck. Saying so is the honest answer; guessing is how a `Vec (Succ n)` match silently acquires
/// a `vnil` branch that cannot exist.
#[test]
fn an_indexed_discriminant_is_refused_and_says_why() {
    let mut ctx = std_ctx();
    ctx.add_inductive(
        "Vec",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(nat()),
            body_type: Box::new(sort(0)),
        },
    );
    ctx.add_constructor("vnil", "Vec", app(g("Vec"), g("Zero")));
    ctx.add_constructor(
        "vcons",
        "Vec",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(nat()),
            body_type: Box::new(Term::Pi {
                param: "x".into(),
                param_type: Box::new(nat()),
                body_type: Box::new(Term::Pi {
                    param: "xs".into(),
                    param_type: Box::new(app(g("Vec"), var("n"))),
                    body_type: Box::new(app(g("Vec"), app(g("Succ"), var("n")))),
                }),
            }),
        },
    );

    let work = ctx.extend("v", app(g("Vec"), g("Zero")));
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "vnil".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: app(g("Succ"), g("Zero")),
        },
    ];
    let err = compile_clauses(&work, &[var("v")], &nat(), &clauses)
        .expect_err("an indexed discriminant must be refused");
    let msg = format!("{err}");
    assert!(
        msg.contains("Vec") && msg.contains("INDEXED"),
        "the refusal must name it: {msg}"
    );
    assert!(
        msg.contains("motive"),
        "and must say what an indexed elimination needs instead: {msg}"
    );

    // The boundary: a PARAMETRIC inductive at the same arity is not indexed and still compiles.
    let plain = ctx.extend("xs", app(g("TList"), nat()));
    compile_clauses(
        &plain,
        &[var("xs")],
        &nat(),
        &[
            Clause {
                patterns: vec![Pattern::Ctor {
                    name: "TNil".into(),
                    args: vec![],
                }],
                body: g("Zero"),
            },
            Clause {
                patterns: vec![Pattern::Ctor {
                    name: "TCons".into(),
                    args: vec![Pattern::Var("h".into()), Pattern::Wildcard],
                }],
                body: var("h"),
            },
        ],
    )
    .expect("one type ARGUMENT is a parameter, not an index");
}

/// The two doors route the SAME commands, and the ONLY difference between them is the equation
/// compiler.
///
/// `parse_command_in_ctx` is a pre-dispatch on two prefixes in front of `parse_command`, not a
/// second dispatcher. The moment it recognises one spelling the other does not, the language has
/// two grammars and which one you get depends on whether your caller happened to hold a context.
/// This enumerates the whole difference: three `Consider` shapes text alone cannot express.
#[test]
fn the_context_door_differs_from_the_text_door_only_by_the_equation_compiler() {
    use logicaffeine_kernel::interface::parse_command;

    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let surface = Surface::new();

    // Every surface that is NOT a `Consider` the compiler is needed for must behave identically.
    let same = [
        "Check Zero.",
        "Eval (Succ Zero).",
        "A Shade is either Pale or Deep.",
        "Definition two : Nat := Succ (Succ Zero).",
        "## To twice (n: Nat) -> Nat: Yield n.",
        "## To id (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. When Succ k: Yield (Succ k).",
        "## Theorem: R\n    Statement: x equals x.\n    Proof: refl.",
        "## Theorem: NoProof\n    Statement: x equals x.",
        "Let t be Zero.",
        "-- a comment declares nothing",
        "Notation \"a %% b\" := ((add a) b).",
        "this is not a command at all",
        "## To broken (n: Nat) ->",
    ];
    for src in same {
        assert_eq!(
            parse_command_in_ctx(&ctx, &surface, src).is_ok(),
            parse_command(src).is_ok(),
            "the two doors disagree about `{src}` — one of them is a second grammar"
        );
    }

    // …and the WHOLE difference, enumerated. Two of the three shapes text alone cannot even
    // REPRESENT, so it refuses them by name rather than guessing a slot.
    for src in [
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield Zero. When Succ (Succ k): Yield k.",
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    ] {
        assert!(
            parse_command_in_ctx(&ctx, &surface, src).is_ok(),
            "the context door must compile `{src}`"
        );
        assert!(
            parse_command(src).is_err(),
            "the text door must REFUSE `{src}` rather than guess a slot"
        );
    }

    // The third shape is the one that PARSES on both doors and diverges at the kernel: N
    // overlapping equations are N cases in source order, which is not a term at all once the
    // constructor count is known. That is exactly the corpus's `WrongNumberOfCases { expected: 2,
    // found: 3 }` — the failure arrives after the parser, which is why nothing in the parser can
    // be blamed for it and why the compiler has to live where the context is.
    let overlapping = "## To pick (a: Nat) -> Nat: Consider a: When Zero: Yield Zero. \
                       When Zero: Yield Zero. When Succ k: Yield k.";
    let flat = parse_command(overlapping).expect("text alone parses it — it just means nothing");
    let Command::Definition { body, .. } = flat else {
        panic!("not a definition")
    };
    assert!(
        infer_type(&ctx, &body).is_err(),
        "three cases for a two-constructor inductive is not a well-typed term"
    );
    let mut work = std_ctx();
    define(&mut work, overlapping).expect("the context door compiles it into a disjoint tree");
    assert!(
        defeq_for_test(
            &work,
            &normalize(&work, &app(g("pick"), peano(0))),
            &peano(0)
        ),
        "and the tree it compiles COMPUTES, first match first"
    );
}

/// Each clause's RIGHT-HAND SIDE is elaborated at the result type the equations declare.
///
/// This is the stage that first knows "this body must have type R" — the kernel's `Match` arm
/// elaborates its cases against nothing, because a case is a λ-telescope whose expected type it
/// would have to reconstruct from the constructor. The compiler already HAS `result_type`, at
/// every leaf, in the scope the leaf is written in.
///
/// What it buys, in Lean's terms, is `OfNat` at the expected type: `Yield 1` in a function
/// declared `-> Cel` is the `Cel` whose single field is `1`, not an `Int` the kernel then refuses.
#[test]
fn a_clause_body_is_elaborated_at_the_result_type_the_equations_declare() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Cel", sort(0));
    ctx.add_constructor(
        "Mk",
        "Cel",
        Term::Pi {
            param: "v".into(),
            param_type: Box::new(g("Int")),
            body_type: Box::new(g("Cel")),
        },
    );

    let work = ctx.extend("b", g("Bool"));
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "true".into(),
                args: vec![],
            }],
            body: Term::Lit(Literal::Int(1)),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "false".into(),
                args: vec![],
            }],
            body: Term::Lit(Literal::Int(2)),
        },
    ];
    let compiled = compile_clauses(&work, &[var("b")], &g("Cel"), &clauses)
        .expect("a numeral right-hand side must be read at the declared result type");
    let f = lam("b", g("Bool"), compiled);
    infer_type(&ctx, &f).expect("the compiled tree must type-check at `Bool -> Cel`");
    assert_eq!(double_check(&ctx, &f), DoubleCheck::Agreed);
    assert_eq!(
        normalize(&ctx, &app(f.clone(), g("true"))),
        app(g("Mk"), Term::Lit(Literal::Int(1))),
        "`Yield 1` at a `Cel` result is `Mk 1`"
    );
    assert_eq!(
        normalize(&ctx, &app(f, g("false"))),
        app(g("Mk"), Term::Lit(Literal::Int(2)))
    );

    // The NEGATIVE, and the reason this is a best-effort pass rather than a gate: a body that
    // elaboration cannot reconcile is handed on UNCHANGED, so the mismatch is still reported by
    // the kernel against the real expected type — never silently coerced into something else.
    let bad = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "true".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "false".into(),
                args: vec![],
            }],
            body: g("Zero"),
        },
    ];
    let compiled = compile_clauses(&work, &[var("b")], &g("Cel"), &bad)
        .expect("the compiler emits the tree; the TYPE is the kernel's to judge");
    assert!(
        infer_type(&ctx, &lam("b", g("Bool"), compiled)).is_err(),
        "a `Nat` body at a `Cel` result must still be a type error — elaboration may CHOOSE a \
         term, never change what one MEANS"
    );

    // …and a RECURSIVE clause set, whose body names a function the context does not hold yet,
    // must be untouched rather than refused: self-recursion is bound after compilation.
    let mut rec = std_ctx();
    define(
        &mut rec,
        "## To countdown (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ k: Yield (countdown k).",
    )
    .expect("a body that mentions the function being defined must survive the leaf pass");
    assert!(defeq_for_test(
        &rec,
        &normalize(&rec, &app(g("countdown"), peano(4))),
        &peano(0)
    ));
}

/// The two doors measure a command the same way.
///
/// `parse_command_prefix_in_ctx` is what a document splitter loops to find each block's EXTENT.
/// If the context door and the text door disagreed about where a command ENDS, the same file
/// would split into different blocks depending on whether its caller happened to hold a context —
/// and the second block of the two would be silently discarded or silently re-read.
#[test]
fn the_two_doors_measure_a_command_the_same_way() {
    use logicaffeine_kernel::interface::{parse_command_prefix_in, parse_command_prefix_in_ctx};

    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let surface = Surface::new();

    // A command, then more document after it. Both doors must stop at the same byte.
    let follow_ons = [
        "Check Zero. Check (Succ Zero).",
        "  Eval Zero.   Check Zero.",
        "A Shade is either Pale or Deep. Check Pale.",
        "## To twice (n: Nat) -> Nat: Yield n. Check Zero.",
        "## To keep (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. When Succ k: Yield k. \
         Check Zero.",
        "## Theorem: R\n    Statement: x equals x.\n    Proof: refl. Check Zero.",
        "Definition two : Nat := Succ (Succ Zero). Check two.",
        "Let t be Zero. Check Zero.",
    ];
    for src in follow_ons {
        let plain = parse_command_prefix_in(&surface, src).map(|(_, n)| n);
        let with_ctx = parse_command_prefix_in_ctx(&ctx, &surface, src).map(|(_, n)| n);
        assert_eq!(
            plain.as_ref().ok(),
            with_ctx.as_ref().ok(),
            "the two doors must consume the same number of bytes of `{src}` — a document splits \
             on this number"
        );
        // …and the extent must actually END the command: what follows parses on its own.
        if let Ok(n) = with_ctx {
            assert!(
                parse_command_prefix_in_ctx(&ctx, &surface, &src[n..]).is_ok(),
                "the remainder after the first block must itself be a command: {:?}",
                &src[n..]
            );
        }
    }

    // A block the CONTEXT door can measure and the text door cannot: a nested pattern. The text
    // door refuses it outright, so a document containing one cannot even be split.
    let nested = "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
                  When Succ Zero: Yield Zero. When Succ (Succ k): Yield k. Check Zero.";
    let (_, n) = parse_command_prefix_in_ctx(&ctx, &surface, nested)
        .expect("the context door measures a nested-pattern block");
    assert_eq!(
        nested[n..].trim(),
        "Check Zero.",
        "and it stops exactly at the next block"
    );
    assert!(
        parse_command_prefix_in(&surface, nested).is_err(),
        "the text door cannot measure what it cannot parse"
    );
}

// =============================================================================
// ONE NAME GRAMMAR, EVERY DECLARATION DOOR (wave 3)
// =============================================================================

/// Every literate declaration surface refuses a name the term door cannot spell back.
///
/// ★ THE DEFECT THIS CLOSES. The literate doors built `Command::Definition { name }` /
/// `Command::Inductive { name }` straight from `parse_ident`, which reads the COMPONENT grammar
/// and nothing else. The vernacular door had, and the literate door had not, the two checks that
/// make a name HOLDABLE — that a term can spell it, and that its flat key reads back as itself —
/// so `## To match (n: Nat) -> Nat: …` keyed the context at `match`, a global no `Check` could
/// ever name, and `## Theorem: «a.b»` keyed at `a.b`, colliding with the two-component name `a.b`
/// that a later `Namespace a. Definition b` would take. A write-only global is the same defect
/// `vernacular_extra::a_declarable_name_is_always_a_referenceable_one` forbids, arriving through
/// a door that test's list could not see.
///
/// The rule now lives ONCE, in `interface::declaration_name`, and every door reads it — which is
/// why this test enumerates the DOORS rather than restating the rule.
#[test]
fn every_literate_declaration_door_refuses_a_write_only_name() {
    use logicaffeine_kernel::interface::Repl;

    // Each door, spelled as a function of the name it declares.
    let doors: &[(&str, fn(&str) -> String)] = &[
        ("## To", |n| {
            format!("## To {n} (x: Nat) -> Nat: Consider x: When Zero: Yield Zero. When Succ k: Yield k.")
        }),
        ("## Theorem:", |n| {
            format!("## Theorem: {n}\n    Statement: x equals x.\n    Proof: refl.")
        }),
        ("Let … be", |n| format!("Let {n} be Zero.")),
        ("A … is either", |n| {
            format!("A {n} is either Yon or Hin.")
        }),
    ];

    // ★ THE INVARIANT, and it does not depend on guessing which words the term door reads as
    // syntax: every name is EITHER refused at the declaration OR spellable by a term afterwards.
    // There is no third answer, and the third answer is the defect.
    for (door, spell) in doors {
        for name in [
            "h",
            "h'",
            "prime'd_name",
            "\u{3b1}",
            "x\u{2081}",
            "goodName",
            "ending",
            "Typeface",
            "end",
            "by",
            "in",
            "with",
            "return",
            "fun",
            "forall",
            "fix",
            "match",
            "let",
            "Type",
            "Prop",
            "Type0",
            "Sort",
            "\u{ab}a name with spaces\u{bb}",
            "\u{ab}a.b\u{bb}",
            "1abc",
        ] {
            let mut repl = Repl::new();
            if repl.execute(&spell(name)).is_err() {
                assert!(
                    repl.context().get_global(name).is_none(),
                    "`{door}` refused `{name}` and registered it anyway"
                );
                continue;
            }
            repl.execute(&format!("Check {name}.")).unwrap_or_else(|e| {
                panic!(
                    "`{door}` declared `{name}`, so a term must be able to spell it back — a name \
                     the context holds and no command can reach is a write-only global: {e:?}"
                )
            });
        }
    }

    // …and the invariant is not satisfied vacuously by refusing nothing. These are the two routes
    // that used to mint a write-only name through a literate door: a word the term door reads as
    // SYNTAX, and a quoted component holding a `.` (one component keying at `a.b`, which reads
    // back as the TWO-component name `a.b`, so the two would share one key).
    for (door, spell) in doors {
        for bad in [
            "fun",
            "forall",
            "fix",
            "match",
            "let",
            "Type",
            "Prop",
            "\u{ab}a.b\u{bb}",
        ] {
            let mut repl = Repl::new();
            let declared = repl.execute(&spell(bad));
            assert!(
                declared.is_err(),
                "the `{door}` door must refuse `{bad}` — the context would be keyed at a name no \
                 term can spell: {declared:?}"
            );
            for key in [bad, "a.b"] {
                assert!(
                    repl.context().get_global(key).is_none(),
                    "`{door}` refused `{bad}` and then registered `{key}` anyway"
                );
            }
        }
    }

    // A quoted name with no `.` in it is the escape it was built to be, through a literate door
    // too: its key reads back as nothing at all, so there is nothing for it to collide with.
    let mut quoted = Repl::new();
    quoted
        .execute("Let \u{ab}a name with spaces\u{bb} be Zero.")
        .expect("a quoted component with no dot is holdable");
    quoted
        .execute("Check \u{ab}a name with spaces\u{bb}.")
        .expect("…and a term spells it back");
}

/// The two declaration doors never DISAGREE about a name.
///
/// One rule, read by both, is the only thing that keeps `## To f …` and `Definition f …` from
/// drifting into two grammars — and a drift would not surface as an error, it would surface as a
/// name one door holds and the other cannot. The grid deliberately contains the rows both doors
/// get WRONG today (`Outer.«a b»`, which the term door's dot-postfix cannot spell and which
/// `V_vernacular_extra::a_name_the_term_door_cannot_spell_is_refused_at_the_declaration` pins):
/// this test asserts AGREEMENT, so it stays honest about what the shared rule currently says
/// while that rule is being corrected in one place.
#[test]
fn the_literate_and_vernacular_doors_agree_about_a_declaration_name() {
    use logicaffeine_kernel::interface::Repl;

    for name in [
        "h",
        "h'",
        "prime'd_name",
        "\u{3b1}",
        "x\u{2081}",
        "goodName",
        "\u{ab}a name with spaces\u{bb}",
        "\u{ab}weird?!name\u{bb}",
        "end",
        "fun",
        "Type",
        "Prop",
        "\u{ab}a.b\u{bb}",
        "\u{ab}unclosed",
        "1abc",
        "Outer.\u{ab}a b\u{bb}",
    ] {
        let vernacular = Repl::new()
            .execute(&format!("Definition {name} : Nat := Zero."))
            .is_ok();
        let literate = Repl::new().execute(&format!("Let {name} be Zero.")).is_ok();
        assert_eq!(
            vernacular, literate,
            "`{name}`: the vernacular door says {vernacular} and the literate door says \
             {literate}. One name grammar means one verdict — a name one door holds and the \
             other cannot spell is a write-only global by construction"
        );
    }
}

// =============================================================================
// THE BULLET VOCABULARY IS A CLOSED SET, AND NONE OF IT PROVES A FALSEHOOD
// =============================================================================

/// ★ THE RATCHET. No entry of [`ONE_SHOT_TACTICS`] may certify a FALSE statement, in either
/// position it can occupy — as the whole `Proof:` and as a bullet under `induction`.
///
/// This is the invariant that makes a soundness hole in the elimination layer un-shippable
/// through this surface rather than one table row away from it. `try_ddestruct_conclude` used to
/// check only the CASE COUNT before conceding `∀x:I, P x` — verified end to end: a `try_destruct`
/// over `Bool` with two `DAxiom (SName "True")` cases concluded `∀b:Bool, False`. Nothing in the
/// literate surface could reach it, for one reason only: `destruct` was not a row in the table.
/// The moment Lean's `cases` is added — the obvious next feature in this lane — every ∀-statement
/// over every inductive becomes certifiable unless the elimination checks its cases.
///
/// So the vocabulary is read OFF THE TABLE rather than listed here: a row added without the
/// checking behind it turns this test red at the moment it is written, not at the moment someone
/// notices. `a_case_analysis_derivation_cannot_concede_a_forall_its_cases_never_proved` pins the
/// same rule one layer down, at the derivation former itself.
#[test]
fn no_entry_in_the_bullet_vocabulary_certifies_a_false_statement() {
    use logicaffeine_kernel::interface::literate_parser::ONE_SHOT_TACTICS;
    use logicaffeine_kernel::interface::Repl;

    assert!(
        !ONE_SHOT_TACTICS.is_empty(),
        "reading the vocabulary off the table is the whole point — an empty table would make \
         this test vacuous"
    );

    // (setup commands, a FALSE statement, the variable to induct on, how many bullets it needs)
    let falsehoods: &[(&[&str], &str, &str, usize)] = &[
        (
            &["A Hue is either Ruby or Jade or Slate."],
            "forall h : Hue, h equals Ruby",
            "h",
            3,
        ),
        (
            &[
                "A Tag is either Zed or a Val with n: Nat.",
                "## To num (t: Tag) -> Nat: Consider t: When Zed: Yield Zero. When Val n: Yield n.",
            ],
            "forall t : Tag, (num t) equals Zero",
            "t",
            2,
        ),
        (
            &["A Chain is either Stop or a Link with rest: Chain."],
            "forall c : Chain, c equals Stop",
            "c",
            2,
        ),
    ];

    for (keyword, _builder) in ONE_SHOT_TACTICS {
        // (a) As the whole proof, on a closed falsehood no quantifier can excuse.
        let mut repl = Repl::new();
        let direct = repl.execute(&format!(
            "## Theorem: Direct_{keyword}\n    Statement: Zero equals (Succ Zero).\n    \
             Proof: {keyword}."
        ));
        assert!(direct.is_err(), "`{keyword}` certified `0 = 1`: {direct:?}");
        assert!(
            repl.context()
                .get_definition_body(&format!("Direct_{keyword}"))
                .is_none(),
            "`{keyword}` refused `0 = 1` and stored a derivation for it anyway"
        );

        // (b) As every bullet of an elimination, on a statement false at some constructor. An
        // elimination that counts its cases instead of CHECKING them concedes exactly here.
        for (i, (setup, statement, v, bullets)) in falsehoods.iter().enumerate() {
            let mut repl = Repl::new();
            for s in *setup {
                repl.execute(s).expect("the setup must register");
            }
            let name = format!("Cases_{keyword}_{i}");
            let script = std::iter::repeat(format!("    - {keyword}."))
                .take(*bullets)
                .collect::<Vec<_>>()
                .join("\n");
            let got = repl.execute(&format!(
                "## Theorem: {name}\n    Statement: {statement}.\n    Proof: induction {v}.\n{script}"
            ));
            assert!(
                got.is_err(),
                "`{statement}` is FALSE — `{keyword}` bullets must not certify it: {got:?}"
            );
            assert!(
                repl.context().get_definition_body(&name).is_none(),
                "`{statement}` was refused and a derivation for it was stored anyway"
            );
        }
    }
}

/// ★ THE SAME RULE ONE LAYER DOWN. A case-analysis derivation may not concede `∀x:I, P x` on the
/// strength of its case COUNT.
///
/// `DDestruct` is a declared `Derivation` constructor and `try_destruct` is a declared tactic
/// that builds one, so the object language can reach this whether or not the literate surface
/// can. The repro is six lines and was confirmed end to end: two cases proving `True`, an
/// unrelated motive, and `concludes` handing back `∀b:Bool, False`. The cases must be CHECKED
/// against the proposition each constructor owes, exactly as `DElim` checks its own — and a
/// derivation whose cases do not match must conclude the error sentinel, never the quantifier.
#[test]
fn a_case_analysis_derivation_cannot_concede_a_forall_its_cases_never_proved() {
    use logicaffeine_kernel::interface::Repl;

    for former in ["try_destruct", "DDestruct"] {
        let mut repl = Repl::new();
        repl.execute(&format!(
            "Definition fake_{former} : Derivation := (({former} (SName \"Bool\")) \
             ((SLam (SName \"Bool\")) (SName \"False\"))) \
             ((DCase (DAxiom (SName \"True\"))) ((DCase (DAxiom (SName \"True\"))) DCaseEnd))."
        ))
        .unwrap_or_else(|e| {
            panic!("`{former}` is a declared former, so this term is well-typed: {e:?}")
        });

        let concluded = repl
            .execute(&format!("Eval concludes fake_{former}."))
            .unwrap_or_else(|e| panic!("`concludes` must reduce: {e:?}"));

        assert!(
            !concluded.contains("Forall"),
            "`{former}` conceded a quantified proposition from two cases that prove `True` and \
             have nothing to do with the motive. The case COUNT is not the case CHECK: {concluded}"
        );
        assert!(
            concluded.contains("Error"),
            "…and an elimination it cannot verify must conclude the error sentinel so the \
             conclusion gate refuses it, rather than something the gate might match: {concluded}"
        );
    }

    // THE BOUNDARY, so this is not a ban on the former: a case analysis whose cases DO prove what
    // their constructors owe still concludes its quantifier.
    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    repl.execute(
        "## Theorem: HueReflByCases\n    Statement: forall h : Hue, h equals h.\n    \
         Proof: induction h.\n    - refl.\n    - refl.\n    - refl.",
    )
    .expect("an elimination whose bullets close their own cases must still certify");
}

// =============================================================================
// THE RECURSIVE HALF OF LITERATE INDUCTION
// =============================================================================

/// A bullet is handed the goal its constructor OWES, with the constructor's fields standing as
/// eigenvariables — and for a recursive field, the induction hypothesis in scope.
///
/// Lean writes `| succ k ih => …` and hands the branch the OPEN goal `P (Nat.succ k)` with
/// `k` fixed-but-arbitrary and `ih : P k` available. LOGOS handed the bullet the CLOSED
/// `∀k:Nat. P k → P (Succ k)` instead, and no entry in the bullet vocabulary dispatches on a
/// `Forall`-headed goal — so the step case of every induction over every recursive inductive was
/// unreachable, which is `lean_conformance_t12_vernacular::literate_induction_over_nat_closes_its_two_
/// bullet_cases` and the standing "literate induction can close NO goal of ANY type" claim.
///
/// Three shapes, so a fix that special-cases `Nat` does not pass: a constructor with a
/// NON-recursive field, one with a RECURSIVE field, and `Nat` itself. Each goal is reflexivity,
/// which `refl` closes at any instantiation and which therefore tests the goal's SHAPE and
/// nothing else — and each is paired below with the falsehood that must still be refused.
#[test]
fn literate_induction_closes_a_case_whose_constructor_carries_a_field() {
    use logicaffeine_kernel::interface::Repl;

    let shapes: &[(&str, &[&str], &str, &str, usize)] = &[
        // (theorem, setup, statement, variable, bullets)
        (
            "TagRefl",
            &["A Tag is either Zed or a Val with n: Nat."],
            "forall t : Tag, t equals t",
            "t",
            2,
        ),
        (
            "ChainRefl",
            &["A Chain is either Stop or a Link with rest: Chain."],
            "forall c : Chain, c equals c",
            "c",
            2,
        ),
        ("NatRefl", &[], "forall n : Nat, n equals n", "n", 2),
    ];

    for (name, setup, statement, v, bullets) in shapes {
        let mut repl = Repl::new();
        for s in *setup {
            repl.execute(s).expect("the setup must register");
        }
        let script = std::iter::repeat("    - refl.".to_string())
            .take(*bullets)
            .collect::<Vec<_>>()
            .join("\n");
        repl.execute(&format!(
            "## Theorem: {name}\n    Statement: {statement}.\n    Proof: induction {v}.\n{script}"
        ))
        .unwrap_or_else(|e| {
            panic!(
                "`{statement}` is reflexivity at every constructor — a bullet handed the goal its \
                 constructor owes closes it with `refl`. This is the step case of every induction \
                 over a recursive inductive: {e:?}"
            )
        });

        // The stored derivation re-checks, and the SECOND kernel agrees about it — the case goal
        // is computed inside `reduction.rs`, which BOTH kernels reach through `concludes`, so a
        // change to its shape is exactly the kind that a one-kernel check cannot see.
        let ctx = repl.context();
        let body = ctx
            .get_definition_body(name)
            .cloned()
            .unwrap_or_else(|| panic!("`{name}` certified, so a derivation is stored"));
        infer_type(ctx, &body)
            .unwrap_or_else(|e| panic!("`{name}`'s stored derivation must re-check: {e:?}"));
        assert_eq!(
            double_check(ctx, &body),
            DoubleCheck::Agreed,
            "both kernels must agree about `{name}`"
        );
    }
}

// =============================================================================
// SEVERAL COLUMNS AT ONCE — Lean's equation surface, not only its compiler
// =============================================================================

/// `Consider a and b:` matches a TUPLE of scrutinees, which is how an overlap is actually
/// written.
///
/// The matrix compiler has always been multi-column (`a_two_column_matrix_splits_into_the_
/// disjoint_tree` drives it directly), and the SURFACE could only ever name one scrutinee — so
/// `def pick : Nat → Nat → Nat | 0, _ => 1 | _, 0 => 2 | _, _ => 3`, the shape T04-009 is about,
/// could be compiled but not written. This is the surface that can say it. The separator is
/// `and`, the same word that already separates the parameters these columns usually are, and
/// nothing in the term grammar reads it, so a one-column `Consider` is unchanged.
#[test]
fn a_multi_column_consider_splits_the_overlap_the_way_lean_does() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When Zero and _: Yield (Succ Zero). \
         When _ and Zero: Yield (Succ (Succ Zero)). \
         When _ and _: Yield (Succ (Succ (Succ Zero))).",
    )
    .expect("a two-column equation set must compile");

    // First match wins, per ROW, across both columns — which is the only reading under which
    // `pick 0 0` is 1 rather than 2.
    for (a, b, want) in [(0usize, 0usize, 1usize), (0, 5, 1), (5, 0, 2), (5, 5, 3)] {
        let got = normalize(&ctx, &apps(g("pick"), &[peano(a), peano(b)]));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "pick {a} {b} must be {want}, got {got}"
        );
    }
}

/// The columns are placed by the constructor each pattern NAMES, in every column.
///
/// `Bool` is registered `true` then `false`. A two-column clause set written with the `false`
/// rows first must compute conjunction, not its De Morgan twin — for an equal-arity inductive
/// the positional case vector is where source order silently becomes a different function, and
/// a second column is a second place for that to happen.
#[test]
fn every_column_places_its_clause_by_the_constructor_it_names() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To both (x: Bool) and (y: Bool) -> Bool: Consider x and y: \
         When false and _: Yield false. \
         When _ and false: Yield false. \
         When _ and _: Yield true.",
    )
    .expect("a scrambled two-column clause set must compile");

    for (x, y, want) in [
        ("true", "true", "true"),
        ("true", "false", "false"),
        ("false", "true", "false"),
        ("false", "false", "false"),
    ] {
        assert_eq!(
            normalize(&ctx, &apps(g("both"), &[g(x), g(y)])),
            g(want),
            "both {x} {y} must be {want} — source order would answer the other way"
        );
    }
}

/// A binder bound in ANY column is in scope for the body, with the type that column's
/// constructor gives it.
#[test]
fn a_binder_from_any_column_is_in_scope_with_its_real_type() {
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To second (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When Zero and _: Yield Zero. \
         When Succ k and Zero: Yield k. \
         When Succ k and Succ m: Yield m.",
    )
    .expect("binders from both columns must be usable in the body");

    for (a, b, want) in [(0usize, 7usize, 0usize), (4, 0, 3), (4, 7, 6)] {
        let got = normalize(&ctx, &apps(g("second"), &[peano(a), peano(b)]));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "second {a} {b} must be {want}, got {got}"
        );
    }

    // A nested pattern in the SECOND column nests exactly as it does in the first.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To deep (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When Zero and Succ (Succ k): Yield k. \
         When Zero and _: Yield Zero. \
         When Succ _ and _: Yield Zero.",
    )
    .expect("a nested pattern in a later column must compile");
    for (a, b, want) in [(0usize, 5usize, 3usize), (0, 1, 0), (3, 9, 0)] {
        let got = normalize(&ctx, &apps(g("deep"), &[peano(a), peano(b)]));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "deep {a} {b} must be {want}, got {got}"
        );
    }
}

/// THE NEGATIVES. A column count that disagrees with the `Consider` line is refused and says so;
/// a name bound twice ACROSS columns is refused exactly as it is within one; and a
/// non-exhaustive matrix is still non-exhaustive.
#[test]
fn a_multi_column_clause_set_is_refused_when_it_does_not_line_up() {
    let mut ctx = std_ctx();

    // (a) Too few patterns for the columns named.
    let short = define(
        &mut ctx,
        "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When Zero: Yield Zero. When Succ k: Yield k.",
    );
    let msg = format!("{short:?}");
    assert!(
        short.is_err(),
        "one pattern cannot cover two columns: {msg}"
    );
    for needle in ["2", "1", "`a`", "`b`"] {
        assert!(
            msg.contains(needle),
            "the refusal must name both counts and the columns: {needle} missing from {msg}"
        );
    }

    // (b) Too many.
    assert!(
        define(
            &mut ctx,
            "## To one (a: Nat) -> Nat: Consider a: \
             When Zero and _: Yield Zero. When Succ k and _: Yield k.",
        )
        .is_err(),
        "two patterns cannot cover one column"
    );

    // (c) A name bound twice across the columns reads ONE field for both, so it is refused —
    // `When Succ k and Succ k:` is not a way to say "the two are equal".
    let twice = define(
        &mut ctx,
        "## To same (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When Succ k and Succ k: Yield k. \
         When _ and _: Yield Zero.",
    );
    assert!(
        twice.is_err(),
        "a repeated binder across columns must be refused: {twice:?}"
    );
    assert!(
        format!("{twice:?}").contains('k'),
        "…and the refusal must name the binder: {twice:?}"
    );

    // (d) Totality is not relaxed by a second column.
    assert!(
        define(
            &mut ctx,
            "## To partial (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
             When Zero and Zero: Yield Zero.",
        )
        .is_err(),
        "a matrix that reaches only one of four constructor pairs is not total"
    );

    // Nothing was registered by any of the refusals.
    for name in ["pick", "one", "same", "partial"] {
        assert!(
            ctx.get_global(name).is_none(),
            "`{name}` was refused and registered anyway"
        );
    }
}

/// The TEXT-ONLY door refuses a multi-column `Consider` by name, and says where the compiler is.
///
/// A second column is a matrix, and splitting a matrix needs the constructor list — which is the
/// one thing a parser holding only a `&str` does not have. Guessing would silently compute a
/// different function, so it refuses, exactly as it already refuses a nested pattern and a
/// catch-all.
#[test]
fn the_text_only_door_refuses_a_multi_column_consider() {
    use logicaffeine_kernel::interface::literate_parser::parse_definition;

    let src = "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
               When Zero and _: Yield Zero. When _ and _: Yield (Succ Zero).";
    let refused = parse_definition(src);
    let msg = format!("{refused:?}");
    assert!(
        refused.is_err(),
        "the text door cannot split a matrix: {msg}"
    );
    assert!(
        msg.contains("parse_command_in_ctx") || msg.contains("parse_definition_in"),
        "…and it must say which door can: {msg}"
    );

    // …while a ONE-column `Consider` still reads on the text door exactly as it always did.
    parse_definition(
        "## To pred (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. When Succ k: Yield k.",
    )
    .expect("one column is the reading the text door has always had");
}

// =============================================================================
// THE SEAMS THIS SURFACE MEETS THE TACTIC LAYER AT
// =============================================================================

/// The `Repl` reads its commands through the door that HAS a context.
///
/// Three call sites, and every capability in this file depends on all three: `execute` (one
/// command), `read_document` (a block of one) and `split_blocks` (which finds a block's EXTENT by
/// parsing it, so a document holding a nested pattern cannot even be SPLIT by the text-only
/// parser). Wired, T04-008/009/010 are a surface a user can type; unwired, the equation compiler
/// is reachable only from a test. This locks the wiring so it cannot be quietly undone: each
/// assertion below is a shape the TEXT-ONLY door refuses outright.
#[test]
fn the_repl_reads_its_commands_through_the_door_that_has_a_context() {
    use logicaffeine_kernel::interface::Repl;

    // (1) `execute` — a nested pattern, a catch-all and a scrambled clause order, each of which
    // the text-only door cannot express at all.
    let mut repl = Repl::new();
    repl.execute(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("`Repl::execute` must compile a nested pattern");
    repl.execute(
        "## To flip (b: Bool) -> Bool: Consider b: When false: Yield true. When true: Yield false.",
    )
    .expect("`Repl::execute` must place a clause by the constructor it names");
    assert_eq!(
        repl.execute("Eval flip true.").ok().as_deref(),
        repl.execute("Eval false.").ok().as_deref(),
        "`flip true` is `false` — source order would answer `true`"
    );
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    repl.execute(
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    )
    .expect("`Repl::execute` must fan out a catch-all");

    // (2) `execute_document` — the same, as a block among others, which exercises `split_blocks`.
    let mut doc = Repl::new();
    doc.execute_document(
        "## To pred (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ (Succ k): Yield (Succ k). When Succ Zero: Yield Zero.\n\
         Check pred.\n",
    )
    .expect("a document holding a nested pattern must SPLIT and then check");
}

/// Both spellings of implication on this surface prove the same things.
///
/// `(implies A B)` reifies as `SApp (SApp (SName "implies") A) B`; the infix `A implies B`
/// desugars to a non-dependent `Pi`, which reifies as `SPi A B`. They are the same proposition
/// and were two dialects: the tactic layer read only the applied form, so `Proof: auto.` closed
/// one and refused the other, and `intro.` could introduce only one of them. A surface with two
/// spellings of one connective and one of them unreachable is a trap, not a feature.
#[test]
fn both_spellings_of_implication_prove_the_same_things() {
    use logicaffeine_kernel::interface::Repl;

    for (i, statement) in ["(implies True True)", "True implies True"]
        .iter()
        .enumerate()
    {
        for (j, proof) in ["auto.", "intro.\n    assumption."].iter().enumerate() {
            let mut repl = Repl::new();
            let name = format!("Impl_{i}_{j}");
            repl.execute(&format!(
                "## Theorem: {name}\n    Statement: {statement}.\n    Proof: {proof}"
            ))
            .unwrap_or_else(|e| {
                panic!(
                    "`{statement}` by `{proof}` must certify — the two spellings are one \
                        proposition, so a tactic that closes one must close the other: {e:?}"
                )
            });
        }
    }

    // THE NEGATIVE, in both spellings: a FALSE implication is refused, so this is a rule about
    // reading one connective two ways and not about accepting more.
    for statement in ["(implies True False)", "True implies False"] {
        for proof in ["auto.", "intro.\n    assumption."] {
            let mut repl = Repl::new();
            let r = repl.execute(&format!(
                "## Theorem: Bad\n    Statement: {statement}.\n    Proof: {proof}"
            ));
            assert!(
                r.is_err(),
                "`{statement}` is false — `{proof}` must not certify it: {r:?}"
            );
        }
    }
}

// =============================================================================
// FRESHNESS IS CHECKED, NEVER ASSUMED
// =============================================================================

/// ★ A NAME THE COMPILER THOUGHT IT OWNED. A user binder spelled like a compiler-introduced
/// field binder must not capture one.
///
/// The compiler names each field it introduces `#fld0`, `#fld1`, … and the variable and
/// catch-all rules bind a clause's pattern variable by SUBSTITUTING it — column by column, left
/// to right. That is capture-free only if no name the author can write ever equals a name the
/// compiler generates, and the reason given was that `#` cannot occur in an identifier. It can:
/// `«#fld0»` is a `«quoted name»`, which `scan_name` accepts everywhere an identifier is read,
/// so a clause binder may be spelled exactly like the compiler's second field.
///
/// The damage is a SILENT WRONG ANSWER. `When Pair a «#fld0»: Yield a.` substitutes `a := #fld0`
/// for column 0, and then column 1's substitution `#fld0 := #fld1` rewrites the `#fld0` that
/// substitution had just introduced — so the case returns the SECOND field where the author
/// wrote the FIRST. Both fields have the same type, so the term type-checks, both kernels
/// certify it, and the function is simply not the one that was written.
#[test]
fn a_user_binder_spelled_like_a_compiler_binder_does_not_capture() {
    let mut ctx = std_ctx();
    ctx.add_inductive("Pair2", sort(0));
    ctx.add_constructor(
        "Pair",
        "Pair2",
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body_type: Box::new(g("Pair2")),
            }),
        },
    );

    // `fst` returns the FIRST field. The second binder is spelled as the compiler's own.
    define(
        &mut ctx,
        "## To fst (p: Pair2) -> Nat: Consider p: When Pair a \u{ab}#fld0\u{bb}: Yield a.",
    )
    .expect("a quoted binder is a binder like any other");

    let got = normalize(&ctx, &app(g("fst"), apps(g("Pair"), &[peano(1), peano(2)])));
    assert!(
        defeq_for_test(&ctx, &got, &peano(1)),
        "`fst (Pair 1 2)` must be 1. A compiler binder captured by a name the author wrote is a \
         well-typed term denoting a DIFFERENT function — both kernels certify it and nothing \
         reports it. Got {got}"
    );

    // …and the mirror image, so this is freshness and not an accident of which column wins:
    // `snd` returns the SECOND field with the FIRST binder spelled as the compiler's.
    define(
        &mut ctx,
        "## To snd (p: Pair2) -> Nat: Consider p: When Pair \u{ab}#fld0\u{bb} b: Yield b.",
    )
    .expect("a quoted binder is a binder like any other");
    let got = normalize(&ctx, &app(g("snd"), apps(g("Pair"), &[peano(1), peano(2)])));
    assert!(
        defeq_for_test(&ctx, &got, &peano(2)),
        "`snd (Pair 1 2)` must be 2, got {got}"
    );

    // …and the binder the author spelled still denotes what they said it did, when they use it.
    define(
        &mut ctx,
        "## To odd_one (p: Pair2) -> Nat: Consider p: When Pair \u{ab}#fld0\u{bb} b: \
         Yield \u{ab}#fld0\u{bb}.",
    )
    .expect("a quoted binder is usable in the body");
    let got = normalize(
        &ctx,
        &app(g("odd_one"), apps(g("Pair"), &[peano(1), peano(2)])),
    );
    assert!(
        defeq_for_test(&ctx, &got, &peano(1)),
        "`odd_one (Pair 1 2)` must be 1 — the binder the author NAMED `#fld0` is the first \
         field, whatever the compiler calls its own. Got {got}"
    );
}

/// ★ THE SAME ASSUMPTION, IN THE OTHER FILE. An eigenvariable must be fresh against the
/// statement it is introduced into.
///
/// A peeled `forall` binder is reified as a CONSTANT standing for a fixed-but-arbitrary element
/// — `n` becomes `n#eig` — and ∀-introduction is sound only if that constant is fresh. The
/// freshness argument was the same one the equation compiler made, and it fails the same way:
/// `«n#eig»` is a name a term can spell, so a statement can NAME the eigenvariable the surface is
/// about to introduce. Both then reify to `SName "n#eig"` and become one atom.
///
/// `∀n:Nat, n equals «n#eig»` is FALSE for every `«n#eig»` that is not the identity of `Nat`;
/// under the collision it reifies as `x = x` and `refl` closes it, so the surface certifies a
/// theorem whose statement says something else entirely. That is exactly the hazard
/// `EIGENVARIABLE_MARK`'s own doc comment exists to forbid, arriving through the quoting escape.
#[test]
fn an_eigenvariable_cannot_be_named_by_the_statement_it_is_introduced_into() {
    use logicaffeine_kernel::interface::Repl;

    for tactic in [
        "refl", "simp", "auto", "cc", "ring", "lia", "omega", "decide",
    ] {
        let mut repl = Repl::new();
        repl.execute("Definition \u{ab}n#eig\u{bb} : Nat := Zero.")
            .expect("a quoted name is a name");
        let r = repl.execute(&format!(
            "## Theorem: Collide_{tactic}\n    \
             Statement: forall n : Nat, n equals \u{ab}n#eig\u{bb}.\n    Proof: {tactic}."
        ));
        assert!(
            r.is_err(),
            "`\u{2200}n:Nat, n = \u{ab}n#eig\u{bb}` says every natural number is `Zero`, and it is false. \
             `{tactic}` may not certify it — an eigenvariable the statement can NAME is not an \
             eigenvariable: {r:?}"
        );
    }

    // THE BOUNDARY. The surface's reading of a peeled binder is unchanged for every statement
    // that does not name the mark: `∀n:Nat, n equals n` still certifies, and the FALSE twin
    // over a genuinely different constant is still refused.
    let mut repl = Repl::new();
    repl.execute(
        "## Theorem: SelfEq\n    Statement: forall n : Nat, n equals n.\n    Proof: refl.",
    )
    .expect("∀-introduction at a fresh eigenvariable is the whole point of the reading");
    let r = repl.execute(
        "## Theorem: AllZero\n    Statement: forall n : Nat, n equals Zero.\n    Proof: refl.",
    );
    assert!(
        r.is_err(),
        "`∀n:Nat, n = Zero` is false and must stay refused: {r:?}"
    );
}

/// The absurd version of the test above: no SPELLING of a clause's binders changes what the
/// clause set denotes.
///
/// A three-field constructor, and a selector for each field written five ways — ordinary names,
/// every compiler name in order, every compiler name reversed, and two interleavings. The
/// compiler's own names are `#fld0`, `#fld1`, … in generation order, so the reversed and
/// interleaved rows are the ones where a sequential substitution rewrites its own output. All
/// fifteen definitions must select the field their author named, and both kernels must agree.
#[test]
fn no_spelling_of_a_compiler_binder_changes_what_a_clause_set_denotes() {
    let nat3 = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body_type: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body_type: Box::new(g("Trip")),
            }),
        }),
    };

    let spellings: &[[&str; 3]] = &[
        ["x", "y", "z"],
        ["#fld0", "#fld1", "#fld2"],
        ["#fld2", "#fld1", "#fld0"],
        ["#fld1", "x", "#fld0"],
        ["a", "#fld0", "b"],
    ];

    for (s, names) in spellings.iter().enumerate() {
        for field in 0..3usize {
            let mut ctx = std_ctx();
            ctx.add_inductive("Trip", sort(0));
            ctx.add_constructor("Triple", "Trip", nat3.clone());
            let quoted: Vec<String> = names.iter().map(|n| format!("\u{ab}{n}\u{bb}")).collect();
            let src = format!(
                "## To sel (t: Trip) -> Nat: Consider t: When Triple {} {} {}: Yield {}.",
                quoted[0], quoted[1], quoted[2], quoted[field]
            );
            define(&mut ctx, &src).unwrap_or_else(|e| panic!("`{src}` must compile: {e}"));
            let got = normalize(
                &ctx,
                &app(g("sel"), apps(g("Triple"), &[peano(1), peano(2), peano(3)])),
            );
            assert!(
                defeq_for_test(&ctx, &got, &peano(field + 1)),
                "spelling {s} selecting field {field}: `sel (Triple 1 2 3)` must be {}, got {got}. \
                 A generated binder that collides with a name the author wrote makes the \
                 substitutions rewrite one another, and the result is a well-typed term denoting \
                 a different function",
                field + 1
            );
        }
    }
}

/// The absurd version for eigenvariables: however many of the candidate names a context already
/// holds, the constant the surface introduces is not one of them.
///
/// `n#eig` is stepped to `n#eig1`, `n#eig2`, … until it is fresh, so a context that defines the
/// first three candidates must not make the fourth statement provable either. Each statement is
/// `∀n:Nat, n equals «candidate»` — false unless every natural number is that constant.
#[test]
fn an_eigenvariable_is_stepped_past_every_name_the_context_already_holds() {
    use logicaffeine_kernel::interface::Repl;

    let candidates = ["n#eig", "n#eig1", "n#eig2", "n#eig3"];
    for held in 1..=candidates.len() {
        let mut repl = Repl::new();
        for c in &candidates[..held] {
            repl.execute(&format!("Definition \u{ab}{c}\u{bb} : Nat := Zero."))
                .unwrap_or_else(|e| panic!("`{c}` is a name: {e:?}"));
        }
        for c in &candidates[..held] {
            let r = repl.execute(&format!(
                "## Theorem: Step_{held}_{}\n    \
                 Statement: forall n : Nat, n equals \u{ab}{c}\u{bb}.\n    Proof: refl.",
                c.replace('#', "_")
            ));
            assert!(
                r.is_err(),
                "with {held} candidate(s) already held, `\u{2200}n:Nat, n = \u{ab}{c}\u{bb}` is still the \
                 false claim that every natural number is `Zero`: {r:?}"
            );
        }
        // …and a statement that names none of them still gets its eigenvariable and certifies.
        repl.execute(&format!(
            "## Theorem: Fine_{held}\n    Statement: forall n : Nat, n equals n.\n    Proof: refl."
        ))
        .expect("∀-introduction at a fresh constant is unaffected by how many are taken");
    }
}

// =============================================================================
// THE ELIMINATION INTRODUCES THE CONSTRUCTOR'S BINDERS — LEAN'S `introN`, COUNTED
// =============================================================================

/// The number of binders a bullet is handed, against TWO independent models.
///
/// ★ WHY THIS IS A DIFFERENTIAL AND NOT AN ASSERTION. The count of things an elimination must
/// introduce before a bullet's tactic runs is decided in one place — `reduction.rs`'s
/// `build_case_expected`, which is what `induction_step_goal` reduces through and what `DElim`
/// re-derives when it checks the case. The parser must agree with it EXACTLY: one too few and
/// the bullet is handed a quantified goal no tactic dispatches on; one too many and `intro`
/// fails on a goal with nothing left to introduce. A parser that walked the constructor
/// telescope a SECOND time to get the number would be a second authority on the same fact, and
/// the two would drift silently — the case formula changed shape twice in this campaign already.
///
/// So the parser reads the count off the kernel's own reduced case goal, and this test checks
/// that number against two models it does not share:
///
/// * the CONSTRUCTOR TELESCOPE, walked here from `ctx.get_constructors` — one binder per value
///   field, plus one more per RECURSIVE field for its induction hypothesis;
/// * the REFLECTED GOAL's own `Forall`/`Implies`/`SPi` depth, peeled here by a walk written
///   independently of the parser's.
///
/// Lean counts it the same way and from the same side: `Lean/Meta/Tactic/Induction.lean:97-106`
/// takes `nparams := getTargetArity d - initialArity` off the MINOR PREMISE'S TYPE and calls
/// `introN nparams`, so `| succ k ih => rfl` never sees the quantifier. The grid deliberately
/// contains a constructor with two recursive fields (two hypotheses, not one), a mixed
/// constructor, and a statement whose own body is an implication — the case the subtraction
/// exists for.
#[test]
fn a_bullet_is_handed_as_many_binders_as_its_constructor_owes() {
    // ---- an independent peeler: the same three shapes `intro` accepts, written again ----
    fn sname_is(t: &Term, want: &str) -> bool {
        match t {
            Term::App(f, x) => {
                matches!(f.as_ref(), Term::Global(n) if n == "SName")
                    && matches!(x.as_ref(), Term::Lit(Literal::Text(s)) if s == want)
            }
            _ => false,
        }
    }
    fn sapp2(t: &Term, head: &str) -> Option<(Term, Term)> {
        let Term::App(f, b) = t else { return None };
        let Term::App(s1, inner) = f.as_ref() else {
            return None;
        };
        if !matches!(s1.as_ref(), Term::Global(n) if n == "SApp") {
            return None;
        }
        let Term::App(g2, a) = inner.as_ref() else {
            return None;
        };
        let Term::App(s2, head_t) = g2.as_ref() else {
            return None;
        };
        if !matches!(s2.as_ref(), Term::Global(n) if n == "SApp") || !sname_is(head_t, head) {
            return None;
        }
        Some(((**a).clone(), (**b).clone()))
    }
    fn slam_body(t: &Term) -> Option<Term> {
        let Term::App(f, body) = t else { return None };
        let Term::App(sl, _) = f.as_ref() else {
            return None;
        };
        matches!(sl.as_ref(), Term::Global(n) if n == "SLam").then(|| (**body).clone())
    }
    /// Lean's `getTargetArity`: how many arrow-like layers a reflected proposition carries.
    /// `∀` and `→` are one node in Lean and three in the reflected language, so all three count.
    fn depth(t: &Term) -> usize {
        let mut d = 0;
        let mut cur = t.clone();
        loop {
            let next = if let Some((_, m)) = sapp2(&cur, "Forall") {
                match slam_body(&m) {
                    Some(b) => b,
                    None => break,
                }
            } else if let Some((_, q)) = sapp2(&cur, "Implies") {
                q
            } else if let Some((_, q)) = sapp2(&cur, "implies") {
                q
            } else if let Term::App(f, b) = &cur {
                match f.as_ref() {
                    Term::App(spi, _) if matches!(spi.as_ref(), Term::Global(n) if n == "SPi") => {
                        (**b).clone()
                    }
                    _ => break,
                }
            } else {
                break;
            };
            d += 1;
            cur = next;
        }
        d
    }

    // ---- reading the emitted derivation back: the bullets, and each one's leading `intro`s ----
    fn script_names(list: &Term) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut cur = list.clone();
        loop {
            if sname_is(&cur, "end") {
                return Some(out);
            }
            let (n, rest) = sapp2(&cur, "step")?;
            let Term::App(_, txt) = &n else { return None };
            let Term::Lit(Literal::Text(s)) = txt.as_ref() else {
                return None;
            };
            out.push(s.clone());
            cur = rest;
        }
    }
    /// `(leading intro count, the goal the bullet was handed)`.
    fn bullet_shape(proof: &Term) -> (usize, Term) {
        if let Term::App(outer, goal) = proof {
            if let Term::App(f, list) = outer.as_ref() {
                if matches!(f.as_ref(), Term::Global(n) if n == "try_script") {
                    let names = script_names(list).expect("a script list must decode");
                    let leading = names.iter().take_while(|n| *n == "intro").count();
                    return (leading, (**goal).clone());
                }
            }
            return (0, (**goal).clone());
        }
        panic!("a bullet proof is an application, got {proof}");
    }
    fn cases_of(proof: &Term) -> Vec<Term> {
        // try_induction ind motive chain
        let Term::App(f, chain) = proof else {
            panic!("not an application: {proof}")
        };
        let Term::App(f2, _motive) = f.as_ref() else {
            panic!("not try_induction: {proof}")
        };
        let Term::App(head, _ind) = f2.as_ref() else {
            panic!("not try_induction: {proof}")
        };
        assert!(
            matches!(head.as_ref(), Term::Global(n) if n == "try_induction"),
            "an induction proof is a `try_induction`: {proof}"
        );
        let mut out = Vec::new();
        let mut cur = (**chain).clone();
        loop {
            if matches!(&cur, Term::Global(n) if n == "DCaseEnd") {
                return out;
            }
            let Term::App(inner, tail) = &cur else {
                panic!("bad case chain: {cur}")
            };
            let Term::App(dcase, head) = inner.as_ref() else {
                panic!("bad case chain: {cur}")
            };
            assert!(matches!(dcase.as_ref(), Term::Global(n) if n == "DCase"));
            out.push((**head).clone());
            cur = (**tail).clone();
        }
    }

    // ---- the grid ----
    let grid: &[(&[&str], &str, &str, &str)] = &[
        // (setup, inductive, statement, induction variable)
        (
            &["A Hue is either Ruby or Jade or Slate."],
            "Hue",
            "forall h : Hue, h equals h",
            "h",
        ),
        (&[], "Bool", "forall b : Bool, b equals b", "b"),
        (&[], "Nat", "forall n : Nat, n equals n", "n"),
        (
            &["A Tag is either Zed or a Val with n: Nat."],
            "Tag",
            "forall t : Tag, t equals t",
            "t",
        ),
        (
            &["A Chain is either Stop or a Link with rest: Chain."],
            "Chain",
            "forall c : Chain, c equals c",
            "c",
        ),
        (
            &["A Duo is either a Both with a: Nat and b: Nat."],
            "Duo",
            "forall d : Duo, d equals d",
            "d",
        ),
        (
            &["A Tree is either Leaf or a Node with l: Tree and r: Tree."],
            "Tree",
            "forall t : Tree, t equals t",
            "t",
        ),
        (
            &["A Roll is either Nil2 or a Cons2 with h: Nat and t: Roll."],
            "Roll",
            "forall r : Roll, r equals r",
            "r",
        ),
        // The statement's OWN body is an implication: the case goal is one layer deeper at every
        // constructor, and the count must not follow it. This is Lean's `initialArity`
        // subtraction (`Lean/Meta/Tactic/Induction.lean:98`), and without it every bullet over
        // an implication goal would be handed one `intro` too many.
        (
            &[],
            "Nat",
            "forall n : Nat, (n equals n) implies (n equals n)",
            "n",
        ),
        (
            &["A Chain is either Stop or a Link with rest: Chain."],
            "Chain",
            "forall c : Chain, (c equals c) implies (c equals c)",
            "c",
        ),
    ];

    let surface = Surface::new();
    let mut disagreements: Vec<String> = Vec::new();

    for (setup, ind, statement, v) in grid {
        let mut repl = logicaffeine_kernel::interface::Repl::new();
        for s in *setup {
            repl.execute(s)
                .unwrap_or_else(|e| panic!("setup `{s}` must register: {e:?}"));
        }
        let ctx = repl.context().clone();

        // MODEL ONE: the constructor telescope. One binder per value field, plus one induction
        // hypothesis per RECURSIVE value field.
        let expected: Vec<usize> = ctx
            .get_constructors(ind)
            .iter()
            .map(|(_, ty)| {
                let mut fields = 0usize;
                let mut recursive = 0usize;
                let mut cur = *ty;
                while let Term::Pi {
                    param_type,
                    body_type,
                    ..
                } = cur
                {
                    fields += 1;
                    if format!("{param_type}")
                        .split(|c: char| !c.is_alphanumeric())
                        .any(|w| w == *ind)
                    {
                        recursive += 1;
                    }
                    cur = body_type;
                }
                fields + recursive
            })
            .collect();

        let bullets = (0..expected.len())
            .map(|_| "    - refl.".to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let src = format!(
            "## Theorem: Shape\n    Statement: {statement}.\n    Proof: induction {v}.\n{bullets}"
        );
        let cmd = parse_command_in_ctx(&ctx, &surface, &src)
            .unwrap_or_else(|e| panic!("`{src}` must parse: {e:?}"));
        let Command::Definition { body, .. } = cmd else {
            panic!("a theorem is a definition")
        };

        let cases = cases_of(&body);
        assert_eq!(
            cases.len(),
            expected.len(),
            "one bullet per constructor of `{ind}`"
        );
        for (i, case) in cases.iter().enumerate() {
            let (got, goal) = bullet_shape(case);
            if got != expected[i] {
                disagreements.push(format!(
                    "{ind} case {i}: the parser introduces {got} binder(s); the constructor \
                     telescope owes {}",
                    expected[i]
                ));
            }
            // MODEL TWO: the reflected goal the KERNEL derives for this case, peeled here.
            let reduced = normalize(&ctx, &goal);
            let motive_depth = {
                // `induction_step_goal ind motive i` — the motive is the second argument.
                let Term::App(f, _) = &goal else {
                    panic!("case goal is an application")
                };
                let Term::App(f2, motive) = f.as_ref() else {
                    panic!("case goal shape")
                };
                let _ = f2;
                depth(&normalize(
                    &ctx,
                    &slam_body(motive).expect("the motive is an SLam"),
                ))
            };
            let by_goal = depth(&reduced).saturating_sub(motive_depth);
            if by_goal != expected[i] {
                disagreements.push(format!(
                    "{ind} case {i}: the reduced case goal has {by_goal} introducible binder(s) \
                     over the motive, the telescope owes {} — `build_case_expected` and the \
                     constructor disagree: {reduced}",
                    expected[i]
                ));
            }
        }
    }

    assert!(
        disagreements.is_empty(),
        "the elimination must introduce exactly the binders the constructor owes — one too few \
         hands the bullet a quantifier no tactic dispatches on, one too many makes `intro` fail \
         on a goal with nothing left to introduce: {disagreements:#?}"
    );
}

// =============================================================================
// T04-017 — A DEFINITION'S EQUATIONS, AS CITABLE NAMES
// =============================================================================

/// `dbl`, exactly as `lean_conformance_t04_recursion` builds it: `fix dbl. λn. match n with …`.
fn dbl_definition() -> (Term, Term) {
    let ty = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body_type: Box::new(nat()),
    };
    let succ = |t: Term| app(g("Succ"), t);
    let body = Term::Fix {
        name: "dbl".to_string(),
        body: Box::new(lam(
            "n",
            nat(),
            match_(
                var("n"),
                lam("_", nat(), nat()),
                vec![
                    g("Zero"),
                    lam("k", nat(), succ(succ(app(var("dbl"), var("k"))))),
                ],
            ),
        )),
    };
    (ty, body)
}

/// Defining a function must also mint the NAMES its equations are cited by.
///
/// ★ WHAT WAS MISSING WAS NEVER THE CONTENT. `add_definition` is δ-transparent and `Fix`/`Match`
/// ι-compute, so `dbl 3 ≡ 6` and `dbl (Succ n) ≡ Succ (Succ (dbl n))` are true by conversion the
/// moment `dbl` is declared. What a proof cannot do is NAME one. `rw [f]`, `simp [f]` and
/// `unfold f` all cite a constant, and with no constant to cite every appeal to a definition
/// degrades to a full `normalize` whose 10000-step fuel returns a partially reduced term,
/// silently, when it runs out.
///
/// Lean's names are `f.eq_def` and `f.eq_1 … f.eq_n` (`Lean/Meta/Eqns.lean:59-68`:
/// `eqnThmSuffixBase := "eq"`, `eqn1ThmSuffix := "eq_1"`, `unfoldThmSuffix := "eq_def"`), and its
/// statements are the ones `#check @dbl.eq_def` prints on this machine:
///
/// ```text
/// dbl.eq_def : ∀ (x : Nat), dbl x = match x with | 0 => 0 | n.succ => (dbl n).succ.succ
/// dbl.eq_1   : dbl 0 = 0
/// dbl.eq_2   : ∀ (n : Nat), dbl n.succ = (dbl n).succ.succ
/// ```
///
/// LOGOS's context is flat, so the same suffixes are spelled `dbl_eq_def` and `dbl_eq_1`/
/// `dbl_eq_2`. Every one is proved by `refl` — the statement is true by conversion — and every
/// one goes through `Context::add_definition`, the CHECKED gateway, so a generator cannot insert
/// a proposition the kernel has not agreed with.
#[test]
fn a_definition_gets_the_equation_lemmas_lean_generates_and_names() {
    use logicaffeine_kernel::equations;

    let mut ctx = std_ctx();
    let (ty, body) = dbl_definition();
    ctx.add_definition("dbl".to_string(), ty, body);
    equations::generate(&mut ctx, "dbl");

    // THE NAME the corpus asks for, and the two the per-constructor rewrites need.
    for lemma in ["dbl_eq_def", "dbl_eq_1", "dbl_eq_2"] {
        infer_type(&ctx, &g(lemma)).unwrap_or_else(|e| {
            panic!(
                "`{lemma}` must be a citable constant — a Lean proof that says `rw [dbl]` has \
                    no LOGOS counterpart to name without it: {e:?}"
            )
        });
        let stored = ctx
            .get_definition_body(lemma)
            .cloned()
            .unwrap_or_else(|| panic!("`{lemma}` must be stored WITH its proof, not declared"));
        infer_type(&ctx, &stored)
            .unwrap_or_else(|e| panic!("`{lemma}`'s proof must re-check: {e:?}"));
        assert_eq!(
            double_check(&ctx, &stored),
            DoubleCheck::Agreed,
            "both kernels must agree about `{lemma}` — a generated lemma is a new inhabitant of \
             the trusted surface and gets the same scrutiny as a hand-written one"
        );
    }

    // …and nothing was pushed past the checker. An empty refusal ledger is the healthy state,
    // and a generator that fills it with its own rejected candidates would destroy that signal.
    assert!(
        ctx.refused_declarations().is_empty(),
        "generation must offer candidates to the checker, never to the refusal ledger: {:?}",
        ctx.refused_declarations()
    );

    // THE STATEMENTS ARE THE REAL EQUATIONS, not merely well-typed propositions. `dbl_eq_1` says
    // `dbl Zero = Zero` at `Nat`; `dbl_eq_2` says `∀k. dbl (Succ k) = Succ (Succ (dbl k))`.
    let eq1 = ctx
        .get_definition_type("dbl_eq_1")
        .cloned()
        .expect("eq_1 has a statement");
    let want1 = apps(g("Eq"), &[nat(), app(g("dbl"), g("Zero")), g("Zero")]);
    assert!(
        defeq_for_test(&ctx, &eq1, &want1),
        "`dbl_eq_1` must state `dbl Zero = Zero`, got {eq1}"
    );

    let eq2 = ctx
        .get_definition_type("dbl_eq_2")
        .cloned()
        .expect("eq_2 has a statement");
    let Term::Pi {
        param,
        param_type,
        body_type,
    } = eq2.clone()
    else {
        panic!("`dbl_eq_2` quantifies over the constructor's field, got {eq2}")
    };
    assert!(
        defeq_for_test(&ctx, &param_type, &nat()),
        "the field is a `Nat`"
    );
    let k = var(&param);
    let succ = |t: Term| app(g("Succ"), t);
    let want2 = apps(
        g("Eq"),
        &[
            nat(),
            app(g("dbl"), succ(k.clone())),
            succ(succ(app(g("dbl"), k))),
        ],
    );
    let open = ctx.extend(&param, nat());
    assert_eq!(
        format!("{body_type}"),
        format!("{want2}"),
        "`dbl_eq_2` must state `dbl (Succ k) = Succ (Succ (dbl k))` SYNTACTICALLY, not merely up          to conversion: a lemma is cited by `rw`, which matches its shape, so an un-β-reduced          `(λk. …) k` on the right is a different lemma to rewrite with"
    );
    assert!(
        defeq_for_test(&open, &body_type, &want2),
        "…and it is the same proposition"
    );

    // ★ THE ONE PLACE OUR REDUCTION IS WEAKER THAN LEAN'S, RECORDED RATHER THAN HIDDEN.
    // Lean's `dbl.eq_def` is the APPLIED form — `∀ x, dbl x = match x with …`, printed by
    // `#check @dbl.eq_def` on this machine — and Lean proves it with a real derivation
    // (`Lean/Elab/PreDefinition/Eqns.lean:312` `mkUnfoldEq`), not with `rfl`. It cannot be `rfl`
    // here: `reduction.rs`'s `try_fix_unroll` unrolls a fixpoint only when its STRUCTURAL
    // argument is already a constructor, so `dbl n` for a free `n` is a normal form and is not
    // convertible with the `match` it would step to. The generator offers the applied form
    // FIRST, the kernel declines it, and the unapplied `dbl = fix dbl. …` — true by one δ step —
    // is what lands. If a later wave teaches the guard to unroll under a binder, the applied
    // form starts checking and this assertion is the thing that says so.
    let unfold = ctx
        .get_definition_type("dbl_eq_def")
        .cloned()
        .expect("eq_def has a statement");
    assert!(
        !matches!(unfold, Term::Pi { .. }),
        "the APPLIED unfold theorem now checks — the fixpoint guard has changed, and `eq_def`          should be re-read against Lean's shape rather than left as the δ-step fallback: {unfold}"
    );
    let stored_body = ctx
        .get_definition_body("dbl")
        .cloned()
        .expect("dbl is a definition");
    let want_unfold = apps(
        g("Eq"),
        &[
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body_type: Box::new(nat()),
            },
            g("dbl"),
            stored_body,
        ],
    );
    assert_eq!(
        format!("{unfold}"),
        format!("{want_unfold}"),
        "the fallback unfold theorem is `dbl = <its stored body>`, one δ step and nothing else"
    );
}

/// The negative that makes the test above mean something: a FALSE equation must not check.
///
/// A generator is a machine that writes propositions, so the thing to prove about it is that the
/// kernel is still the one deciding which ones are true. Both halves are checked: the equation
/// stated at the WRONG constructor's case body is refused by `infer_type`, and the generator is
/// shown never to have minted it.
#[test]
fn a_false_equation_lemma_is_refused_and_the_generator_never_mints_one() {
    use logicaffeine_kernel::equations;

    let mut ctx = std_ctx();
    let (ty, body) = dbl_definition();
    ctx.add_definition("dbl".to_string(), ty, body);
    equations::generate(&mut ctx, "dbl");

    // `dbl Zero = Succ Zero` is FALSE — `dbl Zero` computes to `Zero`. `refl` cannot prove it,
    // and neither can anything else.
    let succ = |t: Term| app(g("Succ"), t);
    let false_eq = apps(g("Eq"), &[nat(), app(g("dbl"), g("Zero")), succ(g("Zero"))]);
    let sham = apps(g("refl"), &[nat(), app(g("dbl"), g("Zero"))]);
    let inferred = infer_type(&ctx, &sham).expect("`refl Nat (dbl Zero)` is well-typed");
    assert!(
        !is_subtype(&ctx, &inferred, &false_eq),
        "`refl Nat (dbl Zero)` proves `dbl Zero = Zero`, and `Zero` is not `Succ Zero` — if the \
         kernel accepted this the generator could mint any equation at all"
    );

    // The equation at the OTHER constructor's body: `dbl (Succ k) = Zero`, the `Zero` case's
    // answer read at the `Succ` slot. This is the exact mistake an off-by-one in the generator
    // makes, and it must be unprovable.
    let open = ctx.extend("k", nat());
    let crossed = apps(g("Eq"), &[nat(), app(g("dbl"), succ(var("k"))), g("Zero")]);
    let sham2 = apps(g("refl"), &[nat(), app(g("dbl"), succ(var("k")))]);
    let inferred2 = infer_type(&open, &sham2).expect("`refl Nat (dbl (Succ k))` is well-typed");
    assert!(
        !is_subtype(&open, &inferred2, &crossed),
        "`dbl (Succ k)` is `Succ (Succ (dbl k))`, never `Zero` — a case vector read at the wrong \
         index would state exactly this"
    );

    // …and the generator did not mint either of them: every lemma it DID mint re-checks.
    for (name, _ty, stored) in ctx.iter_definitions() {
        if !equations::is_equation_name(name) {
            continue;
        }
        infer_type(&ctx, stored).unwrap_or_else(|e| {
            panic!("the generator minted `{name}`, which does not check: {e:?}")
        });
    }
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
}

/// Generation must not eat itself, must not displace a name, and must be silent before `Eq`
/// exists.
///
/// The three hazards of generating declarations FROM declarations, each pinned:
///
/// * **Re-entrancy.** A lemma is a definition, so generating its lemmas would generate its own,
///   for ever. Lean's guard is the reserved-suffix test (`Lean/Meta/Eqns.lean:63-66`); ours is
///   the same test, and it is checked first.
/// * **Displacement.** `Context::declare` DISPLACES the name it registers. A user who has
///   already proved `dbl_eq_1` themselves must keep their proof.
/// * **Bootstrap.** Hundreds of prelude definitions are registered before `register_eq`, and
///   there is no equality to state an equation in until it runs.
#[test]
fn equation_generation_is_not_re_entrant_and_never_displaces_a_name() {
    use logicaffeine_kernel::equations;

    // RE-ENTRANCY: the suffix test names exactly Lean's reserved shapes.
    for reserved in ["dbl_eq_def", "dbl_eq_1", "dbl_eq_27", "f_g_eq_2"] {
        assert!(
            equations::is_equation_name(reserved),
            "`{reserved}` is a generated name"
        );
    }
    for ordinary in [
        "dbl",
        "eq_def",
        "dbl_eq_",
        "dbl_eq_x",
        "dbl_equals",
        "_eq_1x",
    ] {
        assert!(
            !equations::is_equation_name(ordinary),
            "`{ordinary}` is a name a user writes, not one the generator mints"
        );
    }
    let mut ctx = std_ctx();
    let (ty, body) = dbl_definition();
    ctx.add_definition("dbl".to_string(), ty, body);
    equations::generate(&mut ctx, "dbl");
    let before: Vec<String> = ctx
        .iter_definitions()
        .map(|(n, _, _)| n.to_string())
        .filter(|n| equations::is_equation_name(n))
        .collect();
    assert!(
        !before.is_empty(),
        "the setup must have generated something to re-enter on"
    );
    for n in &before {
        equations::generate(&mut ctx, n);
    }
    let after: Vec<String> = ctx
        .iter_definitions()
        .map(|(n, _, _)| n.to_string())
        .filter(|n| equations::is_equation_name(n))
        .collect();
    assert_eq!(
        {
            let mut b = before.clone();
            b.sort();
            b
        },
        {
            let mut a = after.clone();
            a.sort();
            a
        },
        "generating a lemma's lemmas must produce nothing — otherwise the first one recurses \
         for ever"
    );

    // DISPLACEMENT: a name already held is left exactly as it was.
    let mut ctx = std_ctx();
    let (ty, body) = dbl_definition();
    ctx.add_definition("dbl".to_string(), ty, body);
    let mine = apps(g("Eq"), &[nat(), g("Zero"), g("Zero")]);
    let my_proof = apps(g("refl"), &[nat(), g("Zero")]);
    ctx.add_definition("dbl_eq_1".to_string(), mine.clone(), my_proof.clone());
    equations::generate(&mut ctx, "dbl");
    assert_eq!(
        ctx.get_definition_body("dbl_eq_1"),
        Some(&my_proof),
        "a name the user already holds must survive generation untouched"
    );

    // BOOTSTRAP: with no `Eq` in the context there is nothing to state an equation in, and the
    // generator must say nothing rather than mint an unbound reference.
    let mut bare = Context::new();
    bare.add_inductive("N", sort(0));
    bare.add_constructor("Z", "N", g("N"));
    bare.add_definition(
        "idN".to_string(),
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(g("N")),
            body_type: Box::new(g("N")),
        },
        lam("x", g("N"), var("x")),
    );
    equations::generate(&mut bare, "idN");
    assert!(
        bare.iter_definitions()
            .all(|(n, _, _)| !equations::is_equation_name(n)),
        "before `Eq` exists there is no equation to state"
    );
    assert!(
        bare.refused_declarations().is_empty(),
        "{:?}",
        bare.refused_declarations()
    );
}

// =============================================================================
// THE CORPUS'S OWN SPELLING, THROUGH THE REPL, WITH LEAN AS THE ARBITER
// =============================================================================

/// T04-008's nested pattern, driven through `Repl::execute` — the door the corpus drives — in
/// the spelling LEAN ACCEPTS, plus the spelling Lean refuses and the reason it gives.
///
/// ★ LEAN REFUSES THE TWO-CLAUSE SPELLING, and this was checked by running it, not inferred.
/// With `~/.elan/toolchains/leanprover--lean4---v4.14.0/bin/lean`, on
///
/// ```lean
/// def classify : Nat → Nat
///   | 0 => 0
///   | Nat.succ (Nat.succ k) => k
/// ```
///
/// answers `error: missing cases: (Nat.succ Nat.zero)`. The corpus case's own prose describes
/// THREE equations — *"`| 0 => 10 | 1 => 20 | n+2 => n` is three equations over a
/// two-constructor inductive"* — and three is what both compilers require: a LOGOS definition is
/// a total function and so is a Lean one. The refusal is the shared behaviour, and ours names the
/// missing shape and the repair, which Lean's message does not.
#[test]
fn the_corpus_nested_pattern_compiles_through_the_repl_in_the_spelling_lean_accepts() {
    use logicaffeine_kernel::interface::Repl;

    // THE CAPABILITY, through `Repl::execute` itself.
    let mut repl = Repl::new();
    repl.execute(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("a nested constructor pattern must compile to the positional tree through the REPL");
    let ctx = repl.context();
    for (input, want) in [(0usize, 0usize), (1, 1), (5, 3), (9, 7)] {
        let got = normalize(ctx, &app(g("classify"), peano(input)));
        assert!(
            defeq_for_test(ctx, &got, &peano(want)),
            "classify {input} must be {want}, got {got}"
        );
    }

    // …and the two-clause spelling is refused, naming exactly the shape Lean names.
    let mut repl = Repl::new();
    let short = repl.execute(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ (Succ k): Yield k.",
    );
    let msg = format!("{short:?}");
    assert!(
        short.is_err(),
        "`| 0 | .succ (.succ k)` leaves `Nat.succ Nat.zero` uncovered; Lean answers `missing \
         cases: (Nat.succ Nat.zero)` and a total language cannot do less: {msg}"
    );
    assert!(
        msg.contains("Succ Zero"),
        "the refusal must NAME the shape no equation covers, which is the half of Lean's message \
         a reader acts on: {msg}"
    );
    assert!(
        repl.context().get_global("classify").is_none(),
        "a refused definition must leave nothing behind"
    );
}

/// ★ THE ARMED GUARD FOR ∀-INTRODUCTION'S FRESHNESS.
///
/// The step case of an induction over a field-carrying constructor is closed by FIXING the
/// field as an arbitrary constant — `∀x:Chain. P x → P (Link x)` becomes `P (Link c)` with `c`
/// rigid. That rule is sound only while `c` is genuinely arbitrary, and a constant is not
/// arbitrary if the author can NAME it: `«a quoted name»` is spellable everywhere an identifier
/// is read, so a `Definition` can give the introduced constant a VALUE, and then the case
/// checked is about that value rather than about an arbitrary element.
///
/// The statement below is FALSE — `num (Val (Succ Zero))` is `Succ Zero`, not `Zero` — so it must
/// be refused whether or not any binder can be introduced. That is what makes this test USEFUL
/// BEFORE the capability exists and a trap the moment it does: it passes today because nothing
/// introduces anything, and it keeps passing only if the introduced constant is checked fresh
/// against the context rather than assumed fresh. Every plausible generated name is pre-empted,
/// so a generator that steps through a fixed series walks into a definition at every step.
#[test]
fn an_introduced_constant_cannot_be_given_a_value_by_the_statements_author() {
    use logicaffeine_kernel::interface::Repl;

    let mut repl = Repl::new();
    repl.execute("A Tag is either Zed or a Val with n: Nat.")
        .expect("the inductive registers");
    repl.execute(
        "## To num (t: Tag) -> Nat: Consider t: When Zed: Yield Zero. When Val n: Yield n.",
    )
    .expect("the definition registers");

    // Pre-empt every name a generated eigenvariable is likely to take, with the ONE value that
    // would make the false statement close: `Zero`.
    let mut poisoned = 0;
    for mark in ["#intro", "#eig", "#fld", "x", "c", "n"] {
        for k in 0..4 {
            for spelling in [format!("«{mark}{k}»"), format!("«{mark}»")] {
                if repl
                    .execute(&format!("Definition {spelling} : Nat := Zero."))
                    .is_ok()
                {
                    poisoned += 1;
                }
            }
        }
    }
    assert!(
        poisoned > 0,
        "the trap is only a trap if a quoted name CAN be defined — if this surface stopped \
         accepting them the test would be vacuous and would have to be rewritten, not deleted"
    );

    let r = repl.execute(
        "## Theorem: Poisoned\n    Statement: forall t : Tag, (num t) equals Zero.\n    \
         Proof: induction t.\n    - refl.\n    - refl.",
    );
    assert!(
        r.is_err(),
        "`∀t:Tag, num t = Zero` is FALSE — `num (Val (Succ Zero))` is `Succ Zero`. If the \
         constant the `Val` case introduces is one the author has already defined as `Zero`, the \
         case CHECKED is `num (Val Zero) = Zero`, which is true, while the ∀ CONCEDED covers \
         every `Val n`. Freshness must be CHECKED against the context, never assumed: {r:?}"
    );
    assert!(
        repl.context().get_definition_body("Poisoned").is_none(),
        "a refused theorem must leave nothing behind"
    );
}

// =============================================================================
// NUMERAL PATTERNS — Lean's `| 0 => … | 1 => …`, derived rather than hard-coded
// =============================================================================

/// T04-008's own prose describes `| 0 => 10 | 1 => 20 | n+2 => n`. The NUMERALS were not
/// writable: the `When` grammar read an identifier, and a digit is a parse error.
///
/// Lean expands a `Nat` literal pattern inside the matrix compiler —
/// `Lean/Meta/Match/Match.lean:629` (`expandNatValuePattern`), reached through
/// `isNatValueTransition` at :171 — and confirms the reading on this machine:
///
/// ```lean
/// def classify : Nat → Nat
///   | 0 => 10
///   | 1 => 20
///   | n+2 => n
/// ```
///
/// `#eval` answers `10`, `20`, `5` at `0`, `1`, `7`. The same three answers are required here.
///
/// ★ ONE THING IS DELIBERATELY BETTER THAN LEAN. Lean hard-codes `Nat.zero`/`Nat.succ`; this
/// compiler DERIVES the pair from the declaration — the constructor with no value fields, and
/// the one carrying a single value of the inductive itself. So a user's own unary numeral type
/// gets numeral patterns for free, and a type that has no unary form is told so by name instead
/// of being silently expanded into constructors that happen to be spelled `Zero` and `Succ`.
#[test]
fn a_numeral_pattern_is_the_unary_spelling_lean_expands_it_to() {
    // THE CAPABILITY, in the corpus's own spelling.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To classify (n: Nat) -> Nat: Consider n: When 0: Yield Zero. \
         When 1: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("a numeral pattern must be readable and compile");
    for (input, want) in [(0usize, 0usize), (1, 1), (7, 5), (12, 10)] {
        let got = eval1(&ctx, "classify", peano(input));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "classify {input} must be {want}, got {got}"
        );
    }

    // …and it denotes the SAME function as the constructor spelling, which is the only thing
    // "sugar" can mean.
    let mut spelled = std_ctx();
    define(
        &mut spelled,
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("the constructor spelling must still compile");
    for input in [0usize, 1, 7, 12] {
        assert_eq!(
            format!("{}", eval1(&ctx, "classify", peano(input))),
            format!("{}", eval1(&spelled, "classify", peano(input))),
            "the numeral spelling and the constructor spelling must be one function at {input}"
        );
    }

    // A numeral NESTED inside a constructor pattern, and a numeral in a LATER column.
    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To deep (n: Nat) -> Nat: Consider n: When Succ 0: Yield (Succ (Succ Zero)). \
         When _: Yield Zero.",
    )
    .expect("a numeral inside a constructor pattern must compile");
    for (input, want) in [(1usize, 2usize), (0, 0), (5, 0)] {
        let got = eval1(&ctx, "deep", peano(input));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "deep {input} = {want}, got {got}"
        );
    }

    let mut ctx = std_ctx();
    define(
        &mut ctx,
        "## To pick (a: Nat) and (b: Nat) -> Nat: Consider a and b: \
         When 0 and _: Yield (Succ Zero). When _ and 0: Yield (Succ (Succ Zero)). \
         When _ and _: Yield (Succ (Succ (Succ Zero))).",
    )
    .expect("a numeral in any column must compile");
    for (a, b, want) in [(0usize, 0usize, 1usize), (0, 5, 1), (5, 0, 2), (5, 5, 3)] {
        let got = normalize(&ctx, &apps(g("pick"), &[peano(a), peano(b)]));
        assert!(
            defeq_for_test(&ctx, &got, &peano(want)),
            "pick {a} {b} = {want}, got {got}"
        );
    }

    // THE GENERALISATION: a user's own unary numeral type, which Lean's hard-coded expansion
    // would not reach.
    let mut ctx = std_ctx();
    let surface = Surface::new();
    let cmd = parse_command_in_ctx(
        &ctx,
        &surface,
        "A Tally is either Nought or a Tick with prev: Tally.",
    )
    .expect("the inductive must parse");
    let Command::Inductive {
        name,
        constructors,
        sort: isort,
        ..
    } = cmd
    else {
        panic!("not an inductive")
    };
    ctx.add_inductive(&name, isort);
    for (cn, ct) in constructors {
        ctx.add_constructor(&cn, &name, ct);
    }
    define(
        &mut ctx,
        "## To isTwo (t: Tally) -> Bool: Consider t: When 2: Yield true. When _: Yield false.",
    )
    .expect("a numeral over a user's own unary type must compile");
    let two = app(g("Tick"), app(g("Tick"), g("Nought")));
    let three = app(g("Tick"), two.clone());
    assert_eq!(eval1(&ctx, "isTwo", two), g("true"), "isTwo 2 = true");
    assert_eq!(eval1(&ctx, "isTwo", three), g("false"), "isTwo 3 = false");
    assert_eq!(
        eval1(&ctx, "isTwo", g("Nought")),
        g("false"),
        "isTwo 0 = false"
    );
}

/// The negatives that make the numeral pattern a reading rather than a guess.
#[test]
fn a_numeral_pattern_is_refused_where_there_is_no_unary_spelling() {
    // An ENUMERATION has no successor, so `2` names nothing. The refusal must say so and name
    // the type, not mis-expand into whatever constructors happen to exist.
    let mut ctx = std_ctx();
    with_hue(&mut ctx);
    let err = define(
        &mut ctx,
        "## To isJade (h: Hue) -> Bool: Consider h: When 1: Yield true. When _: Yield false.",
    )
    .expect_err("`1` is not a `Hue`");
    assert!(
        err.contains("Hue") && err.contains("numeral"),
        "the refusal must name the type and the reason: {err}"
    );
    assert!(
        ctx.get_global("isJade").is_none(),
        "a refused definition leaves nothing behind"
    );

    // A type whose one-field constructor carries something ELSE is not a successor either.
    let mut ctx = std_ctx();
    let surface = Surface::new();
    let cmd = parse_command_in_ctx(&ctx, &surface, "A Tag is either Zed or a Val with n: Nat.")
        .expect("the inductive must parse");
    let Command::Inductive {
        name,
        constructors,
        sort: isort,
        ..
    } = cmd
    else {
        panic!("not an inductive")
    };
    ctx.add_inductive(&name, isort);
    for (cn, ct) in constructors {
        ctx.add_constructor(&cn, &name, ct);
    }
    let err = define(
        &mut ctx,
        "## To isOne (t: Tag) -> Bool: Consider t: When 1: Yield true. When _: Yield false.",
    )
    .expect_err("`Val` carries a `Nat`, not a `Tag`, so it is not a successor");
    assert!(
        err.contains("Val"),
        "the refusal must name the constructor that is not one: {err}"
    );

    // The TEXT-ONLY door has no constructor list, so it cannot know what `0` expands to — and it
    // must say that rather than guess, exactly as it does for a catch-all and a nested pattern.
    use logicaffeine_kernel::interface::parse_command;
    let text = parse_command(
        "## To classify (n: Nat) -> Nat: Consider n: When 0: Yield Zero. When _: Yield Zero.",
    );
    assert!(
        text.is_err(),
        "text alone cannot expand a numeral into constructors it cannot see: {text:?}"
    );
}

// =============================================================================
// THE PRELUDE IS BUILT ONCE — measured, then pinned
// =============================================================================

/// `interface::standard_context()` must be INDISTINGUISHABLE from a freshly registered prelude,
/// and mutating what it hands back must not reach the cache.
///
/// ★ WHY THERE IS A CACHE AT ALL. Measured in this crate's own harness on this box:
/// `StandardLibrary::register` costs 314.564 ms; `Repl::new()` costs ≈355 ms; cloning an
/// already-registered `Context` costs ≈1 µs — the tables are `Arc`-backed and every mutating
/// accessor goes through `Arc::make_mut`, so a clone is a pointer bump that copies only the one
/// table a caller writes to. Two tests in THIS file build 136 and 48 REPLs respectively and
/// spend essentially all of their >60 s inside `register`, and every one of those registrations
/// produces a byte-identical context.
///
/// A cache of a value everything else mutates is exactly the kind of sharing that produces a
/// bug nobody can reproduce, so the equivalence is CHECKED rather than argued: every inductive,
/// every constructor, every declaration and every definition is compared, in both directions,
/// and then the clone is mutated and the next clone is shown not to have noticed.
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

// =============================================================================
// THE MOTIVE — what a compiled node says about the type it eliminates
// =============================================================================

/// Every `Match` node in a compiled tree carries the motive `λ_:I. T`, never the bare `T`.
///
/// The kernel accepts both: its `Match` rule wraps a `Sort`-typed motive as the constant
/// `λ_:I. T` before it checks the branches, so a bare result type type-checks, computes, and is
/// invisible from inside. What it throws away is the one fact only the motive records — WHICH
/// inductive the node eliminates. The discriminant of a compiled node is a `Var`, and a `Var`'s
/// type is knowable only with the local telescope in scope, which a consumer walking a STORED
/// term does not have. Program extraction reads exactly this to enumerate the constructors; a
/// bare motive left it with none, and it emitted `match n { }` — a match with no arms that the
/// kernel accepts and `rustc` refuses.
///
/// The compiler already HOLDS the fact at every node: it computes the discriminant's type to
/// split the column at all. So the richer form is pinned at the producer rather than re-derived,
/// differently, by each consumer.
///
/// ★ MUTATION PROBE: in `match_compile::compile`, emit `motive: Box::new(result_type.clone())`
/// instead of the λ — this goes RED naming the offending node, and
/// `phase84_extraction_e2e::test_e2e_add_execution` goes red with a non-exhaustive `match n { }`.
#[test]
fn every_compiled_match_carries_a_motive_naming_its_inductive() {
    fn inductive_head(ctx: &Context, ty: &Term) -> Option<String> {
        let mut head = ty;
        while let Term::App(f, _) = head {
            head = f;
        }
        match head {
            Term::Global(n) | Term::Const { name: n, .. } if ctx.is_inductive(n) => Some(n.clone()),
            _ => None,
        }
    }
    fn walk(ctx: &Context, t: &Term, found: &mut usize) {
        if let Term::Match {
            motive,
            cases,
            discriminant,
        } = t
        {
            *found += 1;
            let Term::Lambda { param_type, .. } = motive.as_ref() else {
                panic!(
                    "a compiled `match` on `{discriminant}` carries the bare motive `{motive}`. \
                     The kernel tolerates it, every consumer that must enumerate the \
                     constructors cannot: nothing in a stored term says what `{discriminant}` \
                     is the type of."
                )
            };
            assert!(
                inductive_head(ctx, param_type).is_some(),
                "the motive's binder must be typed at the inductive being eliminated, got \
                 `{param_type}`"
            );
            walk(ctx, discriminant, found);
            for c in cases {
                walk(ctx, c, found);
            }
        }
        match t {
            Term::Lambda {
                param_type, body, ..
            } => {
                walk(ctx, param_type, found);
                walk(ctx, body, found);
            }
            Term::Pi {
                param_type,
                body_type,
                ..
            } => {
                walk(ctx, param_type, found);
                walk(ctx, body_type, found);
            }
            Term::App(f, a) => {
                walk(ctx, f, found);
                walk(ctx, a, found);
            }
            Term::Fix { body, .. } => walk(ctx, body, found),
            _ => {}
        }
    }

    let ctx = std_ctx();

    // One column, one node.
    let work = ctx.extend("n", nat());
    let clauses = vec![
        Clause {
            patterns: vec![Pattern::Ctor {
                name: "Zero".into(),
                args: vec![],
            }],
            body: lit(1),
        },
        Clause {
            patterns: vec![Pattern::Wildcard],
            body: lit(2),
        },
    ];
    let one = compile_clauses(&work, &[var("n")], &nat(), &clauses).expect("compiles");
    let mut nodes = 0;
    walk(&work, &one, &mut nodes);
    assert_eq!(nodes, 1, "a one-column split is one node");

    // Two columns: the INNER node is where a motive computed once at the top would be wrong,
    // so the walk must reach it.
    let work2 = ctx.extend("a", nat()).extend("b", g("Bool"));
    let zero = || Pattern::Ctor {
        name: "Zero".into(),
        args: vec![],
    };
    let tru = || Pattern::Ctor {
        name: "true".into(),
        args: vec![],
    };
    let two = compile_clauses(
        &work2,
        &[var("a"), var("b")],
        &nat(),
        &[
            Clause {
                patterns: vec![zero(), tru()],
                body: lit(1),
            },
            Clause {
                patterns: vec![zero(), Pattern::Wildcard],
                body: lit(2),
            },
            Clause {
                patterns: vec![Pattern::Wildcard, Pattern::Wildcard],
                body: lit(3),
            },
        ],
    )
    .expect("a two-column matrix compiles");
    let mut nodes2 = 0;
    walk(&work2, &two, &mut nodes2);
    assert!(
        nodes2 >= 2,
        "a two-column split has an inner node too, got {nodes2}"
    );

    // …and the richer motive is still the term the kernel accepts.
    let f = lam("a", nat(), lam("b", g("Bool"), two.clone()));
    infer_type(&ctx, &f).expect("the compiled tree must still type-check");
    for (a, b, want) in [
        (0usize, true, 1u64),
        (0, false, 2),
        (4, true, 3),
        (4, false, 3),
    ] {
        let got = normalize(
            &ctx,
            &apps(f.clone(), &[peano(a), g(if b { "true" } else { "false" })]),
        );
        assert!(
            defeq_for_test(&ctx, &got, &lit(want)),
            "the motive change must not move the function: f {a} {b} = {want}, got {got}"
        );
    }
}
