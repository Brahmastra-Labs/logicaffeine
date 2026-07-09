//! E2E Codegen Tests: dimensioned physical `Quantity` (the AOT compile-to-Rust tier).
//!
//! The base type, tree-walker, and VM already carry `Quantity` exactly. These tests prove the
//! SAME exact dimensional arithmetic on the compiled-to-Rust path: `quantity(2, "inch")` becomes
//! a `LogosQuantity`, conversions are lossless (the golden `2 in + 5 cm in feet = 42/127 ft`),
//! `+ −` stay same-dimension, `× ÷` combine dimensions, and a quantity scales by an integer.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_compiled_equals_interpreted, assert_exact_output, assert_interpreter_output};

// ---- TYPE-SYSTEM CERTAINTY: a dimensioned `Quantity` must thread through a function param + return
//      and an `If` comparison, not just `Show` — on every tier. (This caught the same codegen
//      type-name-mapping gap the temporal types had.) ----

#[cfg(not(target_arch = "wasm32"))]
const QUANTITY_IN_CONTEXT: &str = "## To doubled (q: Quantity) -> Quantity:\n\
\x20   Return q + q.\n\
## Main\n\
Let d be 5 meters.\n\
Show doubled(d) in feet.\n\
If d is greater than 1 meter:\n\
\x20   Show \"long\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_in_program_contexts_on_interpreter_vm_and_treewalker() {
    // doubled(5 m) = 10 m = 12500/381 ft exactly; 5 m > 1 m.
    assert_interpreter_output(QUANTITY_IN_CONTEXT, "12500/381 ft\nlong");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_in_program_contexts_on_aot() {
    common::assert_output_lines(QUANTITY_IN_CONTEXT, &["12500/381 ft", "long"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_in_program_contexts_all_tiers_agree() {
    assert_compiled_equals_interpreted(QUANTITY_IN_CONTEXT);
}

// ---- TIER CONSISTENCY: the interpreter rejects a dimension mismatch at ANALYSIS time too — before
//      any output is produced — matching the AOT compiler (not a mid-execution failure). ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn dimension_mismatch_rejected_before_execution_on_interpreter() {
    let r = common::run_interpreter("## Main\nShow \"starting\".\nShow 2 meters + 1 gram.");
    assert!(!r.success, "interpreter must reject a dimension mismatch");
    assert!(r.error.contains("different dimensions"), "expected a dimension error, got: {}", r.error);
    // The rejection is at analysis time, BEFORE the first `Show` runs — so nothing was printed.
    assert!(
        !r.output.contains("starting"),
        "dimension mismatch should be caught before any output; got: {:?}",
        r.output
    );
}

// ---- COMPILE-TIME DIMENSION SAFETY: adding incompatible dimensions is a *compile* error (caught by
//      the DimensionChecker analysis pass), not a runtime panic. A length + a mass cannot mean
//      anything, so the program must be rejected before it ever runs. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_dimension_mismatch_is_a_compile_error() {
    // `LOGOS compile error:` proves it was rejected at compile time (a runtime panic lacks that prefix).
    common::assert_compile_fails(
        "## Main\nShow 2 meters + 1 gram.",
        "LOGOS compile error",
    );
    common::assert_compile_fails("## Main\nShow 2 meters + 1 gram.", "different dimensions");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_dimension_mismatch_through_a_let_is_a_compile_error() {
    // The checker tracks a Let-bound quantity's dimension across statements.
    common::assert_compile_fails(
        "## Main\nLet d be 5 meters.\nLet m be 3 grams.\nShow d + m.",
        "different dimensions",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_same_dimension_still_compiles_and_runs() {
    // The pass must NOT reject coherent programs: same-dimension add, and cross-dimension MULTIPLY
    // (which legitimately combines dimensions) both compile and run.
    common::assert_output_lines(
        "## Main\nShow 2 meters + 3 meters.\nShow 2 meters * 3 meters.",
        &["5 m", "6 L^2"],
    );
}

// ---- FULL SUPPORT: the dimension is part of the TYPE (`Quantity of Length`), so a typed parameter
//      and a typed return are dimension-checked at COMPILE time — no runtime hole. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn typed_quantity_param_dimension_is_checked_at_compile_time() {
    // `q` is declared a Length; adding a Mass inside the body is a compile error.
    common::assert_compile_fails(
        "## To stretch (q: Quantity of Length) -> Quantity of Length:\n\
\x20   Return q + 1 gram.\n\
## Main\nShow stretch(5 meters).",
        "different dimensions",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn typed_quantity_return_dimension_flows_to_call_sites() {
    // `area_of` is declared to return an Area; adding that Area to a Length at the CALL site is a
    // compile error — the return dimension flows out of the function.
    common::assert_compile_fails(
        "## To area_of (w: Quantity of Length) and (h: Quantity of Length) -> Quantity of Area:\n\
\x20   Return w * h.\n\
## Main\nShow area_of(2 meters, 3 meters) + 1 meter.",
        "different dimensions",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn typed_quantity_param_coherent_body_compiles_and_runs() {
    // A `Quantity of Length` parameter used coherently compiles and runs.
    common::assert_output_lines(
        "## To doubled (q: Quantity of Length) -> Quantity of Length:\n\
\x20   Return q + q.\n\
## Main\nShow doubled(5 meters).",
        &["10 m"],
    );
}

// ---- COLLECTIONS: a `Seq of Quantity` must store dimensioned quantities and let them be read +
//      converted back — the generic type threads (`LogosSeq<LogosQuantity>`) on every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const SEQ_OF_QUANTITY: &str = "## Main\n\
Let qs be a new Seq of Quantity.\n\
Push 2 inches to qs.\n\
Push 5 centimeters to qs.\n\
Show item 1 of qs in centimeters.\n\
Show item 2 of qs in millimeters.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_quantity_on_interpreter_vm_and_treewalker() {
    // 2 in = 127/25 cm; 5 cm = 50 mm.
    assert_interpreter_output(SEQ_OF_QUANTITY, "127/25 cm\n50 mm");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_quantity_on_aot() {
    common::assert_output_lines(SEQ_OF_QUANTITY, &["127/25 cm", "50 mm"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn seq_of_quantity_all_tiers_agree() {
    assert_compiled_equals_interpreted(SEQ_OF_QUANTITY);
}

// ---- `Return <expr> in <unit>`: the conversion postfix must work in a Return position, not just
//      `Show`/`Let` (Return routed through `parse_comparison`, below the postfix in `parse_condition`). ----

#[cfg(not(target_arch = "wasm32"))]
const QUANTITY_RETURN_CONVERT: &str = "## To to_feet (q: Quantity) -> Quantity:\n\
\x20   Return q in feet.\n\
## Main\n\
Show to_feet(1 meter).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_return_convert_on_interpreter_vm_and_treewalker() {
    // 1 m in feet = 1250/381 ft exactly.
    assert_interpreter_output(QUANTITY_RETURN_CONVERT, "1250/381 ft");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_return_convert_on_aot() {
    common::assert_output_lines(QUANTITY_RETURN_CONVERT, &["1250/381 ft"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_return_convert_all_tiers_agree() {
    assert_compiled_equals_interpreted(QUANTITY_RETURN_CONVERT);
}

// ---- Same-dimension Quantity ORDERING (`>`, `<`, `>=`) must agree on every tier. (The interpreter's
//      ordering was missing — `compare.rs` only had equality — until the certainty test above.) ----

#[cfg(not(target_arch = "wasm32"))]
const QUANTITY_ORDERING: &str = "## Main\n\
If 2 meters is greater than 1 meter:\n\x20   Show \"a\".\n\
If 50 centimeters is less than 1 meter:\n\x20   Show \"b\".\n\
If 1 meter is at least 100 centimeters:\n\x20   Show \"c\".";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_ordering_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(QUANTITY_ORDERING, "a\nb\nc");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_ordering_on_aot() {
    common::assert_output_lines(QUANTITY_ORDERING, &["a", "b", "c"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn quantity_ordering_all_tiers_agree() {
    assert_compiled_equals_interpreted(QUANTITY_ORDERING);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_quantity_golden_two_inches_plus_five_cm_in_feet() {
    // THE GOLDEN, compiled to native: 2 inches + 5 centimeters, in feet, is EXACTLY 42/127 ft.
    assert_exact_output(
        "## Main\nLet a be quantity(2, \"inch\").\nLet b be quantity(5, \"centimeter\").\nShow convert(a + b, \"foot\").",
        "42/127 ft",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_quantity_subtraction_keeps_the_left_unit() {
    // 1 meter − 50 centimeters = 1/2 m (exact, shown in the left operand's unit).
    assert_exact_output(
        "## Main\nShow quantity(1, \"meter\") - quantity(50, \"centimeter\").",
        "1/2 m",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_quantity_scaling_preserves_the_unit() {
    // Scaling a quantity by an integer preserves its unit: 2 in × 3 = 6 in.
    assert_exact_output("## Main\nShow quantity(2, \"inch\") * 3.", "6 in");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_quantity_product_combines_dimensions() {
    // Length × Length = Area, shown in dimension form: 3 m × 4 m = 12 L^2.
    assert_exact_output(
        "## Main\nShow quantity(3, \"meter\") * quantity(4, \"meter\").",
        "12 L^2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_quantity_comparison_is_same_dimension() {
    // 2 meters > 1 meter compiles and runs (PartialOrd by magnitude within a dimension).
    assert_exact_output(
        "## Main\nShow quantity(2, \"meter\") > quantity(1, \"meter\").",
        "true",
    );
}

// ---- Natural unit-word syntax: `2 inches` desugars to `quantity(2, "inch")`. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_natural_quantity_literal() {
    // A number followed by a unit word is a quantity literal.
    assert_exact_output("## Main\nShow 2 inches.", "2 in");
    assert_exact_output("## Main\nShow 20 celsius.", "20 °C");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_natural_golden_two_inches_plus_five_cm_in_feet() {
    // The golden written almost entirely in natural syntax: `2 inches + 5 centimeters`, converted
    // to feet, is EXACTLY 42/127 ft.
    assert_exact_output(
        "## Main\nShow convert(2 inches + 5 centimeters, \"foot\").",
        "42/127 ft",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_natural_quantity_arithmetic() {
    // Natural literals compose with the full dimensional algebra.
    assert_exact_output("## Main\nShow 3 meters * 4 meters.", "12 L^2");
    assert_exact_output("## Main\nShow 2 inches * 3.", "6 in");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_natural_golden_in_feet() {
    // The user's exact golden, fully natural: `2 inches + 5 centimeters in feet` = 42/127 ft.
    assert_exact_output(
        "## Main\nShow 2 inches + 5 centimeters in feet.",
        "42/127 ft",
    );
    // And `in <unit>` is exact across other dimensions too.
    assert_exact_output("## Main\nShow 1 kilometer in meters.", "1000 m");
}
