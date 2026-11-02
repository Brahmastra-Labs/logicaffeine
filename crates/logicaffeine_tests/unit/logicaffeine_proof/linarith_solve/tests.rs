use super::*;

fn cst(c: i64) -> ProofTerm {
    ProofTerm::Constant(c.to_string())
}
fn var(s: &str) -> ProofTerm {
    ProofTerm::Constant(s.to_string())
}
fn f(n: &str, a: Vec<ProofTerm>) -> ProofTerm {
    ProofTerm::Function(n.to_string(), a)
}
/// The constraint from `l ≤ r`: `l - r ≤ 0`.
fn le_constraint(l: ProofTerm, r: ProofTerm) -> LinExpr {
    parse_lin(&l).unwrap().sub(&parse_lin(&r).unwrap())
}

#[test]
fn parses_linear_forms() {
    // 2*x + 3  -  (x + 1)  =  x + 2
    let e = le_constraint(
        f("add", vec![f("mul", vec![cst(2), var("x")]), cst(3)]),
        f("add", vec![var("x"), cst(1)]),
    );
    assert_eq!(e.coeff("x"), 1);
    assert_eq!(e.constant, 2);
}

#[test]
fn rejects_nonlinear() {
    assert!(parse_lin(&f("mul", vec![var("x"), var("y")])).is_none());
}

#[test]
fn chain_contradiction_5_le_x_le_3() {
    // 5 ≤ x  →  5 - x ≤ 0 ;  x ≤ 3  →  x - 3 ≤ 0.  Sum = 2 ≤ 0 (contradiction).
    let cs = vec![
        le_constraint(cst(5), var("x")),
        le_constraint(var("x"), cst(3)),
    ];
    let cert = find_farkas(&cs).expect("5≤x≤3 is unsatisfiable");
    assert_eq!(cert.get(&0), Some(&1));
    assert_eq!(cert.get(&1), Some(&1));
}

#[test]
fn scaling_contradiction_needs_a_multiplier() {
    // 2 ≤ x  →  2 - x ≤ 0 ;  2*x ≤ 3  →  2x - 3 ≤ 0.
    // 2·(2 - x) + 1·(2x - 3) = 1 ≤ 0  — contradiction, multiplier 2 on the first.
    let cs = vec![
        le_constraint(cst(2), var("x")),
        le_constraint(f("mul", vec![cst(2), var("x")]), cst(3)),
    ];
    let cert = find_farkas(&cs).expect("2≤x and 2x≤3 is unsatisfiable");
    assert_eq!(cert.get(&0), Some(&2), "needs to scale `2 ≤ x` by 2");
    assert_eq!(cert.get(&1), Some(&1));
}

#[test]
fn two_variable_contradiction() {
    // x ≤ y, y ≤ z, z ≤ x - 1  →  x ≤ y ≤ z ≤ x-1  →  x ≤ x-1  →  1 ≤ 0.
    let cs = vec![
        le_constraint(var("x"), var("y")),
        le_constraint(var("y"), var("z")),
        le_constraint(var("z"), f("sub", vec![var("x"), cst(1)])),
    ];
    let cert = find_farkas(&cs).expect("the cycle is unsatisfiable");
    assert_eq!(cert.get(&0), Some(&1));
    assert_eq!(cert.get(&1), Some(&1));
    assert_eq!(cert.get(&2), Some(&1));
}

#[test]
fn satisfiable_system_has_no_certificate() {
    // 1 ≤ x, x ≤ 5  — consistent.
    let cs = vec![
        le_constraint(cst(1), var("x")),
        le_constraint(var("x"), cst(5)),
    ];
    assert!(find_farkas(&cs).is_none(), "1≤x≤5 is satisfiable");
}
