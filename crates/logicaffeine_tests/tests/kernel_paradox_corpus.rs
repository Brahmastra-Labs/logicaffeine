//! The known paradoxes of a kernel of this shape, each written out in full in the kernel's own
//! vernacular and refused.
//!
//! LOGOS's kernel combines rules no other system combines: Coq's guarded `fix` and cumulative
//! universes, Lean's definitionally proof-irrelevant `Prop`, an `SProp`, and Lean's axioms
//! (`propext`, `Classical_choice`, `Quot_sound`). Each rule is sound where it came from; the risk is
//! at the seams, and the documented inconsistencies of other kernels are exactly the seams to test.
//! Every document here would derive a closed proof of `False` if the rule that stops it were
//! missing. Each test pins three things: the well-formed part of the document is accepted (so the
//! refusal is not an incidental type error), the paradoxical step is refused by the rule that must
//! refuse it, and nothing inhabiting `False` reaches the environment.

use logicaffeine_kernel::interface::{Repl, TermParser};
use logicaffeine_kernel::{infer_type, normalize, recheck, MutualInductive, Term, Universe};

fn accepted(repl: &mut Repl, command: &str) {
    if let Err(e) = repl.execute(command) {
        panic!("`{command}` must be accepted: {e}");
    }
}

fn refused(repl: &mut Repl, command: &str) -> String {
    match repl.execute(command) {
        Ok(out) => panic!("`{command}` must be refused, but printed {out:?}"),
        Err(e) => e.to_string(),
    }
}

/// Refused, and by the rule that names itself in `rule`: a refusal for an incidental reason (a parse
/// or inference failure) would leave the paradox untested.
fn refused_by(repl: &mut Repl, command: &str, rule: &str) {
    let refusal = refused(repl, command);
    assert!(
        refusal.contains(rule),
        "`{command}` must be refused by `{rule}`, not: {refusal}"
    );
}

/// Coq 8.4's guard condition took a value cast along an equality of types for a structural subterm
/// (Schepler and Dénès, December 2013). With `propext`, `(False → Tw) = Tw` is provable for
/// `Tw := tw : (False → Tw) → Tw`, since both propositions are inhabited by each other, so the field
/// `f : False → Tw` of `tw f` cast to `Tw` passed as smaller than `tw f`, `con` recursed forever, and
/// `con (tw (λ x. match x with end))` was a closed proof of `False`. Coq closed it by refusing to see
/// through a `match` whose scrutinee is not the recursion's own data; LOGOS's guard admits a `match`
/// as a subterm only when it destructures a recursive parameter (or a value already known smaller).
#[test]
fn a_value_cast_along_propext_is_not_a_subterm_for_the_guard() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Tw : Prop := tw : (False -> Tw) -> Tw.",
    );
    accepted(
        &mut repl,
        "Definition tw_back : Tw -> False -> Tw := fun (t : Tw) => fun (x : False) => t.",
    );
    accepted(
        &mut repl,
        "Definition heq : Eq Prop (False -> Tw) Tw := \
         propext (False -> Tw) Tw (conj ((False -> Tw) -> Tw) (Tw -> False -> Tw) tw tw_back).",
    );
    // The cast itself is well-typed in exactly the position the paradox uses it: the refusal below is
    // the guard's, not a type error.
    accepted(
        &mut repl,
        "Definition peel : Tw -> Tw := fun (x : Tw) => \
         match x return (fun (q : Tw) => Tw) with \
         fun (f : False -> Tw) => Eq_rec Prop (False -> Tw) (fun (T : Prop) => T) f Tw heq end.",
    );
    refused_by(
        &mut repl,
        "Definition con : Tw -> False := fix con => fun (x : Tw) => \
         match x return (fun (q : Tw) => False) with \
         fun (f : False -> Tw) => con (Eq_rec Prop (False -> Tw) (fun (T : Prop) => T) f Tw heq) end.",
        "Termination violation",
    );
    refused(
        &mut repl,
        "Definition boom : False := con (tw (fun (x : False) => match x return (fun (q : False) => Tw) with end)).",
    );
    for name in ["con", "boom"] {
        assert!(
            repl.context().get_global(name).is_none(),
            "`{name}` must never reach the environment"
        );
    }
}

/// Girard's paradox needs a type that holds a copy of its own universe. A constructor whose argument
/// lives in a larger universe than its inductive would be one, directly or through a function.
#[test]
fn a_constructor_cannot_store_a_universe_at_least_its_own() {
    let mut repl = Repl::new();
    refused_by(
        &mut repl,
        "Inductive U : Type := mk : Type -> U.",
        "universe inconsistency",
    );
    refused_by(
        &mut repl,
        "Inductive V : Type := mkv : (Type -> Prop) -> V.",
        "universe inconsistency",
    );
    refused_by(
        &mut repl,
        "Inductive W (A : Type1) : Type := mkw : A -> W A.",
        "universe inconsistency",
    );
    accepted(&mut repl, "Inductive Up : Type1 := mkup : Type -> Up.");
}

/// No universe contains itself, through a definition or a universe-polymorphic instance.
#[test]
fn no_universe_is_a_member_of_itself() {
    let mut repl = Repl::new();
    refused_by(
        &mut repl,
        "Definition self : Type := Type.",
        "expected Type0, found Type1",
    );
    accepted(&mut repl, "Definition Uni.{u} : Sort (u+2) := Sort (u+1).");
    refused_by(
        &mut repl,
        "Definition loop : Uni.{0} := Uni.{0}.",
        "found Type1",
    );
    accepted(&mut repl, "Definition lift : Uni.{1} := Uni.{0}.");
}

/// `Prop` is impredicative, so a proposition may quantify over or store anything; what it may not do
/// is hand data back out. A large elimination of a proposition that is not a subsingleton would
/// recover the stored universe (Coquand's paradox) or, because LOGOS's conversion identifies any two
/// proofs of one proposition, compute two different numbers from equal proofs.
#[test]
fn a_proposition_with_data_or_two_proofs_does_not_eliminate_into_data() {
    let mut repl = Repl::new();
    accepted(&mut repl, "Inductive Bx : Prop := bmk : Type -> Bx.");
    refused_by(
        &mut repl,
        "Definition unbx : Bx -> Type := fun (b : Bx) => match b return (fun (q : Bx) => Type) with fun (T : Type) => T end.",
        "large elimination of proposition 'Bx'",
    );
    accepted(&mut repl, "Inductive PB : Prop := pt : PB | pf : PB.");
    accepted(&mut repl, "Definition same : Eq PB pt pf := refl PB pt.");
    refused_by(
        &mut repl,
        "Definition pick : PB -> Nat := fun (p : PB) => match p return (fun (q : PB) => Nat) with | pt => Zero | pf => Succ Zero end.",
        "large elimination of proposition 'PB'",
    );
    accepted(&mut repl, "Inductive PU : Prop := pu : PU.");
    accepted(
        &mut repl,
        "Definition one : PU -> Nat := fun (p : PU) => match p return (fun (q : PU) => Nat) with | pu => Succ Zero end.",
    );
}

/// Any two inhabitants of an `SProp` are definitionally equal, so an `SProp` eliminates into `SProp`
/// alone: into data or into `Prop`, two constructors would make two different answers equal.
#[test]
fn an_sprop_eliminates_only_into_sprop() {
    let mut repl = Repl::new();
    accepted(&mut repl, "Inductive SB : SProp := st : SB | sf : SB.");
    refused_by(
        &mut repl,
        "Definition spick : SB -> Nat := fun (s : SB) => match s return (fun (q : SB) => Nat) with | st => Zero | sf => Succ Zero end.",
        "definitionally-irrelevant proposition 'SB'",
    );
    refused_by(
        &mut repl,
        "Definition sprop_split : SB -> Prop := fun (s : SB) => match s return (fun (q : SB) => Prop) with | st => True | sf => False end.",
        "definitionally-irrelevant proposition 'SB'",
    );
}

/// Coquand's paradox for an impredicative `Set` with large elimination: a type stored in an
/// impredicative sort and read back out makes that sort a universe of every type, and Girard's paradox
/// follows. `SProp` is impredicative and stores arguments of any sort, as Rocq's does (`Squash`); it
/// stays sound because it eliminates into `SProp` alone, so a stored type never comes back out.
#[test]
fn a_type_stored_in_an_sprop_never_comes_back_out() {
    let mut repl = Repl::new();
    accepted(&mut repl, "Inductive SBig : SProp := sbig : Type -> SBig.");
    accepted(&mut repl, "Definition packed : SBig := sbig Nat.");
    accepted(
        &mut repl,
        "Definition repacked : SBig -> SBig := fun (s : SBig) => \
         match s return (fun (q : SBig) => SBig) with fun (T : Type) => sbig (T -> T) end.",
    );
    refused_by(
        &mut repl,
        "Definition carrier : SBig -> Type := fun (s : SBig) => \
         match s return (fun (q : SBig) => Type) with fun (T : Type) => T end.",
        "definitionally-irrelevant proposition 'SBig'",
    );
    assert!(
        repl.context().get_global("carrier").is_none(),
        "`carrier` must never reach the environment"
    );
}

/// Russell's paradox as an inductive: a type whose constructor consumes a function out of the type.
/// Strict positivity refuses it, written directly or nested inside a container.
#[test]
fn a_type_never_occurs_left_of_an_arrow_in_its_own_constructor() {
    let mut repl = Repl::new();
    refused_by(
        &mut repl,
        "Inductive Bad : Type := mk : (Bad -> False) -> Bad.",
        "Positivity violation",
    );
    refused_by(
        &mut repl,
        "Inductive Bad2 : Type := mk2 : TList (Bad2 -> False) -> Bad2.",
        "through container 'TList'",
    );
    accepted(
        &mut repl,
        "Inductive Good : Type := mkg : TList Good -> Good.",
    );
}

/// A fixpoint that calls itself on an argument no smaller than its own proves anything; the guard
/// refuses it. The `match` states the result type, so the call itself is the only thing at fault, and
/// the same shape recursing on the predecessor is accepted.
#[test]
fn a_fixpoint_without_a_decreasing_argument_is_refused() {
    let mut repl = Repl::new();
    refused_by(
        &mut repl,
        "Definition spin : Nat -> False := fix spin => fun (n : Nat) => \
         match n return (fun (q : Nat) => False) with | Zero => spin Zero | Succ k => spin n end.",
        "Termination violation",
    );
    accepted(
        &mut repl,
        "Definition down : Nat -> Nat := fix down => fun (n : Nat) => \
         match n return (fun (q : Nat) => Nat) with | Zero => Zero | Succ k => down k end.",
    );
}

/// LOGOS's own guard rule, the refutation refinement, reads `¬(x = C)` off the `isFalse` branch of a
/// kernel-checked decision procedure and drops `C` from the subterm meet: at `x ↝ C` the branch is
/// unreachable, because the procedure would then reduce to a closed proof of `¬(C = C)`. That holds
/// only if the discriminant is closed once `x` is `C`. With another open argument in it, here a
/// hypothesis `e : False`, the procedure answers `isFalse (λh. e)` at `Zero` without inspecting `e`,
/// the branch is reached, and `λ(e : False). spinz Zero e` reduces forever: a fixpoint the guard
/// admitted does not terminate. The same shape over a decision closed but for `n` is the
/// `decreasing_by` capability and stays accepted.
#[test]
fn a_refinement_needs_the_decision_closed_but_for_its_refined_parameter() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Definition pred0 : Nat -> Nat := fun (n : Nat) => \
         match n return (fun (q : Nat) => Nat) with | Zero => Zero | Succ k => k end.",
    );
    accepted(
        &mut repl,
        "Definition dz : forall (n : Nat) (e : False), Decidable (Eq Nat n Zero) := \
         fun (n : Nat) (e : False) => isFalse (Eq Nat n Zero) (fun (h : Eq Nat n Zero) => e).",
    );
    refused_by(
        &mut repl,
        "Definition spinz : Nat -> False -> Nat := fix spinz => fun (n : Nat) (e : False) => \
         match dz n e return (fun (q : Decidable (Eq Nat n Zero)) => Nat) with \
         | isTrue hp => Zero | isFalse hn => Succ (spinz (pred0 n) e) end.",
        "Termination violation",
    );
    accepted(
        &mut repl,
        "Definition down : Nat -> Nat := fix down => fun (n : Nat) => \
         match decEqNat n Zero return (fun (q : Decidable (Eq Nat n Zero)) => Nat) with \
         | isTrue hp => Zero | isFalse hn => Succ (down (pred0 n)) end.",
    );
}

/// A universe-polymorphic family whose sort may be `Prop` (`Sort u`, unlike `Sort (u+1)`) is a
/// proposition at `u := 0`. A match on it into data, checked with `u` abstract, meets a sort that is
/// not literally `Prop`; at `u := 0` proof irrelevance equates its two constructors, so
/// `f.{0} t1.{0} ≡ f.{0} t2.{0}`, which is `Zero ≡ Succ Zero`. Lean gives such a family a recursor
/// into `Prop` only. LOGOS refuses the elimination unless every instance at which the family is a
/// proposition also eliminates into `Prop`: the motive's level is at most the family's.
#[test]
fn a_family_that_may_be_a_proposition_does_not_eliminate_into_data() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    refused_by(
        &mut repl,
        "Definition f.{u} : Two.{u} -> Nat := fun (x : Two.{u}) => \
         match x return (fun (q : Two.{u}) => Nat) with Zero | Succ Zero end.",
        "large elimination",
    );
    refused_by(
        &mut repl,
        "Definition sel.{u} : Two.{u} -> Type := fun (x : Two.{u}) => \
         match x return (fun (q : Two.{u}) => Type) with Nat | Bool end.",
        "large elimination",
    );
    refused(
        &mut repl,
        "Definition zero_is_one : Eq Nat (f.{0} t1.{0}) (f.{0} t2.{0}) := refl Nat (f.{0} t1.{0}).",
    );
    // At its own level the family eliminates into itself, which at `u := 0` is Prop into Prop.
    accepted(
        &mut repl,
        "Definition swap.{u} : Two.{u} -> Two.{u} := fun (x : Two.{u}) => \
         match x return (fun (q : Two.{u}) => Two.{u}) with t2.{u} | t1.{u} end.",
    );
    // Into `SProp` the results are irrelevant whichever case computes them.
    accepted(&mut repl, "Inductive ST : SProp := sti : ST.");
    accepted(
        &mut repl,
        "Definition into_sprop.{u} : Two.{u} -> ST := fun (x : Two.{u}) => \
         match x return (fun (q : Two.{u}) => ST) with sti | sti end.",
    );
    // A subsingleton eliminates into anything, and so does a family that is never a proposition.
    accepted(&mut repl, "Inductive One.{u} : Sort u := o : One.{u}.");
    accepted(
        &mut repl,
        "Definition one.{u} : One.{u} -> Nat := fun (x : One.{u}) => \
         match x return (fun (q : One.{u}) => Nat) with Zero end.",
    );
    accepted(
        &mut repl,
        "Inductive Big.{u} : Sort (u+1) := b1 : Big.{u} | b2 : Big.{u}.",
    );
    accepted(
        &mut repl,
        "Definition big.{u} : Big.{u} -> Nat := fun (x : Big.{u}) => \
         match x return (fun (q : Big.{u}) => Nat) with Zero | Succ Zero end.",
    );
    for name in ["f", "sel", "zero_is_one"] {
        assert!(
            repl.context().get_global(name).is_none(),
            "`{name}` must never reach the environment"
        );
    }
}

/// The same seam through an indexed family: the indexed `match` rule decides large elimination by
/// its own path and must read the family's level the same way.
#[test]
fn an_indexed_family_that_may_be_a_proposition_does_not_eliminate_into_data() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive TwoI.{u} : Nat -> Sort u := i1 : TwoI.{u} Zero | i2 : TwoI.{u} Zero.",
    );
    refused_by(
        &mut repl,
        "Definition fi.{u} : TwoI.{u} Zero -> Nat := fun (x : TwoI.{u} Zero) => \
         match x return (fun (n : Nat) (q : TwoI.{u} n) => Nat) with Zero | Succ Zero end.",
        "large elimination",
    );
    accepted(
        &mut repl,
        "Definition swapi.{u} : TwoI.{u} Zero -> TwoI.{u} Zero := fun (x : TwoI.{u} Zero) => \
         match x return (fun (n : Nat) (q : TwoI.{u} n) => TwoI.{u} n) with i2.{u} | i1.{u} end.",
    );
    assert!(
        repl.context().get_global("fi").is_none(),
        "`fi` must never reach the environment"
    );
}

/// `recheck` decides large elimination by its own reading of the rule; both kernels refuse, so
/// `double_check` cannot be satisfied by two kernels that share the gap.
#[test]
fn the_independent_kernel_refuses_a_polymorphic_large_elimination_too() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    accepted(
        &mut repl,
        "Inductive TwoI.{u} : Nat -> Sort u := i1 : TwoI.{u} Zero | i2 : TwoI.{u} Zero.",
    );
    let ctx = repl.context();
    let refused_terms = [
        "fun (x : Two.{u}) => match x return (fun (q : Two.{u}) => Nat) with Zero | Succ Zero end",
        "fun (x : TwoI.{u} Zero) => match x return (fun (n : Nat) (q : TwoI.{u} n) => Nat) with Zero | Succ Zero end",
    ];
    for text in refused_terms {
        let t = TermParser::parse(text).expect("parses");
        let main = infer_type(ctx, &t);
        assert!(
            main.is_err(),
            "the main kernel must refuse `{text}`, not type it as {main:?}"
        );
        match recheck(ctx, &t) {
            Err(e) => assert!(
                format!("{e:?}").contains("large elimination"),
                "refused by the wrong rule: {e:?}"
            ),
            Ok(ty) => panic!("recheck must refuse `{text}`, not type it as {ty}"),
        }
    }
    let swap = TermParser::parse(
        "fun (x : Two.{u}) => match x return (fun (q : Two.{u}) => Two.{u}) with t2.{u} | t1.{u} end",
    )
    .expect("parses");
    assert!(
        infer_type(ctx, &swap).is_ok() && recheck(ctx, &swap).is_ok(),
        "the elimination into the family's own level stands"
    );
}

/// A `match` on an indexed family owes no case for a constructor whose declared result index
/// clashes with the scrutinee's, and the only ground for a clash is constructor disjointness, which
/// holds between values and not between proofs: two constructors of a proposition are equal proofs.
/// A family in `Sort u` is a proposition at `u := 0`. So `t1` and `t2` of `Two.{u}` are no ground for
/// a clash: `Idx.{0} t2.{0}` is inhabited by `mk1.{0}`, because `t1.{0} ≡ t2.{0}`, and a match on it
/// owing no case would turn that inhabitant into a proof of `False`. Coverage reads a constructor
/// written with its levels (`t1.{0}`) as the constructor it is, so the guard on the family's sort
/// is what refuses this route and the one below (an arity through a definition): with the guard
/// gone, each is a proof of `False`.
#[test]
fn constructors_of_a_family_that_may_be_a_proposition_never_clash() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    accepted(
        &mut repl,
        "Inductive Idx.{u} : Two.{u} -> Type := mk1 : Idx.{u} t1.{u}.",
    );
    accepted(&mut repl, "Definition witness : Idx.{0} t2.{0} := mk1.{0}.");
    let empty = "fun (x : Idx.{0} t2.{0}) => match x return (fun (i : Two.{0}) (q : Idx.{0} i) => False) with end";
    refused_by(
        &mut repl,
        &format!("Definition absurd_at_prop : Idx.{{0}} t2.{{0}} -> False := {empty}."),
        "Wrong number of cases",
    );
    let t = TermParser::parse(empty).expect("parses");
    let independent = recheck(repl.context(), &t);
    assert!(
        independent.is_err(),
        "recheck must refuse the empty match too, not type it as {independent:?}"
    );
    let poly = "fun (x : Idx.{u} t2.{u}) => match x return (fun (i : Two.{u}) (q : Idx.{u} i) => False) with end";
    let t = TermParser::parse(poly).expect("parses");
    assert!(
        infer_type(repl.context(), &t).is_err(),
        "nor at a level that may be zero"
    );
    assert!(
        recheck(repl.context(), &t).is_err(),
        "nor in the independent kernel at a level that may be zero"
    );
}

/// The same seam through an arity written as a definition: `Two2 : MyProp`, with `MyProp := Prop`,
/// is a proposition whose arity does not spell `Prop`, and its constructors are equal proofs all the
/// same.
#[test]
fn constructors_of_a_family_whose_arity_is_prop_through_a_definition_never_clash() {
    let mut repl = Repl::new();
    accepted(&mut repl, "Definition MyProp : Type := Prop.");
    accepted(
        &mut repl,
        "Inductive Two2 : MyProp := s1 : Two2 | s2 : Two2.",
    );
    accepted(&mut repl, "Inductive Idx2 : Two2 -> Type := mk2 : Idx2 s1.");
    accepted(&mut repl, "Definition witness2 : Idx2 s2 := mk2.");
    let empty =
        "fun (x : Idx2 s2) => match x return (fun (i : Two2) (q : Idx2 i) => False) with end";
    refused_by(
        &mut repl,
        &format!("Definition absurd2 : Idx2 s2 -> False := {empty}."),
        "Wrong number of cases",
    );
    let t = TermParser::parse(empty).expect("parses");
    let independent = recheck(repl.context(), &t);
    assert!(
        independent.is_err(),
        "recheck must refuse the empty match too, not type it as {independent:?}"
    );
}

/// The same seam through bare references is closed at the declaration: a family indexed by the
/// polymorphic `Two` with a constructor written against its bare constructor `t1` does not type,
/// because a bare reference to a universe-polymorphic constant carries its own level variables
/// (`Two.{u}`), which the bare family `Two` in the arity is not. With no such family there is no
/// `Idx t2` to read at `u := 0`.
#[test]
fn a_family_written_against_bare_references_to_a_polymorphic_one_is_refused() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    let g = |n: &str| Term::Global(n.to_string());
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let pi = |p: &str, t: Term, b: Term| Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    };
    let mut ctx = repl.context().clone();
    let declared = ctx.add_universe_poly_inductives(
        &["u".to_string()],
        &[MutualInductive {
            name: "Idx".to_string(),
            sort: pi("_", g("Two"), Term::Sort(Universe::Type(0))),
            num_params: 0,
            constructors: vec![("mk1".to_string(), app(g("Idx"), g("t1")))],
        }],
    );
    assert!(
        declared.is_err(),
        "a constructor at a bare `t1` is no constructor of a family over the bare `Two`"
    );
    assert!(
        !ctx.is_inductive("Idx"),
        "and nothing of the refused block is registered"
    );
}

/// A family written over `Sort u` with no level telescope (`B2 : Sort u`) is as universe-polymorphic
/// as `B2.{u}`, and a bare reference to it that is not an application's head stands for it at its
/// own level, `B2.{u}`, which the declaration around the reference binds. Were that level a second
/// `u`, one the declaration's own `u` meets only while the declaration is checked, `P2 : Sort u :=
/// B2` would be checked at `u = u` and used apart: `P2.{0}` a proposition whose proofs are `B2`'s
/// two constructors, so proof irrelevance proves them equal, and the derived `B2_rec.{1}` an
/// elimination of `B2` into data that tells them apart, so the two together prove `False`.
#[test]
fn a_bare_reference_carries_its_level_into_the_declaration_around_it() {
    let mut repl = Repl::new();
    accepted(&mut repl, "Inductive B2 : Sort u := B2.t : B2 | B2.f : B2.");
    accepted(&mut repl, "Definition P2 : Sort u := B2.");
    accepted(
        &mut repl,
        "Definition P3 : Sort u := (fun (A : Sort u) => A) B2.",
    );
    accepted(
        &mut repl,
        "Definition pirr : forall (P : Prop) (p : P) (q : P), Eq P p q := \
         fun (P : Prop) => fun (p : P) => fun (q : P) => refl P p.",
    );
    // `P2.{0}` is `B2.{0}`, where the two constructors are equal proofs, and `B2.{1}` eliminates
    // into `Prop`: each half of the paradox stands at a level of its own.
    let p2_at_prop = normalize(
        repl.context(),
        &TermParser::parse("P2.{0}").expect("parses"),
    );
    assert_eq!(
        p2_at_prop,
        Term::Const {
            name: "B2".to_string(),
            levels: vec![Universe::Prop],
        },
        "`P2.{{0}}` unfolds to the family at the level `P2` is instantiated at"
    );
    let p3_at_prop = normalize(
        repl.context(),
        &TermParser::parse("P3.{0}").expect("parses"),
    );
    assert_eq!(
        p3_at_prop, p2_at_prop,
        "and so does a reference in an argument's place"
    );
    accepted(
        &mut repl,
        "Definition e0 : Eq P2.{0} B2.t.{0} B2.f.{0} := pirr P2.{0} B2.t.{0} B2.f.{0}.",
    );
    accepted(
        &mut repl,
        "Definition D1 : B2.{1} -> Prop := B2_rec.{1} (fun (x : B2.{1}) => Prop) True False.",
    );
    // Neither half reaches the other's level through a bare reference.
    refused_by(
        &mut repl,
        "Definition e_bare : Eq P2.{0} B2.t B2.f := pirr P2.{0} B2.t B2.f.",
        "Type mismatch",
    );
    refused_by(
        &mut repl,
        "Definition D_bare : B2 -> Prop := B2_rec.{1} (fun (x : B2) => Prop) True False.",
        "Type mismatch",
    );
    refused(
        &mut repl,
        "Definition bad : False := Eq_rec P2.{0} B2.t (fun (x : P2.{0}) => D_bare x) I B2.f e_bare.",
    );
    refused(
        &mut repl,
        "Definition bad1 : False := Eq_rec P2.{0} B2.t.{0} (fun (x : P2.{0}) => D1 x) I B2.f.{0} e0.",
    );
    for name in ["e_bare", "D_bare", "bad", "bad1"] {
        assert!(
            repl.context().get_global(name).is_none(),
            "`{name}` must never reach the environment"
        );
    }
    let ctx = repl.context();
    let at_prop = TermParser::parse("pirr P2.{0} B2.t B2.f").expect("parses");
    assert!(
        recheck(ctx, &at_prop).is_err(),
        "the independent kernel reads the bare constructors at their own level too"
    );
    let motive = TermParser::parse("B2_rec.{1} (fun (x : B2) => Prop)").expect("parses");
    assert!(
        infer_type(ctx, &motive).is_err() && recheck(ctx, &motive).is_err(),
        "`B2_rec.{{1}}` eliminates `B2.{{1}}`, not the bare `B2`"
    );
}

/// `SProp` is a sort and never a universe level: a level variable ranges over `Prop` and the `Type`
/// levels. An instance at `SProp` would make `Two.{SProp}` a definitionally irrelevant family whose
/// polymorphic eliminations were checked as if it were relevant, so both kernels refuse one.
#[test]
fn no_universe_level_is_sprop() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    let ctx = repl.context();
    for name in ["Two", "t1"] {
        let t = Term::Const {
            name: name.to_string(),
            levels: vec![Universe::SProp],
        };
        let main = infer_type(ctx, &t);
        assert!(
            main.is_err(),
            "the main kernel must refuse `{name}.{{SProp}}`, not type it as {main:?}"
        );
        let independent = recheck(ctx, &t);
        assert!(
            independent.is_err(),
            "recheck must refuse `{name}.{{SProp}}`, not type it as {independent:?}"
        );
    }
    let prop = Term::Const {
        name: "Two".to_string(),
        levels: vec![Universe::Prop],
    };
    assert!(
        infer_type(ctx, &prop).is_ok() && recheck(ctx, &prop).is_ok(),
        "`Two.{{0}}` is a proposition, and well formed"
    );
}

/// A bare polymorphic reference applied to a type in `SProp` solves its level from that type's
/// sort, and the least LEVEL at or above `SProp` is `Prop`: the application stands, through
/// cumulativity, at `Prop`, and never instantiates anything at `SProp`.
#[test]
fn an_sprop_argument_solves_a_level_at_prop() {
    let mut repl = Repl::new();
    accepted(
        &mut repl,
        "Definition idk.{u} : forall (A : Sort u), A -> A := fun (A : Sort u) (a : A) => a.",
    );
    accepted(&mut repl, "Inductive ST : SProp := sti : ST.");
    accepted(&mut repl, "Definition use_idk : ST := idk ST sti.");
    let body = repl
        .context()
        .get_definition_body("use_idk")
        .expect("registered")
        .to_string();
    assert!(
        !body.contains("SProp"),
        "the instance is solved at Prop, not SProp: {body}"
    );
    accepted(
        &mut repl,
        "Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}.",
    );
    accepted(
        &mut repl,
        "Definition mk2.{u} : forall (A : Sort u), Two.{u} := fun (A : Sort u) => t1.{u}.",
    );
    accepted(&mut repl, "Definition two_at_prop : Two.{0} := mk2 ST.");
}

/// The K-like rule reads a proof of `a = b` as `refl a` only when `refl a` has the proof's type,
/// which needs `a ≡ b`, as Lean's rule does. So `cast` computes along a hypothesis `h : A = A` and
/// stays stuck along `h : A = B`: computing there would move a value across a type it is not, the
/// step Abel and Coquand's non-normalizing term takes in a theory with definitional proof
/// irrelevance and an impredicative sort.
#[test]
fn a_cast_computes_only_along_an_equation_whose_sides_convert() {
    let mut repl = Repl::new();
    for command in [
        "Axiom A : Type0.",
        "Axiom B : Type0.",
        "Axiom x : A.",
        "Axiom same : Eq Type0 A A.",
        "Axiom other : Eq Type0 A B.",
    ] {
        accepted(&mut repl, command);
    }
    let ctx = repl.context();
    let x = Term::Global("x".to_string());
    let along_same = normalize(ctx, &TermParser::parse("cast A A same x").expect("parses"));
    assert_eq!(
        along_same, x,
        "along `same : A = A` the cast is the identity"
    );
    let along_other = normalize(ctx, &TermParser::parse("cast A B other x").expect("parses"));
    assert_ne!(
        along_other, x,
        "along `other : A = B` the cast must not compute: {along_other}"
    );
}
