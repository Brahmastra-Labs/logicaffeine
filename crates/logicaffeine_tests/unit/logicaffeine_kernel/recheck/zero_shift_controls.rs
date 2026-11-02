//! Zero shifting is syntax identity, independently of sharing admission epochs.
use super::*;

fn owned(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn app(a: Db, b: Db) -> Db {
    Db::App(owned(a), owned(b))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(owned(a), owned(b))
}
fn global(name: &str) -> Db {
    Db::Global(name.into())
}
fn live() -> usize {
    FIX_SHARED_LIVE.with(|s| s.get())
}
fn spent() -> usize {
    FIX_FACTORY.with(|s| s.get().spent)
}

struct Reset {
    depth: usize,
    fuel: usize,
    exhausted: bool,
    work: Option<(usize, usize)>,
}
impl Drop for Reset {
    fn drop(&mut self) {
        WHNF_DEPTH.with(|s| s.set(self.depth));
        WHNF_FUEL.with(|s| s.set(self.fuel));
        WHNF_EXHAUSTED.with(|s| s.set(self.exhausted));
        BETA_WORK.with(|s| s.set(self.work));
    }
}
fn reset() -> Reset {
    Reset {
        depth: WHNF_DEPTH.with(|s| s.replace(0)),
        fuel: WHNF_FUEL.with(|s| s.replace(MAX_RECHECK_STEPS)),
        exhausted: WHNF_EXHAUSTED.with(|s| s.replace(false)),
        work: BETA_WORK.with(|s| s.replace(None)),
    }
}

// Structural comparison without treating NaN reflexivity or signed zero as a
// requirement of Literal::PartialEq. Float payload bits must remain unchanged.
fn same_syntax(a: &Db, b: &Db) -> bool {
    match (a, b) {
        (Db::Lit(Literal::Float(a)), Db::Lit(Literal::Float(b))) => a.to_bits() == b.to_bits(),
        (Db::Pi(a, b), Db::Pi(c, d))
        | (Db::Lam(a, b), Db::Lam(c, d))
        | (Db::App(a, b), Db::App(c, d)) => same_syntax(a, c) && same_syntax(b, d),
        (Db::Let(a, b, c), Db::Let(d, e, f)) => {
            same_syntax(a, d) && same_syntax(b, e) && same_syntax(c, f)
        }
        (
            Db::Match {
                disc: a,
                motive: b,
                cases: c,
            },
            Db::Match {
                disc: d,
                motive: e,
                cases: f,
            },
        ) => {
            same_syntax(a, d)
                && same_syntax(b, e)
                && c.len() == f.len()
                && c.iter().zip(f).all(|(a, b)| same_syntax(a, b))
        }
        (Db::Fix(a), Db::Fix(b)) => same_syntax(a, b),
        (Db::MutualFix { defs: a, index: i }, Db::MutualFix { defs: b, index: j }) => {
            i == j && a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_syntax(a, b))
        }
        _ => a == b,
    }
}

// Physical Db nodes constructed by cloning this representation. Shared edges
// retain their allocation; their logical descendants are not newly constructed.
fn clone_nodes(t: &Db) -> usize {
    fn edge(t: &DbBox) -> usize {
        match t {
            DbBox::Owned(t) => clone_nodes(t),
            DbBox::Shared(_) => 0,
        }
    }
    1 + match t {
        Db::Pi(a, b) | Db::Lam(a, b) | Db::App(a, b) => edge(a) + edge(b),
        Db::Let(a, b, c) => edge(a) + edge(b) + edge(c),
        Db::Match {
            disc,
            motive,
            cases,
        } => edge(disc) + edge(motive) + cases.iter().map(clone_nodes).sum::<usize>(),
        Db::Fix(body) => match body {
            FixBody::Owned(t) => clone_nodes(t),
            FixBody::Shared(_) => 0,
        },
        Db::MutualFix { defs, .. } => defs.iter().map(clone_nodes).sum(),
        _ => 0,
    }
}
fn forms(k: usize) -> Vec<Db> {
    vec![
        Db::Var(k),
        Db::Sort(Universe::Succ(Box::new(Universe::Var("u".into())))),
        Db::Const {
            name: "poly".into(),
            levels: vec![
                Universe::Var("u".into()),
                Universe::IMax(Box::new(Universe::Prop), Box::new(Universe::Type(1))),
            ],
        },
        Db::Pi(owned(Db::Var(k)), owned(app(Db::Var(0), Db::Var(k)))),
        lam(Db::Var(k), app(Db::Var(0), Db::Var(k))),
        app(Db::Var(k), Db::Var(k)),
        Db::Let(
            owned(Db::Var(k)),
            owned(Db::Var(k)),
            owned(lam(Db::Var(0), Db::Var(k))),
        ),
        Db::Match {
            disc: owned(Db::Var(k)),
            motive: owned(lam(Db::Var(k), Db::Var(0))),
            cases: vec![Db::Var(k), lam(Db::Var(0), Db::Var(k))],
        },
        Db::Fix(FixBody::owned(Box::new(app(Db::Var(0), Db::Var(k))))),
        Db::MutualFix {
            defs: vec![],
            index: 0,
        },
        Db::MutualFix {
            defs: vec![app(Db::Var(0), Db::Var(k))],
            index: 0,
        },
        Db::MutualFix {
            defs: vec![lam(Db::Var(k), Db::Var(2)), Db::Var(k)],
            index: 1,
        },
    ]
}

#[test]
fn zero_shift_preserves_every_binding_form_and_capture_position() {
    let _reset = reset();
    for k in 0..=5 {
        for original in forms(k) {
            let alias = original.clone();
            for cutoff in [0, 1, 5] {
                let shifted = shift(&original, 0, cutoff);
                assert!(
                    same_syntax(&shifted, &original),
                    "index {k}, cutoff {cutoff}"
                );
                assert!(same_syntax(&original, &alias));
                assert!(same_syntax(
                    &shift_owned(original.clone(), 0, cutoff),
                    &original
                ));
            }
        }
    }
}

#[test]
fn zero_shift_preserves_nan_payload_and_signed_zero_bits() {
    let _reset = reset();
    for bits in [
        0x7ff8_0000_0000_0042u64,
        0xfff8_0000_0000_0042,
        0x7ff0_0000_0000_0001,
        0,
        0x8000_0000_0000_0000,
    ] {
        let literal = Db::Lit(Literal::Float(f64::from_bits(bits)));
        let original = Db::Let(
            owned(Db::Sort(Universe::Type(0))),
            owned(literal.clone()),
            owned(Db::Match {
                disc: owned(Db::Var(0)),
                motive: owned(global("motive")),
                cases: vec![lam(global("A"), literal)],
            }),
        );
        for cutoff in [0, 2] {
            assert!(same_syntax(&shift(&original, 0, cutoff), &original));
            assert!(same_syntax(
                &shift_owned(original.clone(), 0, cutoff),
                &original
            ));
        }
    }
}

#[test]
fn zero_shift_of_owned_syntax_does_not_attempt_optional_admission() {
    let _reset = reset();
    let c = Context::new();
    let _scope = PrefixScope::enter(&c);
    assert!(fix_factory_begin());
    let original = lam(global("A"), app(Db::Var(1), Db::Var(0)));
    let expected_nodes = clone_nodes(&original);
    let before_spent = spent();
    let before_live = live();
    BETA_WORK.with(|s| s.set(Some((0, 0))));
    let shifted = shift(&original, 0, 0);
    let (_, constructed) = BETA_WORK.with(|s| s.replace(None).unwrap());
    assert!(same_syntax(&shifted, &original));
    assert_eq!(
        constructed, expected_nodes,
        "a deep Owned clone still constructs every Owned Db node"
    );
    assert_eq!(
        spent(),
        before_spent,
        "identity shifting needs no sharing admission"
    );
    assert_eq!(
        live(),
        before_live,
        "identity shifting must not create shared allocations"
    );
}

#[test]
fn zero_shift_retains_shared_edge_before_and_after_its_generation_expires() {
    let _reset = reset();
    let c = Context::new();
    let baseline = live();
    let edge = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        let edge = DbBox::new(lam(global("A"), app(Db::Var(1), Db::Var(0))));
        assert!(edge.is_shared());
        let before_spent = spent();
        let active = edge.shifted(0, 0);
        assert!(std::ptr::eq(edge.as_ref(), active.as_ref()));
        assert_eq!(spent(), before_spent);
        edge
    };
    assert!(edge.facts().is_none());
    let bytes = edge.retained_bytes();
    let before_spent = spent();
    assert!(bytes > 0);
    assert_eq!(live(), baseline + bytes);
    for cutoff in [0, 1, 5] {
        BETA_WORK.with(|s| s.set(Some((0, 0))));
        let shifted = edge.shifted(0, cutoff);
        let (_, constructed) = BETA_WORK.with(|s| s.replace(None).unwrap());
        assert!(same_syntax(&shifted, &edge));
        assert!(
            shifted.is_shared(),
            "identity does not require current-generation facts"
        );
        assert!(std::ptr::eq(edge.as_ref(), shifted.as_ref()));
        assert_eq!(constructed, 0, "cloning an Rc edge constructs no Db nodes");
        assert_eq!(spent(), before_spent);
        assert_eq!(live(), baseline + bytes);
        drop(shifted);
        assert_eq!(live(), baseline + bytes);
    }
    drop(edge);
    assert_eq!(live(), baseline);
}

#[test]
fn zero_shift_of_root_preserves_expired_shared_children_and_counts_actual_clones() {
    let _reset = reset();
    let c = Context::new();
    let baseline = live();
    let original = {
        let _scope = PrefixScope::enter(&c);
        assert!(fix_factory_begin());
        Db::App(
            DbBox::new(lam(global("A"), app(Db::Var(1), Db::Var(0)))),
            DbBox::new(app(global("f"), Db::Var(3))),
        )
    };
    let Db::App(a, b) = &original else {
        panic!("application")
    };
    assert!(a.is_shared() && b.is_shared());
    assert!(a.facts().is_none() && b.facts().is_none());
    let bytes = a.retained_bytes() + b.retained_bytes();
    assert_eq!(clone_nodes(&original), 1);
    let before_spent = spent();
    BETA_WORK.with(|s| s.set(Some((0, 0))));
    let shifted = shift(&original, 0, 0);
    let (_, constructed) = BETA_WORK.with(|s| s.replace(None).unwrap());
    assert!(same_syntax(&shifted, &original));
    let Db::App(c, d) = &shifted else {
        panic!("application")
    };
    assert!(std::ptr::eq(a.as_ref(), c.as_ref()));
    assert!(std::ptr::eq(b.as_ref(), d.as_ref()));
    assert_eq!(
        constructed, 1,
        "only the root Db is cloned above shared edges"
    );
    assert_eq!(spent(), before_spent);
    assert_eq!(live(), baseline + bytes);
    drop(original);
    assert_eq!(
        live(),
        baseline + bytes,
        "result retains both persistent child tickets"
    );
    drop(shifted);
    assert_eq!(live(), baseline);
}
