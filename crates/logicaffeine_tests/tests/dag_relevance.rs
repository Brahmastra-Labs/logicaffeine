//! Argument relevance (`dag::relevance`): which argument positions of a definition its unfolding
//! cannot distinguish, so that comparing two applications of it may skip them.
//!
//! Both engines compute the proof positions, each by its own inference, and share the
//! absent-parameter analysis; a wrong mask is unsound (a skipped argument that matters), so the
//! two must agree on every definition, and the known cases hold exactly.

use logicaffeine_kernel::dag::env::{Kind, Safety};
use logicaffeine_kernel::dag::export::Reader;
use logicaffeine_kernel::dag::expr::{ExprId, Node};
use logicaffeine_kernel::dag::name::NameId;
use logicaffeine_kernel::dag::Kernel;
use std::io::BufReader;
use std::path::PathBuf;

fn load(path: &str) -> Kernel {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arena_corpus")
        .join(path);
    let mut k = Kernel::new();
    Reader::new(&mut k)
        .run(BufReader::new(std::fs::File::open(file).unwrap()))
        .expect("the case is accepted");
    k
}

fn name(k: &mut Kernel, dotted: &str) -> NameId {
    let n = k.s.names.dotted(dotted);
    assert!(k.env.get(n).is_some(), "{dotted} is declared");
    n
}

/// The mask the substitution engine computes (absent positions, and proof positions by its own
/// inference), at the definition's own universe parameters.
fn classic_mask(k: &mut Kernel, n: NameId) -> u64 {
    let ps = k.env.get(n).unwrap().lparams.clone();
    let ls = k.const_levels_of(&ps);
    k.with_decl(ps, Safety::Safe, |k| k.ignorable_args(n, ls))
        .unwrap()
}

/// The same mask with the proof positions found by the value engine.
fn value_mask(k: &mut Kernel, n: NameId) -> Option<u64> {
    let ps = k.env.get(n).unwrap().lparams.clone();
    let ls = k.const_levels_of(&ps);
    let absent = k.absent_mask(n, ls)?;
    let proofs = k
        .with_decl(ps, Safety::Safe, |k| {
            k.with_sem(|sem| {
                let ty = sem.const_type(n, ls)?;
                sem.proof_positions(ty)
            })
        })
        .unwrap();
    Some(absent | proofs)
}

fn run<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn an_argument_the_definition_drops_is_ignorable() {
    run(|| {
        let mut k = load("good/perf/discarded-argument.ndjson");
        let drop_arg = name(&mut k, "dropArg");
        let add = name(&mut k, "N.add");
        assert_eq!(
            classic_mask(&mut k, drop_arg),
            1,
            "dropArg never uses its only argument"
        );
        assert_eq!(
            classic_mask(&mut k, add),
            0,
            "N.add uses both of its arguments"
        );
        assert_eq!(value_mask(&mut k, drop_arg), Some(1));
        assert_eq!(value_mask(&mut k, add), Some(0));
    })
}

#[test]
fn a_proof_argument_is_ignorable() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let cast = name(&mut k, "cast");
        let m = classic_mask(&mut k, cast);
        assert_eq!(
            m & 0b100,
            0b100,
            "cast's third argument, h : α = β, is a proof"
        );
        assert_eq!(
            m & 0b1011,
            0,
            "cast's types and its value argument all matter"
        );
        assert_eq!(value_mask(&mut k, cast), Some(m));
    })
}

#[test]
fn both_engines_find_the_same_ignorable_arguments_for_every_definition() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let defs: Vec<NameId> = (0..k.s.names.len() as u32)
            .map(NameId)
            .filter(|n| {
                matches!(
                    k.env.get(*n).map(|c| &c.kind),
                    Some(Kind::Defn {
                        safety: Safety::Safe,
                        ..
                    })
                )
            })
            .collect();
        assert!(
            defs.len() >= 300,
            "init-prelude has its definitions ({})",
            defs.len()
        );
        let mut ignorable = 0;
        let mut differ = Vec::new();
        for n in defs {
            let classic = classic_mask(&mut k, n);
            let value = value_mask(&mut k, n).unwrap_or(0);
            ignorable += (classic != 0) as usize;
            if classic != value {
                differ.push(format!(
                    "{}: substitution {classic:#b}, value {value:#b}",
                    k.s.names.display(n)
                ));
            }
        }
        assert!(
            differ.is_empty(),
            "the engines disagree:\n{}",
            differ.join("\n")
        );
        assert!(
            ignorable >= 20,
            "some definitions do have ignorable arguments ({ignorable})"
        );
    })
}

/// The loose de Bruijn indices of `e` below 64, relative to `depth`, found by walking it: the
/// reference the analysis is held to, which reads none of the masks the store keeps.
fn walked_mask(
    k: &Kernel,
    e: ExprId,
    depth: u32,
    memo: &mut std::collections::HashMap<(ExprId, u32), u64>,
) -> u64 {
    if let Some(&m) = memo.get(&(e, depth)) {
        return m;
    }
    let m = match k.s.node(e) {
        Node::BVar(i) if i >= depth && i - depth < 64 => 1 << (i - depth),
        Node::App(a, b) => walked_mask(k, a, depth, memo) | walked_mask(k, b, depth, memo),
        Node::Lam(a, b) | Node::Pi(a, b) => {
            walked_mask(k, a, depth, memo) | walked_mask(k, b, depth + 1, memo)
        }
        Node::Let(a, b, c) => {
            walked_mask(k, a, depth, memo)
                | walked_mask(k, b, depth, memo)
                | walked_mask(k, c, depth + 1, memo)
        }
        Node::Proj(_, _, a) => walked_mask(k, a, depth, memo),
        _ => 0,
    };
    memo.insert((e, depth), m);
    m
}

/// The absent parameters of a definition by the rule: the positions its value's body (under at
/// most 64 λs) never mentions and the rest of its type does not depend on.
fn walked_absent(k: &Kernel, ty: ExprId, value: ExprId) -> u64 {
    let mut body = value;
    let mut arity = 0;
    while let Node::Lam(_, b) = k.s.node(body) {
        if arity == 64 {
            break;
        }
        body = b;
        arity += 1;
    }
    let used = walked_mask(k, body, 0, &mut Default::default());
    let mut absent = 0;
    let mut rest = ty;
    for i in 0..arity {
        let Node::Pi(_, cod) = k.s.node(rest) else {
            break;
        };
        let unused = (used >> (arity - 1 - i)) & 1 == 0;
        let independent = walked_mask(k, cod, 0, &mut Default::default()) & 1 == 0;
        if unused && independent {
            absent |= 1 << i;
        }
        rest = cod;
    }
    absent
}

#[test]
fn every_absent_mask_is_the_one_a_walk_of_the_definition_finds() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let defs: Vec<NameId> = (0..k.s.names.len() as u32)
            .map(NameId)
            .filter(|n| matches!(k.env.get(*n).map(|c| &c.kind), Some(Kind::Defn { .. })))
            .collect();
        let (mut checked, mut absent, mut differ) = (0, 0, Vec::new());
        for n in defs {
            let c = k.env.get(n).unwrap().clone();
            let ls = k.const_levels_of(&c.lparams);
            let Some(mask) = k.absent_mask(n, ls) else {
                continue;
            };
            let want = walked_absent(&k, c.ty, c.delta_value().unwrap());
            checked += 1;
            absent += (want != 0) as usize;
            if mask != want {
                differ.push(format!(
                    "{}: {mask:#b}, walked {want:#b}",
                    k.s.names.display(n)
                ));
            }
        }
        assert!(differ.is_empty(), "masks differ:\n{}", differ.join("\n"));
        assert!(checked >= 300, "{checked} definitions checked");
        assert!(absent >= 10, "some definitions drop a parameter ({absent})");
    })
}

// ---- signatures: what a constant's type says about its applications ----
//
// A constant's type, walked once at a universe instantiation, says which argument positions take
// proofs and, for every arity, whether an application to that many arguments is a proof. The
// value engine asks this instead of inferring a type each time proof irrelevance could apply.

use logicaffeine_kernel::dag::level::{LevelId, LevelsId};
use logicaffeine_kernel::dag::relevance::Signature;
use logicaffeine_kernel::dag::sem::value::{EnvId, Val, V};
use logicaffeine_kernel::dag::sem::Sem;
use logicaffeine_kernel::dag::{KResult, KernelFailure};
use std::rc::Rc;

fn signature(k: &mut Kernel, c: NameId, ls: LevelsId) -> Signature {
    let ps = k.env.get(c).unwrap().lparams.clone();
    k.with_decl(ps, Safety::Safe, |k| k.with_sem(|sem| sem.signature(c, ls)))
        .unwrap()
        .expect("a settled constant")
}

fn at_levels(k: &mut Kernel, ls: &[u64]) -> LevelsId {
    let ls: Vec<LevelId> = ls.iter().map(|n| k.s.levels.of_nat(*n)).collect();
    k.s.levels.list(&ls)
}

fn own_levels(k: &mut Kernel, c: NameId) -> LevelsId {
    let ps = k.env.get(c).unwrap().lparams.clone();
    k.const_levels_of(&ps)
}

#[test]
fn a_signature_knows_which_applications_are_proofs() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let succ = name(&mut k, "Nat.succ");
        let ls = own_levels(&mut k, succ);
        let s = signature(&mut k, succ, ls);
        assert_eq!(s.proof_args(), 0);
        assert_eq!(
            (s.is_proof(0), s.is_proof(1), s.is_proof(2)),
            (Some(false), Some(false), None),
            "Nat.succ : Nat → Nat"
        );

        let refl = name(&mut k, "Eq.refl");
        let ls = own_levels(&mut k, refl);
        let s = signature(&mut k, refl, ls);
        assert_eq!(
            s.proof_args(),
            0,
            "neither the type nor the element of Eq.refl is a proof"
        );
        for n in 0..=2 {
            assert_eq!(
                s.is_proof(n),
                Some(true),
                "Eq.refl applied to {n} arguments is a proof"
            );
        }

        let intro = name(&mut k, "And.intro");
        let s = signature(&mut k, intro, LevelsId::EMPTY);
        assert_eq!(
            s.proof_args(),
            0b1100,
            "And.intro {{a b : Prop}} (left : a) (right : b)"
        );
        assert_eq!(s.is_proof(4), Some(true));
        assert_eq!(
            s.is_proof(2),
            Some(true),
            "a → b → a ∧ b is itself a proposition"
        );

        let mk = name(&mut k, "Subtype.mk");
        let ls = own_levels(&mut k, mk);
        let s = signature(&mut k, mk, ls);
        assert_eq!(s.proof_args(), 0b1000, "Subtype.mk's property is a proof");
        assert_eq!(s.is_proof(4), Some(false));
        assert_eq!(
            s.is_proof(3),
            Some(false),
            "p v → Subtype p is a type of data"
        );

        let subtype = name(&mut k, "Subtype");
        let ls = own_levels(&mut k, subtype);
        let s = signature(&mut k, subtype, ls);
        assert_eq!(
            s.is_proposition(2),
            Some(false),
            "Subtype p lives in Sort (max 1 u)"
        );

        let and = name(&mut k, "And");
        let s = signature(&mut k, and, LevelsId::EMPTY);
        assert_eq!(s.is_proposition(2), Some(true), "a ∧ b is a proposition");
        assert_eq!(s.is_proposition(1), None, "And a is a function, not a type");
    })
}

#[test]
fn a_signature_depends_on_the_universe_instantiation() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let rec = name(&mut k, "Nat.rec");
        let ls = own_levels(&mut k, rec);
        let general = signature(&mut k, rec, ls);
        assert_eq!(
            general.is_proof(4),
            Some(false),
            "Nat.rec.{{u}} ... t : motive t, a Sort u that need not be Prop"
        );
        assert_eq!(
            general.is_proof(5),
            None,
            "past its telescope an application's type is unknown"
        );
        let zero = at_levels(&mut k, &[0]);
        let into_prop = signature(&mut k, rec, zero);
        assert_eq!(
            into_prop.is_proof(4),
            Some(true),
            "Nat.rec.{{0}} proves its motive"
        );
        assert_eq!(
            into_prop.proof_args() & 0b110,
            0b110,
            "into Prop, the minor premises are proofs too"
        );
    })
}

/// Every settled constant at every arity its telescope covers: the signature's answer equals the
/// answer inference gives on an application to fresh variables.
fn signature_agrees_with_inference(
    k: &mut Kernel,
    c: NameId,
    ls: LevelsId,
) -> Result<usize, String> {
    let sig = signature(k, c, ls);
    let ps = k.env.get(c).unwrap().lparams.clone();
    let shown = k.s.names.display(c);
    k.with_decl(ps, Safety::Safe, |k| {
        k.with_sem(|sem| {
            let e = sem.k.s.cnst(c, ls);
            let v = sem.eval(EnvId::EMPTY, e)?;
            Ok(walk(sem, v, &sig, 0))
        })
    })
    .map_err(|e| format!("{shown}: {e:?}"))?
    .map_err(|e| format!("{shown}: {e}"))
}

fn pi_domain(sem: &Sem, v: V) -> Option<V> {
    match sem.st.arena.get(v) {
        Val::Pi { dom, .. } | Val::PiLam { dom, .. } => Some(dom),
        _ => None,
    }
}

fn walk(sem: &mut Sem, app: V, sig: &Signature, n: usize) -> Result<usize, String> {
    let err = |e: KernelFailure| format!("{e:?}");
    let ty = sem.type_of(app).map_err(err)?;
    let probe = sem.is_prop(ty).map_err(err)?;
    if let Some(s) = sig.is_proof(n) {
        if s != probe {
            return Err(format!(
                "applied to {n}: signature says proof={s}, inference says {probe}"
            ));
        }
    }
    let w = sem.whnf(ty).map_err(err)?;
    let Some(dom) = pi_domain(sem, w) else {
        if let Some(p) = sig.is_proposition(n) {
            if !matches!(sem.st.arena.get(w), Val::Sort(_)) {
                return Err(format!(
                    "applied to {n}: signature says proposition={p}, but it is not a type"
                ));
            }
            if p != sem.is_prop(app).map_err(err)? {
                return Err(format!(
                    "applied to {n}: signature says proposition={p}, inference differs"
                ));
            }
        }
        return Ok(n + 1);
    };
    if n >= 64 {
        return Ok(n + 1);
    }
    let arg_is_proof = sem.is_prop(dom).map_err(err)?;
    if arg_is_proof != ((sig.proof_args() >> n) & 1 == 1) {
        return Err(format!(
            "argument {n}: signature says proof={}, inference says {arg_is_proof}",
            !arg_is_proof
        ));
    }
    sem.under(dom, |sem, x| {
        let next = sem.apply(app, x)?;
        Ok(walk(sem, next, sig, n + 1))
    })
    .map_err(err)?
}

#[test]
fn every_signature_agrees_with_inference_at_every_arity_and_instantiation() {
    run(|| {
        let mut k = load("good/init-prelude.ndjson");
        let consts: Vec<NameId> = (0..k.s.names.len() as u32)
            .map(NameId)
            .filter(|n| k.env.get(*n).is_some() && k.env.is_settled(*n))
            .collect();
        assert!(
            consts.len() >= 1000,
            "init-prelude has its constants ({})",
            consts.len()
        );
        let mut checked = 0;
        let mut bad = Vec::new();
        for c in consts {
            let nps = k.env.get(c).unwrap().lparams.len();
            let own = own_levels(&mut k, c);
            let zeros = at_levels(&mut k, &vec![0; nps]);
            let ones = at_levels(&mut k, &vec![1; nps]);
            for ls in [own, zeros, ones] {
                match signature_agrees_with_inference(&mut k, c, ls) {
                    Ok(n) => checked += n,
                    Err(e) => bad.push(e),
                }
            }
        }
        assert!(
            bad.is_empty(),
            "signatures disagree with inference:\n{}",
            bad.join("\n")
        );
        assert!(checked >= 10_000, "arities checked ({checked})");
    })
}

// ---- the proof-irrelevance gate: conversion asks the signature, not inference ----

/// Runs `f` inside a declaration with the value engine, under `α : Type`, `p : α → Prop`,
/// `v w : α`, `h₁ h₂ : p v`, `x y : Nat`, `k : α → p v` (in that order), and reports how many
/// values' proof status had to be inferred during `f`.
fn in_context(f: impl FnOnce(&mut Sem, &[V]) -> bool + Send + 'static) -> (bool, u64) {
    run(move || {
        let mut k = load("good/init-prelude.ndjson");
        let nat = name(&mut k, "Nat");
        let s = &mut k.s;
        let (b0, b1, b2, b3) = (s.bvar(0), s.bvar(1), s.bvar(2), s.bvar(3));
        let (ty, prop) = (s.levels.of_nat(1), s.prop());
        let ty = s.sort(ty);
        let pred = s.pi(b0, prop);
        let (h1, h2) = (s.app(b2, b1), s.app(b3, b2));
        let n = s.cnst(nat, LevelsId::EMPTY);
        let (b6, b7) = (s.bvar(6), s.bvar(7));
        let pv = s.app(b7, b6);
        let implication = s.pi(b7, pv);
        let tele = [ty, pred, b1, b2, h1, h2, n, n, implication];
        let whole = tele.iter().rev().fold(prop, |body, d| s.pi(*d, body));
        k.with_decl(Rc::from(Vec::new()), Safety::Safe, |k| {
            k.with_sem(|sem| {
                let v = sem.eval(EnvId::EMPTY, whole)?;
                telescope(sem, v, Vec::new(), f)
            })
        })
        .unwrap()
    })
}

fn telescope(
    sem: &mut Sem,
    pi: V,
    mut xs: Vec<V>,
    f: impl FnOnce(&mut Sem, &[V]) -> bool,
) -> KResult<(bool, u64)> {
    let Some(dom) = pi_domain(sem, pi) else {
        let before = sem.st.counters.proof_probes;
        let r = f(sem, &xs);
        return Ok((r, sem.st.counters.proof_probes - before));
    };
    sem.under(dom, |sem, x| {
        let rest = sem.instantiate(pi, x)?;
        xs.push(x);
        telescope(sem, rest, xs, f)
    })
}

fn konst(sem: &mut Sem, dotted: &str, ls: &[u64]) -> V {
    let n = sem.k.s.names.dotted(dotted);
    let ls: Vec<LevelId> = ls.iter().map(|l| sem.k.s.levels.of_nat(*l)).collect();
    let e = sem.k.s.cnst_levels(n, &ls);
    sem.eval(EnvId::EMPTY, e).unwrap()
}

fn app(sem: &mut Sem, f: V, args: &[V]) -> V {
    args.iter().fold(f, |f, a| sem.apply(f, *a).unwrap())
}

#[test]
fn constructor_applications_differing_only_in_a_proof_are_equal_without_inferring_a_type() {
    let (eq, probes) = in_context(|sem, c| {
        let mk = konst(sem, "Subtype.mk", &[1]);
        let t = app(sem, mk, &[c[0], c[1], c[2], c[4]]);
        let s = app(sem, mk, &[c[0], c[1], c[2], c[5]]);
        sem.is_def_eq(t, s).unwrap()
    });
    assert!(eq, "⟨v, h₁⟩ ≡ ⟨v, h₂⟩ by proof irrelevance");
    assert_eq!(
        probes, 0,
        "the property position is known to hold a proof; nothing is inferred"
    );
}

#[test]
fn constructor_applications_differing_in_data_are_not_equal() {
    let (eq, _) = in_context(|sem, c| {
        let mk = konst(sem, "Subtype.mk", &[1]);
        let t = app(sem, mk, &[c[0], c[1], c[2], c[4]]);
        let s = app(sem, mk, &[c[0], c[1], c[3], c[5]]);
        sem.is_def_eq(t, s).unwrap()
    });
    assert!(!eq, "⟨v, h₁⟩ ≢ ⟨w, h₂⟩: the values differ");
}

#[test]
fn two_hypotheses_proving_one_proposition_are_equal() {
    let (eq, _) = in_context(|sem, c| sem.is_def_eq(c[4], c[5]).unwrap());
    assert!(eq, "h₁ ≡ h₂ : p v");
}

#[test]
fn a_proof_by_a_constructor_equals_a_hypothesis_of_the_same_proposition() {
    let (eq, _) = in_context(|sem, c| {
        let eq = konst(sem, "Eq", &[2]);
        let prop = app(sem, eq, &[c[0], c[2], c[2]]);
        sem.under(prop, |sem, h| {
            let refl = konst(sem, "Eq.refl", &[2]);
            let r = app(sem, refl, &[c[0], c[2]]);
            sem.is_def_eq(r, h)
        })
        .unwrap()
    });
    assert!(eq, "Eq.refl v ≡ h : v = v");
}

#[test]
fn successors_of_distinct_numbers_differ_without_inferring_a_type() {
    let (eq, probes) = in_context(|sem, c| {
        let succ = konst(sem, "Nat.succ", &[]);
        let t = app(sem, succ, &[c[6]]);
        let s = app(sem, succ, &[c[7]]);
        sem.is_def_eq(t, s).unwrap()
    });
    assert!(!eq, "x.succ ≢ y.succ for distinct variables");
    assert_eq!(
        probes, 0,
        "neither a successor nor a variable of type Nat can be a proof"
    );
}

#[test]
fn a_hypothesis_equals_a_lambda_proving_the_same_implication() {
    let (eq, _) = in_context(|sem, c| {
        let env = c
            .iter()
            .fold(EnvId::EMPTY, |env, x| sem.st.arena.extend(env, *x));
        let (alpha, h1) = (sem.k.s.bvar(8), sem.k.s.bvar(5));
        let lam = sem.k.s.lam(alpha, h1);
        let lam = sem.eval(env, lam).unwrap();
        assert!(
            matches!(sem.st.arena.get(lam), Val::Lam { .. }),
            "fun (_ : α) => h₁ is a lambda"
        );
        sem.is_def_eq(c[8], lam).unwrap()
    });
    assert!(
        eq,
        "k ≡ fun _ => h₁ : α → p v, both proofs of one implication"
    );
}

#[test]
fn a_signature_knows_nothing_past_its_tracked_arity() {
    let s = Signature::default();
    for k in [64, 65, 100, 1_000, usize::MAX] {
        assert_eq!(
            s.is_proof(k),
            None,
            "an application to {k} arguments is past what a signature tracks"
        );
        assert_eq!(s.is_proposition(k), None);
    }
}
