use std::collections::BTreeMap;

use super::*;

fn big(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

/// A head state holding `values` at locations `0, 1, …`.
fn state(values: &[i64]) -> State<u32> {
    State {
        env: values.iter().enumerate().map(|(l, &v)| (l as u32, SymVal::Int(Int::from_i64(v)))).collect(),
        depth: 0,
    }
}

fn name(v: &Var<u32>) -> String {
    match v {
        Var::Loc(l) => ["i", "total", "n"][*l as usize].to_string(),
        Var::Input(j) => format!("in{j}"),
        Var::From(l) => format!("{l}.first"),
        Var::To(l) => format!("{l}.last"),
    }
}

fn loc(l: &u32) -> String {
    name(&Var::Loc(*l))
}

/// Each variable's least and greatest value, and each difference's, over the rows: every bound offered holds on every
/// row, and each is met by some row.
#[test]
fn the_bounds_offered_are_the_extremes_the_rows_reach() {
    let vars = vec![Var::Loc(0), Var::Loc(1)];
    let rows: Vec<Vec<BigInt>> = [(1, 0), (2, 1), (3, 3), (4, 6), (5, 10)].iter().map(|&(a, b)| vec![big(a), big(b)]).collect();
    let facts = offered(&vars, &rows, &rows);
    for f in &facts {
        assert!(rows.iter().all(|r| f.on(&vars, r)), "{f:?} holds on every row");
    }
    for want in [
        Fact::AtLeast(Var::Loc(0), big(1)),
        Fact::AtMost(Var::Loc(0), big(5)),
        Fact::AtLeast(Var::Loc(1), big(0)),
        Fact::AtMost(Var::Loc(1), big(10)),
        Fact::GapAtLeast(Var::Loc(0), Var::Loc(1), big(-5)),
        Fact::GapAtMost(Var::Loc(0), Var::Loc(1), big(1)),
    ] {
        assert!(facts.contains(&want), "{want:?} in {facts:?}");
    }
    assert!(facts.iter().any(|f| matches!(f, Fact::Poly(_))), "the triangular numbers are a quadratic relation: {facts:?}");
    assert!(offered::<u32>(&vars, &[], &[]).is_empty());
    let far: Vec<Vec<BigInt>> = vec![vec![big(1 << 40), big(7)]];
    let mixed = offered(&vars, &rows, &[rows.clone(), far].concat());
    assert!(mixed.contains(&Fact::AtMost(Var::Loc(0), big(1 << 40))), "bounds read every row: {mixed:?}");
    assert!(
        mixed.iter().any(|f| matches!(f, Fact::Poly(_))),
        "relations are read from the small rows, so the far one does not break the triangular relation: {mixed:?}"
    );
}

/// The exact evaluation that drops a candidate before Z3 is asked and the Z3 term the proof assumes agree on every
/// fact and every row: a candidate is never dropped for a reason the proof would not see.
#[test]
fn a_fact_on_a_row_is_what_its_term_says_at_that_row() {
    let vars = vec![Var::Loc(0), Var::Loc(1), Var::Input(0)];
    let facts = vec![
        Fact::Poly(vec![(big(1), vec![(Var::Loc(0), 2)]), (big(-2), vec![(Var::Loc(1), 1)]), (big(-1), vec![(Var::Loc(0), 1)])]),
        Fact::Poly(vec![(big(3), vec![(Var::Loc(0), 1), (Var::Input(0), 1)]), (big(-7), vec![])]),
        Fact::AtLeast(Var::Loc(0), big(-2)),
        Fact::AtMost(Var::Input(0), big(3)),
        Fact::GapAtLeast(Var::Loc(0), Var::Input(0), big(0)),
        Fact::GapAtMost(Var::Loc(1), Var::Loc(0), big(4)),
    ];
    let mut checked = (0, 0);
    for i in -4..=6 {
        for t in [-3, 0, 1, 6, 10, 15] {
            for n in [-2, 0, 1, 3, 5] {
                let row = vec![big(i), big(t), big(n)];
                let at = state(&[i, t]);
                let inputs = vec![SymVal::Int(Int::from_i64(n))];
                for f in &facts {
                    let exact = f.on(&vars, &row);
                    assert_eq!(f.holds(&at, &inputs).simplify().as_bool(), Some(exact), "{f:?} at {row:?}");
                    if exact {
                        checked.0 += 1
                    } else {
                        checked.1 += 1
                    }
                }
            }
        }
    }
    assert!(checked.0 > 100 && checked.1 > 100, "both answers are exercised: {checked:?}");
}

/// A fact over a variable the state does not hold is false, never vacuously true.
#[test]
fn a_fact_over_a_missing_variable_is_false() {
    let f = Fact::AtLeast(Var::Loc(7), big(0));
    assert_eq!(f.holds(&state(&[1, 2]), &[]).simplify().as_bool(), Some(false));
    let g = Fact::GapAtMost(Var::Loc(0), Var::Input(3), big(0));
    assert_eq!(g.holds(&state(&[1, 2]), &[]).simplify().as_bool(), Some(false));
}

#[test]
fn a_relation_reads_with_each_side_positive() {
    let gauss = Fact::Poly(vec![
        (big(1), vec![(Var::Loc(0), 2)]),
        (big(-2), vec![(Var::Loc(1), 1)]),
        (big(-1), vec![(Var::Loc(0), 1)]),
    ]);
    assert_eq!(gauss.describe(&name, &loc), "i² = 2·total + i");
    let linear = Fact::Poly(vec![(big(1), vec![(Var::Loc(2), 1)]), (big(-1), vec![(Var::Input(0), 1)])]);
    assert_eq!(linear.describe(&name, &loc), "n = in0");
    let constant = Fact::Poly(vec![(big(1), vec![(Var::Loc(0), 1)]), (big(-5), vec![])]);
    assert_eq!(constant.describe(&name, &loc), "i = 5");
    assert_eq!(Fact::<u32>::Poly(vec![(big(1), vec![(Var::Loc(1), 1)])]).describe(&name, &loc), "total = 0");
    assert_eq!(Fact::GapAtMost(Var::Loc(0), Var::Input(0), big(1)).describe(&name, &loc), "i − in0 ≤ 1");
    assert_eq!(Fact::<u32>::Truth(1, true).describe(&name, &loc), "total = true");
}

#[test]
fn a_condition_is_evaluated_at_a_sample_only_when_it_speaks_of_inputs_alone() {
    let n = Int::new_const("n");
    let params = vec![SymVal::Int(n.clone())];
    let cond = n.ge(Int::from_i64(1));
    assert_eq!(evaluate(&cond, &params, &[SymVal::Int(Int::from_i64(0))]), Some(false));
    assert_eq!(evaluate(&cond, &params, &[SymVal::Int(Int::from_i64(3))]), Some(true));
    let other = Int::new_const("m").ge(Int::from_i64(1));
    assert_eq!(evaluate(&other, &params, &[SymVal::Int(Int::from_i64(3))]), None);
}

/// Pruning keeps the conjunction's meaning and drops the linear facts the other linear facts imply: of `i ≥ 0` and
/// `i ≥ 1` the weaker goes. A bound only a relation of higher degree implies (`total ≥ 1` from `total = i²`) stays,
/// and so does the relation: pruning never asks Z3 about a polynomial, which is a search it need not finish.
#[test]
fn pruning_drops_what_the_other_linear_facts_imply_and_keeps_every_relation() {
    let solver = Solver::new();
    let at = State {
        env: BTreeMap::from([(0u32, SymVal::Int(Int::new_const("i"))), (1, SymVal::Int(Int::new_const("total")))]),
        depth: 0,
    };
    let relation = Fact::Poly(vec![(big(1), vec![(Var::Loc(1), 1)]), (big(-1), vec![(Var::Loc(0), 2)])]);
    let facts = vec![
        Fact::AtLeast(Var::Loc(0), big(0)),
        Fact::AtLeast(Var::Loc(0), big(1)),
        Fact::AtLeast(Var::Loc(1), big(1)),
        relation.clone(),
        Fact::AtMost(Var::Loc(0), big(9)),
    ];
    let kept = prune(&solver, facts, &at, &[]);
    assert_eq!(
        kept,
        vec![Fact::AtLeast(Var::Loc(0), big(1)), Fact::AtLeast(Var::Loc(1), big(1)), relation, Fact::AtMost(Var::Loc(0), big(9))]
    );
}

/// A relation of degree one is linear, so it prunes and is pruned: `n = n₀` makes `n − n₀ ≥ 0` and `n − n₀ ≤ 0` go.
#[test]
fn a_linear_relation_drops_the_bounds_it_implies() {
    let solver = Solver::new();
    let at = State { env: BTreeMap::from([(2u32, SymVal::Int(Int::new_const("n")))]), depth: 0 };
    let inputs = vec![SymVal::Int(Int::new_const("n0"))];
    let equal = Fact::Poly(vec![(big(1), vec![(Var::Loc(2), 1)]), (big(-1), vec![(Var::Input(0), 1)])]);
    let facts = vec![
        Fact::GapAtLeast(Var::Loc(2), Var::Input(0), big(0)),
        Fact::GapAtMost(Var::Loc(2), Var::Input(0), big(0)),
        equal.clone(),
    ];
    assert_eq!(prune(&solver, facts, &at, &inputs), vec![equal]);
}

/// What each bound caps from above, the quantity a loop must raise to be left: the variable an upper bound caps, the
/// negation of one a lower bound floors, the difference a gap bounds; a relation caps nothing.
#[test]
fn each_bound_caps_the_quantity_it_bounds_from_above() {
    let at = state(&[3, 10]);
    let inputs = vec![SymVal::Int(Int::from_i64(4))];
    let value = |f: Fact<u32>| f.capped(&at, &inputs).map(|t| t.simplify().as_i64().expect("a numeral"));
    assert_eq!(value(Fact::AtMost(Var::Loc(0), big(9))), Some(3));
    assert_eq!(value(Fact::AtLeast(Var::Loc(1), big(0))), Some(-10));
    assert_eq!(value(Fact::GapAtMost(Var::Loc(0), Var::Input(0), big(1))), Some(-1));
    assert_eq!(value(Fact::GapAtLeast(Var::Loc(1), Var::Loc(0), big(0))), Some(-7));
    assert_eq!(value(Fact::Poly(vec![(big(1), vec![(Var::Loc(0), 1)])])), None);
    assert_eq!(value(Fact::Truth(0, true)), None);
    for f in [Fact::AtMost(Var::Loc(0), big(9)), Fact::GapAtLeast(Var::Loc(1), Var::Loc(0), big(0))] {
        let bound = f.capped(&at, &inputs).unwrap();
        let cap = match &f {
            Fact::AtMost(_, c) | Fact::GapAtLeast(_, _, c) => c.clone(),
            _ => unreachable!(),
        };
        let claimed = match &f {
            Fact::AtMost(..) => bound.le(Int::from_i64(cap.to_i64().unwrap())),
            _ => bound.le(Int::from_i64(-cap.to_i64().unwrap())),
        };
        assert_eq!(
            claimed.simplify().as_bool(),
            f.holds(&at, &inputs).simplify().as_bool(),
            "the capped quantity is at most the cap exactly when {f:?} holds"
        );
    }
}
