//! tensor_sugar_tier_lock — Phase 2 of the tensor-compiler arc: **write it in LOGOS with tensor sugar**.
//!
//! A `Tensor` in LOGOS is a compile-time concept — a statically-known shape (`R by C`) over a flat, row-major
//! backing `Seq`. Two pieces of sugar:
//!   • `Let A be a R by C tensor of Int.` — declares a zero-filled `R×C` tensor (desugars to a flat `Seq`);
//!   • `Let C be A matmul B.` — shape-checked matrix product (desugars to the flat-Seq triple-loop).
//!
//! The desugaring happens in the PARSER (a `Stmt::Splice` of ordinary statements every tier already runs), so:
//!   • it runs ≡ the VM on EVERY execution tier for free (the tree-walker/VM see only `Seq` + loops);
//!   • the desugared product is byte-identical to the hand-written flat-Seq matmul a user would write;
//!   • it is the SAME loop idiom `recognize_matmul` lifts — so the sugar flows straight into the optimized
//!     CUDA backend (Phase 1), closing the loop: readable LOGOS → the fast kernel.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_sugar_tier_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_cuda_matmul, tw_outcome, vm_outcome};

/// A 2×2 · 2×2 matmul written with tensor SUGAR. `A=[[1,2],[3,4]]`, `B=[[5,6],[7,8]]` (row-major flat fill),
/// `C = A·B = [[19,22],[43,50]]`.
const SUGAR_2X2: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

/// The SAME computation with NO sugar — flat `Seq`s + an explicit triple loop. This is what the sugar must
/// desugar to, verbatim in behavior.
const MANUAL_2X2: &str = r#"## Main
Let mutable A be a new Seq of Int.
Push 1 to A.
Push 2 to A.
Push 3 to A.
Push 4 to A.
Let mutable B be a new Seq of Int.
Push 5 to B.
Push 6 to B.
Push 7 to B.
Push 8 to B.
Let mutable C be a new Seq of Int.
Let mutable f be 0.
While f is less than 4:
    Push 0 to C.
    Set f to f + 1.
Let mutable i be 0.
While i is less than 2:
    Let mutable k be 0.
    While k is less than 2:
        Let mutable j be 0.
        While j is less than 2:
            Set item (i * 2 + j + 1) of C to item (i * 2 + j + 1) of C + item (i * 2 + k + 1) of A * item (k * 2 + j + 1) of B.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;

#[test]
fn tensor_matmul_sugar_computes_the_right_product() {
    let out = tw_outcome(SUGAR_2X2).output;
    assert_eq!(
        out.split_whitespace().collect::<Vec<_>>(),
        ["19", "22", "43", "50"],
        "C = A·B row-major"
    );
}

#[test]
fn tensor_matmul_sugar_equals_the_hand_written_loops() {
    // The sugar is exactly the flat-Seq matmul idiom — same output as the no-sugar program.
    assert_eq!(
        tw_outcome(SUGAR_2X2).output.trim(),
        tw_outcome(MANUAL_2X2).output.trim(),
        "sugar must desugar to the loops"
    );
}

#[test]
fn tensor_matmul_sugar_is_tier_identical() {
    // Because it desugars to primitives every tier already agrees on, the tree-walker and VM must match.
    // (Compared trimmed — the tw/VM harness differ only in a trailing newline, per matmul_multibackend_lock.)
    assert_eq!(
        tw_outcome(SUGAR_2X2).output.trim(),
        vm_outcome(SUGAR_2X2).output.trim(),
        "tw ≡ VM on the sugar program"
    );
}

#[test]
fn a_bigger_tensor_matmul_sugar_matches_the_manual_loops() {
    // 2×3 · 3×2 ⇒ 2×2, to exercise a non-square contraction (K≠M,N).
    let sugar = r#"## Main
Let A be a 2 by 3 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Set item 5 of A to 5.
Set item 6 of A to 6.
Let B be a 3 by 2 tensor of Int.
Set item 1 of B to 7.
Set item 2 of B to 8.
Set item 3 of B to 9.
Set item 4 of B to 10.
Set item 5 of B to 11.
Set item 6 of B to 12.
Let C be A matmul B.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
"#;
    // A=[[1,2,3],[4,5,6]], B=[[7,8],[9,10],[11,12]] ⇒ C=[[58,64],[139,154]].
    assert_eq!(
        tw_outcome(sugar)
            .output
            .split_whitespace()
            .collect::<Vec<_>>(),
        ["58", "64", "139", "154"]
    );
    assert_eq!(
        tw_outcome(sugar).output.trim(),
        vm_outcome(sugar).output.trim(),
        "tw ≡ VM on the 2×3·3×2 sugar"
    );
}

#[test]
fn the_desugared_tensor_matmul_is_recognized_by_the_cuda_backend() {
    // The payoff: the SUGAR desugars to the exact loop idiom `recognize_matmul` lifts, so a program written
    // with tensor sugar flows straight into the optimized CUDA backend — readable LOGOS → the fast kernel.
    let (spec, _cu) = compile_to_cuda_matmul(SUGAR_2X2).expect(
        "the desugared tensor matmul must be a recognizable GEMM the CUDA backend can lower",
    );
    assert_eq!(
        (spec.m, spec.n, spec.k),
        (2, 2, 2),
        "dims recovered from the desugared sugar"
    );
}
