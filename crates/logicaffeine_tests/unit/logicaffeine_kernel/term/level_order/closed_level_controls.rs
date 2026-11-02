use super::*;

fn closed_values() -> Vec<Universe> {
    let leaves = vec![
        Universe::SProp,
        Universe::Prop,
        Universe::Type(0),
        Universe::Type(1),
        Universe::Type(u32::MAX),
    ];
    let mut result = leaves.clone();
    for a in &leaves {
        result.push(Universe::Succ(Box::new(a.clone())));
        for b in &leaves {
            result.push(Universe::Max(Box::new(a.clone()), Box::new(b.clone())));
            result.push(Universe::IMax(Box::new(a.clone()), Box::new(b.clone())));
        }
    }
    result
}

#[test]
fn closed_order_matches_the_existing_symbolic_decision_procedure() {
    let values = closed_values();
    for a in &values {
        for b in &values {
            assert_eq!(
                leq(a, b),
                ordered(a, b, Unmetered).unwrap(),
                "{a:?} <= {b:?}"
            );
        }
    }
}

#[test]
fn deep_closed_universe_order_uses_constant_native_stack() {
    let depth = 4096;
    let mut u = Universe::Type(u32::MAX);
    for _ in 0..depth {
        u = Universe::Succ(Box::new(u));
    }
    assert!(leq(&Universe::Type(u32::MAX), &u));
    assert!(!leq(&u, &Universe::Type(u32::MAX)));
    assert!(leq(&u, &u));
    // Fixture destruction is iterative too; the assertion exercises ordering.
    while let Universe::Succ(inner) = u {
        u = *inner;
    }
}

#[test]
fn symbolic_inputs_keep_the_existing_decision_procedure() {
    let var = Universe::Var("u".into());
    let values = vec![
        var.clone(),
        Universe::Succ(Box::new(var.clone())),
        Universe::Max(Box::new(var.clone()), Box::new(Universe::Type(2))),
        Universe::IMax(Box::new(Universe::Type(2)), Box::new(var)),
    ];
    for a in values.iter().chain(closed_values().iter()) {
        for b in &values {
            assert_eq!(leq(a, b), ordered(a, b, Unmetered).unwrap());
            assert_eq!(leq(b, a), ordered(b, a, Unmetered).unwrap());
        }
    }
}
