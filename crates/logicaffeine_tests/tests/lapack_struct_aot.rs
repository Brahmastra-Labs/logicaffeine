//! The real (Float) struct-returning LAPACK functions compile to Rust and match the interpreter —
//! closing the pre-existing AOT gaps that blocked prelude functions returning a result-record
//! (LuResult, QrResult, SymEig, EigVals, …): (1) the record declarations, bare in the module's
//! leading region, were stripped by the prelude splice → moved under `## Definition`; (2) a
//! `Seq of <T>` field's element parses as `FieldType::Named`, which emitted a bare undefined type →
//! `codegen_field_type` now maps primitives/value-tower like `map_type_to_rust`; (3) `Set x to
//! sqrt(..)`/`abs(..)`/`pow(..)` emitted a BARE call → the `Stmt::Set` value path now routes through
//! the same math-builtin lowering as `Let`/`Return`. These lock all three closed.
mod common;
use common::assert_compiled_equals_interpreted;

#[test]
fn getrf_getrs_lu_result_in_aot() {
    assert_compiled_equals_interpreted(
        "## Main\nLet a be [4.0, 3.0, 6.0, 3.0].\nLet f be getrf(2, a).\nShow getrs(2, f, [1.0, 2.0]).\n",
    );
}

#[test]
fn geqrf_qr_result_in_aot() {
    assert_compiled_equals_interpreted(
        "## Main\nLet a be [1.0, 2.0, 3.0, 4.0].\nLet f be geqrf(2, 2, a).\nShow f's r.\n",
    );
}

#[test]
fn geev_eigvals_in_aot() {
    assert_compiled_equals_interpreted(
        "## Main\nLet a be [2.0, 1.0, 0.0, 3.0].\nLet e be geev(2, a).\nShow e's wr.\n",
    );
}

#[test]
fn syev_symeig_in_aot() {
    assert_compiled_equals_interpreted(
        "## Main\nLet a be [2.0, 0.0, 0.0, 3.0].\nLet e be syev(2, a).\nShow e's vals.\n",
    );
}
