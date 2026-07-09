//! Wave 7: the power operator `**` (right-associative, binds tighter than
//! `* / %`). Integer power is EXACT — promotes to BigInt on overflow (the
//! overflow ruling); float base uses `powf`; a negative integer exponent is
//! a loud error (an Int can't hold the fractional result).

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter, run_logos};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The bytecode VM (via `Op::Pow`) must agree byte-for-byte with the tree-walker
/// on `**` — output AND error text. Cells that error are as load-bearing as cells
/// that succeed.
fn assert_vm_matches_tw(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim_end(), tw.output.trim_end(), "VM/tw output diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}");
    assert_eq!(vm.error, tw.error, "VM/tw error diverged for:\n{src}");
}

#[test]
fn vm_matches_tw_on_power() {
    for src in [
        "## Main\nShow 2 ** 10.\n",
        "## Main\nShow 5 ** 0.\n",
        "## Main\nShow 2 * 3 ** 2.\n",
        "## Main\nShow 2 ** 3 ** 2.\n",
        "## Main\nShow 2 ** 100.\n",       // BigInt promotion on the VM
        "## Main\nShow 2.0 ** 0.5.\n",     // float powf
        "## Main\nLet b be 3.\nLet e be 4.\nShow b ** e.\n",
        "## Main\nShow 2 ** -1.\n",        // negative Int exponent — loud error, both tiers
    ] {
        assert_vm_matches_tw(src);
    }
}

#[test]
fn int_power_basic() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 2 ** 10.\n", "1024");
}

#[test]
fn power_of_zero_is_one() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 5 ** 0.\n", "1");
}

#[test]
fn power_binds_tighter_than_multiply() {
    // 2 * 3 ** 2 = 2 * 9 = 18, not (2*3)**2 = 36.
    assert_compiled_equals_interpreted_eq("## Main\nShow 2 * 3 ** 2.\n", "18");
}

#[test]
fn power_is_right_associative() {
    // 2 ** 3 ** 2 = 2 ** 9 = 512, not (2**3)**2 = 64.
    assert_compiled_equals_interpreted_eq("## Main\nShow 2 ** 3 ** 2.\n", "512");
}

#[test]
fn int_power_overflows_to_bigint_exactly() {
    // 2 ** 100 is far beyond i64 — exact arithmetic promotes.
    assert_compiled_equals_interpreted_eq(
        "## Main\nShow 2 ** 100.\n",
        "1267650600228229401496703205376",
    );
}

#[test]
fn float_power() {
    assert_compiled_equals_interpreted_eq("## Main\nShow 2.0 ** 0.5.\n", "1.4142135623730951");
}

#[test]
fn power_on_a_variable() {
    assert_compiled_equals_interpreted_eq(
        "## Main\nLet b be 3.\nLet e be 4.\nShow b ** e.\n",
        "81",
    );
}

#[test]
fn negative_int_exponent_is_loud() {
    let src = "## Main\nShow 2 ** -1.\n";
    let interp = run_interpreter(src);
    assert!(!interp.success, "interp must reject a negative Int exponent");
    let compiled = run_logos(src);
    assert!(!compiled.success, "compiled must reject a negative Int exponent: {}", compiled.stdout);
}
