//! Constructor cost and semantic controls for symbolic universe levels.
use super::*;
fn var(n: &str) -> Universe {
    Universe::Var(n.into())
}
fn eval(u: &Universe, values: &[u64; 3]) -> u64 {
    match u {
        Universe::SProp | Universe::Prop => 0,
        Universe::Type(n) => *n as u64 + 1,
        Universe::Var(n) => {
            values[match n.as_str() {
                "u" => 0,
                "v" => 1,
                "w" => 2,
                _ => panic!("unknown variable"),
            }]
        }
        Universe::Succ(a) => eval(a, values) + 1,
        Universe::Max(a, b) => eval(a, values).max(eval(b, values)),
        Universe::IMax(a, b) => {
            let b = eval(b, values);
            if b == 0 {
                0
            } else {
                eval(a, values).max(b)
            }
        }
    }
}
#[test]
fn constructing_symbolic_pi_sorts_does_not_run_the_universe_solver() {
    let mut codomain = var("u");
    for i in 0..9 {
        codomain = Universe::IMax(Box::new(var(&format!("p{i}"))), Box::new(codomain));
    }
    LEVEL_COMPARISONS.with(|n| n.set(0));
    let _result = var("domain").imax(&codomain);
    assert_eq!(
        LEVEL_COMPARISONS.with(|n| n.get()),
        0,
        "forming a symbolic Pi sort must not enumerate universe assignments"
    );
}
#[test]
fn symbolic_constructor_agrees_with_independent_natural_model() {
    let mut levels = vec![
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(7),
        Universe::Type(u32::MAX),
        var("u"),
        var("v"),
        var("w"),
    ];
    let base = levels.clone();
    for a in &base {
        for b in &base {
            levels.push(Universe::Max(Box::new(a.clone()), Box::new(b.clone())));
            levels.push(Universe::IMax(Box::new(a.clone()), Box::new(b.clone())));
        }
    }
    for a in &levels {
        for b in &levels {
            let built = a.imax(b);
            for u in [0, 1, 3] {
                for v in [0, 1, 8] {
                    for w in [0, 1, 20] {
                        let values = [u, v, w];
                        let right = eval(b, &values);
                        let expected = if right == 0 {
                            0
                        } else {
                            eval(a, &values).max(right)
                        };
                        assert_eq!(eval(&built, &values), expected, "{a:?}, {b:?}, {values:?}");
                    }
                }
            }
        }
    }
}
#[test]
fn symbolic_constructor_preserves_sprop_and_semantic_equality() {
    let u = var("u");
    let duplicated = Universe::Max(Box::new(u.clone()), Box::new(u.clone()));
    assert!(u.imax(&duplicated).equiv(&u));
    assert_eq!(u.imax(&Universe::SProp), Universe::SProp);
    assert_eq!(u.imax(&Universe::Prop), Universe::Prop);
    assert!(!u.imax(&var("v")).equiv(&u.max(&var("v"))));
}
