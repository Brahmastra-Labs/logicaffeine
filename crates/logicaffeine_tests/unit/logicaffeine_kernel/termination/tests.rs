use super::*;
use crate::term::Universe;

/// A context with `Nat` (with `Zero`/`Succ`) and `False : Prop` — enough to exercise the guard.
fn nat_context() -> Context {
    let mut ctx = Context::new();
    let nat = Term::Global("Nat".to_string());
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Zero", "Nat", nat.clone());
    ctx.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(nat),
        },
    );
    ctx.add_inductive("False", Term::Sort(Universe::Prop));
    ctx
}

fn nat() -> Term {
    Term::Global("Nat".to_string())
}

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}

fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}

/// THE SOUNDNESS RED TEST (CRITIQUE finding #1). The structural-recursion guard must reject a
/// fixpoint that smuggles its own recursive name `f` as a FIRST-CLASS ARGUMENT — `(λg:Nat→False. g
/// Zero) f` — instead of applying it to a structurally-smaller value. With the higher-order escape,
/// `f` is visited only as an inert `Var` leaf, the guard passes, and `boom Zero : False` inhabits
/// `False` with zero axioms. The guard MUST reject this body.
#[test]
fn recursive_name_smuggled_as_a_first_class_argument_is_rejected() {
    let ctx = nat_context();
    let nat_to_false = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body_type: Box::new(Term::Global("False".to_string())),
    };

    // Zero case: (λg:Nat→False. g Zero) f  — `f` (the fixpoint) passed as an argument.
    let zero_case = app(
        Term::Lambda {
            param: "g".to_string(),
            param_type: Box::new(nat_to_false),
            body: Box::new(app(var("g"), Term::Global("Zero".to_string()))),
        },
        var("f"),
    );
    // Succ case: λk:Nat. f k  — a genuinely-guarded recursive call (only the Zero case escapes).
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(var("f"), var("k"))),
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(Term::Global("False".to_string())),
            }),
            cases: vec![zero_case, succ_case],
        }),
    };

    let result = check_termination(&ctx, "f", &body);
    assert!(
        result.is_err(),
        "kernel soundness: a fixpoint that passes its recursive name as a first-class value inhabits \
             False and MUST be rejected by the termination guard, but it was accepted"
    );
}

/// Regression guard: genuine structural recursion `fix f. λn. match n with Zero => Zero | Succ k => f k`
/// must still pass. The fix for the escape must not reject honest fully-applied decreasing calls.
#[test]
fn genuine_structural_recursion_still_passes() {
    let ctx = nat_context();
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(var("f"), var("k"))), // f k — k is structurally smaller
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(nat()),
            }),
            cases: vec![Term::Global("Zero".to_string()), succ_case],
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_ok(),
        "honest structural recursion must still pass"
    );
}

/// The recursive name returned bare from a branch (`match n with Zero => f | …`) is the same escape
/// in a different costume — `f` as a value, not a guarded call. Must be rejected.
#[test]
fn recursive_name_returned_bare_from_a_branch_is_rejected() {
    let ctx = nat_context();
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(var("f"), var("k"))),
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(nat()),
            }),
            cases: vec![var("f"), succ_case], // Zero => f  (bare recursive name)
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "a branch returning the bare fixpoint must be rejected"
    );
}

/// APPLIED-SMALLER FENCE #1 (the `Acc` extension's over-admission risk). The
/// applied-smaller rule admits `rec (h a…)` ONLY when the HEAD `h` is a
/// structurally-smaller variable. A CONSTRUCTOR head is not smaller: `f (Succ k)`
/// passes `Succ k`, which is strictly LARGER than the matched `k`. Accepting it
/// would make `fix f. λn. match n with Zero => … | Succ k => f (Succ k)` loop
/// forever (it recurses on a value bigger than the one it destructed). The guard
/// MUST reject the constructor-headed structural argument.
#[test]
fn recursive_call_on_a_constructor_applied_to_a_smaller_var_is_rejected() {
    let ctx = nat_context();
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        // f (Succ k) — structural argument headed by the constructor `Succ`, not a smaller var.
        body: Box::new(app(
            var("f"),
            app(Term::Global("Succ".to_string()), var("k")),
        )),
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(nat()),
            }),
            cases: vec![Term::Global("Zero".to_string()), succ_case],
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "kernel soundness: a recursive call on `Succ k` (larger than the matched `k`) must be rejected"
    );
}

/// APPLIED-SMALLER FENCE #2. The applied head must be a SMALLER variable, not
/// merely *some* variable. Here `h` is an ordinary function parameter — never
/// bound by the guarding match, so not marked smaller — and `f h (h k)` recurses
/// on `h k`. Since `h` could be `Succ`, `h k` is not a subterm of `n`; the guard
/// must reject applying a non-smaller variable in the structural position.
#[test]
fn recursive_call_on_a_non_smaller_variable_applied_is_rejected() {
    let ctx = nat_context();
    let nat_to_nat = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat()),
        body_type: Box::new(nat()),
    };
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        // f h (h k) — structural argument (position 1) is `h k`, headed by the non-smaller `h`.
        body: Box::new(app(app(var("f"), var("h")), app(var("h"), var("k")))),
    };
    let body = Term::Lambda {
        param: "h".to_string(),
        param_type: Box::new(nat_to_nat),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("n")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![Term::Global("Zero".to_string()), succ_case],
            }),
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "kernel soundness: recursing on `h k` where `h` is not structurally smaller must be rejected"
    );
}

/// AUDIT FIX: a non-decreasing recursive call hidden in the match RETURN MOTIVE must be
/// caught. The guard traverses the motive (as the standard CIC guard does), not only the
/// discriminant and cases — otherwise `f n` here would evade the check.
#[test]
fn recursive_call_hidden_in_the_match_motive_is_rejected() {
    let ctx = nat_context();
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(var("f"), var("k"))),
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            // motive λ_:Nat. (f n) — a non-decreasing recursive occurrence.
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(app(var("f"), var("n"))),
            }),
            cases: vec![Term::Global("Zero".to_string()), succ_case],
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "a recursive call in the match motive must be rejected"
    );
}

/// AUDIT FIX: a non-decreasing recursive call hidden in a binder's DOMAIN annotation
/// must be caught — the guard traverses `λ`/`Π` parameter types too.
#[test]
fn recursive_call_hidden_in_a_binder_domain_is_rejected() {
    let ctx = nat_context();
    // Succ case: λk. λ(_ : f n). f k  — `f n` sits in the inner λ's domain.
    let succ_case = Term::Lambda {
        param: "k".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(app(var("f"), var("n"))),
            body: Box::new(app(var("f"), var("k"))),
        }),
    };
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(Term::Lambda {
                param: "_".to_string(),
                param_type: Box::new(nat()),
                body: Box::new(nat()),
            }),
            cases: vec![Term::Global("Zero".to_string()), succ_case],
        }),
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "a recursive call in a binder domain must be rejected"
    );
}

/// The classic non-terminating shape `fix f. λn. f n` — a recursive call on the structural
/// parameter itself, which does not decrease. Must be rejected.
#[test]
fn non_decreasing_recursive_call_is_rejected() {
    let ctx = nat_context();
    let body = Term::Lambda {
        param: "n".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(app(var("f"), var("n"))), // f n — n is the parameter, not smaller
    };
    assert!(
        check_termination(&ctx, "f", &body).is_err(),
        "a non-decreasing self-call must be rejected"
    );
}

/// The lexicographic measure's own fences, at the unit level: `ack`'s inner call holds the
/// first column equal and decreases the second, and the same shape with NOTHING decreasing
/// is refused. The integration file `tests/G_recursor_extra.rs` carries the full fence set;
/// these two keep the module honest on its own.
#[test]
fn a_rebuilt_first_column_plus_a_decreasing_second_is_a_lexicographic_descent() {
    let ctx = nat_context();
    let inner = |call: Term| Term::Lambda {
        param: "m".to_string(),
        param_type: Box::new(nat()),
        body: Box::new(Term::Lambda {
            param: "n".to_string(),
            param_type: Box::new(nat()),
            body: Box::new(Term::Match {
                discriminant: Box::new(var("m")),
                motive: Box::new(Term::Lambda {
                    param: "_".to_string(),
                    param_type: Box::new(nat()),
                    body: Box::new(nat()),
                }),
                cases: vec![
                    Term::Global("Zero".to_string()),
                    Term::Lambda {
                        param: "m1".to_string(),
                        param_type: Box::new(nat()),
                        body: Box::new(Term::Match {
                            discriminant: Box::new(var("n")),
                            motive: Box::new(Term::Lambda {
                                param: "_".to_string(),
                                param_type: Box::new(nat()),
                                body: Box::new(nat()),
                            }),
                            cases: vec![
                                Term::Global("Zero".to_string()),
                                Term::Lambda {
                                    param: "n2".to_string(),
                                    param_type: Box::new(nat()),
                                    body: Box::new(call.clone()),
                                },
                            ],
                        }),
                    },
                ],
            }),
        }),
    };
    let succ = |t: Term| app(Term::Global("Succ".to_string()), t);
    let call = |a: Term, b: Term| app(app(var("f"), a), b);

    assert!(
        check_termination(&ctx, "f", &inner(call(succ(var("m1")), var("n2")))).is_ok(),
        "`f (Succ m1) n2` holds column 0 equal (`Succ m1 ≡ m`) and decreases column 1"
    );
    assert!(
        check_termination(&ctx, "f", &inner(call(succ(var("m1")), var("n")))).is_err(),
        "`f (Succ m1) n` decreases nothing and must be refused"
    );
    assert!(
        check_termination(&ctx, "f", &inner(call(var("n2"), var("m1")))).is_err(),
        "a subterm of column 1 must not discharge column 0"
    );
}
