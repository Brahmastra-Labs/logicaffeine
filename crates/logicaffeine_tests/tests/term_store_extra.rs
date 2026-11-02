//! `term_store` — the hash-consed representation, tested to the point of absurdity.
//!
//! The store is a REPRESENTATION, and a representation is only worth anything if its
//! identity relation is exactly the one the rest of the kernel already believes in. So the
//! tests here come in pairs: what the store now makes cheap, and the neighbouring thing it
//! must still get RIGHT.
//!
//! * Every claim about node structure is locked against an INDEPENDENT oracle written from
//!   the specification in this file — a naive tree substitution, a tree free-variable walk, a
//!   de Bruijn α-equivalence encoder — never by copying the store's own code. Three soundness
//!   bugs this campaign were miscounted identically in two places and every test agreed with
//!   the bug.
//! * The randomised differential (`deterministic_terms`) runs a fixed LCG, so a failure is
//!   reproducible from the seed printed in the assertion.
//! * `beta_zeta_normalize` is additionally checked against `reduction::normalize` — the
//!   kernel's own reducer — on the β-only fragment where the two must agree exactly.
//!
//! The one place where identity and `Term` equality genuinely diverge is
//! floating-point, and it diverges in BOTH directions. That is not a wart to hide: it is a
//! defect in `Literal`'s `Eq`, and the tests below pin the exact behaviour so that nobody
//! swaps `a == b` for an id compare without reading them.

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_base::BigInt;
use logicaffeine_kernel::term_store::{IdCache, Node, Occurrences, StoreNormalForm, TermStore};
use logicaffeine_kernel::{normalize, Context, Literal, Term, Universe};

// ================================================================================================
// HELPERS — per-file, per house style. There is no shared kernel helper module.
// ================================================================================================

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app4(f: Term, w: Term, x: Term, y: Term, z: Term) -> Term {
    app(app(app(app(f, w), x), y), z)
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
fn fix(n: &str, body: Term) -> Term {
    Term::Fix {
        name: n.to_string(),
        body: Box::new(body),
    }
}
fn nat() -> Term {
    g("Nat")
}

/// The node count of a `Term` TREE — the measure the corpus case uses, reproduced here so
/// the store's O(1) `tree_size` is checked against an independent walk.
fn tree_nodes(t: &Term) -> u64 {
    1 + match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => 0,
        Term::Annotated { term, .. } => return tree_nodes(term),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => tree_nodes(param_type) + tree_nodes(body_type),
        Term::Lambda {
            param_type, body, ..
        } => tree_nodes(param_type) + tree_nodes(body),
        Term::App(f, a) => tree_nodes(f) + tree_nodes(a),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            tree_nodes(discriminant)
                + tree_nodes(motive)
                + cases.iter().map(tree_nodes).sum::<u64>()
        }
        Term::Fix { body, .. } => tree_nodes(body),
        Term::MutualFix { defs, .. } => defs.iter().map(|(_, b)| tree_nodes(b)).sum::<u64>(),
        Term::Let {
            ty, value, body, ..
        } => tree_nodes(ty) + tree_nodes(value) + tree_nodes(body),
    }
}

/// The maximum height of a `Term` TREE — the independent oracle for `TermStore::depth`.
fn tree_depth(t: &Term) -> u32 {
    let kids: Vec<u32> = match t {
        Term::Sort(_)
        | Term::Var(_)
        | Term::Global(_)
        | Term::Const { .. }
        | Term::Lit(_)
        | Term::Hole => vec![],
        Term::Annotated { term, .. } => return tree_depth(term),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => vec![tree_depth(param_type), tree_depth(body_type)],
        Term::Lambda {
            param_type, body, ..
        } => vec![tree_depth(param_type), tree_depth(body)],
        Term::App(f, a) => vec![tree_depth(f), tree_depth(a)],
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let mut v = vec![tree_depth(discriminant), tree_depth(motive)];
            v.extend(cases.iter().map(tree_depth));
            v
        }
        Term::Fix { body, .. } => vec![tree_depth(body)],
        Term::MutualFix { defs, .. } => defs.iter().map(|(_, b)| tree_depth(b)).collect(),
        Term::Let {
            ty, value, body, ..
        } => {
            vec![tree_depth(ty), tree_depth(value), tree_depth(body)]
        }
    };
    1 + kids.into_iter().max().unwrap_or(0)
}

/// Free variables of a `Term`, by the textbook rule — the INDEPENDENT oracle for the store's
/// hash-consed free sets. Written from the definition, deliberately not from the store's code.
fn tree_free(t: &Term) -> BTreeSet<String> {
    match t {
        Term::Var(n) => [n.clone()].into_iter().collect(),
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {
            BTreeSet::new()
        }
        Term::Annotated { term, .. } => tree_free(term),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let mut s = tree_free(param_type);
            let mut inner = tree_free(body_type);
            inner.remove(param);
            s.extend(inner);
            s
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let mut s = tree_free(param_type);
            let mut inner = tree_free(body);
            inner.remove(param);
            s.extend(inner);
            s
        }
        Term::App(f, a) => {
            let mut s = tree_free(f);
            s.extend(tree_free(a));
            s
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            let mut s = tree_free(discriminant);
            s.extend(tree_free(motive));
            for c in cases {
                s.extend(tree_free(c));
            }
            s
        }
        Term::Fix { name, body } => {
            let mut inner = tree_free(body);
            inner.remove(name);
            inner
        }
        Term::MutualFix { defs, .. } => {
            let mut inner = BTreeSet::new();
            for (_, b) in defs {
                inner.extend(tree_free(b));
            }
            for (n, _) in defs {
                inner.remove(n);
            }
            inner
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            let mut s = tree_free(ty);
            s.extend(tree_free(value));
            let mut inner = tree_free(body);
            inner.remove(name);
            s.extend(inner);
            s
        }
    }
}

/// Free occurrences of one name in a `Term` TREE — the independent oracle for
/// `occurrence_count`, counting OCCURRENCES and not merely presence.
fn tree_occurrences(t: &Term, name: &str) -> u64 {
    match t {
        Term::Var(n) => u64::from(n == name),
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => 0,
        Term::Annotated { term, .. } => tree_occurrences(term, name),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            tree_occurrences(param_type, name)
                + if param == name {
                    0
                } else {
                    tree_occurrences(body_type, name)
                }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            tree_occurrences(param_type, name)
                + if param == name {
                    0
                } else {
                    tree_occurrences(body, name)
                }
        }
        Term::App(f, a) => tree_occurrences(f, name) + tree_occurrences(a, name),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            tree_occurrences(discriminant, name)
                + tree_occurrences(motive, name)
                + cases.iter().map(|c| tree_occurrences(c, name)).sum::<u64>()
        }
        Term::Fix { name: n, body } => {
            if n == name {
                0
            } else {
                tree_occurrences(body, name)
            }
        }
        Term::MutualFix { defs, .. } => {
            if defs.iter().any(|(n, _)| n == name) {
                0
            } else {
                defs.iter().map(|(_, b)| tree_occurrences(b, name)).sum()
            }
        }
        Term::Let {
            name: n,
            ty,
            value,
            body,
        } => {
            tree_occurrences(ty, name)
                + tree_occurrences(value, name)
                + if n == name {
                    0
                } else {
                    tree_occurrences(body, name)
                }
        }
    }
}

/// A capture-avoiding substitution written NAIVELY from the definition — the independent
/// oracle `TermStore::substitute` is differentially tested against. It renames a binder by
/// appending `'` until the name is unused anywhere in either term, which is a different
/// freshening discipline from the store's, so agreement can only be up to α.
fn naive_subst(t: &Term, name: &str, value: &Term) -> Term {
    match t {
        Term::Var(n) => {
            if n == name {
                value.clone()
            } else {
                t.clone()
            }
        }
        Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {
            t.clone()
        }
        Term::Annotated { annotation, term } => {
            Term::annotated(annotation.clone(), naive_subst(term, name, value))
        }
        Term::App(f, a) => app(naive_subst(f, name, value), naive_subst(a, name, value)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => Term::Match {
            discriminant: Box::new(naive_subst(discriminant, name, value)),
            motive: Box::new(naive_subst(motive, name, value)),
            cases: cases.iter().map(|c| naive_subst(c, name, value)).collect(),
        },
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            let ty = naive_subst(param_type, name, value);
            let (p, body) = naive_binder(param, body_type, name, value);
            Term::Pi {
                param: p,
                param_type: Box::new(ty),
                body_type: Box::new(body),
            }
        }
        Term::Lambda {
            param,
            param_type,
            body,
        } => {
            let ty = naive_subst(param_type, name, value);
            let (p, b) = naive_binder(param, body, name, value);
            Term::Lambda {
                param: p,
                param_type: Box::new(ty),
                body: Box::new(b),
            }
        }
        Term::Fix { name: n, body } => {
            let (p, b) = naive_binder(n, body, name, value);
            Term::Fix {
                name: p,
                body: Box::new(b),
            }
        }
        Term::Let {
            name: n,
            ty,
            value: v,
            body,
        } => {
            let ty2 = naive_subst(ty, name, value);
            let v2 = naive_subst(v, name, value);
            let (p, b) = naive_binder(n, body, name, value);
            Term::Let {
                name: p,
                ty: Box::new(ty2),
                value: Box::new(v2),
                body: Box::new(b),
            }
        }
        Term::MutualFix { defs, index } => {
            if defs.iter().any(|(n, _)| n == name) {
                return t.clone();
            }
            // A mutual block binds every name in every body, so any name the replacement
            // mentions must be renamed across the WHOLE block at once.
            let clashing: Vec<String> = defs
                .iter()
                .map(|(n, _)| n.clone())
                .filter(|n| tree_free(value).contains(n))
                .collect();
            let mut renames: BTreeMap<String, String> = BTreeMap::new();
            for n in &clashing {
                let mut fresh = format!("{n}'");
                while defs
                    .iter()
                    .any(|(m, b)| m == &fresh || tree_free(b).contains(&fresh))
                    || tree_free(value).contains(&fresh)
                {
                    fresh.push('\'');
                }
                renames.insert(n.clone(), fresh);
            }
            let renamed: Vec<(String, Term)> = defs
                .iter()
                .map(|(n, b)| {
                    let mut b2 = b.clone();
                    for (from, to) in &renames {
                        b2 = naive_subst(&b2, from, &var(to));
                    }
                    (renames.get(n).cloned().unwrap_or_else(|| n.clone()), b2)
                })
                .collect();
            Term::MutualFix {
                defs: renamed
                    .into_iter()
                    .map(|(n, b)| (n, naive_subst(&b, name, value)))
                    .collect(),
                index: *index,
            }
        }
    }
}

fn naive_binder(param: &str, body: &Term, name: &str, value: &Term) -> (String, Term) {
    if param == name {
        return (param.to_string(), body.clone());
    }
    if !tree_free(value).contains(param) {
        return (param.to_string(), naive_subst(body, name, value));
    }
    let mut fresh = format!("{param}'");
    while tree_free(body).contains(&fresh)
        || tree_free(value).contains(&fresh)
        || bound_names(body).contains(&fresh)
    {
        fresh.push('\'');
    }
    let renamed = naive_subst(body, param, &var(&fresh));
    (fresh, naive_subst(&renamed, name, value))
}

fn bound_names(t: &Term) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(t: &Term, out: &mut BTreeSet<String>) {
        match t {
            Term::Sort(_)
            | Term::Var(_)
            | Term::Global(_)
            | Term::Const { .. }
            | Term::Lit(_)
            | Term::Hole => {}
            Term::Annotated { term, .. } => walk(term, out),
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                out.insert(param.clone());
                walk(param_type, out);
                walk(body_type, out);
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                out.insert(param.clone());
                walk(param_type, out);
                walk(body, out);
            }
            Term::App(f, a) => {
                walk(f, out);
                walk(a, out);
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                walk(discriminant, out);
                walk(motive, out);
                for c in cases {
                    walk(c, out);
                }
            }
            Term::Fix { name, body } => {
                out.insert(name.clone());
                walk(body, out);
            }
            Term::MutualFix { defs, .. } => {
                for (n, b) in defs {
                    out.insert(n.clone());
                    walk(b, out);
                }
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                out.insert(name.clone());
                walk(ty, out);
                walk(value, out);
                walk(body, out);
            }
        }
    }
    walk(t, &mut out);
    out
}

/// A canonical de Bruijn spelling — two terms are α-equivalent exactly when their spellings
/// are equal. The comparison the differential needs, since the store and the naive oracle
/// freshen differently on purpose.
fn alpha_key(t: &Term) -> String {
    fn go(t: &Term, scope: &mut Vec<String>, out: &mut String) {
        match t {
            Term::Sort(u) => out.push_str(&format!("S<{u}>")),
            Term::Annotated { term, .. } => go(term, scope, out),
            Term::Var(n) => match scope.iter().rev().position(|b| b == n) {
                Some(i) => out.push_str(&format!("b{i};")),
                None => out.push_str(&format!("f<{n}>;")),
            },
            Term::Global(n) => out.push_str(&format!("g<{n}>;")),
            Term::Const { name, levels } => {
                out.push_str(&format!("c<{name}>["));
                for l in levels {
                    out.push_str(&format!("{l},"));
                }
                out.push(']');
            }
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                out.push_str("P(");
                go(param_type, scope, out);
                out.push_str(")(");
                scope.push(param.clone());
                go(body_type, scope, out);
                scope.pop();
                out.push(')');
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                out.push_str("L(");
                go(param_type, scope, out);
                out.push_str(")(");
                scope.push(param.clone());
                go(body, scope, out);
                scope.pop();
                out.push(')');
            }
            Term::App(f, a) => {
                out.push_str("A(");
                go(f, scope, out);
                out.push_str(")(");
                go(a, scope, out);
                out.push(')');
            }
            Term::Match {
                discriminant,
                motive,
                cases,
            } => {
                out.push_str("M(");
                go(discriminant, scope, out);
                out.push_str(")(");
                go(motive, scope, out);
                out.push_str(")[");
                for c in cases {
                    go(c, scope, out);
                    out.push(',');
                }
                out.push(']');
            }
            Term::Fix { name, body } => {
                out.push_str("F(");
                scope.push(name.clone());
                go(body, scope, out);
                scope.pop();
                out.push(')');
            }
            Term::MutualFix { defs, index } => {
                out.push_str(&format!("X{index}["));
                for (n, _) in defs {
                    scope.push(n.clone());
                }
                for (_, b) in defs {
                    go(b, scope, out);
                    out.push(',');
                }
                for _ in defs {
                    scope.pop();
                }
                out.push(']');
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                out.push_str("T(");
                go(ty, scope, out);
                out.push_str(")(");
                go(value, scope, out);
                out.push_str(")(");
                scope.push(name.clone());
                go(body, scope, out);
                scope.pop();
                out.push(')');
            }
            Term::Lit(l) => out.push_str(&format!("l<{l:?}>;")),
            Term::Hole => out.push_str("H;"),
        }
    }
    let mut s = String::new();
    go(t, &mut Vec::new(), &mut s);
    s
}

fn alpha_eq(a: &Term, b: &Term) -> bool {
    alpha_key(a) == alpha_key(b)
}

/// One `Term` of every variant, with every `Literal` and every `Universe` shape represented.
/// The corpus every structural invariant is quantified over.
fn variant_corpus() -> Vec<Term> {
    vec![
        Term::Sort(Universe::SProp),
        Term::Sort(Universe::Prop),
        Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::Type(7)),
        Term::Sort(Universe::Var("u".to_string())),
        Term::Sort(Universe::Succ(Box::new(Universe::Var("u".to_string())))),
        Term::Sort(Universe::Max(
            Box::new(Universe::Type(1)),
            Box::new(Universe::Var("v".to_string())),
        )),
        Term::Sort(Universe::IMax(
            Box::new(Universe::Var("u".to_string())),
            Box::new(Universe::Prop),
        )),
        var("x"),
        g("Nat"),
        Term::Const {
            name: "id".to_string(),
            levels: vec![],
        },
        Term::Const {
            name: "cast".to_string(),
            levels: vec![Universe::Type(0), Universe::Var("u".to_string())],
        },
        pi("x", nat(), nat()),
        pi("x", nat(), var("x")),
        lam("x", nat(), var("x")),
        lam("x", nat(), var("y")),
        app(var("f"), var("x")),
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("k", nat(), nat())),
            cases: vec![g("Zero"), lam("m", nat(), var("m"))],
        },
        Term::Match {
            discriminant: Box::new(var("n")),
            motive: Box::new(lam("k", nat(), nat())),
            cases: vec![],
        },
        fix("f", lam("n", nat(), app(var("f"), var("n")))),
        Term::MutualFix {
            defs: vec![
                (
                    "even".to_string(),
                    lam("n", nat(), app(var("odd"), var("n"))),
                ),
                (
                    "odd".to_string(),
                    lam("n", nat(), app(var("even"), var("n"))),
                ),
            ],
            index: 1,
        },
        let_in("b", nat(), g("Zero"), app(var("b"), var("b"))),
        Term::Lit(Literal::Int(-5)),
        Term::Lit(Literal::Int(0)),
        Term::Lit(Literal::Float(1.5)),
        Term::Lit(Literal::Text("hello".to_string())),
        Term::Lit(Literal::Duration(1_000_000)),
        Term::Lit(Literal::Date(-19_000)),
        Term::Lit(Literal::Moment(1_700_000_000_000_000_000)),
        Term::Lit(Literal::BigInt(BigInt::from_i64(1_234_567_890_123))),
        Term::Lit(Literal::Nat(BigInt::from_i64(42))),
        Term::Hole,
    ]
}

/// A deterministic pseudo-random term generator — the differential's corpus. The LCG means a
/// failure is reproducible from the seed the assertion prints.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const POOL: &[&str] = &["x", "y", "z", "f"];

fn random_term(rng: &mut Rng, depth: u32) -> Term {
    if depth == 0 {
        return match rng.below(4) {
            0 => var(POOL[rng.below(POOL.len() as u64) as usize]),
            1 => g("A"),
            2 => Term::Sort(Universe::Type(0)),
            _ => Term::Lit(Literal::Int(rng.below(3) as i64)),
        };
    }
    match rng.below(9) {
        0 => var(POOL[rng.below(POOL.len() as u64) as usize]),
        1 => g("A"),
        2 => app(random_term(rng, depth - 1), random_term(rng, depth - 1)),
        3 => lam(
            POOL[rng.below(POOL.len() as u64) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        4 => pi(
            POOL[rng.below(POOL.len() as u64) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        5 => let_in(
            POOL[rng.below(POOL.len() as u64) as usize],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
        ),
        6 => fix(
            POOL[rng.below(POOL.len() as u64) as usize],
            random_term(rng, depth - 1),
        ),
        7 => Term::Match {
            discriminant: Box::new(random_term(rng, depth - 1)),
            motive: Box::new(random_term(rng, depth - 1)),
            cases: (0..rng.below(3))
                .map(|_| random_term(rng, depth - 1))
                .collect(),
        },
        _ => Term::MutualFix {
            defs: (0..=rng.below(2))
                .map(|i| {
                    (
                        POOL[((i + 1) as usize) % POOL.len()].to_string(),
                        random_term(rng, depth - 1),
                    )
                })
                .collect(),
            index: 0,
        },
    }
}

/// The β-only fragment, for the differential against the kernel's own `normalize`. No `Let`
/// (the kernel always ζ-expands, the store gates), no `Match`/`Fix` (ι and fixpoint unfolding
/// are deliberately not implemented), no `Global` the prelude gives a meaning to.
fn random_beta_term(rng: &mut Rng, depth: u32) -> Term {
    if depth == 0 {
        return match rng.below(3) {
            0 => var(POOL[rng.below(POOL.len() as u64) as usize]),
            1 => g("A"),
            _ => g("B"),
        };
    }
    match rng.below(5) {
        0 => var(POOL[rng.below(POOL.len() as u64) as usize]),
        1 => g("A"),
        2 => app(
            random_beta_term(rng, depth - 1),
            random_beta_term(rng, depth - 1),
        ),
        3 => lam(
            POOL[rng.below(POOL.len() as u64) as usize],
            g("B"),
            random_beta_term(rng, depth - 1),
        ),
        _ => pi(
            POOL[rng.below(POOL.len() as u64) as usize],
            g("B"),
            random_beta_term(rng, depth - 1),
        ),
    }
}

// ================================================================================================
// THE STRUCTURAL INVARIANTS — the three walkers must agree or the store silently corrupts
// ================================================================================================

/// `for_each_child`, `child_count` and `rebuild_with` are three views of one fact. If they
/// ever disagree, substitution rebuilds a node with the wrong children and NOTHING else in
/// the store notices, so they are locked against each other over every variant.
#[test]
fn for_each_child_child_count_and_rebuild_with_agree_on_every_variant() {
    let mut store = TermStore::new();
    for t in variant_corpus() {
        let id = store.intern(&t);
        let node = store.node(id).clone();
        let mut kids = Vec::new();
        node.for_each_child(&mut |c| kids.push(c));
        assert_eq!(
            kids.len(),
            node.child_count(),
            "child_count must equal what for_each_child yields, for {t}"
        );
        assert_eq!(
            node.rebuild_with(&kids),
            node,
            "rebuilding a node with its own children must reproduce it, for {t}"
        );
    }
}

/// The round trip. Interning is only a representation change if `extract ∘ intern` is the
/// identity on `Term`, exactly — every variant, every literal, every universe shape.
#[test]
fn intern_then_extract_round_trips_every_term_variant() {
    let mut store = TermStore::new();
    for t in variant_corpus() {
        let id = store.intern(&t);
        assert_eq!(
            store.extract(id),
            t,
            "extract ∘ intern must be the identity on {t}"
        );
    }
}

/// And on randomly-generated terms, which reach nesting combinations the hand-written corpus
/// does not.
#[test]
fn intern_then_extract_round_trips_a_randomised_corpus() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x5EED_1234);
    for i in 0..4000 {
        let t = random_term(&mut rng, 4);
        let id = store.intern(&t);
        assert_eq!(
            store.extract(id),
            t,
            "round trip failed at case {i} (seed 0x5EED1234): {t}"
        );
    }
}

/// Identity IS structural equality: interning the same structure twice yields the same id,
/// and interning different structures never does.
///
/// The `!=` half is the one that matters. A positive-only test is satisfied by a store that
/// returns `TermId(0)` for everything.
#[test]
fn interned_identity_is_exactly_structural_equality() {
    let mut store = TermStore::new();
    let corpus: Vec<Term> = variant_corpus()
        .into_iter()
        .filter(|t| !matches!(t, Term::Lit(Literal::Float(_))))
        .collect();
    let ids: Vec<_> = corpus.iter().map(|t| store.intern(t)).collect();
    for (i, a) in corpus.iter().enumerate() {
        for (j, b) in corpus.iter().enumerate() {
            assert_eq!(
                ids[i] == ids[j],
                a == b,
                "id equality and Term equality must agree on ({a}, {b})"
            );
        }
    }
    // Interning a second time must not mint a new id, and must not grow the store.
    let before = store.len();
    for (i, t) in corpus.iter().enumerate() {
        assert_eq!(
            store.intern(t),
            ids[i],
            "re-interning {t} must return the same id"
        );
    }
    assert_eq!(
        before,
        store.len(),
        "re-interning an existing term must add no nodes"
    );
}

/// α-equivalent terms with DIFFERENT binder names are DIFFERENT ids — the store's identity is
/// structural, exactly as `Term`'s derived `PartialEq` is. A store that quietly quotiented by
/// α would make `recheck`'s de Bruijn kernel agree with the named one for the wrong reason.
#[test]
fn alpha_equivalent_but_differently_spelled_terms_are_distinct_ids() {
    let mut store = TermStore::new();
    let a = lam("x", nat(), var("x"));
    let b = lam("y", nat(), var("y"));
    assert!(alpha_eq(&a, &b), "the two spellings are α-equivalent");
    assert_ne!(a, b, "but they are not equal `Term`s");
    assert_ne!(
        store.intern(&a),
        store.intern(&b),
        "so they must not share an id"
    );
}

// ================================================================================================
// THE FLOAT CAVEAT — where identity and `Term` equality genuinely part company
// ================================================================================================

/// `0.0` and `-0.0` are DISTINCT terms: `Literal` equality is bit identity (`term.rs`), as in Lean,
/// where `(0.0 : Float) = -0.0` is not closed by `rfl`. The store splits them for the same reason,
/// so id equality and `Term` equality agree; `float_free` stays the conservative O(1) guard.
#[test]
fn positive_and_negative_zero_are_distinct_terms_with_distinct_ids() {
    let mut store = TermStore::new();
    let pos = Term::Lit(Literal::Float(0.0));
    let neg = Term::Lit(Literal::Float(-0.0));
    assert_ne!(
        pos, neg,
        "`Term` equality never identifies +0.0 with -0.0, whose bits differ"
    );
    let (a, b) = (store.intern(&pos), store.intern(&neg));
    assert_ne!(a, b, "interning splits them, because their bits differ");
    assert_eq!(
        store.same(a, b),
        pos == neg,
        "id equality agrees with `Term` equality"
    );
    assert!(
        !store.float_free(a),
        "the conservative guard still flags every float"
    );
    assert!(!store.float_free(b));
    assert_eq!(
        store.extract(a),
        pos,
        "each side still round-trips to its own spelling"
    );
    assert_eq!(store.extract(b), neg);
    assert_ne!(store.extract(a), store.extract(b));
}

/// And the other direction. `Literal` equality is bit identity, so a `NaN` term IS equal to
/// itself and `Term`'s `Eq` keeps its promise of reflexivity; the store interns identical bits
/// to one id, and a `NaN` with a different payload is a different term with a different id.
#[test]
fn a_nan_term_is_equal_to_itself_and_the_store_agrees() {
    let mut store = TermStore::new();
    let nan = Term::Lit(Literal::Float(f64::NAN));
    assert_eq!(nan, nan.clone(), "`Term` equality is reflexive on a NaN");
    let id = store.intern(&nan);
    assert_eq!(
        store.intern(&nan.clone()),
        id,
        "the store interns identical bits to one id"
    );
    assert_eq!(
        store.extract(id),
        nan,
        "and the extracted term is the same term"
    );
    let other_payload = f64::from_bits(f64::NAN.to_bits() ^ 1);
    assert!(other_payload.is_nan());
    let other = Term::Lit(Literal::Float(other_payload));
    assert_ne!(nan, other, "two NaN payloads are two different literals");
    assert_ne!(store.intern(&other), id, "and two different ids");
    assert!(
        !store.float_free(id),
        "the conservative guard still flags every float"
    );
}

/// Every other term in the corpus IS float-free, so the guard is not vacuously false.
#[test]
fn a_float_free_term_reports_float_free_and_a_nested_float_does_not() {
    let mut store = TermStore::new();
    for t in variant_corpus() {
        let id = store.intern(&t);
        let contains_float = format!("{t:?}").contains("Float(");
        assert_eq!(
            store.float_free(id),
            !contains_float,
            "float_free must be exactly 'contains no Float literal', for {t}"
        );
    }
    // And it propagates UP: a float buried under four binders still poisons the root.
    let buried = lam(
        "a",
        nat(),
        lam("b", nat(), app(var("a"), Term::Lit(Literal::Float(2.5)))),
    );
    let id = store.intern(&buried);
    assert!(
        !store.float_free(id),
        "a float anywhere below makes the whole term unguarded"
    );
}

// ================================================================================================
// THE ANNOTATIONS — O(1) answers, checked against independent walks
// ================================================================================================

/// `depth` and `tree_size` are computed once at construction. They must equal what an
/// independent tree walk reports, on every variant and on a randomised corpus.
#[test]
fn depth_and_tree_size_match_an_independent_tree_walk() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x0D3F_7000_u64.wrapping_mul(3));
    let mut cases = variant_corpus();
    for _ in 0..2000 {
        cases.push(random_term(&mut rng, 4));
    }
    for t in &cases {
        let id = store.intern(t);
        assert_eq!(
            store.depth(id),
            tree_depth(t),
            "depth must match the tree walk for {t}"
        );
        assert_eq!(
            store.tree_size(id),
            tree_nodes(t),
            "tree_size must match the tree walk for {t}"
        );
    }
}

/// The boundary, stated explicitly: a leaf is depth 1, and one rung either side of a shape
/// changes the answer by exactly one.
#[test]
fn depth_counts_the_taller_child_one_rung_either_side() {
    let mut store = TermStore::new();
    let leaf = store.mk_global("Zero");
    assert_eq!(store.depth(leaf), 1, "a leaf is depth 1");
    let one = store.mk_app(leaf, leaf);
    assert_eq!(store.depth(one), 2, "one rung up");
    let two = store.mk_app(leaf, one);
    assert_eq!(
        store.depth(two),
        3,
        "and the TALLER child decides, not the first"
    );
    let two_other = store.mk_app(one, leaf);
    assert_eq!(store.depth(two_other), 3, "whichever side it is on");
    let deep_domain = store.mk_lambda("x", two, leaf);
    assert_eq!(
        store.depth(deep_domain),
        4,
        "a binder's DOMAIN counts toward its depth"
    );
}

/// `tree_size` SATURATES rather than wrapping. A DAG that doubles at every level denotes a
/// tree with 2^n nodes, and a wrapped count would understate a term by an astronomical factor
/// — precisely the reading a caller would use to decide whether unfolding is safe.
#[test]
fn tree_size_saturates_instead_of_wrapping() {
    let mut store = TermStore::new();
    let mut cur = store.mk_global("Zero");
    for _ in 0..80 {
        cur = store.mk_app(cur, cur);
    }
    assert_eq!(
        store.tree_size(cur),
        u64::MAX,
        "80 doublings must saturate, not wrap"
    );
    assert_eq!(store.dag_size(cur), 81, "while the DAG is 81 nodes");
    assert_eq!(store.depth(cur), 81);
    // And the guarded extraction refuses it instead of trying to allocate 2^80 nodes.
    let refused = store.extract_bounded(cur, 1_000_000);
    assert!(
        refused.is_err(),
        "an unfolding this large must be refused, not attempted"
    );
}

/// `dag_size` counts DISTINCT nodes and `tree_size` counts the unfolding. On a term with no
/// sharing they agree; the moment anything is shared they must not.
#[test]
fn dag_size_and_tree_size_agree_exactly_when_nothing_is_shared() {
    let mut store = TermStore::new();
    let unshared = lam("x", nat(), var("y"));
    let id = store.intern(&unshared);
    assert_eq!(
        store.dag_size(id) as u64,
        store.tree_size(id),
        "nothing repeats in λx:Nat. y, so the DAG is the tree"
    );
    let shared = app(var("x"), var("x"));
    let sid = store.intern(&shared);
    assert_eq!(store.tree_size(sid), 3, "a tree walk sees three nodes");
    assert_eq!(store.dag_size(sid), 2, "the DAG holds two");
}

/// The structural hash is O(1), stable across stores, and separates neighbours. A hash that
/// collided on everything would still pass a dedup test — the store would just be slow — so
/// the discrimination is asserted directly.
#[test]
fn the_structural_hash_is_stable_and_separates_neighbours() {
    let corpus = variant_corpus();
    let mut a = TermStore::new();
    let mut b = TermStore::new();
    // Interned in a DIFFERENT order, so the hash cannot be a function of the id.
    let ids_a: Vec<_> = corpus.iter().map(|t| a.intern(t)).collect();
    let ids_b: Vec<_> = corpus.iter().rev().map(|t| b.intern(t)).collect();
    for (i, t) in corpus.iter().enumerate() {
        let j = corpus.len() - 1 - i;
        assert_eq!(
            a.structural_hash(ids_a[i]),
            b.structural_hash(ids_b[j]),
            "the hash must depend on structure, not on insertion order, for {t}"
        );
    }
    let hashes: BTreeSet<u64> = ids_a.iter().map(|&id| a.structural_hash(id)).collect();
    assert_eq!(
        hashes.len(),
        corpus.len(),
        "the corpus must not collide: a hash that collides everywhere still dedups, just slowly"
    );
}

/// Free variables: the store's hash-consed sets against an independent textbook walk, over
/// every variant and a randomised corpus. This covers every binder's SCOPE — a `Let`'s name
/// binds in its body but NOT in its type or its value, and a `MutualFix` binds every name in
/// every body.
#[test]
fn free_variables_match_an_independent_textbook_walk() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x0FEE_0001_u64.wrapping_mul(7));
    let mut cases = variant_corpus();
    cases.push(let_in("b", var("b"), var("b"), var("b")));
    cases.push(Term::MutualFix {
        defs: vec![("p".to_string(), var("q")), ("q".to_string(), var("p"))],
        index: 0,
    });
    cases.push(fix("f", app(var("f"), var("g"))));
    for _ in 0..2000 {
        cases.push(random_term(&mut rng, 4));
    }
    for t in &cases {
        let id = store.intern(t);
        let mine: BTreeSet<String> = store.free_vars(id).into_iter().map(String::from).collect();
        assert_eq!(
            mine,
            tree_free(t),
            "free variables must match the textbook rule for {t}"
        );
        for name in ["x", "y", "z", "f", "b", "p", "q", "u", "nope"] {
            assert_eq!(
                store.occurs_free(id, name),
                tree_free(t).contains(name),
                "occurs_free({name}) must match the set for {t}"
            );
        }
    }
}

/// A `Let`'s name is bound in its BODY only — the scope rule most easily got wrong, stated as
/// its own positive/negative pair.
#[test]
fn a_let_binds_its_name_in_the_body_and_nowhere_else() {
    let mut store = TermStore::new();
    let in_body = store.intern(&let_in("b", nat(), g("Zero"), var("b")));
    assert!(!store.occurs_free(in_body, "b"), "the body's `b` is BOUND");
    let in_type = store.intern(&let_in("b", var("b"), g("Zero"), g("Zero")));
    assert!(store.occurs_free(in_type, "b"), "a `b` in the TYPE is free");
    let in_value = store.intern(&let_in("b", nat(), var("b"), g("Zero")));
    assert!(
        store.occurs_free(in_value, "b"),
        "a `b` in the VALUE is free"
    );
}

// ================================================================================================
// OCCURRENCE COUNTING — what an occurrence-gated ζ rule has to decide on
// ================================================================================================

/// The count against an independent walk, over every variant and a randomised corpus,
/// including shadowing in every binder.
#[test]
fn occurrence_counts_match_an_independent_walk_everywhere() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x0CC0_0001);
    let mut cases = variant_corpus();
    cases.push(lam("x", var("x"), var("x")));
    cases.push(pi("x", var("x"), app(var("x"), var("x"))));
    cases.push(let_in("x", var("x"), var("x"), app(var("x"), var("x"))));
    cases.push(fix("x", app(var("x"), var("x"))));
    cases.push(Term::MutualFix {
        defs: vec![("x".to_string(), app(var("x"), var("x")))],
        index: 0,
    });
    for _ in 0..2000 {
        cases.push(random_term(&mut rng, 4));
    }
    for t in &cases {
        let id = store.intern(t);
        for name in ["x", "y", "z", "f", "b", "nope"] {
            let sym = store.symbol(name);
            assert_eq!(
                u64::from(store.occurrence_count(id, sym)),
                tree_occurrences(t, name),
                "occurrence_count({name}) must match the independent walk for {t}"
            );
        }
    }
}

/// `occurs_free` and `occurrence_count` are two answers to one question. They must never
/// disagree, or a ζ gate and a capture check would read the same term differently.
#[test]
fn occurs_free_and_occurrence_count_never_disagree() {
    let mut store = TermStore::new();
    let mut rng = Rng(0xA6EE_0002);
    for _ in 0..3000 {
        let t = random_term(&mut rng, 4);
        let id = store.intern(&t);
        for name in POOL {
            let sym = store.symbol(name);
            assert_eq!(
                store.occurs_free(id, name),
                store.occurrence_count(id, sym) > 0,
                "occurs_free and occurrence_count must agree on {name} in {t}"
            );
        }
    }
}

/// The classification, at its two boundaries: zero, exactly one, and two.
#[test]
fn occurrences_classifies_zero_once_and_many_at_the_boundary() {
    let mut store = TermStore::new();
    let b = store.symbol("b");
    let none = store.intern(&g("Zero"));
    assert_eq!(store.occurrences(none, b), Occurrences::Zero);
    let one = store.intern(&app(g("Succ"), var("b")));
    assert_eq!(store.occurrences(one, b), Occurrences::Once);
    let two = store.intern(&app(var("b"), var("b")));
    assert_eq!(
        store.occurrences(two, b),
        Occurrences::Many,
        "two is already Many"
    );
    // And shadowing takes it back to Zero — the reason the gate cannot just look at names.
    let shadowed = store.intern(&lam("b", nat(), app(var("b"), var("b"))));
    assert_eq!(store.occurrences(shadowed, b), Occurrences::Zero);
}

// ================================================================================================
// SUBSTITUTION — the differential, and the capture it must refuse
// ================================================================================================

/// The differential. `TermStore::substitute` against a naive tree substitution written from
/// the definition in this file, over 6,000 randomly-generated cases, compared up to α because
/// the two freshen differently ON PURPOSE.
#[test]
fn substitution_agrees_with_an_independent_naive_oracle_up_to_alpha() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x5AB5_7001);
    for i in 0..6000 {
        let body = random_term(&mut rng, 4);
        let value = random_term(&mut rng, 3);
        let name = POOL[rng.below(POOL.len() as u64) as usize];
        let expected = naive_subst(&body, name, &value);
        let bid = store.intern(&body);
        let vid = store.intern(&value);
        let sym = store.symbol(name);
        let subbed = store.substitute(bid, sym, vid);
        let got = store.extract(subbed);
        assert!(
            alpha_eq(&got, &expected),
            "case {i} (seed 0x5AB57001): substituting {name} := {value} into {body}\n  \
             store gave {got}\n  oracle gave {expected}"
        );
    }
}

/// The capture. Substituting `x := y` into `λy. x` must NOT produce `λy. y`.
///
/// This is the single bug a naive substitution has, and it is a soundness bug: `λy. y` and
/// `λy'. y` have different types. The negative is asserted directly, not merely implied.
#[test]
fn substitution_refuses_to_capture_and_the_wrong_answer_is_named() {
    let mut store = TermStore::new();
    let body = store.intern(&lam("y", nat(), var("x")));
    let value = store.intern(&var("y"));
    let x = store.symbol("x");
    let subbed = store.substitute(body, x, value);
    let got = store.extract(subbed);
    let wrong = lam("y", nat(), var("y"));
    let right = lam("y_renamed", nat(), var("y"));
    assert!(
        !alpha_eq(&got, &wrong),
        "capturing `y` would be the soundness bug; got {got}"
    );
    assert!(
        alpha_eq(&got, &right),
        "the binder must be renamed; got {got}"
    );
    let got_id = store.intern(&got);
    assert!(
        store.occurs_free(got_id, "y"),
        "and the substituted `y` must still be FREE in the answer"
    );
}

/// The shadow. A binder that rebinds the substituted name stops the substitution dead — and
/// because nothing changed, the store must hand back the SAME id, not an equal copy.
#[test]
fn a_shadowing_binder_stops_substitution_and_returns_the_same_id() {
    let mut store = TermStore::new();
    let body = store.intern(&lam("x", nat(), var("x")));
    let value = store.intern(&g("Zero"));
    let x = store.symbol("x");
    assert_eq!(
        store.substitute(body, x, value),
        body,
        "λx. x has no free x, so substituting x is the identity — on the NOSE, not up to =="
    );
}

/// A name that does not occur is an O(1) identity — the property that makes a beta step near
/// the head of a 200,000-deep spine cost nothing.
#[test]
fn substituting_an_absent_name_returns_the_very_same_id() {
    let mut store = TermStore::new();
    let mut deep = store.mk_global("Zero");
    for _ in 0..5000 {
        let f = store.mk_global("Succ");
        deep = store.mk_app(f, deep);
    }
    let value = store.mk_global("One");
    let absent = store.symbol("nowhere");
    let before = store.len();
    assert_eq!(
        store.substitute(deep, absent, value),
        deep,
        "an absent name changes nothing"
    );
    assert_eq!(before, store.len(), "and allocates no node while doing it");
}

/// SIMULTANEOUS, not sequential. `instantiate` with `x ↦ y, y ↦ x` must SWAP, because a
/// sequential pass would substitute `x := y` and then rewrite that fresh `y` back to `x`.
#[test]
fn instantiation_is_simultaneous_and_not_a_sequence_of_substitutions() {
    let mut store = TermStore::new();
    let body = store.intern(&app(var("x"), var("y")));
    let (x, y) = (store.symbol("x"), store.symbol("y"));
    let vx = store.mk_var_sym(x);
    let vy = store.mk_var_sym(y);
    let swapped = store.instantiate(body, &[(x, vy), (y, vx)]);
    assert_eq!(
        store.extract(swapped),
        app(var("y"), var("x")),
        "a simultaneous map swaps; a sequential one would give `x x` or `y y`"
    );
}

/// The telescope — the case `substituting_one_argument_into_a_telescope_copies_it_per_binder`
/// is really about, measured both ways.
///
/// `infer_type`'s `App` rule substitutes ONE argument per binder, so a 128-binder telescope
/// whose binders mention `x` twice each turns one argument into 256 copies. The corpus case
/// asks for `out ≤ 4 × (telescope + argument)` measured with a TREE walk.
///
/// The finding this test records is that the bound is a REPRESENTATION bound, not an algorithm
/// bound. The same single-pass substitution, measured on the DAG, adds the argument's nodes
/// plus a constant — it MEETS the bound. Measured on the tree the DAG denotes, it exceeds it by
/// more than six times. So no amount of cleverness in `substitute` can satisfy T11-007 while
/// the output is a `Box`-recursive tree: either the output shares (a DAG, or ζ via `Term::Let`),
/// or the argument is copied 256 times by definition.
#[test]
fn a_telescope_instantiation_costs_the_argument_once_in_the_dag_and_256_times_in_the_tree() {
    let mut store = TermStore::new();
    const BINDERS: usize = 128;
    // Π(p₀ : Eq x x). … Π(p₁₂₇ : Eq x x). Nat — every binder's type mentions `x` twice.
    let eq_x = app(app(g("Eq"), var("x")), var("x"));
    let mut telescope = nat();
    for i in (0..BINDERS).rev() {
        telescope = pi(&format!("p{i}"), eq_x.clone(), telescope);
    }
    let arg = (0..40).fold(g("Zero"), |acc, _| app(g("Succ"), acc));
    let tid = store.intern(&telescope);
    let aid = store.intern(&arg);
    let x = store.symbol("x");
    let out = store.substitute(tid, x, aid);

    let (ty_tree, arg_tree) = (store.tree_size(tid), store.tree_size(aid));
    let bound = 4 * (ty_tree + arg_tree);

    // The DAG bound: the answer is the telescope's nodes plus the argument's, plus nothing.
    // 256 occurrences of `x` become 256 REFERENCES to one interned argument.
    assert!(
        store.dag_size(out) <= store.dag_size(tid) + store.dag_size(aid),
        "one pass must not copy the argument per binder: the answer is {} DAG nodes against \
         {} for the telescope plus {} for the argument",
        store.dag_size(out),
        store.dag_size(tid),
        store.dag_size(aid)
    );
    assert!(
        (store.dag_size(out) as u64) <= bound,
        "and it meets the corpus bound of {bound} with {} nodes",
        store.dag_size(out)
    );

    // THE OTHER HALF, and the reason the corpus case is a Gap rather than a bug: the TREE the
    // same answer denotes blows the same bound, because a tree cannot share.
    assert!(
        store.tree_size(out) > bound,
        "the tree spelling of the very same answer is {} nodes against the bound of {bound} — \
         which is why T11-007 needs a sharing REPRESENTATION, not a cleverer substitution",
        store.tree_size(out)
    );
    assert!(
        store.tree_size(out) >= 2 * BINDERS as u64 * arg_tree,
        "and it is at least the 256 copies the binders ask for: {} against {}",
        store.tree_size(out),
        2 * BINDERS as u64 * arg_tree
    );

    // Hash-consing already shares the REPEATED HEAD inside the argument itself, before any
    // substitution: forty `Succ`s are forty `App` nodes over ONE `Global("Succ")`.
    assert_eq!(arg_tree, 81, "forty Succs over Zero is 81 tree nodes");
    assert_eq!(
        store.dag_size(aid),
        42,
        "but only 42 distinct ones — the `Succ` head is shared"
    );

    // And the answer is still RIGHT: every `x` is gone, replaced by the argument.
    assert!(
        !store.occurs_free(out, "x"),
        "no `x` survives the instantiation"
    );
    let expected = naive_subst(&telescope, "x", &arg);
    assert!(
        alpha_eq(&store.extract(out), &expected),
        "and it agrees with the naive oracle"
    );
}

// ================================================================================================
// THE IDENTITY-KEYED CACHE — "checked once, not once per occurrence"
// ================================================================================================

/// The corpus case `a_shared_subterm_is_checked_once_and_not_once_per_occurrence` in one
/// sentence: a fold over `let b := big in quad b b b b` must touch `big`'s forty nodes ONCE,
/// not four times.
#[test]
fn a_shared_subterm_is_visited_once_and_not_once_per_occurrence() {
    let mut store = TermStore::new();
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let shared = let_in(
        "b",
        nat(),
        big.clone(),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    let big_id = store.intern(&big);
    let id = store.intern(&shared);

    let mut cache: IdCache<u64> = IdCache::new();
    let mut visits = 0u64;
    let counted = store.fold(id, &mut cache, &mut |s, _, node, kids| {
        visits += 1;
        let _ = (s, node);
        1 + kids.iter().sum::<u64>()
    });

    assert_eq!(
        counted,
        store.tree_size(id),
        "the fold reproduces the unfolded node count"
    );
    assert_eq!(
        visits,
        store.dag_size(id) as u64,
        "but it VISITS only the distinct nodes: {} against an unfolding of {}",
        visits,
        store.tree_size(id)
    );
    assert!(
        visits < store.tree_size(id),
        "and that is strictly fewer — the whole point of keying on identity"
    );
    assert!(
        cache.contains(big_id),
        "the shared value is in the cache exactly once"
    );
    assert_eq!(
        cache.len(),
        visits as usize,
        "one cache entry per distinct node, no more"
    );
}

/// The cache keys on IDENTITY. Two ids that denote different terms must not share an entry,
/// and a cache miss must be a miss and not a stale neighbour's answer.
#[test]
fn the_id_cache_keys_on_identity_and_never_aliases() {
    let mut store = TermStore::new();
    let a = store.mk_global("A");
    let b = store.mk_global("B");
    let mut cache: IdCache<&'static str> = IdCache::new();
    assert!(cache.is_empty());
    cache.insert(a, "for A");
    assert_eq!(cache.get(a), Some(&"for A"));
    assert_eq!(cache.get(b), None, "B must MISS, not read A's entry");
    assert_eq!(cache.len(), 1);
    cache.insert(a, "replaced");
    assert_eq!(cache.get(a), Some(&"replaced"));
    assert_eq!(cache.len(), 1, "replacing an entry does not grow the cache");
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.get(a), None);
}

// ================================================================================================
// THE DEPTH CEILING — what the store is FOR
// ================================================================================================

/// The corpus case `a_deep_spine_costs_linearly_in_its_depth_and_not_quadratically` does not
/// fail, it ABORTS: `infer_type` blows a 2 MiB stack on its own `App` arm long before the
/// timing assertion is reached.
///
/// Here is the same spine, 200,000 deep — two orders of magnitude past what the native
/// recursion survives — built, measured, β-normalised and compared, without one recursive
/// call. It is built with `mk_app` rather than from a `Term`, because a `Term` this deep
/// cannot even be DROPPED: the derived recursive `Drop` overflows first.
#[test]
fn a_spine_far_deeper_than_the_native_stack_interns_normalises_and_compares() {
    const DEPTH: usize = 200_000;
    let mut store = TermStore::new();
    let nat_id = store.mk_global("Nat");
    let x = store.mk_var("x");
    let id_nat = store.mk_lambda("x", nat_id, x);
    let zero = store.mk_global("Zero");
    let mut spine = zero;
    for _ in 0..DEPTH {
        spine = store.mk_app(id_nat, spine);
    }

    // DEPTH + 2, not DEPTH + 1: the shared `λx:Nat. x` is itself two rungs tall, so the
    // outermost `App` sits at DEPTH + 2. Measured, because guessing it got it wrong.
    assert_eq!(
        store.depth(spine),
        DEPTH as u32 + 2,
        "the annotation is O(1) at any depth"
    );
    assert_eq!(store.tree_size(spine), 4 * DEPTH as u64 + 1);
    assert_eq!(
        store.dag_size(spine),
        DEPTH + 4,
        "the {DEPTH} App nodes plus one shared λ, its domain, its body and Zero"
    );

    // LINEARITY, asserted in a currency that does not flap under load. The budget is a STEP
    // count, so a reduction that succeeded inside `DEPTH + 8` steps took O(depth) of them — a
    // quadratic reducer would need DEPTH² and be refused. This is the corpus case's
    // `ratio ≤ 5.0`, decided by counting instead of by a wall clock on a contended box.
    let before = store.len();
    match store.beta_zeta_normalize(spine, DEPTH as u64 + 8) {
        StoreNormalForm::Reached(nf) => {
            assert_eq!(
                nf, zero,
                "the spine reduces to Zero — and equality is one compare"
            );
            assert_eq!(store.extract(nf), g("Zero"));
        }
        other => panic!("a {DEPTH}-deep spine must reduce in O(depth) steps, got {other:?}"),
    }
    // And it ALLOCATES NOTHING. Every β step substitutes `x := arg` into the body `x`, which
    // the substituter answers with the argument's existing id in O(1) — so 200,000 reductions
    // add no node to the store. That is the whole difference from a tree, where each step
    // rebuilds the spine below it.
    assert_eq!(
        store.len(),
        before,
        "{DEPTH} β steps must allocate no new node; they added {}",
        store.len() - before
    );
}

/// And the round trip survives the same depth, checked BY IDENTITY rather than by `==` —
/// because `Term`'s derived `PartialEq` is itself native recursion and would overflow trying
/// to answer.
#[test]
fn a_deep_term_round_trips_and_the_check_itself_is_one_compare() {
    const DEPTH: usize = 12_000;
    let mut store = TermStore::new();
    let mut spine = store.mk_global("Zero");
    for _ in 0..DEPTH {
        let f = store.mk_global("Succ");
        spine = store.mk_app(f, spine);
    }
    let materialised = store.extract(spine);
    assert_eq!(
        store.intern(&materialised),
        spine,
        "extract then intern is the identity, and the assertion is an integer compare"
    );
    // Dropping `materialised` is recursive, so keep the depth under what the derived `Drop`
    // survives; the store itself has no such ceiling, which is the point of the test above.
    drop(materialised);
}

// ================================================================================================
// β/ζ — the occurrence-gated ζ rule, and the kernel differential
// ================================================================================================

/// The bound the corpus case `a_shared_subterm_is_checked_once_and_not_once_per_occurrence`
/// asserts: `nodes(normalize(let b := big in quad b b b b)) ≤ nodes(big) + 16`.
///
/// Today's unconditional ζ gives 4 × 81 + 5 = 329 against a ceiling of 97. Occurrence-gated ζ
/// KEEPS the `let` as the sharing node and gives 92. Measured here, on the store, so the fix
/// is evidence rather than a proposal.
#[test]
fn occurrence_gated_zeta_meets_the_corpus_node_bound() {
    let mut store = TermStore::new();
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let shared = let_in(
        "b",
        nat(),
        big.clone(),
        app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
    );
    let big_id = store.intern(&big);
    let big_nodes = store.tree_size(big_id);
    assert_eq!(big_nodes, 81, "forty Succs over a free variable");
    let id = store.intern(&shared);

    let nf = match store.beta_zeta_normalize(id, 10_000) {
        StoreNormalForm::Reached(nf) => nf,
        other => panic!("the shared let normalises, got {other:?}"),
    };
    assert!(
        store.tree_size(nf) <= big_nodes + 16,
        "the normal form is {} nodes against a ceiling of {}: unconditional ζ gives 329",
        store.tree_size(nf),
        big_nodes + 16
    );
    assert!(
        matches!(store.node(nf), Node::Let { .. }),
        "and the sharing node SURVIVES — that is what makes it small"
    );
    // The value is still reachable and still definitionally the value: ζ on demand.
    let expanded = store
        .zeta_expand_head(nf)
        .expect("a surviving Let can always ζ-expand");
    assert!(
        store.tree_size(expanded) > big_nodes * 4,
        "expanding on demand gives back the big term, so nothing was lost"
    );
}

/// The negative for the gate. A `let` used ONCE must still ζ-expand — a gate that preserved
/// every `let` would satisfy the node bound above and change the language.
#[test]
fn a_single_use_let_still_zeta_expands() {
    let mut store = TermStore::new();
    let once = let_in("b", nat(), app(g("Succ"), g("Zero")), app(g("f"), var("b")));
    let id = store.intern(&once);
    let nf = match store.beta_zeta_normalize(id, 1000) {
        StoreNormalForm::Reached(nf) => nf,
        other => panic!("a single-use let normalises, got {other:?}"),
    };
    assert_eq!(
        store.extract(nf),
        app(g("f"), app(g("Succ"), g("Zero"))),
        "one occurrence means expanding copies nothing, so it expands"
    );
    // Zero uses expands too — and the value disappears entirely.
    let unused = store.intern(&let_in("b", nat(), g("Zero"), g("Done")));
    let nf = match store.beta_zeta_normalize(unused, 1000) {
        StoreNormalForm::Reached(nf) => nf,
        other => panic!("an unused let normalises, got {other:?}"),
    };
    assert_eq!(store.extract(nf), g("Done"), "an unused let vanishes");
}

/// The gate is re-evaluated AFTER the body is normalised, because reduction ERASES
/// occurrences. A body with three uses of `b` before reduction and one after must expand —
/// this is the case a gate applied only to the unreduced body gets wrong.
#[test]
fn the_zeta_gate_is_re_evaluated_after_the_body_is_normalised() {
    let mut store = TermStore::new();
    // (λk. λ_ . λ_ . k) b b b  —  three occurrences of `b`, two of which reduce away.
    let selector = lam("k", nat(), lam("i", nat(), lam("j", nat(), var("k"))));
    let body = app(app(app(selector, var("b")), var("b")), var("b"));
    let b = store.symbol("b");
    let body_id = store.intern(&body);
    assert_eq!(
        store.occurrences(body_id, b),
        Occurrences::Many,
        "before reduction `b` occurs three times, so a naive gate would PRESERVE the let"
    );
    let whole = store.intern(&let_in("b", nat(), app(g("Succ"), g("Zero")), body));
    let nf = match store.beta_zeta_normalize(whole, 1000) {
        StoreNormalForm::Reached(nf) => nf,
        other => panic!("it normalises, got {other:?}"),
    };
    assert_eq!(
        store.extract(nf),
        app(g("Succ"), g("Zero")),
        "after reduction only one occurrence remains, so the let expands after all"
    );
}

/// The budget is honest. A term with no normal form must report `OutOfFuel`, never a
/// half-reduced term dressed up as normal — the defect
/// `normalize_must_not_hand_back_an_unfinished_reduction_as_a_normal_form` names.
#[test]
fn the_normaliser_is_honest_about_a_budget_it_did_not_finish() {
    let mut store = TermStore::new();
    // Ω = (λx. x x) (λx. x x) — β-reduces to itself forever.
    let omega_half = lam("x", nat(), app(var("x"), var("x")));
    let omega = app(omega_half.clone(), omega_half);
    let id = store.intern(&omega);
    assert_eq!(
        store.beta_zeta_normalize(id, 50),
        StoreNormalForm::OutOfFuel { budget: 50 },
        "Ω has no normal form, so the answer must be a refusal and not a term"
    );
    // A budget of zero on an already-normal term still succeeds: no step is needed.
    let normal = store.intern(&g("Zero"));
    assert_eq!(
        store.beta_zeta_normalize(normal, 0),
        StoreNormalForm::Reached(normal),
        "a normal form costs no fuel"
    );
}

/// The kernel differential. On the β-only fragment, `beta_zeta_normalize` and the kernel's
/// own `reduction::normalize` must agree up to α — two independent reducers over two
/// independent representations. This is the assertion that makes the store's normaliser
/// trustworthy as a MEASUREMENT of the kernel's semantics rather than a private opinion.
#[test]
fn beta_normalisation_agrees_with_the_kernels_own_reducer() {
    let ctx = Context::new();
    let mut rng = Rng(0xBE7A_0003);
    let mut checked = 0usize;
    for i in 0..1500 {
        let mut store = TermStore::new();
        let t = random_beta_term(&mut rng, 4);
        let id = store.intern(&t);
        let mine = match store.beta_zeta_normalize(id, 10_000) {
            StoreNormalForm::Reached(nf) => store.extract(nf),
            // A term with no β normal form is out of scope for the comparison — the kernel
            // answers it with a fuel truncation, which is not a normal form either.
            StoreNormalForm::OutOfFuel { .. } => continue,
        };
        let theirs = normalize(&ctx, &t);
        assert!(
            alpha_eq(&mine, &theirs),
            "case {i} (seed 0xBE7A0003): {t}\n  store gave {mine}\n  kernel gave {theirs}"
        );
        checked += 1;
    }
    assert!(
        checked > 1000,
        "the corpus must actually exercise the comparison, got {checked}"
    );
}

/// And ζ on demand agrees with the kernel too: expanding a surviving `Let` and then
/// normalising gives what the kernel's unconditional ζ gives directly. This is the property a
/// conversion check needs before a `Let` may survive into a normal form — without it, two
/// convertible terms would compare unequal.
#[test]
fn zeta_on_demand_reaches_the_same_answer_as_the_kernels_unconditional_zeta() {
    let ctx = Context::new();
    let mut rng = Rng(0x2E7A_0004);
    let mut checked = 0usize;
    for i in 0..600 {
        let mut store = TermStore::new();
        let value = random_beta_term(&mut rng, 3);
        let body = random_beta_term(&mut rng, 3);
        let t = let_in("b", g("B"), value, body);
        let id = store.intern(&t);
        let gated = match store.beta_zeta_normalize(id, 10_000) {
            StoreNormalForm::Reached(nf) => nf,
            StoreNormalForm::OutOfFuel { .. } => continue,
        };
        // Whether or not the gate preserved the `Let`, ζ-expanding the head and normalising
        // must land on the kernel's answer.
        let forced = match store.zeta_expand_head(gated) {
            Some(expanded) => match store.beta_zeta_normalize(expanded, 10_000) {
                StoreNormalForm::Reached(nf) => store.extract(nf),
                StoreNormalForm::OutOfFuel { .. } => continue,
            },
            None => store.extract(gated),
        };
        let theirs = normalize(&ctx, &t);
        assert!(
            alpha_eq(&forced, &theirs),
            "case {i} (seed 0x2E7A0004): {t}\n  ζ-on-demand gave {forced}\n  kernel gave {theirs}"
        );
        checked += 1;
    }
    assert!(
        checked > 400,
        "the corpus must actually exercise the comparison, got {checked}"
    );
}

// ================================================================================================
// MEASUREMENT — run on purpose
// ================================================================================================

/// The quadratic-to-linear claim, priced. Prints the store's cost against the depth so the
/// growth can be read, rather than asserted against a wall clock on a loaded box.
#[test]
#[ignore = "measurement: prints a cost table; run on purpose"]
fn measure_deep_spine_cost_growth_in_the_store() {
    println!("\n  depth      build+normalise     growth      nodes");
    println!("  ----------------------------------------------------------");
    let mut prev: Option<(usize, f64)> = None;
    for depth in [12_500usize, 25_000, 50_000, 100_000, 200_000, 400_000] {
        let start = std::time::Instant::now();
        let mut store = TermStore::new();
        let nat_id = store.mk_global("Nat");
        let x = store.mk_var("x");
        let id_nat = store.mk_lambda("x", nat_id, x);
        let zero = store.mk_global("Zero");
        let mut spine = zero;
        for _ in 0..depth {
            spine = store.mk_app(id_nat, spine);
        }
        let nf = store.beta_zeta_normalize(spine, 4 * depth as u64);
        let elapsed = start.elapsed().as_secs_f64();
        assert_eq!(nf, StoreNormalForm::Reached(zero));
        let growth = match prev {
            Some((pd, pe)) if pe > 0.0 => {
                format!("{:.2}x for {:.0}x", elapsed / pe, depth as f64 / pd as f64)
            }
            _ => String::new(),
        };
        println!(
            "  {depth:<9}  {elapsed:>12.6}s     {growth:<12}  {}",
            store.len()
        );
        prev = Some((depth, elapsed));
    }
    println!();
}

/// The sharing claim, priced: how much smaller the DAG is than the tree it denotes, on the
/// shapes the corpus complains about.
#[test]
#[ignore = "measurement: prints a sharing table; run on purpose"]
fn measure_sharing_on_the_corpus_shapes() {
    println!("\n  shape                          tree      DAG     ratio");
    println!("  ----------------------------------------------------------");
    let mut store = TermStore::new();
    let big = (0..40).fold(var("u"), |acc, _| app(g("Succ"), acc));
    let cases: Vec<(&str, Term)> = vec![
        (
            "let b := big; quad b b b b",
            let_in(
                "b",
                nat(),
                big.clone(),
                app4(g("quad"), var("b"), var("b"), var("b"), var("b")),
            ),
        ),
        (
            "quad big big big big",
            app4(g("quad"), big.clone(), big.clone(), big.clone(), big),
        ),
        (
            "128-binder dependent telescope",
            (0..128).rev().fold(nat(), |acc, i| {
                pi(&format!("p{i}"), app(app(g("Eq"), var("x")), var("x")), acc)
            }),
        ),
    ];
    for (label, t) in cases {
        let id = store.intern(&t);
        let tree = store.tree_size(id);
        let dag = store.dag_size(id) as u64;
        println!(
            "  {label:<30} {tree:>7}  {dag:>7}    {:.2}x",
            tree as f64 / dag as f64
        );
    }
    println!();
}

// ================================================================================================
// α-CANONICAL IDENTITY — the equality the kernel's conversion checker actually wants
// ================================================================================================

/// An INDEPENDENT α-variant generator: rename every BOUND name to a fresh serial name, leaving
/// every free name alone. Written from the definition of α-conversion, not from the store's
/// canonicaliser, so agreement between the two means something.
fn alpha_variant(t: &Term) -> Term {
    fn go(t: &Term, scope: &mut Vec<(String, String)>, next: &mut u32) -> Term {
        // The original name is irrelevant: an α-variant's job is to be DIFFERENT, so every
        // binder takes the next serial name whatever it was called.
        let bind = |_was: &str, next: &mut u32| {
            *next += 1;
            format!("v{next}")
        };
        match t {
            Term::Var(n) => match scope.iter().rev().find(|(from, _)| from == n) {
                Some((_, to)) => var(to),
                None => t.clone(),
            },
            Term::Sort(_) | Term::Global(_) | Term::Const { .. } | Term::Lit(_) | Term::Hole => {
                t.clone()
            }
            Term::App(f, a) => app(go(f, scope, next), go(a, scope, next)),
            Term::Match {
                discriminant,
                motive,
                cases,
            } => Term::Match {
                discriminant: Box::new(go(discriminant, scope, next)),
                motive: Box::new(go(motive, scope, next)),
                cases: cases.iter().map(|c| go(c, scope, next)).collect(),
            },
            Term::Pi {
                param,
                param_type,
                body_type,
            } => {
                let ty = go(param_type, scope, next);
                let fresh = bind(param, next);
                scope.push((param.clone(), fresh.clone()));
                let body = go(body_type, scope, next);
                scope.pop();
                pi(&fresh, ty, body)
            }
            Term::Lambda {
                param,
                param_type,
                body,
            } => {
                let ty = go(param_type, scope, next);
                let fresh = bind(param, next);
                scope.push((param.clone(), fresh.clone()));
                let b = go(body, scope, next);
                scope.pop();
                lam(&fresh, ty, b)
            }
            Term::Fix { name, body } => {
                let fresh = bind(name, next);
                scope.push((name.clone(), fresh.clone()));
                let b = go(body, scope, next);
                scope.pop();
                fix(&fresh, b)
            }
            Term::Let {
                name,
                ty,
                value,
                body,
            } => {
                let ty2 = go(ty, scope, next);
                let v2 = go(value, scope, next);
                let fresh = bind(name, next);
                scope.push((name.clone(), fresh.clone()));
                let b = go(body, scope, next);
                scope.pop();
                let_in(&fresh, ty2, v2, b)
            }
            Term::MutualFix { defs, index } => {
                let fresh: Vec<String> = defs.iter().map(|(n, _)| bind(n, next)).collect();
                for ((n, _), f) in defs.iter().zip(fresh.iter()) {
                    scope.push((n.clone(), f.clone()));
                }
                let bodies: Vec<Term> = defs.iter().map(|(_, b)| go(b, scope, next)).collect();
                for _ in defs {
                    scope.pop();
                }
                Term::MutualFix {
                    defs: fresh.into_iter().zip(bodies).collect(),
                    index: *index,
                }
            }
            Term::Annotated { annotation, term } => {
                Term::annotated(annotation.clone(), go(term, scope, next))
            }
        }
    }
    go(t, &mut Vec::new(), &mut 0)
}

/// `same` is STRUCTURAL equality, and the kernel's conversion checker does not want structural
/// equality — `type_checker::is_subtype` opens with an `alpha_eq` fast path that walks both
/// trees on every single conversion. `alpha_id` turns that walk into one integer compare.
///
/// Locked against the de Bruijn encoder in this file, in BOTH directions: every α-variant must
/// come out equal, and every pair the encoder separates must come out unequal. The negative half
/// is 90,000 pairs, because a canonicaliser that returned one id for everything would pass the
/// positive half perfectly.
#[test]
fn alpha_identity_agrees_with_an_independent_de_bruijn_encoder() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x0A1F_0005);
    let corpus: Vec<Term> = (0..300).map(|_| random_term(&mut rng, 4)).collect();

    // THE POSITIVE: a systematically α-renamed term must share its canonical id.
    for (i, t) in corpus.iter().enumerate() {
        let variant = alpha_variant(t);
        assert_eq!(
            alpha_key(t),
            alpha_key(&variant),
            "the α-variant generator itself must produce an α-variant, case {i}: {t}"
        );
        let a = store.intern(t);
        let b = store.intern(&variant);
        assert!(
            store.alpha_same(a, b),
            "case {i}: {t}\n  and its α-variant {variant}\n  must share a canonical id"
        );
    }

    // THE NEGATIVE: every pair the independent encoder separates must be separated here too.
    let ids: Vec<_> = corpus.iter().map(|t| store.intern(t)).collect();
    let keys: Vec<String> = corpus.iter().map(alpha_key).collect();
    for i in 0..corpus.len() {
        for j in 0..corpus.len() {
            assert_eq!(
                store.alpha_same(ids[i], ids[j]),
                keys[i] == keys[j],
                "α-identity must agree with the de Bruijn encoder on ({}, {})",
                corpus[i],
                corpus[j]
            );
        }
    }
}

/// α renames BOUND names only. The hand-written pairs, one per binder, each with the
/// neighbouring term it must still keep apart.
#[test]
fn alpha_renames_every_binder_and_never_a_free_variable() {
    let mut store = TermStore::new();
    let same = |store: &mut TermStore, a: Term, b: Term| {
        let (x, y) = (store.intern(&a), store.intern(&b));
        store.alpha_same(x, y)
    };

    assert!(
        same(
            &mut store,
            lam("x", nat(), var("x")),
            lam("y", nat(), var("y"))
        ),
        "λ"
    );
    assert!(
        same(
            &mut store,
            pi("x", nat(), var("x")),
            pi("y", nat(), var("y"))
        ),
        "Π"
    );
    assert!(
        same(
            &mut store,
            let_in("b", nat(), g("Zero"), var("b")),
            let_in("c", nat(), g("Zero"), var("c"))
        ),
        "let"
    );
    assert!(
        same(&mut store, fix("f", var("f")), fix("h", var("h"))),
        "fix"
    );
    assert!(
        same(
            &mut store,
            Term::MutualFix {
                defs: vec![("p".into(), var("q")), ("q".into(), var("p"))],
                index: 1,
            },
            Term::MutualFix {
                defs: vec![("a".into(), var("b")), ("b".into(), var("a"))],
                index: 1,
            },
        ),
        "a mutual block renames as a BLOCK — both names at once, or the cross-references break"
    );

    // THE NEGATIVES. Each is one character away from the positive above.
    assert!(
        !same(
            &mut store,
            lam("x", nat(), var("y")),
            lam("x", nat(), var("z"))
        ),
        "a FREE variable is not renamed: λx. y and λx. z are different terms"
    );
    assert!(
        !same(
            &mut store,
            lam("x", nat(), lam("y", nat(), var("x"))),
            lam("x", nat(), lam("y", nat(), var("y")))
        ),
        "which binder an occurrence points at is the whole content of α"
    );
    assert!(
        !same(
            &mut store,
            lam("x", nat(), var("x")),
            lam("x", g("Bool"), var("x"))
        ),
        "α does not touch a binder's TYPE"
    );
    assert!(
        !same(
            &mut store,
            Term::MutualFix {
                defs: vec![("p".into(), var("q")), ("q".into(), var("p"))],
                index: 0,
            },
            Term::MutualFix {
                defs: vec![("p".into(), var("q")), ("q".into(), var("p"))],
                index: 1,
            },
        ),
        "and it does not touch which member of a mutual block is meant"
    );
    assert!(
        !same(
            &mut store,
            let_in("b", nat(), g("Zero"), var("b")),
            lam("b", nat(), var("b"))
        ),
        "a let is not a λ, however alike they look under α"
    );
}

/// The reason the canonical form uses a NODE and not a reserved NAME.
///
/// A user may write a variable called `⟪⟫` — the exact spelling the canonicaliser gives every
/// binder. If bound occurrences were a reserved NAME too, a free `⟪⟫` and a bound one would
/// canonicalise alike and two terms that are NOT α-equivalent would compare equal. They do not,
/// because a canonical occurrence is a node shape no `Term` can spell.
#[test]
fn a_user_variable_spelled_like_the_reserved_binder_is_not_conflated() {
    let mut store = TermStore::new();
    let reserved = "⟪⟫";
    let bound_here = lam(reserved, nat(), var(reserved));
    let free_inside = lam("q", nat(), var(reserved));
    let plain = lam("x", nat(), var("x"));

    let (a, b, c) = (
        store.intern(&bound_here),
        store.intern(&free_inside),
        store.intern(&plain),
    );
    assert!(store.alpha_same(a, c), "λ⟪⟫. ⟪⟫ IS α-equivalent to λx. x");
    assert!(!store.alpha_same(b, c), "λq. ⟪⟫ is NOT — its ⟪⟫ is free");
    assert!(
        !store.alpha_same(a, b),
        "and the two must not be conflated with each other"
    );
    assert_eq!(
        alpha_key(&bound_here) == alpha_key(&plain),
        true,
        "the oracle agrees"
    );
    assert_ne!(
        alpha_key(&free_inside),
        alpha_key(&plain),
        "…in both directions"
    );
}

/// Canonicalising is idempotent, and a term that binds nothing is already canonical — so the
/// fast path that returns such a term untouched is returning the right answer, not skipping work
/// it should have done.
#[test]
fn canonicalisation_is_idempotent_and_a_binder_free_term_is_already_canonical() {
    let mut store = TermStore::new();
    let mut rng = Rng(0x0A1F_0006);
    for _ in 0..500 {
        let t = random_term(&mut rng, 4);
        let id = store.intern(&t);
        let once = store.alpha_id(id);
        let twice = store.alpha_id(once);
        assert_eq!(once, twice, "α-canonicalisation must be idempotent on {t}");
        if bound_names(&t).is_empty() {
            assert_eq!(
                once, id,
                "a term that binds nothing is its own canonical form: {t}"
            );
            assert!(store.is_alpha_canonical(id));
        }
    }
    // And it is an equivalence relation, spot-checked where transitivity could fail.
    let a = store.intern(&lam("x", nat(), lam("y", nat(), var("x"))));
    let b = store.intern(&lam("p", nat(), lam("q", nat(), var("p"))));
    let c = store.intern(&lam("m", nat(), lam("n", nat(), var("m"))));
    assert!(store.alpha_same(a, a), "reflexive");
    assert!(
        store.alpha_same(a, b) && store.alpha_same(b, a),
        "symmetric"
    );
    assert!(
        store.alpha_same(a, b) && store.alpha_same(b, c) && store.alpha_same(a, c),
        "transitive"
    );
}

/// Every one of `Term`'s thirteen shapes is buildable DIRECTLY in the store, without going
/// through `intern` — otherwise a kernel rewritten against the store would have to materialise
/// a `Term` just to build a node, which is the cost the store exists to remove.
///
/// The assertion is that each builder round-trips to the `Term` it names, so a builder that
/// quietly produced the wrong shape could not pass.
#[test]
fn every_term_shape_is_buildable_directly_in_the_store() {
    let mut store = TermStore::new();
    let sort = store.mk_sort(&Universe::Type(3));
    assert_eq!(store.extract(sort), Term::Sort(Universe::Type(3)));
    let v = store.mk_var("x");
    assert_eq!(store.extract(v), var("x"));
    let gl = store.mk_global("Nat");
    assert_eq!(store.extract(gl), nat());
    let c = store.mk_const("cast", &[Universe::Prop, Universe::Var("u".into())]);
    assert_eq!(
        store.extract(c),
        Term::Const {
            name: "cast".into(),
            levels: vec![Universe::Prop, Universe::Var("u".into())],
        }
    );
    let p = store.mk_pi("x", gl, v);
    assert_eq!(store.extract(p), pi("x", nat(), var("x")));
    let l = store.mk_lambda("x", gl, v);
    assert_eq!(store.extract(l), lam("x", nat(), var("x")));
    let a = store.mk_app(l, gl);
    assert_eq!(store.extract(a), app(lam("x", nat(), var("x")), nat()));
    let m = store.mk_match(v, l, &[gl, v]);
    assert_eq!(
        store.extract(m),
        Term::Match {
            discriminant: Box::new(var("x")),
            motive: Box::new(lam("x", nat(), var("x"))),
            cases: vec![nat(), var("x")],
        }
    );
    let fx = store.mk_fix("f", v);
    assert_eq!(store.extract(fx), fix("f", var("x")));
    let mf = store.mk_mutual_fix(&[("p", v), ("q", gl)], 1);
    assert_eq!(
        store.extract(mf),
        Term::MutualFix {
            defs: vec![("p".into(), var("x")), ("q".into(), nat())],
            index: 1
        }
    );
    let lt = store.mk_let("b", gl, gl, v);
    assert_eq!(store.extract(lt), let_in("b", nat(), nat(), var("x")));
    let li = store.mk_lit(&Literal::Nat(BigInt::from_i64(9)));
    assert_eq!(
        store.extract(li),
        Term::Lit(Literal::Nat(BigInt::from_i64(9)))
    );
    let h = store.mk_hole();
    assert_eq!(store.extract(h), Term::Hole);

    // And a builder must DEDUP exactly as `intern` does — building and interning the same
    // shape are one node, not two, or the store would be only half hash-consed.
    assert_eq!(store.intern(&pi("x", nat(), var("x"))), p);
    assert_eq!(store.intern(&lam("x", nat(), var("x"))), l);
    assert_eq!(store.intern(&Term::Hole), h);
}
