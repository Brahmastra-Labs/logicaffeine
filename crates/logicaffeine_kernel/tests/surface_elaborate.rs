//! R4 — the elaborator wired into the SURFACE, locked in by TDD.
//!
//! The user declares an implicit argument with `{A : Type}` and then simply writes
//! `id 0` — no type argument. The REPL's elaboration pass inserts and infers it, so the
//! kernel sees the fully-explicit `id Int 0`. This is the seam that makes the surface
//! language usable: less to write, the elaborator fills the rest, the kernel still
//! certifies every term.

use logicaffeine_kernel::interface::Repl;

#[test]
fn implicit_argument_is_inferred_at_the_surface() {
    let mut repl = Repl::new();
    repl.execute("Definition id : {A : Type} -> A -> A := fun A : Type => fun a : A => a.")
        .expect("define id with an implicit type argument");

    // `id 0`: the type argument is OMITTED. The elaborator infers A := Int from `0`.
    assert_eq!(
        repl.execute("Eval (id 0).").expect("eval id 0").trim(),
        "0",
        "id 0 elaborates to `id Int 0` and computes to 0"
    );

    // `Check` shows the fully-explicit, elaborated term — the implicit Int is now present.
    let checked = repl.execute("Check (id 0).").expect("check id 0");
    assert!(checked.contains("Int"), "the inferred implicit `Int` is visible: {checked}");
}

#[test]
fn the_same_implicit_function_works_at_different_types() {
    let mut repl = Repl::new();
    repl.execute("Definition id : {A : Type} -> A -> A := fun A : Type => fun a : A => a.")
        .expect("define id");

    // One definition, two uses at different types — each implicit inferred independently.
    assert_eq!(repl.execute("Eval (id 0).").unwrap().trim(), "0", "id 0 ⇝ 0 (A := Int)");
    assert_eq!(
        repl.execute("Eval (id true).").unwrap().trim(),
        "true",
        "id true ⇝ true (A := Bool)"
    );
}

#[test]
fn a_definition_body_may_apply_an_implicit_function() {
    // A later definition whose body uses `id` (omitting the type) elaborates too.
    let mut repl = Repl::new();
    repl.execute("Definition id : {A : Type} -> A -> A := fun A : Type => fun a : A => a.")
        .expect("define id");
    repl.execute("Definition five : Int := id 5.").expect("define five via id");
    assert_eq!(repl.execute("Eval five.").unwrap().trim(), "5", "five = id 5 ⇝ 5");
}

#[test]
fn ordinary_explicit_definitions_are_unaffected() {
    // No implicit binders ⇒ surface elaboration is the identity; nothing changes.
    let mut repl = Repl::new();
    repl.execute("Definition twice : Int -> Int := fun n : Int => n.").expect("define twice");
    assert_eq!(repl.execute("Eval (twice 7).").unwrap().trim(), "7");
}

#[test]
fn a_two_argument_implicit_function_infers_from_the_first_use() {
    // `const : {A : Type} -> {B : Type} -> A -> B -> A` — two implicit type arguments,
    // both inferred from the explicit values. `const 0 true ⇝ 0`.
    let mut repl = Repl::new();
    repl.execute(
        "Definition const : {A : Type} -> {B : Type} -> A -> B -> A := \
         fun A : Type => fun B : Type => fun a : A => fun b : B => a.",
    )
    .expect("define const");
    assert_eq!(
        repl.execute("Eval (const 0 true).").unwrap().trim(),
        "0",
        "const 0 true ⇝ 0 (A := Int, B := Bool, both inferred)"
    );
}

// ===========================================================================
// EXPECTED-TYPE PROPAGATION — infer an implicit from the surrounding type
// ===========================================================================

#[test]
fn an_implicit_is_inferred_from_the_expected_type() {
    // `nil : {A : Type} -> TList A` has NO value argument, so its `A` cannot come from an
    // argument — only from the EXPECTED type. `empty : TList Int := nil` must infer A := Int.
    let mut repl = Repl::new();
    repl.execute("Definition nil : {A : Type} -> TList A := fun A : Type => TNil A.")
        .expect("define nil");
    repl.execute("Definition empty : TList Int := nil.")
        .expect("nil's A is inferred from the declared TList Int");
    let result = repl.execute("Eval empty.").unwrap();
    assert!(result.contains("TNil"), "empty = nil Int ⇝ TNil Int: {result}");
}

#[test]
fn the_expected_type_drives_the_same_definition_to_different_implicits() {
    // The same `nil`, two declared types ⇒ two different inferred `A`s.
    let mut repl = Repl::new();
    repl.execute("Definition nil : {A : Type} -> TList A := fun A : Type => TNil A.")
        .expect("define nil");
    repl.execute("Definition ints : TList Int := nil.").expect("A := Int from context");
    repl.execute("Definition bools : TList Bool := nil.").expect("A := Bool from context");
    // Both elaborate and type-check — the proof is that neither `execute` errored.
    assert!(repl.execute("Eval ints.").is_ok());
    assert!(repl.execute("Eval bools.").is_ok());
}

#[test]
fn an_expected_type_that_cannot_match_is_a_type_error() {
    // `nil` is a `TList _`, never an `Int`; the expected type `Int` must be rejected.
    let mut repl = Repl::new();
    repl.execute("Definition nil : {A : Type} -> TList A := fun A : Type => TNil A.")
        .expect("define nil");
    assert!(
        repl.execute("Definition bad : Int := nil.").is_err(),
        "nil : Int is a type error (TList A cannot unify with Int)"
    );
}

#[test]
fn a_bare_implicit_global_without_an_expected_type_stays_the_function() {
    // With no expected type and no arguments, `nil` is just the polymorphic function value
    // (Π(A). TList A) — NOT an eager unsolvable metavariable application.
    let mut repl = Repl::new();
    repl.execute("Definition nil : {A : Type} -> TList A := fun A : Type => TNil A.")
        .expect("define nil");
    assert!(repl.execute("Check nil.").is_ok(), "bare `nil` is the polymorphic function");
}

// ===========================================================================
// AUTO-BOUND IMPLICITS — write `id : A -> A`, get `{A:Type} -> A -> A` for free
// ===========================================================================

#[test]
fn a_free_type_variable_is_auto_bound_as_an_implicit() {
    // No `{A : Type}` is written — `A` is a free type variable, auto-generalized into a
    // leading implicit. Then `id 0` infers it as usual.
    let mut repl = Repl::new();
    repl.execute("Definition id : A -> A := fun a : A => a.")
        .expect("A is auto-bound as an implicit");
    assert_eq!(repl.execute("Eval (id 0).").unwrap().trim(), "0", "id 0 ⇝ 0 (A := Int)");
    assert_eq!(repl.execute("Eval (id true).").unwrap().trim(), "true", "id true ⇝ true (A := Bool)");
}

#[test]
fn multiple_free_variables_are_auto_bound_in_order() {
    // `const : A -> B -> A` auto-binds A then B (first appearance), inferred from the args.
    let mut repl = Repl::new();
    repl.execute("Definition const : A -> B -> A := fun a : A => fun b : B => a.")
        .expect("A and B auto-bound");
    assert_eq!(
        repl.execute("Eval (const 0 true).").unwrap().trim(),
        "0",
        "const 0 true ⇝ 0 (A := Int, B := Bool)"
    );
}

#[test]
fn an_auto_bound_implicit_is_inferred_from_the_expected_type() {
    // `nil : TList A := TNil A` — `A` is auto-bound AND has no value argument, so it must
    // come from the expected type when used: `empty : TList Int := nil`.
    let mut repl = Repl::new();
    repl.execute("Definition nil : TList A := TNil A.").expect("A auto-bound");
    repl.execute("Definition empty : TList Int := nil.").expect("A inferred from TList Int");
    assert!(repl.execute("Eval empty.").unwrap().contains("TNil"), "empty ⇝ TNil Int");
}

#[test]
fn explicitly_bound_parameters_are_not_auto_bound() {
    // A definition that binds its type parameter explicitly references it as a bound
    // variable, not a free global — so auto-binding leaves it completely alone.
    let mut repl = Repl::new();
    repl.execute("Definition idT : forall A : Type, A -> A := fun A : Type => fun a : A => a.")
        .expect("explicitly-bound A is untouched");
    // Still needs the type argument explicitly (it was NOT marked implicit).
    assert_eq!(repl.execute("Eval (idT Int 0).").unwrap().trim(), "0");
}

// ===========================================================================
// SURFACE INDUCTIVE ELIMINATORS — declare an inductive, get its recursor free
// ===========================================================================

#[test]
fn declaring_an_inductive_auto_derives_a_computing_recursor() {
    // Declaring `Color` registers `Color_rec` automatically — the dependent eliminator,
    // no hand-written match/fix. It type-checks and COMPUTES: selecting the `green` case.
    let mut repl = Repl::new();
    repl.execute("Inductive Color := red : Color | green : Color | blue : Color.")
        .expect("declare Color");

    // The eliminator exists and its type opens with a motive Π.
    let checked = repl.execute("Check Color_rec.").expect("Color_rec was auto-derived");
    assert!(checked.contains("Color"), "Color_rec mentions Color: {checked}");

    // Color_rec (λc. Nat) Zero (Succ Zero) (Succ (Succ Zero)) green ⇝ Succ Zero (green case).
    let r = repl
        .execute(
            "Eval (Color_rec (fun c : Color => Nat) Zero (Succ Zero) (Succ (Succ Zero)) green).",
        )
        .expect("the recursor computes");
    assert!(r.contains("Succ") && r.contains("Zero"), "green case ⇝ 1: {r}");
}

#[test]
fn the_auto_derived_nat_like_recursor_does_real_recursion() {
    // A recursive inductive: its recursor threads the induction hypothesis. `Cnt_rec`
    // computing "depth" of `s (s z)` must give 2.
    let mut repl = Repl::new();
    repl.execute("Inductive Cnt := z : Cnt | s : Cnt -> Cnt.").expect("declare Cnt");
    // depth = Cnt_rec (λ_. Nat) Zero (λc. λih. Succ ih)  — base 0, step Succ∘ih.
    let r = repl
        .execute("Eval (Cnt_rec (fun c : Cnt => Nat) Zero (fun c : Cnt => fun ih : Nat => Succ ih) (s (s z))).")
        .expect("Cnt_rec recurses");
    // s (s z) has depth 2 = Succ (Succ Zero).
    assert!(r.matches("Succ").count() == 2, "depth (s (s z)) = 2: {r}");
}

// ===========================================================================
// MATCH SUGAR — `match e with …` (no `return`), motive inferred
// ===========================================================================

#[test]
fn match_without_a_return_clause_infers_a_constant_motive() {
    // The motive of `match c with …` is inferred from the declared result type Nat.
    let mut repl = Repl::new();
    repl.execute("Inductive Color := red : Color | green : Color | blue : Color.")
        .expect("declare Color");
    repl.execute(
        "Definition rank : Color -> Nat := fun c : Color => \
         match c with | red => Zero | green => Succ Zero | blue => Succ (Succ Zero) end.",
    )
    .expect("match motive inferred from the declared Color -> Nat");
    assert_eq!(repl.execute("Eval (rank green).").unwrap().trim(), "(Succ Zero)", "rank green = 1");
    assert_eq!(
        repl.execute("Eval (rank blue).").unwrap().trim(),
        "(Succ (Succ Zero))",
        "rank blue = 2"
    );
}

#[test]
fn bare_match_infers_its_motive_from_a_nullary_first_branch() {
    // No declared type (`Eval`), but the first branch `red => Zero` is a nullary case, so
    // the motive's result type Nat is read off it.
    let mut repl = Repl::new();
    repl.execute("Inductive Color := red : Color | green : Color | blue : Color.")
        .expect("declare Color");
    let r = repl
        .execute("Eval (match green with | red => Zero | green => Succ Zero | blue => Zero end).")
        .expect("motive inferred from the first nullary branch");
    assert_eq!(r.trim(), "(Succ Zero)", "the green branch ⇝ 1");
}

#[test]
fn match_sugar_works_on_a_recursive_inductive_with_binders() {
    // `match n with | z => Zero | s k => k end` — the `s k` case binds `k`; the motive is
    // the declared codomain Cnt. (A recursive datatype, predecessor-like.)
    let mut repl = Repl::new();
    repl.execute("Inductive Cnt := z : Cnt | s : Cnt -> Cnt.").expect("declare Cnt");
    repl.execute(
        "Definition pred : Cnt -> Cnt := fun n : Cnt => match n with | z => z | s k => k end.",
    )
    .expect("match with a binder, motive inferred");
    assert_eq!(repl.execute("Eval (pred (s (s z))).").unwrap().trim(), "(s z)", "pred (s (s z)) = s z");
}

// ===========================================================================
// DEPENDENT MATCH MOTIVE — inferred by abstracting the discriminant
// ===========================================================================

#[test]
fn a_dependent_match_motive_is_inferred_from_the_expected_type() {
    // `P n` is a type that VARIES with n (Cnt at z, Bool at s _). `elim_p : Π(n). P n`
    // matches on n and must return a Cnt in the `z` branch but a Bool in the `s k` branch.
    // That only type-checks with the DEPENDENT motive `λn. P n` — a constant motive would
    // demand both branches share one type. The motive is inferred by abstracting the
    // discriminant `n` out of the expected `P n`.
    let mut repl = Repl::new();
    repl.execute("Inductive Cnt := z : Cnt | s : Cnt -> Cnt.").expect("declare Cnt");
    repl.execute(
        "Definition P : Cnt -> Type := fun n : Cnt => match n with | z => Cnt | s k => Bool end.",
    )
    .expect("declare the dependent type family P");
    repl.execute(
        "Definition elim_p : forall n : Cnt, P n := \
         fun n : Cnt => match n with | z => z | s k => true end.",
    )
    .expect("dependent motive λn. P n inferred — branches z:Cnt, s k:Bool");

    // elim_p z : P z = Cnt ⇝ z ;  elim_p (s z) : P (s z) = Bool ⇝ true.
    assert_eq!(repl.execute("Eval (elim_p z).").unwrap().trim(), "z", "P z = Cnt branch");
    assert_eq!(repl.execute("Eval (elim_p (s z)).").unwrap().trim(), "true", "P (s z) = Bool branch");
}

// ===========================================================================
// LET EXPRESSIONS — `let x := e in body`
// ===========================================================================

#[test]
fn let_expression_binds_a_local() {
    let mut repl = Repl::new();
    // let x := 2 in add x x  ≡  add 2 2  ⇝  4
    assert_eq!(repl.execute("Eval (let x := 2 in add x x).").unwrap().trim(), "4");
    // A `: T` annotation is accepted (and ignored): mul 5 5 ⇝ 25.
    assert_eq!(repl.execute("Eval (let y : Int := 5 in mul y y).").unwrap().trim(), "25");
    // Nested lets.
    assert_eq!(repl.execute("Eval (let a := 3 in let b := 4 in add a b).").unwrap().trim(), "7");
}

#[test]
fn let_in_a_definition_body() {
    let mut repl = Repl::new();
    repl.execute("Definition nine : Int := let three := 3 in mul three three.")
        .expect("let inside a definition body");
    assert_eq!(repl.execute("Eval nine.").unwrap().trim(), "9");
}

// ===========================================================================
// RECURSIVE DEFINITIONS — `Definition f : T := <body that calls f>` (Fix sugar)
// ===========================================================================

#[test]
fn recursive_definition_via_explicit_fix() {
    // Baseline: an explicit `fix` recursive definition computes.
    let mut repl = Repl::new();
    repl.execute(
        "Definition add : Nat -> Nat -> Nat := \
         fix add => fun n : Nat => fun m : Nat => \
         match n with | Zero => m | Succ k => Succ (add k m) end.",
    )
    .expect("explicit-fix recursive add");
    assert_eq!(
        repl.execute("Eval (add (Succ (Succ Zero)) (Succ Zero)).").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "2 + 1 = 3",
    );
}

#[test]
fn recursive_definition_auto_wraps_in_fix() {
    // The sugar: no `fix`, no self-binding — the body just calls `add` by name and the
    // elaborator wraps the definition in a `Fix` (the kernel's termination guard then
    // certifies the recursion structurally decreases on its first argument).
    let mut repl = Repl::new();
    repl.execute(
        "Definition add : Nat -> Nat -> Nat := \
         fun n : Nat => fun m : Nat => \
         match n with | Zero => m | Succ k => Succ (add k m) end.",
    )
    .expect("recursive add with no explicit fix");
    assert_eq!(
        repl.execute("Eval (add (Succ (Succ Zero)) (Succ Zero)).").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "2 + 1 = 3",
    );

    // factorial via the same sugar, reusing the recursive `add`.
    repl.execute(
        "Definition mulAdd : Nat -> Nat -> Nat := \
         fun n : Nat => fun m : Nat => \
         match n with | Zero => Zero | Succ k => add m (mulAdd k m) end.",
    )
    .expect("recursive multiply");
    // mulAdd 2 3 = 3 + (3 + 0) = 6
    assert_eq!(
        repl.execute(
            "Eval (mulAdd (Succ (Succ Zero)) (Succ (Succ (Succ Zero)))).",
        )
        .unwrap()
        .trim(),
        "(Succ (Succ (Succ (Succ (Succ (Succ Zero))))))",
        "2 * 3 = 6",
    );
}

#[test]
fn recursive_definition_rejects_nonterminating_recursion() {
    // The sugar wraps in `fix`, so the kernel's termination guard fires: a recursion that
    // does NOT structurally decrease (no `match` peels the argument) is REJECTED. The sugar
    // cannot smuggle a non-terminating definition past the guard.
    let mut repl = Repl::new();
    let result = repl.execute("Definition loop : Nat -> Nat := fun n : Nat => loop n.");
    assert!(result.is_err(), "non-decreasing recursion must be rejected, got {result:?}");
}

#[test]
fn recursive_definition_is_two_kernel_verified() {
    // A sugar-defined recursive function is an ordinary `Fix` term; the INDEPENDENT de
    // Bruijn re-checker must agree with the main kernel on it (the de Bruijn criterion) —
    // including its own structural-termination guard.
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let mut repl = Repl::new();
    repl.execute(
        "Definition add : Nat -> Nat -> Nat := \
         fun n : Nat => fun m : Nat => \
         match n with | Zero => m | Succ k => Succ (add k m) end.",
    )
    .expect("recursive add");
    let body = repl.context().get_definition_body("add").expect("add is defined").clone();
    match double_check(repl.context(), &body) {
        DoubleCheck::Agreed => {}
        other => panic!("re-checker must agree on the recursive definition, got {other:?}"),
    }
}
