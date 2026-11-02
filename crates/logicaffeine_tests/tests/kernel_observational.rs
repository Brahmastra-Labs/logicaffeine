//! Observational equality: `a ~[A] b` in `SProp` for every type `A`,
//! computing on the weak-head normal form of `A`, so function extensionality holds by conversion.
//! Registered by `observational::register` beside the standard library, which it leaves unchanged;
//! the computation rules fire only for the constants its certificate names.

use logicaffeine_kernel::interface::TermParser;
use logicaffeine_kernel::observational;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, normalize, recheck, Context, Term};

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

fn obs_ctx() -> Context {
    let mut ctx = StandardLibrary::context().clone();
    observational::register(&mut ctx)
        .expect("the observational constants register over the standard library");
    ctx
}

/// A test's setup declared every name it wrote: a refused setup leaves a term over unknown names,
/// which no rule reduces, and a seam asserted stuck would hold for that reason alone.
fn registered(ctx: &Context, names: &[&str]) {
    for name in names {
        assert!(
            ctx.get_global(name).is_some(),
            "the setup declares `{name}`"
        );
    }
}

/// `Sized`, a structure whose second field's type mentions its first: `bits : BVec n`.
fn sized(ctx: &mut Context) {
    let bits = Term::App(Box::new(term("BVec")), Box::new(Term::Var("n".to_string())));
    ctx.add_structure("Sized", &[], &[("n", term("Nat")), ("bits", bits)]);
    registered(ctx, &["Sized", "Sized_mk"]);
}

/// `k` checked in both kernels against `ty` through an application, so each kernel's conversion
/// decides `k`'s inferred type against `ty`.
fn checks_against(ctx: &Context, k: &str, ty: &str) -> (bool, bool) {
    let probe = term(&format!("(fun (wanted : {ty}) => wanted) ({k})"));
    let main = infer_type(ctx, &probe).is_ok();
    let independent = recheck(ctx, &probe).is_ok();
    (main, independent)
}

#[test]
fn the_constants_are_registered_at_their_signatures() {
    let ctx = obs_ctx();
    for (name, ty) in [
        ("Obs.{u}", "forall (A : Sort u) (a : A) (b : A), SProp"),
        ("Obs_refl.{u}", "forall (A : Sort u) (a : A), Obs.{u} A a a"),
        (
            "Obs_cast.{u}",
            "forall (A : Sort u) (B : Sort u), Obs.{u+1} (Sort u) A B -> A -> B",
        ),
    ] {
        let found = infer_type(&ctx, &term(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            is_subtype(&ctx, &found, &term(ty)),
            "{name} : {found}, not {ty}"
        );
    }
    assert!(
        infer_type(&ctx, &term("Obs_transp.{u}")).is_ok(),
        "Obs_transp is registered"
    );
}

#[test]
fn equality_of_functions_is_pointwise_equality_by_computation() {
    let ctx = obs_ctx();
    let funext = "fun (A : Type) (B : Type) (f : A -> B) (g : A -> B) (h : forall (x : A), Obs.{1} B (f x) (g x)) => h";
    let stated = "forall (A : Type) (B : Type) (f : A -> B) (g : A -> B), \
                  (forall (x : A), Obs.{1} B (f x) (g x)) -> Obs.{1} (A -> B) f g";
    assert_eq!(
        checks_against(&ctx, funext, stated),
        (true, true),
        "funext holds by conversion in both kernels"
    );
}

#[test]
fn equality_at_a_type_that_does_not_compute_stays_stuck() {
    let mut ctx = obs_ctx();
    ctx.add_declaration("X", term("Type"));
    ctx.add_declaration("a", term("X"));
    registered(&ctx, &["X", "a"]);
    let stuck = term("Obs.{1} X a a");
    assert_eq!(
        normalize(&ctx, &stuck),
        stuck,
        "nothing is known about `X`, so nothing reduces"
    );
}

#[test]
fn the_rules_fire_only_for_the_certified_constants() {
    let mut ctx = obs_ctx();
    ctx.add_universe_poly_declaration(
        "Obs2",
        vec!["u".to_string()],
        term("forall (A : Sort u) (a : A) (b : A), SProp"),
    )
    .expect("a look-alike");
    let funext2 = "fun (A : Type) (B : Type) (f : A -> B) (g : A -> B) (h : forall (x : A), Obs2.{1} B (f x) (g x)) => h";
    let stated2 = "forall (A : Type) (B : Type) (f : A -> B) (g : A -> B), \
                   (forall (x : A), Obs2.{1} B (f x) (g x)) -> Obs2.{1} (A -> B) f g";
    assert_eq!(
        checks_against(&ctx, funext2, stated2),
        (false, false),
        "an uncertified constant does not compute"
    );
}

/// Whether `from` and `to` are one type under `binders`, in each kernel: the identity on `from` is
/// checked at `from → to`.
fn same(ctx: &Context, binders: &str, from: &str, to: &str) -> (bool, bool) {
    let (open, close) = if binders.is_empty() {
        (String::new(), String::new())
    } else {
        (format!("forall {binders}, "), format!("fun {binders} => "))
    };
    checks_against(
        ctx,
        &format!("{close}fun (h : {from}) => h"),
        &format!("{open}{from} -> {to}"),
    )
}

#[test]
fn equality_of_strict_propositions_is_logical_equivalence() {
    let ctx = obs_ctx();
    assert_eq!(
        same(
            &ctx,
            "(P : SProp) (Q : SProp)",
            "Obs.{1} SProp P Q",
            "SAnd (P -> Q) (Q -> P)"
        ),
        (true, true)
    );
}

#[test]
fn equality_of_propositions_is_their_equivalence_squashed() {
    let ctx = obs_ctx();
    assert_eq!(
        same(
            &ctx,
            "(P : Prop) (Q : Prop)",
            "Obs.{1} Prop P Q",
            "SAnd (SBox.{0} (P -> Q)) (SBox.{0} (Q -> P))"
        ),
        (true, true)
    );
}

#[test]
fn different_constructors_are_never_equal() {
    let ctx = obs_ctx();
    assert_eq!(
        same(&ctx, "", "Obs.{1} Nat Zero (Succ Zero)", "SFalse"),
        (true, true)
    );
    assert_eq!(
        same(&ctx, "", "Obs.{1} Bool true false", "SFalse"),
        (true, true)
    );
}

#[test]
fn the_same_constructor_is_equal_when_its_fields_are() {
    let ctx = obs_ctx();
    assert_eq!(
        same(
            &ctx,
            "(n : Nat) (m : Nat)",
            "Obs.{1} Nat (Succ n) (Succ m)",
            "Obs.{1} Nat n m"
        ),
        (true, true)
    );
    assert_eq!(
        same(&ctx, "", "Obs.{1} Nat Zero Zero", "STrue"),
        (true, true)
    );
    assert_eq!(
        same(
            &ctx,
            "(a : Nat) (b : Bool) (c : Nat) (d : Bool)",
            "Obs.{1} (Prod Nat Bool) (Prod_mk Nat Bool a b) (Prod_mk Nat Bool c d)",
            "SAnd (Obs.{1} Nat a c) (Obs.{1} Bool b d)"
        ),
        (true, true)
    );
}

#[test]
fn a_field_whose_type_mentions_an_earlier_field_is_compared_across_a_cast() {
    let ctx = obs_ctx();
    let carried = "SSigma (Obs.{2} Type (BVec k) (BVec k)) \
                   (fun (e : Obs.{2} Type (BVec k) (BVec k)) => Obs.{1} (BVec k) (Obs_cast.{1} (BVec k) (BVec k) e v) w)";
    assert_eq!(
        same(
            &ctx,
            "(k : Nat) (b : Bit) (c : Bit) (v : BVec k) (w : BVec k)",
            "Obs.{1} (BVec (Succ k)) (BVCons b k v) (BVCons c k w)",
            &format!("SAnd (Obs.{{1}} Bit b c) (SAnd (Obs.{{1}} Nat k k) ({carried}))")
        ),
        (true, true)
    );
}

#[test]
fn numerals_compare_as_their_constructors() {
    let ctx = obs_ctx();
    let nat = |n: i64| {
        Term::Lit(logicaffeine_kernel::Literal::Nat(
            logicaffeine_kernel::BigInt::from_i64(n),
        ))
    };
    let equation = |a: Term, b: Term| {
        [term("Nat"), a, b]
            .into_iter()
            .fold(term("Obs.{1}"), |f, x| Term::App(Box::new(f), Box::new(x)))
    };
    assert_eq!(normalize(&ctx, &equation(nat(3), nat(3))), term("STrue"));
    assert_eq!(normalize(&ctx, &equation(nat(3), nat(4))), term("SFalse"));
    assert_eq!(
        normalize(&ctx, &equation(nat(2), term("Succ (Succ Zero)"))),
        term("STrue")
    );
}

/// Two constructors of a proposition are one proof: comparing them by their constructors would
/// make `Obs` say `SFalse` of a proof `Obs_refl` proves.
#[test]
fn the_constructors_of_a_proposition_are_never_compared() {
    let ctx = obs_ctx();
    let (left, right) = ("left True True I", "right True True I");
    let equation = format!("Obs.{{0}} (Or True True) ({left}) ({right})");
    assert_eq!(
        same(&ctx, "", &equation, "SFalse"),
        (false, false),
        "no clash between two proofs"
    );
    assert_eq!(
        checks_against(
            &ctx,
            &format!("Obs_refl.{{0}} (Or True True) ({left})"),
            &equation
        ),
        (true, true),
        "the two proofs are one by irrelevance"
    );
}

#[test]
fn equality_of_types_reads_their_formers() {
    let ctx = obs_ctx();
    assert_eq!(
        same(&ctx, "", "Obs.{2} Type Nat Bool", "SFalse"),
        (true, true),
        "two families"
    );
    assert_eq!(
        same(&ctx, "", "Obs.{2} Type Nat Nat", "STrue"),
        (true, true),
        "one family without arguments"
    );
    assert_eq!(
        same(&ctx, "", "Obs.{2} Type Prop Nat", "SFalse"),
        (true, true),
        "a universe and a family"
    );
    assert_eq!(
        same(&ctx, "", "Obs.{2} Type (Nat -> Nat) Nat", "SFalse"),
        (true, true),
        "a function type and a family"
    );
    assert_eq!(
        same(&ctx, "", "Obs.{3} (Sort 2) Type Type", "STrue"),
        (true, true),
        "one universe"
    );
    assert_eq!(
        same(&ctx, "", "Obs.{3} (Sort 2) Type Prop", "SFalse"),
        (true, true),
        "two universes"
    );
    assert_eq!(
        same(
            &ctx,
            "(A : Type) (B : Type)",
            "Obs.{2} Type (TList A) (TList B)",
            "Obs.{2} Type A B"
        ),
        (true, true),
        "a family by its arguments"
    );
    assert_eq!(
        same(
            &ctx,
            "",
            "Obs.{2} Type (Nat -> Nat) (Nat -> Bool)",
            "SSigma (Obs.{2} Type Nat Nat) (fun (e : Obs.{2} Type Nat Nat) => forall (x : Nat), Obs.{2} Type Nat Bool)"
        ),
        (true, true),
        "function types by their domains and codomains"
    );
}

/// `Nat → False` and `Bool → False` are one proposition (each implies the other), so they are
/// equal at `Prop`. Seen at `Type` by cumulativity, reading them by their formers would make them
/// unequal (`Nat` is not `Bool`): with `Obs`'s congruence from `Prop` to `Type`, a proof of
/// `SFalse`.
#[test]
fn a_proposition_seen_as_a_type_is_not_compared_by_its_former() {
    let ctx = obs_ctx();
    let at_type = "Obs.{2} Type (Nat -> False) (Bool -> False)";
    assert_eq!(
        head(&normalize(&ctx, &term(at_type))),
        Some("Obs"),
        "{at_type} stays stuck"
    );
    for consequence in ["SFalse", "SSigma (Obs.{2} Type Bool Nat) (fun (e : Obs.{2} Type Bool Nat) => forall (x : Bool), Obs.{1} Prop False False)"] {
        assert_eq!(same(&ctx, "", at_type, consequence), (false, false), "{at_type} stays stuck, not {consequence}");
    }
    assert_eq!(
        same(
            &ctx,
            "",
            "Obs.{1} Prop (Nat -> False) (Bool -> False)",
            "SAnd (SBox.{0} ((Nat -> False) -> Bool -> False)) (SBox.{0} ((Bool -> False) -> Nat -> False))"
        ),
        (true, true),
        "at `Prop` they are compared as propositions"
    );
}

/// Hur's paradox: a family `Hur : (Type → Type) → Type` that is injective in its argument is an
/// injection from `Type → Type` into `Type`, which excluded middle refutes. An argument larger than
/// the family's universe keeps the equality stuck.
#[test]
fn a_family_over_an_argument_larger_than_its_universe_is_not_injective() {
    let mut ctx = obs_ctx();
    ctx.add_indexed_inductive("Hur", term("(Type -> Type) -> Type"), 1);
    ctx.add_constructor("hur", "Hur", term("forall (F : Type -> Type), Hur F"));
    registered(&ctx, &["Hur", "hur"]);
    let equation = "Obs.{2} Type (Hur (fun (X : Type) => X)) (Hur (fun (X : Type) => X -> X))";
    let injective = "Obs.{3} (Type -> Type) (fun (X : Type) => X) (fun (X : Type) => X -> X)";
    assert_eq!(
        same(&ctx, "", equation, injective),
        (false, false),
        "{equation} stays stuck"
    );
    assert_eq!(
        head(&normalize(&ctx, &term(equation))),
        Some("Obs"),
        "the equation is still an application of `Obs`"
    );
    let small = "Obs.{2} Type (TList (Nat -> Nat)) (TList (Nat -> Bool))";
    assert_eq!(
        head(&normalize(&ctx, &term(small))),
        Some("SSigma"),
        "the control: a family over a small argument computes"
    );
}

/// The constant at the head of `t`'s application spine.
fn head(t: &Term) -> Option<&str> {
    match t {
        Term::App(f, _) => head(f),
        Term::Global(name) | Term::Const { name, .. } => Some(name),
        _ => None,
    }
}

/// At `u := 0`, `Nat → P` and `Bool → P` are propositions (`imax 1 0 = 0`), compared by `Prop`'s
/// rule; at `u ≥ 1` they are types. With `u` abstract, no rule is the right one for every `u`.
#[test]
fn types_that_are_propositions_at_some_level_are_not_compared_by_their_formers() {
    let mut ctx = obs_ctx();
    ctx.add_universe_poly_declaration("P", vec!["u".to_string()], term("Sort u"))
        .expect("a type at any sort");
    let equation = term("Obs.{u+1} (Sort u) (Nat -> P.{u}) (Bool -> P.{u})");
    assert_eq!(
        head(&normalize(&ctx, &equation)),
        Some("Obs"),
        "stuck: `imax 1 u` may be `Prop`"
    );
}

#[test]
fn a_numeral_carried_along_an_equality_of_types_is_still_a_numeral() {
    let ctx = obs_ctx();
    let carried = term("Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) (Succ (Succ Zero))");
    assert_eq!(
        normalize(&ctx, &carried),
        term("Succ (Succ Zero)"),
        "canonicity: a closed `Nat` through `Obs_cast` is a numeral"
    );
    let literal = term("Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) 2n");
    assert_eq!(normalize(&ctx, &literal), term("Succ (Succ Zero)"));
    assert_eq!(
        checks_against(
            &ctx,
            "refl Nat (Succ (Succ Zero))",
            "Eq Nat (Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) (Succ (Succ Zero))) (Succ (Succ Zero))"
        ),
        (true, true),
        "both kernels compute the cast"
    );
}

#[test]
fn a_cast_along_one_universe_is_the_identity() {
    let ctx = obs_ctx();
    assert_eq!(
        normalize(
            &ctx,
            &term("Obs_cast.{2} Type Type (Obs_refl.{3} (Sort 2) Type) Nat")
        ),
        term("Nat")
    );
    assert_eq!(
        same(
            &ctx,
            "",
            "Obs.{2} Type (Obs_cast.{2} Type Type (Obs_refl.{3} (Sort 2) Type) Nat) Nat",
            "STrue"
        ),
        (true, true)
    );
}

#[test]
fn a_constructor_is_carried_to_the_target_parameters_with_its_fields_carried() {
    let ctx = obs_ctx();
    assert_eq!(
        same(
            &ctx,
            "(A : Type) (B : Type) (e : Obs.{2} Type (TList A) (TList B))",
            "Obs.{1} (TList B) (Obs_cast.{1} (TList A) (TList B) e (TNil A)) (TNil B)",
            "STrue"
        ),
        (true, true),
        "a constructor without fields moves to the target parameters"
    );
    let list = "TCons Nat Zero (TCons Nat (Succ Zero) (TNil Nat))";
    assert_eq!(
        normalize(
            &ctx,
            &term(&format!(
                "Obs_cast.{{1}} (TList Nat) (TList Nat) (Obs_refl.{{2}} Type (TList Nat)) ({list})"
            ))
        ),
        term(list),
        "every field is carried, the recursive one included"
    );
}

#[test]
fn a_function_is_carried_by_carrying_its_argument_back_and_its_result_forth() {
    let ctx = obs_ctx();
    let carried = "Obs_cast.{1} (Nat -> Nat) (Nat -> Nat) (Obs_refl.{2} Type (Nat -> Nat)) Succ";
    assert_eq!(
        normalize(&ctx, &term(&format!("({carried}) Zero"))),
        term("Succ Zero")
    );
    assert_eq!(
        checks_against(
            &ctx,
            "refl Nat (Succ Zero)",
            &format!("Eq Nat (({carried}) Zero) (Succ Zero)")
        ),
        (true, true)
    );
}

#[test]
fn equality_and_observational_equality_imply_each_other() {
    let ctx = obs_ctx();
    let forth = "fun (A : Type) (a : A) (b : A) (h : Eq A a b) => Eq_rec A a (fun (c : A) => Obs.{1} A a c) (Obs_refl.{1} A a) b h";
    assert_eq!(
        checks_against(
            &ctx,
            forth,
            "forall (A : Type) (a : A) (b : A), Eq A a b -> Obs.{1} A a b"
        ),
        (true, true),
        "`Eq` eliminates into `Obs`"
    );
    let back = "fun (A : Type) (a : A) (b : A) (h : Obs.{1} A a b) => \
                Obs_cast.{0} (Eq A a a) (Eq A a b) \
                (Obs_transp.{1} A a (fun (c : A) (hc : Obs.{1} A a c) => Obs.{1} Prop (Eq A a a) (Eq A a c)) \
                (Obs_refl.{1} Prop (Eq A a a)) b h) \
                (refl A a)";
    assert_eq!(
        checks_against(
            &ctx,
            back,
            "forall (A : Type) (a : A) (b : A), Obs.{1} A a b -> Eq A a b"
        ),
        (true, true),
        "`Obs` carries `refl` to `Eq` along the equality of the two propositions"
    );
}

/// The seams a computing equality could open, each locked closed.
mod seams {
    use super::*;

    #[test]
    fn a_cast_along_evidence_of_sfalse_does_not_reduce() {
        let mut ctx = obs_ctx();
        ctx.add_declaration("absurd", term("Obs.{2} Type Nat Bool"));
        registered(&ctx, &["absurd"]);
        let stuck = term("Obs_cast.{1} Nat Bool absurd Zero");
        assert_eq!(
            head(&normalize(&ctx, &stuck)),
            Some("Obs_cast"),
            "no `Bool` is made out of a `Nat`"
        );
    }

    #[test]
    fn a_cast_between_propositions_does_not_reduce() {
        let mut ctx = obs_ctx();
        ctx.add_declaration("e", term("Obs.{1} Prop True True"));
        registered(&ctx, &["e"]);
        let stuck = term("Obs_cast.{0} True True e I");
        assert_eq!(head(&normalize(&ctx, &stuck)), Some("Obs_cast"));
    }

    #[test]
    fn a_cast_along_a_type_that_does_not_compute_does_not_reduce() {
        let mut ctx = obs_ctx();
        ctx.add_declaration("X", term("Type"));
        ctx.add_declaration("x", term("X"));
        registered(&ctx, &["X", "x"]);
        let stuck = term("Obs_cast.{1} X X (Obs_refl.{2} Type X) x");
        assert_eq!(
            head(&normalize(&ctx, &stuck)),
            Some("Obs_cast"),
            "no `Obs_cast A A e t ≡ t` for a stuck `A`"
        );
    }

    #[test]
    fn a_cast_of_a_variable_does_not_reduce() {
        let ctx = obs_ctx();
        let stuck = "Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) n";
        assert_eq!(
            same(
                &ctx,
                "(n : Nat)",
                &format!("Obs.{{1}} Nat ({stuck}) n"),
                "STrue"
            ),
            (false, false)
        );
    }

    /// A field whose type mentions an earlier field cannot be carried at its own type: its type at
    /// the target is taken at the earlier field's carried value. Until that is derived, stuck.
    #[test]
    fn a_constructor_with_a_dependent_field_is_not_carried_field_by_field() {
        let mut ctx = obs_ctx();
        sized(&mut ctx);
        let stuck = term("Obs_cast.{1} Sized Sized (Obs_refl.{2} Type Sized) (Sized_mk (Succ Zero) (BVCons B0 Zero BVNil))");
        assert_eq!(head(&normalize(&ctx, &stuck)), Some("Obs_cast"));
    }

    /// A field whose type is an earlier field: its type at the target is the earlier field's carried
    /// value, so it is not carried at its own type either. Until that is derived, stuck.
    #[test]
    fn a_constructor_whose_field_is_typed_by_an_earlier_field_is_not_carried() {
        let mut ctx = obs_ctx();
        ctx.add_structure(
            "Dep",
            &[],
            &[("A", term("Type")), ("a", Term::Var("A".to_string()))],
        );
        registered(&ctx, &["Dep", "Dep_mk"]);
        let stuck = term("Obs_cast.{2} Dep Dep (Obs_refl.{3} (Sort 2) Dep) (Dep_mk Nat Zero)");
        assert_eq!(head(&normalize(&ctx, &stuck)), Some("Obs_cast"));
    }

    /// An indexed family stays stuck even when its constructor's fields are independent: the rule for
    /// a family is stated without indices, and its fields guard says nothing about them.
    #[test]
    fn a_value_of_an_indexed_family_with_independent_fields_is_not_carried() {
        let mut ctx = obs_ctx();
        ctx.add_indexed_inductive("Tagged", term("Nat -> Type"), 0);
        ctx.add_constructor("tag", "Tagged", term("Bool -> Tagged Zero"));
        registered(&ctx, &["Tagged", "tag"]);
        let stuck = term(
            "Obs_cast.{1} (Tagged Zero) (Tagged Zero) (Obs_refl.{2} Type (Tagged Zero)) (tag true)",
        );
        assert_eq!(head(&normalize(&ctx, &stuck)), Some("Obs_cast"));
    }

    /// An index read as a parameter would carry `flag false : Flag true` to `flag true`, the
    /// constructor at the "parameter" `true` with its one field dropped: two distinct values made one
    /// by a cast along a reflexivity. Stuck in both kernels.
    #[test]
    fn an_index_read_as_a_parameter_never_equates_two_values() {
        let mut ctx = obs_ctx();
        ctx.add_indexed_inductive("Flag", term("Bool -> Type"), 0);
        ctx.add_constructor("flag", "Flag", term("Bool -> Flag true"));
        registered(&ctx, &["Flag", "flag"]);
        let carried =
            "Obs_cast.{1} (Flag true) (Flag true) (Obs_refl.{2} Type (Flag true)) (flag false)";
        assert_eq!(head(&normalize(&ctx, &term(carried))), Some("Obs_cast"));
        assert_eq!(
            same(&ctx, "", carried, "flag true"),
            (false, false),
            "`flag false` is never `flag true`"
        );
    }

    /// An indexed family's constructor fixes its own indices, so carrying it to other indices needs
    /// the index equalities turned into fields first (Fording). Until then, stuck.
    #[test]
    fn a_value_of_an_indexed_family_is_not_carried() {
        let ctx = obs_ctx();
        let stuck = term("Obs_cast.{1} (BVec (Succ Zero)) (BVec (Succ Zero)) (Obs_refl.{2} Type (BVec (Succ Zero))) (BVCons B0 Zero BVNil)");
        assert_eq!(head(&normalize(&ctx, &stuck)), Some("Obs_cast"));
    }

    #[test]
    fn equality_and_observational_equality_are_never_identified_by_conversion() {
        let ctx = obs_ctx();
        assert_eq!(
            same(&ctx, "(a : Nat) (b : Nat)", "Eq Nat a b", "Obs.{1} Nat a b"),
            (false, false)
        );
    }

    #[test]
    fn a_closed_equality_at_a_stuck_type_proves_nothing_false() {
        let mut ctx = obs_ctx();
        ctx.add_declaration("X", term("Type"));
        ctx.add_declaration("a", term("X"));
        ctx.add_declaration("b", term("X"));
        registered(&ctx, &["X", "a", "b"]);
        assert_eq!(same(&ctx, "", "Obs.{1} X a b", "SFalse"), (false, false));
        assert_eq!(same(&ctx, "", "Obs.{1} X a b", "STrue"), (false, false));
    }
}

#[test]
fn two_classes_are_equal_when_the_equivalence_closure_relates_their_representatives() {
    let ctx = obs_ctx();
    assert_eq!(
        same(
            &ctx,
            "(A : Type) (r : A -> A -> Prop) (a : A) (b : A)",
            "Obs.{1} (Quot A r) (Quot_mk A r a) (Quot_mk A r b)",
            "SBox.{0} (EqvGen.{1} A r a b)"
        ),
        (true, true)
    );
}

/// `Quot_sound`, an axiom of the standard library, is a theorem once the classes' equality
/// computes: `r a b` gives the closure, the closure gives `~`, and `~` gives `Eq`.
#[test]
fn quotient_soundness_is_a_theorem() {
    let ctx = obs_ctx();
    let to_eq = "(fun (T : Type) (x : T) (y : T) (h : Obs.{1} T x y) => \
                 Obs_cast.{0} (Eq T x x) (Eq T x y) \
                 (Obs_transp.{1} T x (fun (c : T) (hc : Obs.{1} T x c) => Obs.{1} Prop (Eq T x x) (Eq T x c)) \
                 (Obs_refl.{1} Prop (Eq T x x)) y h) \
                 (refl T x))";
    let sound = format!(
        "fun (A : Type) (r : A -> A -> Prop) (a : A) (b : A) (h : r a b) => \
         {to_eq} (Quot A r) (Quot_mk A r a) (Quot_mk A r b) \
         (sbox.{{0}} (EqvGen.{{1}} A r a b) (EqvGen_rel A r a b h))"
    );
    assert_eq!(
        checks_against(
            &ctx,
            &sound,
            "forall (A : Type) (r : A -> A -> Prop) (a : A) (b : A), r a b -> Eq (Quot A r) (Quot_mk A r a) (Quot_mk A r b)"
        ),
        (true, true)
    );
}

#[test]
fn equality_of_quotients_reads_their_carriers_and_relations() {
    let ctx = obs_ctx();
    let r = "(fun (x : Nat) (y : Nat) => True)";
    let s = "(fun (x : Bool) (y : Bool) => True)";
    assert_eq!(
        same(
            &ctx,
            "",
            &format!("Obs.{{2}} Type (Quot Nat {r}) Nat"),
            "SFalse"
        ),
        (true, true),
        "a quotient and a family are different formers"
    );
    let carriers = format!("Obs.{{2}} Type (Quot Nat {r}) (Quot Bool {s})");
    let relations = "(Nat -> Nat -> Prop)";
    let other = "(Bool -> Bool -> Prop)";
    let types = format!("Obs.{{2}} Type {relations} {other}");
    let rest = format!("SSigma ({types}) (fun (e : {types}) => Obs.{{1}} {other} (Obs_cast.{{1}} {relations} {other} e {r}) {s})");
    assert_eq!(
        checks_against(
            &ctx,
            &format!("fun (h : {carriers}) => SAnd_left (Obs.{{2}} Type Nat Bool) ({rest}) h"),
            &format!("{carriers} -> SFalse")
        ),
        (true, true),
        "the carriers' equality comes first, the relations' across the carriers' cast after it"
    );
}

#[test]
fn a_class_is_carried_by_carrying_its_representative() {
    let ctx = obs_ctx();
    let quotient = "Quot Nat (fun (x : Nat) (y : Nat) => True)";
    let class = "Quot_mk Nat (fun (x : Nat) (y : Nat) => True) (Succ Zero)";
    let carried = format!(
        "Obs_cast.{{1}} ({quotient}) ({quotient}) (Obs_refl.{{2}} Type ({quotient})) ({class})"
    );
    assert_eq!(
        normalize(&ctx, &term(&carried)),
        normalize(&ctx, &term(class))
    );
    assert_eq!(
        checks_against(
            &ctx,
            &format!("refl ({quotient}) ({class})"),
            &format!("Eq ({quotient}) ({carried}) ({class})")
        ),
        (true, true)
    );
}

/// A literal is observed as the constructor it names only while that constructor is `Nat`'s. With
/// `Succ` assumed in place of its constructor (and `Other` beside `Zero`, so `Nat` is no unit
/// type), `1 ~[Nat] Succ Zero` does not compute in either kernel, and `0 ~[Nat] Zero` does.
#[test]
fn a_literal_is_observed_only_as_the_constructors_its_family_has() {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", term("Type"));
    ctx.add_constructor("Zero", "Nat", term("Nat"));
    ctx.add_constructor("Succ", "Nat", term("Nat -> Nat"));
    ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
    ctx.add_declaration("Succ", term("Nat -> Nat"));
    observational::register(&mut ctx).expect("registers");
    assert!(
        ctx.refused_declarations().is_empty(),
        "the setup is admitted: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        checks_against(&ctx, "sI", "Obs.{1} Nat 1n (Succ Zero)"),
        (false, false),
        "an assumed `Succ` is no successor to observe `1` through"
    );
    assert_eq!(
        checks_against(&ctx, "sI", "Obs.{1} Nat 0n Zero"),
        (true, true),
        "`0` is `Zero`, which is `Nat`'s"
    );
}

/// A literal no reading holds for is opaque to observation: with `Succ` a constructor of another
/// family, `1 ~[Nat] Other Zero` is no constructor clash, in either kernel.
#[test]
fn a_literal_no_reading_holds_for_is_opaque_to_observation() {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", term("Type"));
    ctx.add_constructor("Zero", "Nat", term("Nat"));
    ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
    ctx.add_inductive("Foo", term("Type"));
    ctx.add_constructor("Succ", "Foo", term("Nat -> Foo"));
    observational::register(&mut ctx).expect("registers");
    assert!(
        ctx.refused_declarations().is_empty(),
        "the setup is admitted: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        checks_against(
            &ctx,
            "fun (h : SFalse) => h",
            "Obs.{1} Nat 1n (Other Zero) -> SFalse"
        ),
        (false, false),
        "`1` reads as no constructor of `Nat`, so it clashes with none"
    );
    assert_eq!(
        checks_against(
            &ctx,
            "fun (h : SFalse) => h",
            "Obs.{1} Nat 0n (Other Zero) -> SFalse"
        ),
        (true, true),
        "`0` is `Zero`, which clashes with `Other`"
    );
}
