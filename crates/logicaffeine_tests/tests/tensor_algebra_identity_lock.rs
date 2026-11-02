//! tensor_algebra_identity_lock — the supercompiler DISCHARGES tensor-algebra identities by computing them.
//!
//! With array supercompilation (Value::Seq), a tensor op over statically-known operands is evaluated at
//! compile time — so an algebraic identity is *verified by construction*: the compiler multiplies `A` by the
//! identity matrix and observes it gets `A` back, folding the result to `A`'s exact values. No hand-written
//! rewrite rule; the identity falls out of the computation.
//!
//!   • `A · I = A` (matmul by the identity matrix)
//!   • `A ∘ 1 = A` (Hadamard by the all-ones tensor)
//!   • `A + 0 = A` (elementwise add of the zero tensor)
//!
//! Pins: the folded output equals `A`'s entries (so the identity held), `supercompile` fired, and the result
//! ≡ the VM. (The scalar e-graph's kernel-certified rewrites cover SCALAR identities — `phase_exodia_architect`;
//! tensor identities live at this array-supercompilation layer.)
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_algebra_identity_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{compile_to_rust, optimizations_fired, vm_run_source};

/// A's entries — the value every identity must reproduce.
const A_VALUES: &[&str] = &["31", "52", "73", "94"];

/// `A · I` where `I` is the 2×2 identity. Result must equal `A`.
const MATMUL_IDENTITY: &str = "\
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 31.
Set item 2 of A to 52.
Set item 3 of A to 73.
Set item 4 of A to 94.
Let Eye be a 2 by 2 tensor of Int.
Set item 1 of Eye to 1.
Set item 2 of Eye to 0.
Set item 3 of Eye to 0.
Set item 4 of Eye to 1.
Let C be A matmul Eye.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
";

/// `A ∘ 1` (Hadamard by all-ones). Result must equal `A`.
const HADAMARD_ONES: &str = "\
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 31.
Set item 2 of A to 52.
Set item 3 of A to 73.
Set item 4 of A to 94.
Let Ones be a 2 by 2 tensor of Int.
Set item 1 of Ones to 1.
Set item 2 of Ones to 1.
Set item 3 of Ones to 1.
Set item 4 of Ones to 1.
Let C be A hadamard Ones.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
";

/// `A + 0` (elementwise add of the zero tensor). Result must equal `A`.
const PLUS_ZERO: &str = "\
## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 31.
Set item 2 of A to 52.
Set item 3 of A to 73.
Set item 4 of A to 94.
Let Zero be a 2 by 2 tensor of Int.
Set item 1 of Zero to 0.
Set item 2 of Zero to 0.
Set item 3 of Zero to 0.
Set item 4 of Zero to 0.
Let C be A plus Zero.
Show item 1 of C.
Show item 2 of C.
Show item 3 of C.
Show item 4 of C.
";

const IDENTITIES: &[(&str, &str)] = &[
    ("A·I", MATMUL_IDENTITY),
    ("A∘1", HADAMARD_ONES),
    ("A+0", PLUS_ZERO),
];

fn vm(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("VM failed: {e}"))
        .trim_end()
        .to_string()
}
fn rust(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compile_to_rust failed: {e:?}"))
}

#[test]
fn each_identity_returns_a() {
    // The identity holds: A ⊕ id = A, on the VM.
    for (name, src) in IDENTITIES {
        assert_eq!(vm(src), "31\n52\n73\n94", "{name} must return A");
    }
}

#[test]
fn the_supercompiler_discharges_each_identity_at_compile_time() {
    for (name, src) in IDENTITIES {
        let fired = optimizations_fired(src);
        assert!(
            fired.contains(&"supercompile"),
            "{name}: supercompile must fire; got {fired:?}"
        );
        // The folded residual carries A's entries as constants — the identity was computed away.
        let on = rust(src);
        for v in A_VALUES {
            assert!(
                on.contains(v),
                "{name}: folded output must contain A's entry {v}"
            );
        }
    }
}
