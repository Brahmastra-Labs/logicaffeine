//! ★ UNIVERSE PARAMETERS HAVE A SPELLING — `Sort ℓ`, `max`, `imax`, `ℓ+k`, `name.{ℓ, …}`,
//! `Definition name.{u, v} …`, `Inductive Name.{u} … : arity := … with …`.
//!
//! The kernel has held universe-polymorphic constants (`Term::Const { name, levels }`,
//! `declare_universe_poly`, `add_universe_poly_inductives`) for a long time; until this surface grew,
//! nothing under `interface/` ever PRODUCED one, so a polymorphic declaration could be built only in
//! Rust. Everything here reads a level STRUCTURALLY — `u+1` is `Succ(Var u)`, `1` is `Type 0`,
//! `0` is `Prop` — so the printer can be the parser's exact inverse, and every command is checked by
//! the `Repl` through the same kernel door the Lean importer uses.

use logicaffeine_kernel::interface::printer::{print_command, print_term};
use logicaffeine_kernel::interface::{parse_command, Command, Repl, TermParser};
use logicaffeine_kernel::{Context, MutualInductive, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(p: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(p: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn uvar(n: &str) -> Universe {
    Universe::Var(n.to_string())
}
fn succ(u: Universe) -> Universe {
    Universe::Succ(Box::new(u))
}
fn umax(a: Universe, b: Universe) -> Universe {
    Universe::Max(Box::new(a), Box::new(b))
}
fn uimax(a: Universe, b: Universe) -> Universe {
    Universe::IMax(Box::new(a), Box::new(b))
}
fn konst(n: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: n.to_string(),
        levels,
    }
}

/// Structural α-equality (binder names aside), enough for the closed terms in this file.
fn alpha_eq(a: &Term, b: &Term) -> bool {
    fn go(a: &Term, b: &Term, env: &mut Vec<(String, String)>) -> bool {
        match (a, b) {
            (Term::Var(x), Term::Var(y)) => {
                match (
                    env.iter().rposition(|(l, _)| l == x),
                    env.iter().rposition(|(_, r)| r == y),
                ) {
                    (Some(i), Some(j)) => i == j,
                    (None, None) => x == y,
                    _ => false,
                }
            }
            (
                Term::Pi {
                    param: p,
                    param_type: t,
                    body_type: x,
                },
                Term::Pi {
                    param: q,
                    param_type: s,
                    body_type: y,
                },
            )
            | (
                Term::Lambda {
                    param: p,
                    param_type: t,
                    body: x,
                },
                Term::Lambda {
                    param: q,
                    param_type: s,
                    body: y,
                },
            ) => {
                if !go(t, s, env) {
                    return false;
                }
                env.push((p.clone(), q.clone()));
                let r = go(x, y, env);
                env.pop();
                r
            }
            (Term::App(f, x), Term::App(h, y)) => go(f, h, env) && go(x, y, env),
            _ => a == b,
        }
    }
    go(a, b, &mut Vec::new())
}

fn parse(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("{text:?} must parse: {e:?}"))
}

#[test]
fn a_level_is_read_structurally() {
    for (text, want) in [
        ("Sort u", uvar("u")),
        ("Sort 0", Universe::Prop),
        ("Sort 1", Universe::Type(0)),
        ("Sort 2", Universe::Type(1)),
        ("Sort 4294967296", Universe::Type(u32::MAX)),
        ("Sort (u+1)", succ(uvar("u"))),
        ("Sort (u + 2)", succ(succ(uvar("u")))),
        ("Sort (0+1)", succ(Universe::Prop)),
        ("Sort (max u v)", umax(uvar("u"), uvar("v"))),
        ("Sort (imax u v)", uimax(uvar("u"), uvar("v"))),
        (
            "Sort (max (u+1) (imax v 0))",
            umax(succ(uvar("u")), uimax(uvar("v"), Universe::Prop)),
        ),
        ("Sort ((max u v)+1)", succ(umax(uvar("u"), uvar("v")))),
        ("Sort \u{ab}max\u{bb}", uvar("max")),
        ("Sort \u{ab}Prop\u{bb}", uvar("Prop")),
        ("Sort \u{ab}/s:\"u\"\u{bb}", uvar("/s:\"u\"")),
    ] {
        assert_eq!(parse(text), Term::Sort(want.clone()), "{text}");
    }
    // The spellings that were already there are untouched.
    assert_eq!(parse("Prop"), Term::Sort(Universe::Prop));
    assert_eq!(parse("SProp"), Term::Sort(Universe::SProp));
    assert_eq!(parse("Type"), Term::Sort(Universe::Type(0)));
    assert_eq!(parse("Type 3"), Term::Sort(Universe::Type(3)));
    assert_eq!(parse("Type3"), Term::Sort(Universe::Type(3)));
    // A level atom ends the sort: what follows is an ordinary argument or arrow.
    assert_eq!(
        parse("Sort u -> Sort v"),
        pi("_", Term::Sort(uvar("u")), Term::Sort(uvar("v")))
    );
    assert_eq!(
        parse("f (Sort u) x"),
        app(app(g("f"), Term::Sort(uvar("u"))), g("x"))
    );
}

#[test]
fn a_malformed_level_is_refused_not_guessed() {
    for text in [
        "Sort",
        "Sort (max u)",
        "Sort (imax u v w)",
        "Sort 4294967297",
        "Sort _",
        "Sort (u+0)",
        "Sort (u+)",
        "Sort Prop",
        "Sort max",
        "Sort (u+1",
        "f.{u",
        "f.{u,}",
        "f.{,u}",
        "f.{u v}",
    ] {
        assert!(
            TermParser::parse(text).is_err(),
            "{text:?} must be refused, got {:?}",
            TermParser::parse(text)
        );
    }
}

#[test]
fn a_constant_takes_explicit_level_arguments() {
    assert_eq!(parse("f.{u}"), konst("f", vec![uvar("u")]));
    assert_eq!(parse("f.{}"), konst("f", vec![]));
    assert_eq!(
        parse("Nat.rec.{u, v+1}"),
        konst("Nat.rec", vec![uvar("u"), succ(uvar("v"))]),
        "a qualified name with levels is ONE constant"
    );
    assert_eq!(
        parse("f.{max u v, 0} x"),
        app(
            konst("f", vec![umax(uvar("u"), uvar("v")), Universe::Prop]),
            g("x")
        )
    );
    assert_eq!(parse("\u{ab}a b\u{bb}.{u}"), konst("a b", vec![uvar("u")]));
    assert_eq!(
        parse("\u{ab}Prop\u{bb}.{u}"),
        konst("Prop", vec![uvar("u")]),
        "a quoted name is never a sort"
    );
    // A LOCAL variable is not a constant and takes no levels.
    assert!(TermParser::parse("fun (x : A) => x.{u}").is_err());
}

#[test]
fn a_quoted_name_is_always_a_name() {
    assert_eq!(parse("\u{ab}Prop\u{bb}"), g("Prop"));
    assert_eq!(parse("\u{ab}Type\u{bb}"), g("Type"));
    assert_eq!(parse("\u{ab}Sort\u{bb}"), g("Sort"));
    assert_eq!(parse("\u{ab}Type3\u{bb}"), g("Type3"));
    assert_eq!(
        parse("(\u{ab}fun\u{bb} : A) -> \u{ab}fun\u{bb}"),
        pi("fun", g("A"), v("fun"))
    );
    assert_eq!(
        parse("fun (\u{ab}Prop\u{bb} : A) => \u{ab}Prop\u{bb}"),
        lam("Prop", g("A"), v("Prop"))
    );
}

fn repl() -> Repl {
    Repl::new()
}

#[test]
fn a_universe_polymorphic_definition_is_checked_registered_and_instantiated() {
    let mut r = repl();
    r.execute("Definition pid.{u} : (A : Sort u) -> A -> A := fun (A : Sort u) (a : A) => a.")
        .expect("a universe-polymorphic identity");
    let (params, ty, body) = r
        .context()
        .get_universe_poly("pid")
        .expect("registered with its telescope")
        .clone();
    assert_eq!(params, vec!["u".to_string()]);
    let sort_u = Term::Sort(uvar("u"));
    assert!(
        alpha_eq(&ty, &pi("A", sort_u.clone(), pi("_", v("A"), v("A")))),
        "{ty:?}"
    );
    assert!(
        alpha_eq(&body, &lam("A", sort_u, lam("a", v("A"), v("a")))),
        "{body:?}"
    );
    let out = r
        .execute("Check pid.{1} Nat Zero.")
        .expect("instantiated at `Type 0`");
    assert!(out.ends_with(": Nat"), "{out}");
    r.execute("Definition z : Nat := pid.{1} Nat Zero.")
        .expect("usable in a later definition");
    // …and at `Prop`, where `A` is a proposition.
    r.execute("Check pid.{0} True I.")
        .expect("instantiated at `Prop`");
}

#[test]
fn an_unbound_or_repeated_universe_parameter_is_refused() {
    let mut r = repl();
    let unbound = r.execute("Definition bad.{u} : Sort (v+1) := Sort v.");
    assert!(unbound.is_err(), "`v` is not in the telescope: {unbound:?}");
    assert!(
        r.context().get_global("bad").is_none() && r.context().get_universe_poly("bad").is_none()
    );
    let repeated = r.execute("Definition dup.{u, u} : Sort (u+1) := Sort u.");
    assert!(
        repeated.is_err(),
        "a telescope names each parameter once: {repeated:?}"
    );
    assert!(r.context().get_universe_poly("dup").is_none());
    // A body whose type is not the declared type is refused like any other definition.
    let wrong = r.execute("Definition lie.{u} : Sort u := Sort u.");
    assert!(
        wrong.is_err(),
        "`Sort u : Sort (u+1)`, not `Sort u`: {wrong:?}"
    );
    assert!(r.context().get_universe_poly("lie").is_none());
}

#[test]
fn a_universe_polymorphic_inductive_registers_exactly_as_the_kernel_door_does() {
    let text = "Inductive PBox.{u} (A : Sort u) : Sort (max 1 u) := pmk : A -> PBox.{u} A.";
    let mut r = repl();
    r.execute(text).expect("a universe-polymorphic box");

    // The same block through the kernel's own checked door, as the Lean importer registers it.
    let mut direct: Context = logicaffeine_kernel::interface::standard_context();
    let sort_u = Term::Sort(uvar("u"));
    let arity = pi(
        "A",
        sort_u.clone(),
        Term::Sort(umax(Universe::Type(0), uvar("u"))),
    );
    let ctor = pi(
        "A",
        sort_u,
        pi("_", v("A"), app(konst("PBox", vec![uvar("u")]), v("A"))),
    );
    direct
        .add_universe_poly_inductives(
            &["u".to_string()],
            &[MutualInductive {
                name: "PBox".into(),
                sort: arity.clone(),
                num_params: 1,
                constructors: vec![("pmk".into(), ctor.clone())],
            }],
        )
        .expect("the direct registration");

    let ctx = r.context();
    let (params, got_arity) = ctx.poly_declaration("PBox").expect("a polymorphic header");
    assert_eq!(params, ["u".to_string()]);
    assert!(alpha_eq(got_arity, &arity), "{got_arity:?}");
    assert!(
        alpha_eq(ctx.get_global("pmk").unwrap(), &ctor),
        "{:?}",
        ctx.get_global("pmk")
    );
    assert_eq!(
        ctx.inductive_num_params("PBox"),
        direct.inductive_num_params("PBox")
    );
    assert_eq!(
        ctx.inductive_declared_params("PBox"),
        direct.inductive_declared_params("PBox")
    );
    assert_eq!(ctx.get_constructors("PBox").len(), 1);
    r.execute("Check pmk.{1} Nat Zero.")
        .expect("the constructor instantiates");
}

#[test]
fn a_declared_arity_fixes_the_parameters_and_the_rest_are_indices() {
    let mut r = repl();
    r.execute("Inductive Fam : Nat -> Type := fmk : (n : Nat) -> Fam n.")
        .expect("an indexed family");
    assert_eq!(
        r.context().inductive_declared_params("Fam"),
        Some(0),
        "the header declares no parameters, so `n` is an INDEX — inference would have called it a parameter"
    );
    assert_eq!(r.context().inductive_num_indices("Fam"), 1);

    r.execute(
        "Inductive Vec (A : Type) : Nat -> Type := vnil : Vec A Zero | vcons : (n : Nat) -> A -> Vec A n -> Vec A (Succ n).",
    )
    .expect("length-indexed vectors");
    let ctx = r.context();
    assert_eq!(ctx.inductive_declared_params("Vec"), Some(1));
    assert_eq!(ctx.inductive_num_indices("Vec"), 1);
    let ty0 = Term::Sort(Universe::Type(0));
    assert!(alpha_eq(
        ctx.get_global("Vec").unwrap(),
        &pi("A", ty0.clone(), pi("_", g("Nat"), ty0.clone()))
    ));
    assert!(alpha_eq(
        ctx.get_global("vcons").unwrap(),
        &pi(
            "A",
            ty0,
            pi(
                "n",
                g("Nat"),
                pi(
                    "_",
                    v("A"),
                    pi(
                        "_",
                        app(app(g("Vec"), v("A")), v("n")),
                        app(app(g("Vec"), v("A")), app(g("Succ"), v("n")))
                    )
                )
            )
        )
    ));
    assert!(
        ctx.get_global("Vec_rec").is_some(),
        "the eliminator is derived at the declaration"
    );

    r.execute("Inductive Empty2 : Type.")
        .expect("an inductive with no constructors");
    assert!(
        r.context().is_inductive("Empty2") && r.context().get_constructors("Empty2").is_empty()
    );
}

#[test]
fn a_mutual_block_is_one_block_exactly_as_written() {
    let mut r = repl();
    r.execute(
        "Inductive Ev : Nat -> Prop := ev0 : Ev Zero | evS : (n : Nat) -> Od n -> Ev (Succ n) \
         with Od : Nat -> Prop := odS : (n : Nat) -> Ev n -> Od (Succ n).",
    )
    .expect("even/odd");
    assert_eq!(
        r.context().mutual_block_of("Ev"),
        Some(&["Ev".to_string(), "Od".to_string()][..])
    );
    // A declared block is honoured even when no member mentions another — the reference graph is
    // not consulted, because the writer said which declarations form one block.
    r.execute("Inductive Lone1 : Type := l1 : Lone1 with Lone2 : Type := l2 : Lone2.")
        .expect("a block");
    assert_eq!(
        r.context().mutual_block_of("Lone1"),
        Some(&["Lone1".to_string(), "Lone2".to_string()][..])
    );
}

#[test]
fn a_mutual_block_shares_one_universe_telescope() {
    let disagree = parse_command(
        "Inductive A1.{u} : Sort (u+1) := a1 : A1.{u} with B1.{v} : Sort (v+1) := b1 : B1.{v}.",
    );
    assert!(
        disagree.is_err(),
        "two telescopes in one block: {disagree:?}"
    );
    let restated = parse_command(
        "Inductive A2.{u} : Sort (u+1) := a2 : A2.{u} with B2.{u} : Sort (u+1) := b2 : B2.{u}.",
    )
    .expect("restating the same telescope is allowed");
    let Command::Universes { params, inner } = restated else {
        panic!("a polymorphic block")
    };
    assert_eq!(params, vec!["u".to_string()]);
    let Command::InductiveBlock { members } = *inner else {
        panic!("a block")
    };
    assert_eq!(members.len(), 2);
}

#[test]
fn every_universe_form_prints_back_to_itself() {
    for text in [
        "Definition pid.{u} : (A : Sort u) -> A -> A := fun (A : Sort u) (a : A) => a.",
        "Definition two.{u, v} : Sort (max (u+1) (imax v 0)) := Sort (max u v).",
        "Inductive PBox.{u} (A : Sort u) : Sort (max 1 u) := pmk : A -> PBox.{u} A.",
        "Axiom pchoice.{u} : (A : Sort u) -> A.",
        "Theorem pmono.{u} : (A : Sort u) -> (P : A -> Prop) -> (a : A) -> P a -> P a := fun (A : Sort u) (P : A -> Prop) (a : A) (h : P a) => h.",
        "Check pid.{4294967296} Nat Zero.",
    ] {
        let cmd = parse_command(text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        let printed = print_command(&cmd).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert_eq!(print_command(&parse_command(&printed).unwrap()).unwrap(), printed, "{text}");
    }
    assert_eq!(
        print_term(&Term::Sort(umax(
            succ(uvar("u")),
            uimax(uvar("v"), Universe::Prop)
        )))
        .unwrap(),
        "Sort (max (u+1) (imax v 0))"
    );
    assert_eq!(
        print_term(&konst("Nat.rec", vec![uvar("u"), Universe::Type(0)])).unwrap(),
        "Nat.rec.{u, 1}"
    );
}

#[test]
fn a_registered_polymorphic_declaration_prints_as_the_command_that_registers_it() {
    // Print the kernel's own copy, run the text in a FRESH session, and compare the two registries.
    let mut first = repl();
    first.execute("Definition pconst.{u, v} : (A : Sort u) -> (B : Sort v) -> A -> B -> A := fun (A : Sort u) (B : Sort v) (a : A) (b : B) => a.").unwrap();
    let (params, ty, body) = first.context().get_universe_poly("pconst").unwrap().clone();
    let cmd = Command::Universes {
        params: params.clone(),
        inner: Box::new(Command::Definition {
            name: "pconst".into(),
            ty: Some(ty.clone()),
            body: body.clone(),
            is_hint: false,
            implicit_count: 0,
            proof_goal: None,
        }),
    };
    let text = print_command(&cmd).expect("a spelling for the registered declaration");
    let mut second = repl();
    second
        .execute(&text)
        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
    let (p2, t2, b2) = second
        .context()
        .get_universe_poly("pconst")
        .unwrap()
        .clone();
    assert_eq!(params, p2);
    assert!(alpha_eq(&ty, &t2) && alpha_eq(&body, &b2), "{text}");
}

#[test]
fn the_kernel_quotient_is_declared_primitive_by_primitive_and_checked() {
    let mut r = repl();
    // A canonical universe-polymorphic equality, declared as Lean declares `Eq`: two parameters,
    // one index, reflexivity its one constructor.
    r.execute("Inductive PEq.{u} (A : Sort u) (a : A) : A -> Prop := prefl : PEq.{u} A a a.")
        .expect("a canonical equality");
    // The constructor cannot come before the type it constructs.
    assert!(
        r.execute("Quotient Constructor.{u}.").is_err(),
        "the quotient type comes first"
    );
    assert!(r.context().poly_declaration("kernel.quot.mk").is_none());
    r.execute("Quotient Type.{u}.").expect("the quotient type");
    r.execute("Quotient Constructor.{u}.")
        .expect("its constructor");
    // A lift stated with something that is not the canonical equality is refused.
    assert!(
        r.execute("Quotient Lift.{u, v} using Nat.").is_err(),
        "a lift over a non-equality"
    );
    r.execute("Quotient Lift.{u, v} using PEq.")
        .expect("the lift, over the canonical equality");
    r.execute("Quotient Induction.{u}.")
        .expect("the induction principle");
    for (name, arity) in [
        ("kernel.quot.type", 1),
        ("kernel.quot.mk", 1),
        ("kernel.quot.lift", 2),
        ("kernel.quot.ind", 1),
    ] {
        let (params, _) = r
            .context()
            .poly_declaration(name)
            .unwrap_or_else(|| panic!("{name} installed"));
        assert_eq!(params.len(), arity, "{name}");
    }
    // The primitives are usable: the quotient of `Nat` by any relation is a type.
    r.execute("Check kernel.quot.type.{1} Nat (fun (a : Nat) (b : Nat) => True).")
        .expect("a quotient type");
    // A primitive needs its telescope, and only `Lift` names an equality.
    assert!(parse_command("Quotient Type.").is_err());
    assert!(parse_command("Quotient Type.{u} using PEq.").is_err());
    assert!(parse_command("Quotient Lift.{u, v}.").is_err());
    assert!(parse_command("Quotient Everything.{u}.").is_err());
    for text in [
        "Quotient Type.{u}.",
        "Quotient Lift.{u, v} using PEq.",
        "Quotient Induction.{\u{ab}/s:\"u\"\u{bb}}.",
    ] {
        let printed = print_command(&parse_command(text).unwrap()).unwrap();
        assert_eq!(printed, text, "a quotient command prints back exactly");
    }
}

/// Every registry fact an inductive carries, in comparable form: arity, parameter and index
/// counts, constructors in order with their types, and the level telescope.
fn inductive_facts(
    ctx: &Context,
    name: &str,
) -> (Term, usize, usize, Vec<(String, Term)>, Option<Vec<String>>) {
    (
        ctx.get_global(name)
            .unwrap_or_else(|| panic!("`{name}` registered"))
            .clone(),
        ctx.inductive_num_params(name),
        ctx.inductive_num_indices(name),
        ctx.get_constructors(name)
            .into_iter()
            .map(|(n, t)| (n.to_string(), t.clone()))
            .collect(),
        ctx.poly_declaration(name).map(|(p, _)| p.to_vec()),
    )
}

fn assert_same_inductive(a: &Context, b: &Context, name: &str) {
    let (arity, params, indices, ctors, levels) = inductive_facts(a, name);
    let (arity2, params2, indices2, ctors2, levels2) = inductive_facts(b, name);
    assert!(
        alpha_eq(&arity, &arity2),
        "`{name}`'s arity: {arity:?} vs {arity2:?}"
    );
    assert_eq!(
        (params, indices, levels),
        (params2, indices2, levels2),
        "`{name}`'s shape"
    );
    assert_eq!(ctors.len(), ctors2.len(), "`{name}`'s constructors");
    for ((n, t), (n2, t2)) in ctors.iter().zip(&ctors2) {
        assert_eq!(n, n2, "constructor order");
        assert!(alpha_eq(t, t2), "`{n}`: {t:?} vs {t2:?}");
    }
}

#[test]
fn a_registered_inductive_prints_as_the_block_that_registers_it() {
    use logicaffeine_kernel::interface::printer::inductive_command;
    // Kernel binder names chosen to COLLIDE: the parameter is named `Nat` while the index and a
    // field mention the GLOBAL `Nat`, and a field's own type binds another `Nat`. Referring to the
    // parameter by its kernel name would capture both.
    let mut direct = logicaffeine_kernel::interface::standard_context();
    let ty0 = Term::Sort(Universe::Type(0));
    let arity = pi("Nat", ty0.clone(), pi("_", g("Nat"), ty0.clone()));
    let ctor = pi(
        "Nat",
        ty0.clone(),
        pi(
            "k",
            g("Nat"),
            pi(
                "f",
                pi("Nat", g("Nat"), g("Nat")),
                pi("x", v("Nat"), app(app(g("Tricky"), v("Nat")), v("k"))),
            ),
        ),
    );
    direct
        .add_mutual_inductives(&[MutualInductive {
            name: "Tricky".into(),
            sort: arity,
            num_params: 1,
            constructors: vec![("tmk".into(), ctor)],
        }])
        .expect("the direct registration");
    let cmd = inductive_command(&direct, "Tricky").expect("a block command");
    let text = print_command(&cmd).expect("a spelling");
    let mut replayed = repl();
    replayed
        .execute(&text)
        .unwrap_or_else(|e| panic!("{text}: {e:?}"));
    assert_same_inductive(&direct, replayed.context(), "Tricky");

    // A universe-polymorphic inductive and a mutual block, registered from source, printed back
    // from the kernel's copy, and registered again in a fresh session.
    let mut first = repl();
    first
        .execute("Inductive PBox.{u} (A : Sort u) : Sort (max 1 u) := pmk : A -> PBox.{u} A.")
        .unwrap();
    first
        .execute(
            "Inductive Ev : Nat -> Prop := ev0 : Ev Zero | evS : (n : Nat) -> Od n -> Ev (Succ n) \
             with Od : Nat -> Prop := odS : (n : Nat) -> Ev n -> Od (Succ n).",
        )
        .unwrap();
    first
        .execute("Inductive Vec (A : Type) : Nat -> Type := vnil : Vec A Zero | vcons : (n : Nat) -> A -> Vec A n -> Vec A (Succ n).")
        .unwrap();
    let mut second = repl();
    for name in ["PBox", "Ev", "Vec"] {
        let text = print_command(&inductive_command(first.context(), name).unwrap()).unwrap();
        second
            .execute(&text)
            .unwrap_or_else(|e| panic!("{text}: {e:?}"));
    }
    for name in ["PBox", "Ev", "Od", "Vec"] {
        assert_same_inductive(first.context(), second.context(), name);
    }
    assert_eq!(
        second.context().mutual_block_of("Od").map(|b| b.len()),
        Some(2)
    );
    assert!(
        inductive_command(first.context(), "Zero").is_err(),
        "a constructor is not an inductive"
    );
}

#[test]
fn a_constructor_restates_its_parameter_telescope_and_is_registered_as_written() {
    // Lean's `HAdd`: the normalised arity binds `γ : Type w`, the constructor keeps `outParam`.
    let mut r = repl();
    r.execute("Definition outP.{u} : Sort u -> Sort u := fun (A : Sort u) => A.")
        .unwrap();
    let text = "Inductive HAdd2.{u, v, w} (a : Sort (u+1)) (b : Sort (v+1)) (c : Sort (w+1)) : \
                Sort (max (u+1) (max (v+1) (w+1))) := \
                hmk (a : Sort (u+1)) (b : Sort (v+1)) (c : outP.{w+2} (Sort (w+1))) : (a -> b -> c) -> HAdd2.{u, v, w} a b c.";
    let Command::Universes { inner, .. } = parse_command(text).expect("a restated block") else {
        panic!("polymorphic")
    };
    let Command::InductiveBlock { members } = *inner else {
        panic!("a block")
    };
    assert!(
        members[0].full_constructors,
        "the constructors restate their parameters"
    );
    r.execute(text).unwrap_or_else(|e| panic!("{e:?}"));
    let ctor = r.context().get_global("hmk").expect("registered").clone();
    let Term::Pi { body_type, .. } = &ctor else {
        panic!()
    };
    let Term::Pi { body_type, .. } = body_type.as_ref() else {
        panic!()
    };
    let Term::Pi { param_type, .. } = body_type.as_ref() else {
        panic!()
    };
    assert!(
        matches!(param_type.as_ref(), Term::App(f, _) if matches!(f.as_ref(), Term::Const { name, .. } if name == "outP")),
        "the restated parameter type is kept verbatim: {param_type:?}"
    );
    // Printed back from the kernel, it restates again, and a fresh session registers it identically.
    let cmd =
        logicaffeine_kernel::interface::printer::inductive_command(r.context(), "HAdd2").unwrap();
    let printed = print_command(&cmd).unwrap();
    assert!(
        printed.contains("hmk ("),
        "the printed constructor restates its telescope: {printed}"
    );
    let mut second = repl();
    second
        .execute("Definition outP.{u} : Sort u -> Sort u := fun (A : Sort u) => A.")
        .unwrap();
    second
        .execute(&printed)
        .unwrap_or_else(|e| panic!("{printed}: {e:?}"));
    assert_same_inductive(r.context(), second.context(), "HAdd2");
    // Mixing the two forms in one member, or restating a telescope that is not there, is refused.
    assert!(
        parse_command("Inductive M (a : Type) : Type := m1 (a : Type) : M a | m2 : M a.").is_err()
    );
    assert!(parse_command("Inductive N : Type := n1 (a : Type) : N.").is_err());
}

#[test]
fn structure_eta_is_granted_only_through_projections_that_compute_their_fields() {
    let mut r = repl();
    r.execute("Inductive Pt.{u} (A : Sort (u+1)) : Sort (u+1) := ptmk : A -> A -> Pt.{u} A.")
        .unwrap();
    r.execute("Definition px.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => a end.").unwrap();
    r.execute("Definition py.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => b end.").unwrap();
    // The projections in the wrong order do not compute their fields: refused, nothing granted.
    let swapped = r.execute("Structure Pt := py px.");
    assert!(swapped.is_err(), "{swapped:?}");
    assert!(r.context().struct_info("Pt").is_none());
    // Too few, and a name that is not a definition, are refused too.
    assert!(r.execute("Structure Pt := px.").is_err());
    assert!(r.execute("Structure Pt := px ptmk.").is_err());
    // The right projections: η is granted, and only once.
    r.execute("Structure Pt := px py.")
        .expect("the projections compute their fields");
    let info = r.context().struct_info("Pt").expect("registered");
    assert_eq!(
        (info.mk.as_str(), info.num_params, info.projections.clone()),
        ("ptmk", 1, vec!["px".to_string(), "py".to_string()])
    );
    assert!(
        r.execute("Structure Pt := px py.").is_err(),
        "a structure is declared once"
    );
    // η is live: a point is its reconstruction — checked on a monomorphic instance, where the
    // prelude's `Eq` applies directly.
    r.execute("Theorem pt_eta : (p : Pt.{0} Nat) -> Eq (Pt.{0} Nat) p (ptmk.{0} Nat (px.{0} Nat p) (py.{0} Nat p)) := fun (p : Pt.{0} Nat) => refl (Pt.{0} Nat) p.")
        .expect("structure η makes the reconstruction definitionally the point");
    // …and without η the same statement is refused: the control.
    let mut plain = repl();
    plain
        .execute("Inductive Pt.{u} (A : Sort (u+1)) : Sort (u+1) := ptmk : A -> A -> Pt.{u} A.")
        .unwrap();
    plain.execute("Definition px.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => a end.").unwrap();
    plain.execute("Definition py.{u} : (A : Sort (u+1)) -> Pt.{u} A -> A := fun (A : Sort (u+1)) (p : Pt.{u} A) => match p return (fun (q : Pt.{u} A) => A) with fun (a : A) (b : A) => b end.").unwrap();
    assert!(
        plain.execute("Theorem pt_eta : (p : Pt.{0} Nat) -> Eq (Pt.{0} Nat) p (ptmk.{0} Nat (px.{0} Nat p) (py.{0} Nat p)) := fun (p : Pt.{0} Nat) => refl (Pt.{0} Nat) p.").is_err(),
        "without structure η a variable is not its reconstruction"
    );
    // A recursive or multi-constructor inductive is never a structure.
    r.execute("Inductive Two : Type := t1 : Two | t2 : Two.")
        .unwrap();
    assert!(r.execute("Structure Two := .").is_err());
    // Printed back from the kernel, a fresh session grants the same η.
    let mut second = repl();
    second
        .execute(
            &print_command(
                &logicaffeine_kernel::interface::printer::inductive_command(r.context(), "Pt")
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    for cmd in
        logicaffeine_kernel::interface::printer::structure_commands(r.context(), "Pt").unwrap()
    {
        let text = print_command(&cmd).unwrap();
        second
            .execute(&text)
            .unwrap_or_else(|e| panic!("{text}: {e:?}"));
    }
    assert_eq!(
        second
            .context()
            .struct_info("Pt")
            .map(|i| i.projections.clone()),
        Some(vec!["px".to_string(), "py".to_string()])
    );
}
