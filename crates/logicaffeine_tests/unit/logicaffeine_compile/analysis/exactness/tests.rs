use super::*;

fn binary(operation: Operation, left: Operation, right: Operation) -> Decision {
    let mut graph = Graph::default();
    let a = graph.def(left, false, false);
    let b = graph.def(right, false, false);
    let out = graph.def(operation, false, false);
    graph.input(out, 0, a, Flow::Sum);
    graph.input(out, 1, b, Flow::Sum);
    decide(&graph)
}

#[test]
fn an_unknown_operand_does_not_prove_a_sum_fits() {
    let decision = binary(
        Operation::Sum { doubles: false },
        Operation::Constant(i64::MAX as i128),
        Operation::Opaque,
    );
    assert!(decision.exact[2], "the opaque operand may be positive");
}

#[test]
fn unknown_factors_do_not_prove_a_product_fits() {
    let decision = binary(Operation::Product, Operation::Opaque, Operation::Opaque);
    assert!(decision.exact[2], "word operands may have an exact product");
}

#[test]
fn bounded_sums_and_products_keep_machine_word_storage() {
    for operation in [Operation::Sum { doubles: false }, Operation::Product] {
        let decision = binary(operation, Operation::Bounded(100), Operation::Bounded(200));
        assert!(
            !decision.exact[2],
            "the complete bound proves this arithmetic fits"
        );
    }
}

#[test]
fn a_small_increment_does_not_bound_an_unbounded_parameter() {
    let decision = binary(
        Operation::Sum { doubles: false },
        Operation::Opaque,
        Operation::Constant(1),
    );
    assert!(
        decision.exact[2],
        "a parameter may already equal the largest word"
    );
}

#[test]
fn a_loop_carried_sum_needs_a_bound_even_with_one_feedback_operand() {
    let mut graph = Graph::default();
    let initial = graph.def(Operation::Constant(i64::MAX as i128), false, false);
    let step = graph.def(Operation::Opaque, false, false);
    let sum = graph.def(Operation::Sum { doubles: false }, false, false);
    graph.input(sum, 0, initial, Flow::Sum);
    graph.input(sum, 0, sum, Flow::Sum);
    graph.input(sum, 1, step, Flow::Sum);
    graph.join(initial, sum);
    let decision = decide(&graph);
    assert!(decision.exact[sum]);
    assert!(
        decision.exact[initial],
        "every definition in the variable's web agrees"
    );
}
