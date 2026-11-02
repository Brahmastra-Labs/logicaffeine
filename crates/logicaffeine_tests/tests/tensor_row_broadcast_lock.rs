//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A ROW COMBINES WITH EVERY ROW OF A BLOCK.
//!
//! `A plus B` and `A hadamard B` take a `B` of one row as wide as `A`'s rows and combine it with each of them — a
//! projection's bias (gpt-oss adds one to its queries, keys, values and outputs) applied to a block of rows as to one.
//!
//!   • every tier computes `a[r][c] ⊕ b[c]`, for `plus` and `hadamard`, on a block and on a single row;
//!   • the tensor front end lifts the broadcast into the op's spec (`ElementwiseSpec::row`), and a same-shape op
//!     carries none;
//!   • a `B` of another width, or of several rows that are not `A`'s, is refused by name;
//!   • the accelerator lowerings either compute the broadcast or refuse it by name.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{compile_to_cuda_forward, tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};

const A: [[f64; 3]; 2] = [[0.5, -1.25, 3.0], [2.0, 0.75, -0.5]];
const B: [f64; 3] = [1.5, -0.25, 4.0];

/// `A` (`rows` by 3) and `B` (1 by 3) filled with `Real` literals, `Y` bound by `op`, every entry shown as
/// `round(y·1e6)`.
fn program(op: &str, rows: usize) -> String {
    let mut s = format!("## Main\nLet A be a {rows} by 3 tensor of Real.\nLet B be a 1 by 3 tensor of Real.\n");
    for r in 0..rows {
        for c in 0..3 {
            s.push_str(&format!("Set item {} of A to {:?}.\n", r * 3 + c + 1, A[r][c]));
        }
    }
    for (c, b) in B.iter().enumerate() {
        s.push_str(&format!("Set item {} of B to {b:?}.\n", c + 1));
    }
    s.push_str(&format!("Let Y be {op}.\n"));
    for i in 1..=rows * 3 {
        s.push_str(&format!("Show round(item {i} of Y * 1000000).\n"));
    }
    s
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn formula(rows: usize, f: fn(f64, f64) -> f64) -> Vec<i64> {
    (0..rows).flat_map(|r| (0..3).map(move |c| (f(A[r][c], B[c]) * 1e6).round() as i64)).collect()
}

#[test]
fn every_row_takes_the_row() {
    assert_eq!(shown(&program("A plus B", 2)), formula(2, |a, b| a + b), "plus over a block of two rows");
    assert_eq!(shown(&program("A hadamard B", 2)), formula(2, |a, b| a * b), "hadamard over a block of two rows");
    assert_eq!(shown(&program("A plus B", 1)), formula(1, |a, b| a + b), "one row is the same-shape op");
}

#[test]
fn the_compiled_tiers_broadcast_the_row() {
    let src = program("A plus B", 2);
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_the_broadcast_into_the_spec() {
    let lifted = |op: &str, rows: usize| {
        let rp = recognize_stages(&program(op, rows)).expect("parses");
        assert!(rp.unrecognized.is_empty(), "{op}: {:?}", rp.unrecognized);
        match rp.stages.into_iter().map(|s| s.op).collect::<Vec<_>>().as_slice() {
            [TensorOpSpec::Elementwise(e)] => *e,
            other => panic!("{op} lifts as one elementwise op, not {other:?}"),
        }
    };
    let block = lifted("A plus B", 2);
    assert_eq!((block.n, block.row), (6, Some(3)), "six elements, B one row of three");
    assert_eq!(lifted("A hadamard B", 2).row, Some(3));
    assert_eq!(lifted("A plus B", 1).row, None, "a single row of A is the same-shape op");
}

#[test]
fn a_row_of_another_shape_is_refused() {
    let refused = |decl: &str, op: &str| {
        let src = format!("## Main\n{decl}\nLet Y be {op}.\nShow item 1 of Y.\n");
        vm_outcome(&src).error.unwrap_or_else(|| panic!("`{decl}` `{op}` is refused"))
    };
    let narrow = refused("Let A be a 2 by 3 tensor of Real.\nLet B be a 1 by 2 tensor of Real.", "A plus B");
    assert!(narrow.contains("2×3") && narrow.contains("1×2"), "the shapes are named: {narrow}");
    let tall = refused("Let A be a 2 by 3 tensor of Real.\nLet B be a 3 by 3 tensor of Real.", "A plus B");
    assert!(tall.contains("one row"), "a B of several rows that are not A's is refused: {tall}");
    let left = refused("Let A be a 1 by 3 tensor of Real.\nLet B be a 2 by 3 tensor of Real.", "A plus B");
    assert!(left.contains("one row"), "the row goes on the right: {left}");
}

#[test]
fn the_single_token_cuda_forward_refuses_a_broadcast_by_name() {
    let forward = compile_to_cuda_forward(&program("A plus B", 2)).expect_err("the forward's elementwise kernel is same-shape");
    assert!(forward.contains("row"), "the forward names the broadcast it refuses: {forward}");
    compile_to_cuda_forward(&program("A plus B", 1)).expect("one row is the forward's own elementwise op");
}
