use super::*;
use logicaffeine_base::numeric::Rational;
use logicaffeine_kernel::lia::{fourier_motzkin_unsat, Constraint, LinearExpr};

const LI: i64 = 1;
const RI: i64 = 2;
const K: i64 = 3;
const I: i64 = 4;
const N: i64 = 5;
const LEN: i64 = -7;

fn v(i: i64) -> LinearExpr {
    LinearExpr::var(i)
}

fn c(n: i64) -> LinearExpr {
    LinearExpr::constant(Rational::from_i64(n))
}

fn set(s: &mut AffineSpace, dim: i64, value: LinearExpr) {
    s.assign(dim, Some(&value));
}

/// Whether the space's equalities imply `goal = 0`, decided by the kernel prover the oracle uses: both `goal < 0`
/// and `goal > 0` must contradict them.
fn implies(s: &AffineSpace, goal: &LinearExpr) -> bool {
    let facts: Vec<Constraint> = s
        .equalities()
        .into_iter()
        .flat_map(|e| {
            [
                Constraint {
                    expr: e.clone(),
                    strict: false,
                },
                Constraint {
                    expr: e.neg(),
                    strict: false,
                },
            ]
        })
        .collect();
    let below = facts
        .iter()
        .cloned()
        .chain([Constraint {
            expr: goal.clone(),
            strict: true,
        }])
        .collect::<Vec<_>>();
    let above = facts
        .iter()
        .cloned()
        .chain([Constraint {
            expr: goal.neg(),
            strict: true,
        }])
        .collect::<Vec<_>>();
    fourier_motzkin_unsat(&below) && fourier_motzkin_unsat(&above)
}

/// The merge loop of a merge sort: `k` advances with one of `li` and `ri` on every pass.
fn merge_pass(header: &AffineSpace) -> AffineSpace {
    let mut took_left = header.clone();
    set(&mut took_left, LI, v(LI).add(&c(1)));
    let mut took_right = header.clone();
    set(&mut took_right, RI, v(RI).add(&c(1)));
    let mut out = took_left.join(&took_right);
    set(&mut out, K, v(K).add(&c(1)));
    out
}

/// The loop header reached from `entry` by `pass`: the hull of the entry and every pass, iterated to its fixpoint.
fn header(entry: &AffineSpace, pass: impl Fn(&AffineSpace) -> AffineSpace) -> (AffineSpace, usize) {
    let mut h = entry.clone();
    for rounds in 1..=16 {
        let next = entry.join(&pass(&h));
        if next.leq(&h) {
            return (h, rounds);
        }
        h = next;
    }
    panic!("no fixpoint in 16 passes");
}

#[test]
fn nothing_is_known_at_the_top() {
    assert!(AffineSpace::top().equalities().is_empty());
    assert!(AffineSpace::top().tracked().is_empty());
}

#[test]
fn constants_are_equalities() {
    let mut s = AffineSpace::top();
    set(&mut s, LI, c(1));
    set(&mut s, RI, c(1));
    assert!(implies(&s, &v(LI).sub(&c(1))));
    assert!(implies(&s, &v(RI).sub(&c(1))));
    assert!(!implies(&s, &v(LI).sub(&c(2))));
}

#[test]
fn an_increment_shifts_the_equalities_through_its_variable() {
    let mut s = AffineSpace::top();
    set(&mut s, I, c(0));
    set(&mut s, N, v(I).add(&c(3)));
    set(&mut s, I, v(I).add(&c(1)));
    assert!(implies(&s, &v(I).sub(&c(1))));
    assert!(implies(&s, &v(N).sub(&c(3))));
    assert!(implies(&s, &v(N).sub(&v(I)).sub(&c(2))));
}

#[test]
fn a_relation_to_an_untracked_quantity_is_kept() {
    let mut s = AffineSpace::top();
    set(&mut s, N, v(LEN));
    assert!(implies(&s, &v(N).sub(&v(LEN))));
    assert!(!implies(&s, &v(N)));
}

#[test]
fn a_value_that_is_not_affine_frees_its_variable_alone() {
    let mut s = AffineSpace::top();
    set(&mut s, I, c(4));
    set(&mut s, N, c(9));
    s.assign(I, None);
    assert!(!implies(&s, &v(I).sub(&c(4))));
    assert!(implies(&s, &v(N).sub(&c(9))));
    assert_eq!(
        s.tracked(),
        &[N],
        "a quantity every value of which is reachable is not tracked"
    );
}

#[test]
fn a_merge_keeps_the_equalities_both_paths_keep() {
    let mut a = AffineSpace::top();
    set(&mut a, LI, c(1));
    set(&mut a, K, c(1));
    let mut b = AffineSpace::top();
    set(&mut b, LI, c(2));
    set(&mut b, K, c(2));
    let j = a.join(&b);
    assert!(implies(&j, &v(K).sub(&v(LI))), "k = li on both paths");
    assert!(
        !implies(&j, &v(LI).sub(&c(1))),
        "li differs between the paths"
    );
}

#[test]
fn the_merge_loop_keeps_k_one_less_than_li_plus_ri() {
    let mut entry = AffineSpace::top();
    for d in [LI, RI, K] {
        set(&mut entry, d, c(1));
    }
    let (h, rounds) = header(&entry, merge_pass);
    let lockstep = v(K).sub(&v(LI)).sub(&v(RI)).add(&c(1));
    assert!(
        implies(&h, &lockstep),
        "k = li + ri - 1 at the header: {:?}",
        h.equalities()
    );
    assert!(!implies(&h, &v(LI).sub(&c(1))));
    assert!(!implies(&h, &v(K).sub(&c(1))));
    assert!(rounds <= 4, "a pass per dimension at most, took {rounds}");
    assert!(implies(&merge_pass(&h), &lockstep), "and after every pass");
}

#[test]
fn a_counted_build_keeps_its_length_one_less_than_its_counter() {
    let mut entry = AffineSpace::top();
    set(&mut entry, LEN, c(0));
    set(&mut entry, I, c(1));
    let pass = |h: &AffineSpace| {
        let mut s = h.clone();
        set(&mut s, LEN, v(LEN).add(&c(1)));
        set(&mut s, I, v(I).add(&c(1)));
        s
    };
    let (h, _) = header(&entry, pass);
    assert!(
        implies(&h, &v(LEN).sub(&v(I)).add(&c(1))),
        "length = i - 1: {:?}",
        h.equalities()
    );
}

#[test]
fn bottom_joins_to_the_other_side_and_proves_nothing() {
    let mut s = AffineSpace::top();
    set(&mut s, I, c(3));
    assert_eq!(AffineSpace::bottom().join(&s), s);
    assert_eq!(s.join(&AffineSpace::bottom()), s);
    assert!(AffineSpace::bottom().equalities().is_empty());
    assert!(AffineSpace::bottom().leq(&s));
    assert!(!s.leq(&AffineSpace::bottom()));
}

#[test]
fn inclusion_and_equality_ignore_column_order() {
    let mut point = AffineSpace::top();
    set(&mut point, LI, c(1));
    set(&mut point, RI, c(2));
    let mut line = AffineSpace::top();
    set(&mut line, RI, v(LI).add(&c(1)));
    assert!(point.leq(&line), "(1, 2) lies on ri = li + 1");
    assert!(!line.leq(&point));
    let mut same = AffineSpace::top();
    set(&mut same, LI, v(RI).sub(&c(1)));
    assert_eq!(line, same, "ri = li + 1 however it was built");
}

#[test]
fn forgetting_every_quantity_tracks_nothing() {
    let mut s = AffineSpace::top();
    set(&mut s, LI, c(1));
    set(&mut s, K, v(LI).add(&v(RI)));
    s.forget_where(|_| true);
    assert!(s.tracked().is_empty());
    assert!(s.equalities().is_empty());
}

#[test]
fn rational_steps_stay_exact() {
    let mut s = AffineSpace::top();
    set(&mut s, I, c(0));
    set(&mut s, N, c(0));
    let pass = |h: &AffineSpace| {
        let mut t = h.clone();
        set(&mut t, I, v(I).add(&c(3)));
        set(&mut t, N, v(N).add(&c(2)));
        t
    };
    let (h, _) = header(&s, pass);
    assert!(
        implies(
            &h,
            &v(I)
                .scale(&Rational::from_i64(2))
                .sub(&v(N).scale(&Rational::from_i64(3)))
        ),
        "2i = 3n"
    );
}

#[test]
fn a_counted_loop_exit_fixes_its_counter_and_what_moved_with_it() {
    let mut s = AffineSpace::top();
    set(&mut s, LEN, v(I).sub(&c(1)));
    s.meet_eq(&v(I).sub(&v(N)).sub(&c(1)));
    assert!(implies(&s, &v(I).sub(&v(N)).sub(&c(1))), "i = n + 1");
    assert!(implies(&s, &v(LEN).sub(&v(N))), "length = n");
}

#[test]
fn meeting_an_equality_the_space_already_keeps_changes_nothing() {
    let mut s = AffineSpace::top();
    set(&mut s, K, v(LI).add(&v(RI)));
    let before = s.clone();
    s.meet_eq(&v(K).sub(&v(LI)).sub(&v(RI)));
    assert_eq!(s, before);
}

#[test]
fn meeting_a_contradiction_reaches_nothing() {
    let mut s = AffineSpace::top();
    set(&mut s, I, c(3));
    s.meet_eq(&v(I).sub(&c(4)));
    assert!(s.is_bottom());
}

#[test]
fn entailment_reads_the_generators() {
    let mut s = AffineSpace::top();
    set(&mut s, LEN, v(N));
    set(&mut s, K, v(LI).add(&c(1)));
    assert!(s.entails_zero(&v(LEN).sub(&v(N))));
    assert!(s.entails_zero(&v(K).sub(&v(LI)).sub(&c(1))));
    assert!(!s.entails_zero(&v(K).sub(&v(LI))));
    assert!(
        !s.entails_zero(&v(I).sub(&v(I)).add(&v(RI))),
        "an untracked quantity takes every value"
    );
    assert!(AffineSpace::bottom().entails_zero(&v(I).sub(&c(1))));
}

/// The solved form names each free dim once and substitutes it by the pivots alone, and substituting it into every
/// equality leaves nothing: it is the same space.
#[test]
fn the_solved_form_substitutes_each_equality_exactly() {
    let mut s = header(
        &{
            let mut entry = AffineSpace::top();
            set(&mut entry, LI, c(1));
            set(&mut entry, RI, c(1));
            set(&mut entry, K, c(1));
            set(&mut entry, I, c(3));
            set(&mut entry, N, v(I).add(&c(4)));
            entry
        },
        merge_pass,
    )
    .0;
    s.assign(LEN, Some(&v(N).sub(&c(1))));
    let solved = s.solved();
    assert!(!solved.is_empty(), "the space keeps equalities");
    let free: Vec<i64> = solved.iter().map(|(d, _)| *d).collect();
    for (d, value) in &solved {
        assert!(
            free.iter().all(|f| !value.coefficients.contains_key(f)),
            "dim {d}'s value names a free dim: {value:?}"
        );
    }
    for eq in s.equalities() {
        let mut rest = eq.clone();
        for (d, value) in &solved {
            if let Some(coef) = rest.coefficients.remove(d) {
                rest = rest.add(&value.scale(&coef));
            }
        }
        assert!(
            rest.coefficients.values().all(|c| c.is_zero()) && rest.constant.is_zero(),
            "an equality survives substitution: {eq:?} -> {rest:?}"
        );
    }
}

#[test]
fn the_top_solves_to_nothing() {
    assert!(AffineSpace::top().solved().is_empty());
}
