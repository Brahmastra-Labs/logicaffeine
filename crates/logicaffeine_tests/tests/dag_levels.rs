//! The core's universe levels (`dag::level`): the questions the kernel asks of a level must be
//! answered for EVERY assignment of its parameters — a level that is zero for one assignment and
//! not another is neither always zero nor never zero.
//!
//! Every level over two parameters up to a small depth is generated and each answer is checked
//! against evaluating the level under every assignment of the parameters (zero-ness depends
//! only on which parameters are zero, so assignments over {0, 1, 2} decide it), and against the
//! general `≤` decision procedure, itself checked pair by pair against every assignment over
//! {0, …, 4} (a level of depth two adds at most two to a parameter, so a counterexample needs no
//! larger value).

use logicaffeine_kernel::dag::level::{LevelId, LevelNode, Levels};
use logicaffeine_kernel::dag::name::{NameId, Names};

fn eval(ls: &Levels, l: LevelId, at: &dyn Fn(NameId) -> u64) -> u64 {
    match ls.node(l) {
        LevelNode::Zero => 0,
        LevelNode::Succ(a) => eval(ls, a, at) + 1,
        LevelNode::Max(a, b) => eval(ls, a, at).max(eval(ls, b, at)),
        LevelNode::IMax(a, b) => match eval(ls, b, at) {
            0 => 0,
            vb => eval(ls, a, at).max(vb),
        },
        LevelNode::Param(p) => at(p),
        LevelNode::SProp => unreachable!("`all_levels` builds from zero and parameters; `SProp` is a LOGOS sort, never one of them"),
    }
}

/// Every level of depth at most `depth` over `ps`, built with the raw constructors (no
/// simplification), so that `max`/`imax` nodes of every shape appear.
fn all_levels(ls: &mut Levels, ps: &[NameId], depth: u32) -> Vec<LevelId> {
    let mut out = vec![LevelId::ZERO];
    out.extend(ps.iter().map(|p| ls.param(*p)));
    for _ in 0..depth {
        let prev = out.clone();
        for &a in &prev {
            out.push(ls.succ(a));
            for &b in &prev {
                out.push(ls.max_raw(a, b));
                out.push(ls.imax_raw(a, b));
            }
        }
        out.sort();
        out.dedup();
    }
    out
}

#[test]
fn always_zero_means_zero_under_every_assignment() {
    let mut names = Names::new();
    let (u, v) = (names.dotted("u"), names.dotted("v"));
    let mut ls = Levels::new();
    let levels = all_levels(&mut ls, &[u, v], 2);
    assert!(levels.len() > 1000, "levels generated ({})", levels.len());
    let (mut zero, mut not) = (0, 0);
    for l in levels {
        let semantic =
            (0..3).all(|a| (0..3).all(|b| eval(&ls, l, &|p| if p == u { a } else { b }) == 0));
        assert_eq!(ls.is_always_zero(l), semantic, "{:?}", ls.node(l));
        assert_eq!(
            ls.leq(l, LevelId::ZERO),
            semantic,
            "the decision procedure agrees on {:?}",
            ls.node(l)
        );
        if semantic {
            zero += 1
        } else {
            not += 1
        }
    }
    assert!(
        zero > 50 && not > 50,
        "both answers are exercised ({zero} zero, {not} not)"
    );
}

#[test]
fn never_zero_means_nonzero_under_every_assignment() {
    let mut names = Names::new();
    let (u, v) = (names.dotted("u"), names.dotted("v"));
    let mut ls = Levels::new();
    for l in all_levels(&mut ls, &[u, v], 2) {
        let semantic =
            (0..3).all(|a| (0..3).all(|b| eval(&ls, l, &|p| if p == u { a } else { b }) > 0));
        if ls.is_never_zero(l) {
            assert!(semantic, "is_never_zero is sound on {:?}", ls.node(l));
        }
    }
}

#[test]
fn leq_holds_exactly_when_it_holds_under_every_assignment() {
    let mut names = Names::new();
    let (u, v) = (names.dotted("u"), names.dotted("v"));
    let mut ls = Levels::new();
    let levels = all_levels(&mut ls, &[u, v], 2);
    let shallow = all_levels(&mut ls, &[u, v], 1);
    let mut pairs: Vec<(LevelId, LevelId)> = shallow
        .iter()
        .flat_map(|&a| shallow.iter().map(move |&b| (a, b)))
        .collect();
    let mut x = 0x5eed_1e7e_u64;
    for _ in 0..30_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let n = levels.len() as u64;
        pairs.push((levels[(x % n) as usize], levels[((x >> 32) % n) as usize]));
    }
    let (mut holds, mut fails) = (0, 0);
    for (a, b) in pairs {
        let semantic = (0..5).all(|i| {
            (0..5).all(|j| {
                let at = |p: NameId| if p == u { i } else { j };
                eval(&ls, a, &at) <= eval(&ls, b, &at)
            })
        });
        assert_eq!(
            ls.leq(a, b),
            semantic,
            "{:?} ≤ {:?}",
            ls.node(a),
            ls.node(b)
        );
        if semantic {
            holds += 1
        } else {
            fails += 1
        }
    }
    assert!(
        holds > 3_000 && fails > 3_000,
        "both answers are exercised ({holds} hold, {fails} fail)"
    );
}
