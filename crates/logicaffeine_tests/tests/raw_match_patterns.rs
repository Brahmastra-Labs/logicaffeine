use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::{double_check, DoubleCheck};

#[test]
fn every_order_of_three_constructors_computes_the_written_function() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut repl = Repl::new();
        repl.execute("A Hue is either Ruby or Jade or Slate.")
            .unwrap();
        let names = ["Ruby", "Jade", "Slate"];
        let clauses = order
            .iter()
            .map(|&i| format!("| {} => {}", names[i], i + 10))
            .collect::<Vec<_>>()
            .join(" ");
        repl.execute(&format!(
            "Definition shade : Hue -> Int := fun h : Hue => match h with {clauses} end."
        ))
        .unwrap();
        for (i, name) in names.iter().enumerate() {
            assert_eq!(
                repl.execute(&format!("Eval shade {name}.")).unwrap(),
                (i + 10).to_string()
            );
        }
        assert_eq!(
            double_check(
                repl.context(),
                repl.context().get_definition_body("shade").unwrap()
            ),
            DoubleCheck::Agreed
        );
    }
}

#[test]
fn nested_patterns_and_wildcards_use_first_match_semantics() {
    let mut repl = Repl::new();
    repl.execute("Definition classify : Nat -> Int := fun n : Nat => match n with | Zero => 10 | Succ (Succ k) => 30 | _ => 20 end.").unwrap();
    for (n, expected) in [
        ("Zero", "10"),
        ("(Succ Zero)", "20"),
        ("(Succ (Succ Zero))", "30"),
    ] {
        assert_eq!(
            repl.execute(&format!("Eval classify {n}.")).unwrap(),
            expected
        );
    }
}

#[test]
fn a_whole_value_pattern_binds_its_name_and_shadows_an_outer_name() {
    let mut repl = Repl::new();
    repl.execute("Definition keep : Nat -> Nat := fun n : Nat => match n with | k => k end.")
        .unwrap();
    assert_eq!(
        repl.execute("Eval keep (Succ Zero).").unwrap(),
        "(Succ Zero)"
    );
    repl.execute("Definition shadow : Nat -> Nat -> Nat := fun k : Nat => fun n : Nat => match n with | Succ k => k | Zero => k end.").unwrap();
    assert_eq!(
        repl.execute("Eval shadow (Succ (Succ Zero)) (Succ Zero).")
            .unwrap(),
        "Zero"
    );
    assert_eq!(
        repl.execute("Eval shadow (Succ (Succ Zero)) Zero.")
            .unwrap(),
        "(Succ (Succ Zero))"
    );
}

#[test]
fn missing_cases_and_wrong_constructor_families_are_diagnosed() {
    let mut repl = Repl::new();
    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .unwrap();
    let missing = repl.execute("Definition bad : Hue -> Int := fun h : Hue => match h with | Ruby => 1 | Jade => 2 end.").unwrap_err().to_string();
    assert!(missing.contains("Slate"), "{missing}");
    let wrong = repl.execute("Definition wrong : Hue -> Int := fun h : Hue => match h with | Ruby => 1 | Jade => 2 | Zero => 3 end.").unwrap_err().to_string();
    assert!(wrong.contains("Zero") && wrong.contains("Hue"), "{wrong}");
    assert!(repl.context().get_global("bad").is_none());
    assert!(repl.context().get_global("wrong").is_none());
}

#[test]
fn nested_missing_cases_are_not_silently_filled() {
    let mut repl = Repl::new();
    let error = repl.execute("Definition partial : Nat -> Int := fun n : Nat => match n with | Zero => 0 | Succ (Succ k) => 2 end.").unwrap_err().to_string();
    assert!(error.contains("Succ") && error.contains("Zero"), "{error}");
}

#[test]
fn namespace_resolution_preserves_constructor_labels_and_local_pattern_variables() {
    let mut repl = Repl::new();
    repl.execute("Namespace Paint.").unwrap();
    repl.execute("A Hue is either Ruby or Jade.").unwrap();
    repl.execute(
        "Definition tag : Hue -> Int := fun h : Hue => match h with | Jade => 7 | Ruby => 9 end.",
    )
    .unwrap();
    repl.execute("End Paint.").unwrap();
    assert_eq!(repl.execute("Eval Paint.tag Paint.Ruby.").unwrap(), "9");
    assert_eq!(
        repl.execute("Eval match Paint.Jade with | Paint.Ruby => 9 | Paint.Jade => 7 end.")
            .unwrap(),
        "7"
    );
}

#[test]
fn repeated_pattern_binders_are_refused() {
    let mut repl = Repl::new();
    repl.execute("Inductive Pair := PairMk : Nat -> Nat -> Pair.")
        .unwrap();
    let error = repl.execute("Definition duplicate : Pair -> Nat := fun p : Pair => match p with | PairMk x x => x end.").unwrap_err().to_string();
    assert!(
        error.contains("x") && error.contains("more than once"),
        "{error}"
    );
}

#[test]
fn numeral_patterns_and_an_early_wildcard_keep_source_priority() {
    let mut repl = Repl::new();
    repl.execute("Definition digit : Nat -> Int := fun n : Nat => match n with | 0 => 8 | 1 => 9 | _ => 10 end.").unwrap();
    assert_eq!(repl.execute("Eval digit (Succ Zero).").unwrap(), "9");
    assert_eq!(
        repl.execute("Eval digit (Succ (Succ Zero)).").unwrap(),
        "10"
    );
    repl.execute("Definition first : Nat -> Int := fun n : Nat => match n with | _ => 4 | Zero => 5 | Succ k => 6 end.").unwrap();
    assert_eq!(repl.execute("Eval first Zero.").unwrap(), "4");
    assert_eq!(repl.execute("Eval first (Succ Zero).").unwrap(), "4");
}

#[test]
fn constructor_names_in_a_body_are_not_pattern_variables() {
    let mut repl = Repl::new();
    repl.execute("Namespace Colors.").unwrap();
    repl.execute("A Hue is either Ruby or Jade.").unwrap();
    repl.execute("Definition keepHue : Hue -> Hue := fun h : Hue => match h with | Jade => Jade | Ruby => Ruby end.").unwrap();
    repl.execute("End Colors.").unwrap();
    assert_eq!(
        repl.execute("Eval Colors.keepHue Colors.Jade.").unwrap(),
        "Colors.Jade"
    );
}

#[test]
fn dependent_branch_types_follow_labels_in_reversed_order() {
    let mut repl = Repl::new();
    repl.execute("Inductive Count := zz : Count | ss : Count -> Count.")
        .unwrap();
    repl.execute("Definition Family : Count -> Type := fun n : Count => match n with | ss k => Bool | zz => Count end.").unwrap();
    repl.execute("Definition inhabit : forall n : Count, Family n := fun n : Count => match n with | ss k => true | zz => zz end.").unwrap();
    assert_eq!(repl.execute("Eval inhabit zz.").unwrap(), "zz");
    assert_eq!(repl.execute("Eval inhabit (ss zz).").unwrap(), "true");
}

#[test]
fn nested_match_bodies_see_pattern_variables_with_their_field_types() {
    let mut repl = Repl::new();
    repl.execute("Definition depth : Nat -> Int := fun n : Nat => match n with | Zero => 0 | Succ k => (match k with | Zero => 1 | Succ rest => 2 end) end.").unwrap();
    assert_eq!(repl.execute("Eval depth (Succ Zero).").unwrap(), "1");
    assert_eq!(repl.execute("Eval depth (Succ (Succ Zero)).").unwrap(), "2");
}

#[test]
fn parametric_nested_patterns_instantiate_each_constructor_telescope() {
    let mut repl = Repl::new();
    repl.execute("Definition second : TList Nat -> Nat := fun xs : TList Nat => match xs with | TCons a (TCons b rest) => b | _ => Zero end.").unwrap();
    assert_eq!(
        repl.execute("Eval second (TCons Nat Zero (TCons Nat (Succ Zero) (TNil Nat))).")
            .unwrap(),
        "(Succ Zero)"
    );
}

#[test]
fn a_nested_body_binder_is_not_removed_with_pattern_metadata() {
    let mut repl = Repl::new();
    repl.execute("Definition choose : Nat -> Nat -> Nat := fun n : Nat => match n with | Zero => (fun Zero : Nat => Zero) | Succ k => (fun x : Nat => k) end.").unwrap();
    assert_eq!(
        repl.execute("Eval choose Zero (Succ Zero).").unwrap(),
        "(Succ Zero)"
    );
    assert_eq!(
        repl.execute("Eval choose (Succ Zero) (Succ Zero).")
            .unwrap(),
        "Zero"
    );
}

#[test]
fn an_explicit_motive_may_be_a_declared_type_family() {
    use logicaffeine_kernel::{Term, Universe};
    fn g(n: &str) -> Term {
        Term::Global(n.into())
    }
    fn app(f: Term, x: Term) -> Term {
        Term::App(Box::new(f), Box::new(x))
    }
    fn pi(n: &str, a: Term, b: Term) -> Term {
        Term::Pi {
            param: n.into(),
            param_type: Box::new(a),
            body_type: Box::new(b),
        }
    }
    let mut repl = Repl::new();
    let ctx = repl.context_mut();
    ctx.add_declaration("Fam", pi("n", g("Nat"), Term::Sort(Universe::Type(0))));
    ctx.add_declaration("atZero", app(g("Fam"), g("Zero")));
    ctx.add_declaration(
        "atSucc",
        pi(
            "k",
            g("Nat"),
            app(g("Fam"), app(g("Succ"), Term::Var("k".into()))),
        ),
    );
    repl.execute("Definition allFam : forall n : Nat, Fam n := fun n : Nat => match n return Fam with | Succ k => atSucc k | Zero => atZero end.").unwrap();
    // …and it is REGISTERED at that type, checked by both kernels (a refusal the door recorded
    // instead of returning would leave `allFam` unbound).
    let ctx = repl.context();
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    let body = ctx
        .get_definition_body("allFam")
        .expect("allFam is defined")
        .clone();
    let declared = pi("n", g("Nat"), app(g("Fam"), Term::Var("n".into())));
    let inferred = logicaffeine_kernel::infer_type(ctx, &body).unwrap();
    assert!(
        logicaffeine_kernel::defeq_for_test(ctx, &inferred, &declared),
        "{inferred}"
    );
    assert_eq!(
        logicaffeine_kernel::double_check(ctx, &body),
        logicaffeine_kernel::DoubleCheck::Agreed
    );
    // A branch at the wrong fibre is still refused: `Zero => atSucc Zero` has type `Fam 1`.
    assert!(repl.execute("Definition wrongFam : forall n : Nat, Fam n := fun n : Nat => match n return Fam with | Succ k => atSucc k | Zero => atSucc Zero end.").is_err());
}

#[test]
fn a_dependent_branch_may_ignore_a_constructor_field() {
    let mut repl = Repl::new();
    repl.execute("Definition NatFamily : Nat -> Type := fun n : Nat => match n with | Zero => Nat | Succ k => Bool end.").unwrap();
    repl.execute("Definition inhabitNat : forall n : Nat, NatFamily n := fun n : Nat => match n with | Succ _ => true | Zero => Zero end.").unwrap();
    assert_eq!(
        repl.execute("Eval inhabitNat (Succ Zero).").unwrap(),
        "true"
    );
}

#[test]
fn indexed_matches_keep_index_binders_separate_from_parameters() {
    use logicaffeine_kernel::{Term, Universe};
    fn g(n: &str) -> Term {
        Term::Global(n.into())
    }
    fn app(f: Term, x: Term) -> Term {
        Term::App(Box::new(f), Box::new(x))
    }
    fn pi(n: &str, a: Term, b: Term) -> Term {
        Term::Pi {
            param: n.into(),
            param_type: Box::new(a),
            body_type: Box::new(b),
        }
    }
    let mut repl = Repl::new();
    let ctx = repl.context_mut();
    ctx.add_indexed_inductive("Ix", pi("n", g("Nat"), Term::Sort(Universe::Type(0))), 0);
    ctx.add_constructor("ZI", "Ix", app(g("Ix"), g("Zero")));
    ctx.add_constructor(
        "SI",
        "Ix",
        pi(
            "k",
            g("Nat"),
            app(g("Ix"), app(g("Succ"), Term::Var("k".into()))),
        ),
    );
    repl.execute("Definition indexedTag : forall n : Nat, Ix n -> Int := fun n : Nat => fun x : Ix n => match x return (fun i : Nat => fun e : Ix i => Int) with | SI k => 1 | ZI => 0 end.").unwrap();
    assert_eq!(repl.execute("Eval indexedTag Zero ZI.").unwrap(), "0");
    assert_eq!(
        repl.execute("Eval indexedTag (Succ Zero) (SI Zero).")
            .unwrap(),
        "1"
    );
}
