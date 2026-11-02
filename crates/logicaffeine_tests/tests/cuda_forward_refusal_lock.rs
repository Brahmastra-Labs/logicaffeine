//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE CUDA FORWARD REFUSES WHAT IT CANNOT LOWER — by name, never by silence.
//!
//! `compile_to_cuda_forward*` lowers a program's tensor statements to one CUDA program of chained kernels. A
//! statement it cannot lift used to be DROPPED: its output then reached the host driver as an "input" with a
//! deterministic fill, so the emitted program ran, printed numbers, and computed a different function than the
//! source. The same held for writes the init collector folded or ignored: a literal `Set item` made AFTER an op
//! read the tensor was folded into the starting fill (moving it before that op), and a write whose value is not
//! a literal was skipped outright.
//!
//! The rule this lock holds: every statement of the program is either lowered, or the compile is refused and the
//! refusal names the statement. The tensor front end (`spatial::passes::recognize_stages`) is the classifier, so
//! the CUDA path and the spatial path agree on what a program's statements ARE; the CUDA path then refuses each
//! lifted op it has no kernel for.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::compile_to_cuda_forward;

fn refusal(src: &str) -> String {
    match compile_to_cuda_forward(src) {
        Ok(cu) => panic!(
            "the CUDA forward lowered a program it cannot compute faithfully; it must refuse.\n--- source ---\n{src}\n--- emitted ({} bytes) ---\n{}",
            cu.len(),
            &cu[..cu.len().min(600)]
        ),
        Err(e) => e,
    }
}

const DECLS: &str = "## Main\n\
    Let A be a 2 by 2 tensor of Real.\n\
    Set item 1 of A to 1.0.\nSet item 2 of A to 2.0.\nSet item 3 of A to 3.0.\nSet item 4 of A to 4.0.\n\
    Let B be a 2 by 2 tensor of Real.\n\
    Set item 1 of B to 5.0.\nSet item 2 of B to 6.0.\nSet item 3 of B to 7.0.\nSet item 4 of B to 8.0.\n";

/// The positive control: a program every statement of which the CUDA forward lowers still compiles. Without it
/// every refusal below could pass by refusing everything.
#[test]
fn a_program_the_cuda_forward_fully_lowers_still_compiles() {
    let src = format!("{DECLS}Let C be A matmul B.\nLet D be C plus A.\nShow item 1 of D.\n");
    let cu = compile_to_cuda_forward(&src).unwrap_or_else(|e| panic!("must lower: {e}"));
    assert!(cu.contains("_matmul"), "the matmul stage must be emitted");
}

/// `sigmoid` is a tensor op the front end lifts and the CUDA forward has no kernel for. Dropping it made `Y` an
/// input with a deterministic fill and `Z = Y matmul B` a product of numbers the program never computed.
#[test]
fn an_op_the_front_end_lifts_but_cuda_cannot_lower_is_refused_by_name() {
    let src =
        format!("{DECLS}Let Y be sigmoid of A.\nLet Z be Y matmul B.\nShow item 1 of Z.\n");
    let e = refusal(&src);
    assert!(
        e.contains("Sigmoid") && e.contains("`Y`"),
        "the refusal must name the op kind and its binder: {e}"
    );
}

/// A hand-written loop that writes a tensor is no op any recognizer lifts. The walk reports it, so the CUDA
/// forward refuses it, naming the loop.
#[test]
fn a_hand_written_loop_is_refused_by_name() {
    let src = format!(
        "{DECLS}Let C be A matmul B.\n\
         Let mutable i be 1.\n\
         While i is at most 4:\n    Set item i of C to 0.0.\n    Set i to i + 1.\n\
         Let D be C plus A.\nShow item 1 of D.\n"
    );
    let e = refusal(&src);
    assert!(
        e.contains("While i"),
        "the refusal must name the unlifted loop: {e}"
    );
}

/// A literal write made after an op has read the tensor is not part of the tensor's starting value. Folding it
/// into the fill would move it before that op: `C = A·B` would see the new `A[1]`.
#[test]
fn a_literal_write_after_an_op_read_the_tensor_is_refused() {
    let src = format!(
        "{DECLS}Let C be A matmul B.\nSet item 1 of A to 9.0.\nLet D be C plus A.\nShow item 1 of D.\n"
    );
    let e = refusal(&src);
    assert!(
        e.contains("`A`") && e.contains("after an op has used the tensor"),
        "the refusal must name the tensor and why the write cannot be folded: {e}"
    );
}

/// A write whose value is not a literal is data no stage computes and no fill can hold.
#[test]
fn a_write_whose_value_is_not_a_literal_is_refused() {
    let src = format!(
        "{DECLS}Let v be 3.\nSet item 1 of A to v.\nLet C be A matmul B.\nShow item 1 of C.\n"
    );
    let e = refusal(&src);
    assert!(
        e.contains("`A`") && e.contains("not a literal"),
        "the refusal must name the tensor and the non-literal write: {e}"
    );
}

/// The refusal is about the statement, not the order of discovery: a program whose FIRST op is unsupported is
/// refused the same way as one whose last op is.
#[test]
fn an_unsupported_first_op_is_refused_like_a_later_one() {
    let src = format!("{DECLS}Let Y be sigmoid of A.\nLet C be A matmul B.\nShow item 1 of C.\n");
    let e = refusal(&src);
    assert!(
        e.contains("Sigmoid") && e.contains("`Y`"),
        "the refusal must name the op kind and its binder: {e}"
    );
}
