//! tensor_recognize_module_lock — the backend-agnostic tensor-op RECOGNIZERS have a NEUTRAL home.
//!
//! The recognizers (`recognize_matmul` / `_elementwise` / `_reduction` / `_fused_row` / `_swiglu`) and their
//! specs are shared by EVERY codegen backend (CUDA, Tenstorrent, RISC-V). They must therefore live in a
//! backend-neutral module — `crate::tensor_recognize` — not under any one backend (`codegen_tt`). This lock
//! pins that neutral module as the canonical import path and proves the recognizers work through it, so a
//! regression that hides them back inside a backend fails RED.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_recognize_module_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::tensor_recognize::{
    recognize_fused_row, recognize_matmul, recognize_swiglu, ElementwiseOp, ReductionKind,
    RowReduceKind,
};
use logicaffeine_compile::ui_bridge::with_parsed_program;

const MATMUL: &str = r#"## Main
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
"#;

const SWIGLU: &str = r#"## Main
Let G be a 1 by 4 tensor of Real.
Set item 1 of G to 1.0.
Set item 2 of G to 2.0.
Set item 3 of G to 3.0.
Set item 4 of G to 4.0.
Let U be a 1 by 4 tensor of Real.
Set item 1 of U to 1.0.
Set item 2 of U to 2.0.
Set item 3 of U to 3.0.
Set item 4 of U to 4.0.
Let Y be swiglu of G and U.
Show item 1 of Y.
"#;

const FUSED_ADD: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let R be a 2 by 3 tensor of Real.
Set item 1 of R to 0.5.
Set item 2 of R to 0.5.
Set item 3 of R to 0.5.
Set item 4 of R to 1.0.
Set item 5 of R to 1.0.
Set item 6 of R to 1.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X plus R with weight W.
Show item 1 of Y.
"#;

#[test]
fn the_recognizers_are_reachable_and_working_through_the_neutral_module() {
    // Types are inferred from the neutral recognizers' signatures — no backend path is named.
    let m = with_parsed_program(MATMUL, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parses");
        Ok::<_, String>(recognize_matmul(stmts, interner))
    })
    .unwrap()
    .expect("matmul recognized via the neutral module");
    assert_eq!((m.m, m.n, m.k), (2, 2, 2));

    let sg = with_parsed_program(SWIGLU, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parses");
        Ok::<_, String>(recognize_swiglu(stmts, interner))
    })
    .unwrap()
    .expect("swiglu recognized via the neutral module");
    assert_eq!(sg.total, 4);

    let fr = with_parsed_program(FUSED_ADD, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parses");
        Ok::<_, String>(recognize_fused_row(stmts, interner))
    })
    .unwrap()
    .expect("fused-add recognized via the neutral module");
    assert_eq!((fr.rows, fr.cols), (2, 3));
    assert!(matches!(fr.reduce, RowReduceKind::RmsNorm));
    assert!(fr.residual.is_some() && fr.weight.is_some());
}

#[test]
fn the_spec_types_are_the_neutral_ones() {
    // The specs are nameable from the neutral module — the type identity every backend shares.
    let _ = ElementwiseOp::Add;
    let _ = ReductionKind::Rmsnorm;
    let _ = RowReduceKind::LayerNorm;
}
