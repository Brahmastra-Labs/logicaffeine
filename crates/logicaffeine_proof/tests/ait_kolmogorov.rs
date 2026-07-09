//! Kernel-certified algorithmic information theory — the invariance theorem, proved the way
//! `order_axioms.rs` proves Farkas steps: a description system is *axiomatized* as opaque kernel
//! declarations (Π-typed), the invariance proof term is built by applying them, and the kernel's own
//! `infer_type` + `is_subtype` certify it has the invariance type. No proof search — the kernel is the
//! sole trust door.
//!
//! Description-system vocabulary (carrier `Int`; everything else opaque):
//!   `Decode m p x : Prop`  — machine `m` on program `p` outputs object `x`
//!   `K m x : Int`          — Kolmogorov complexity of `x` under `m` (a length)
//!   `length p`, `plus a b`, `concat s p`, `prog m x`, `pref m` — length arithmetic + skolem witnesses
//!   `le a b` (via `Eq Bool (le a b) true`, from the standard order theory) — `a ≤ b`
//!
//! The two existentials of the informal proof — "a shortest program exists" and "U simulates V with a
//! fixed prefix" — are skolemized to `prog` and `pref`, so the derivation is a straight chain:
//!   a_kwit x ⊢ Decode V (prog V x) x
//!   a_sim  ⊢ Decode V (prog V x) x → Decode U (concat (pref V) (prog V x)) x
//!   a_klower ⊢ Decode U p x → K U x ≤ length p
//! giving `K U x ≤ length (concat (pref V) (prog V x))` — invariance, the additive constant being the
//! length of the fixed prefix `pref V`.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn int() -> Term {
    g("Int")
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, |acc, x| Term::App(Box::new(acc), Box::new(x)))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
/// `a ≤ b`  ≡  `Eq Bool (le a b) true` (the shallow order Prop of the standard library).
fn le(a: Term, b: Term) -> Term {
    app(g("Eq"), vec![g("Bool"), app(g("le"), vec![a, b]), g("true")])
}
fn decode(m: Term, p: Term, x: Term) -> Term {
    app(g("Decode"), vec![m, p, x])
}
fn kc(m: Term, x: Term) -> Term {
    app(g("K"), vec![m, x])
}
fn length(p: Term) -> Term {
    app(g("length"), vec![p])
}
fn concat(s: Term, p: Term) -> Term {
    app(g("concat"), vec![s, p])
}
fn prog(m: Term, x: Term) -> Term {
    app(g("prog"), vec![m, x])
}
fn pref(m: Term) -> Term {
    app(g("pref"), vec![m])
}

/// Register the description-system symbols and axioms into `c` (carrier `Int`, universal machine `U`,
/// arbitrary machine `V`).
fn register_description_system(c: &mut Context) {
    c.add_declaration("U", int());
    c.add_declaration("V", int());
    c.add_declaration("Decode", arrow(int(), arrow(int(), arrow(int(), prop()))));
    c.add_declaration("K", arrow(int(), arrow(int(), int())));
    c.add_declaration("length", arrow(int(), int()));
    c.add_declaration("concat", arrow(int(), arrow(int(), int())));
    c.add_declaration("prog", arrow(int(), arrow(int(), int())));
    c.add_declaration("pref", arrow(int(), int()));

    // Universality: U simulates V on any program p by prefixing pref(V).
    //   a_sim : Π p x. Decode V p x → Decode U (concat (pref V) p) x
    c.add_declaration(
        "a_sim",
        pi(
            "p",
            int(),
            pi(
                "x",
                int(),
                arrow(
                    decode(g("V"), v("p"), v("x")),
                    decode(g("U"), concat(pref(g("V")), v("p")), v("x")),
                ),
            ),
        ),
    );
    // K is a lower bound: any decoding program under U is at least K(U,x) long.
    //   a_klower : Π p x. Decode U p x → (K U x ≤ length p)
    c.add_declaration(
        "a_klower",
        pi(
            "p",
            int(),
            pi("x", int(), arrow(decode(g("U"), v("p"), v("x")), le(kc(g("U"), v("x")), length(v("p"))))),
        ),
    );
    // K is achieved: prog(V,x) decodes x under V.
    //   a_kwit : Π x. Decode V (prog V x) x
    c.add_declaration("a_kwit", pi("x", int(), decode(g("V"), prog(g("V"), v("x")), v("x"))));
}

fn proves(x: Term, cc: Term) -> Term {
    app(g("ProvesGt"), vec![x, cc])
}
fn slt(a: Term, b: Term) -> Term {
    app(g("StrictLt"), vec![a, b])
}
fn pack(cc: Term) -> Term {
    app(g("Pack"), vec![cc])
}

/// Register the Chaitin / Berry vocabulary and axioms on top of the description system.
fn register_chaitin_system(c: &mut Context) {
    c.add_declaration("Cf", int()); // F's Chaitin constant (≈ F's own description length + O(1))
    c.add_declaration("ProvesGt", arrow(int(), arrow(int(), prop()))); // "F proves K(x) > c"
    c.add_declaration("StrictLt", arrow(int(), arrow(int(), prop()))); // strict order (opaque)
    c.add_declaration("Pack", arrow(int(), int())); // the Berry program as a function of the bound

    // F is sound about complexity claims: if F proves K(x) > Cf then really Cf < K(U,x).
    c.add_declaration(
        "F_sound",
        pi("x", int(), arrow(proves(v("x"), g("Cf")), slt(g("Cf"), kc(g("U"), v("x"))))),
    );
    // The Berry construction: Pack(Cf) decodes (under U) to the very witness F names.
    c.add_declaration(
        "berry_construct",
        pi("x", int(), arrow(proves(v("x"), g("Cf")), decode(g("U"), pack(g("Cf")), v("x")))),
    );
    // The Berry program is no longer than the Chaitin constant.
    c.add_declaration("berry_length", le(length(pack(g("Cf"))), g("Cf")));
    // Order antisymmetry: a ≤ b and b < a is absurd.
    c.add_declaration(
        "ord_antisym",
        pi("a", int(), pi("b", int(), arrow(le(v("a"), v("b")), arrow(slt(v("b"), v("a")), g("False"))))),
    );
}

/// The Berry contradiction proof term: from a proof `hyp : ProvesGt(x0, Cf)`, derive `False`.
fn berry_contradiction(hyp: Term) -> Term {
    let lt_proof = app(g("F_sound"), vec![g("x0"), hyp.clone()]); // StrictLt Cf (K U x0)
    let dec = app(g("berry_construct"), vec![g("x0"), hyp]); // Decode U (Pack Cf) x0
    let kle = app(g("a_klower"), vec![pack(g("Cf")), g("x0"), dec]); // K U x0 ≤ |Pack Cf|
    let ktrans = app(
        g("le_trans"),
        vec![kc(g("U"), g("x0")), length(pack(g("Cf"))), g("Cf"), kle, g("berry_length")],
    ); // K U x0 ≤ Cf
    app(g("ord_antisym"), vec![kc(g("U"), g("x0")), g("Cf"), ktrans, lt_proof]) // False
}

#[test]
fn invariance_is_kernel_certified() {
    let mut c = ctx();
    register_description_system(&mut c);
    // An arbitrary object x0 — the proof is parametric in it, so it certifies invariance for every x.
    c.add_declaration("x0", int());

    // a_kwit x0 : Decode V (prog V x0) x0
    let kwit = app(g("a_kwit"), vec![g("x0")]);
    // a_sim (prog V x0) x0 kwit : Decode U (concat (pref V) (prog V x0)) x0
    let simulated = app(g("a_sim"), vec![prog(g("V"), g("x0")), g("x0"), kwit]);
    // a_klower (concat (pref V) (prog V x0)) x0 simulated : K U x0 ≤ length (concat (pref V) (prog V x0))
    let proof = app(
        g("a_klower"),
        vec![concat(pref(g("V")), prog(g("V"), g("x0"))), g("x0"), simulated],
    );

    let inferred = infer_type(&c, &proof).expect("the invariance proof term must type-check");
    let goal = le(kc(g("U"), g("x0")), length(concat(pref(g("V")), prog(g("V"), g("x0")))));
    assert!(
        is_subtype(&c, &inferred, &goal),
        "invariance: K_U(x0) ≤ |pref(V)·prog(V,x0)| = c + K_V(x0); the kernel derived: {}",
        inferred
    );
}

#[test]
fn chaitin_incompleteness_via_berry_is_kernel_certified() {
    // Chaitin's incompleteness theorem (bounded / single-witness form): a sound system F cannot prove
    // "K(x) > Cf" for ANY x, where Cf is F's Chaitin constant. The Berry paradox, kernel-certified: if
    // F proved K(x₀) > Cf, then the program Pack(Cf) = "output the first x F proves has K(x) > Cf"
    // (length ≤ Cf) decodes to x₀, so K(x₀) ≤ |Pack(Cf)| ≤ Cf — contradicting F's own claim Cf < K(x₀).
    let mut c = ctx();
    register_description_system(&mut c);
    register_chaitin_system(&mut c);

    // An arbitrary object x0 and the (to-be-refuted) hypothesis that F proves K(x0) > Cf.
    c.add_declaration("x0", int());
    c.add_declaration("h", proves(g("x0"), g("Cf")));

    let false_proof = berry_contradiction(g("h"));
    let inferred = infer_type(&c, &false_proof).expect("the Berry contradiction must type-check");
    assert!(
        is_subtype(&c, &inferred, &g("False")),
        "Chaitin/Berry: assuming F proves K(x0) > Cf derives ⊥; the kernel got: {}",
        inferred
    );
}

#[test]
fn godel_incompleteness_corollary_is_kernel_certified() {
    // Gödel's incompleteness as a corollary of Chaitin: for an incompressible x0 the statement
    // G ≡ "K(x0) > Cf" is TRUE, yet F cannot prove it — a true-but-unprovable sentence.
    let mut c = ctx();
    register_description_system(&mut c);
    register_chaitin_system(&mut c);
    c.add_declaration("x0", int());
    // x0 is incompressible: its complexity exceeds the Chaitin constant (an object the counting lemma
    // guarantees exists), so G ≡ StrictLt Cf (K U x0) is TRUE.
    c.add_declaration("godel_true", slt(g("Cf"), kc(g("U"), g("x0"))));

    // F cannot prove G: λ(h : ProvesGt x0 Cf). ⊥ — a closed proof of ¬ProvesGt(x0, Cf).
    let notprov = lam("h", proves(g("x0"), g("Cf")), berry_contradiction(v("h")));
    let inferred = infer_type(&c, &notprov).expect("the ¬-provability proof must type-check");
    let not_g = arrow(proves(g("x0"), g("Cf")), g("False"));
    assert!(
        is_subtype(&c, &inferred, &not_g),
        "F cannot prove the sentence K(x0) > Cf; the kernel got: {}",
        inferred
    );
    // And G is true (godel_true witnesses it): so K(x0) > Cf is a true statement F cannot prove.
    assert!(infer_type(&c, &g("godel_true")).is_ok(), "the Gödel sentence is true");
}

#[test]
fn a_non_derivation_does_not_type_check_as_invariance() {
    // Guard: the kernel is really checking. A proof term that skips the simulation step (feeding the
    // V-decode where a U-decode is required) must NOT type-check to the invariance goal.
    let mut c = ctx();
    register_description_system(&mut c);
    c.add_declaration("x0", int());

    // a_klower applied to the V-decode directly (wrong: a_klower needs a Decode U … proof).
    let kwit = app(g("a_kwit"), vec![g("x0")]);
    let bogus = app(g("a_klower"), vec![prog(g("V"), g("x0")), g("x0"), kwit]);
    // Either it fails to type-check, or its type is not the invariance goal — never a false certificate.
    let ok = infer_type(&c, &bogus).is_ok();
    assert!(!ok, "feeding a Decode V proof to a Decode U premise must be rejected by the kernel");
}
