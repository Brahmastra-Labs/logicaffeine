//! Lock for `Opt::StrengthReducePow2` — the power-of-two modulo/division strength reduction.
//!
//! `x % 2^k` → `x & (2^k - 1)` and `x / 2^k` → `x >> k` when `x` is provably non-negative. rustc
//! CANNOT prove a runtime i64 loop counter is `>= 0`, so it keeps sign-corrected `%`/`/` (several
//! instructions that also block auto-vectorization); LOGOS proves non-negativity (a loop counter from
//! a `>= 0` start with a positive step) and hands LLVM the single `AND`/`SHR`. Measured to be the
//! dominant lever that lets classification loops BEAT C (`i % 128` → `i & 127`). Byte-identical (for
//! `x >= 0`, signed `%`/`/` agree with `&`/`>>`); gates off under `## No strengthreducepow2`.

mod common;

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// A non-negative loop counter `i` (from 0, +1 step) modulo/divided by a power of two. The loop bound
/// (100000) is large enough that comptime won't unroll it, so `i % 128` / `i / 64` survive to codegen
/// as real runtime ops with `i` proven `>= 0`.
const PROG: &str = "\
## Main
Let mutable acc be 0.
Let mutable i be 0.
While i is less than 100000:
    Let c be i % 128.
    Let d be i / 64.
    Set acc to acc + c + d.
    Set i to i + 1.
Show acc.
";

const PROMOTED_NONNEGATIVE: &str = r#"
## Main
Let x be 9223372036854775807 * 4.
Show x / 4.
Show x % 8.
"#;

/// A remainder keeps its dividend's sign (`-7 % 8` is `-7`), so `x % 8` of an `x` that goes negative is no proof
/// of non-negativity, and neither is a name bound to it: `r % 4`, `r / 2` and `(x % 8) % 4` keep their signed forms.
/// The loop runs long enough that nothing folds at compile time. Summed, the operations reach the backends through
/// their accumulator chains; compared, through the plain expression lowering.
const SIGNED_REMAINDER: &str = "\
## Main
Let mutable a be 0.
Let mutable b be 0.
Let mutable c be 0.
Let mutable x be 0 - 100000.
While x is less than 50000:
    Let r be x % 8.
    Set a to a + r % 4.
    Set b to b + r / 2.
    Set c to c + (x % 8) % 4.
    Set x to x + 1.
Show a.
Show b.
Show c.
";

const SIGNED_REMAINDER_COMPARED: &str = "\
## Main
Let mutable threes be 0.
Let mutable halves be 0.
Let mutable direct be 0.
Let mutable x be 0 - 100000.
While x is less than 50000:
    Let r be x % 8.
    If r % 4 is equal to 3:
        Set threes to threes + 1.
    If r / 2 is equal to 0:
        Set halves to halves + 1.
    If (x % 8) % 4 is equal to 3:
        Set direct to direct + 1.
    Set x to x + 1.
Show threes.
Show halves.
Show direct.
";

fn direct_c_output(src: &str) -> String {
    let c = compile_to_c(src).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_pow2_exact_")
        .tempdir()
        .expect("temporary C directory");
    let c_path = dir.path().join("main.c");
    let bin_path = dir.path().join("main");
    std::fs::write(&c_path, c).expect("write generated C");
    let build = common::build_c_release(&c_path, &bin_path).expect("C compiler starts");
    assert!(
        build.status.success(),
        "generated C builds:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin_path)
        .output()
        .expect("generated C starts");
    assert!(
        run.status.success(),
        "generated C exits cleanly:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn pow2_modulo_and_div_lower_to_mask_and_shift() {
    let rust = compile_to_rust(PROG).expect("PROG compiles");
    assert!(
        rust.contains("& 127"),
        "`i % 128` on a non-negative counter must lower to `& 127`; got:\n{rust}"
    );
    assert!(
        rust.contains(">> 6"),
        "`i / 64` on a non-negative counter must lower to `>> 6`; got:\n{rust}"
    );
    assert!(
        !rust.contains("% 128"),
        "the signed `% 128` must be gone (replaced by the mask); got:\n{rust}"
    );

    let c = compile_to_c(PROG).expect("PROG compiles to direct C");
    assert!(
        c.contains("& 127LL") && c.contains(">> 6"),
        "direct C silently lost Rust's mask/shift lowering:\n{c}"
    );
    assert!(
        !c.contains("logos_imod(i, 128LL)"),
        "direct C retained signed modulo in the optimized loop:\n{c}"
    );
}

#[test]
fn pow2_strength_reduce_gates_off() {
    let off = decorate_source(PROG, &["strengthreducepow2"]);
    let rust = compile_to_rust(&off).expect("disabled PROG compiles");
    assert!(
        rust.contains("% 128") && !rust.contains("& 127"),
        "`## No strengthreducepow2` must keep the signed `% 128` (no mask); got:\n{rust}"
    );
    let c = compile_to_c(&off).expect("disabled PROG compiles to direct C");
    let user_c = c
        .split("// ==== LOGOS user code")
        .nth(1)
        .expect("generated user-code marker");
    assert!(
        !user_c.contains("& 127LL") && !user_c.contains(">> 6"),
        "direct C ignored `## No strengthreducepow2`:\n{user_c}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the mask/shift must be BYTE-IDENTICAL to the VM oracle (real
/// signed `%`/`/`) AND to the AOT path with the opt disabled — the reduction is valid ONLY because
/// `i >= 0`, so this catches any misfire on a value that could be negative.
#[test]
fn pow2_strength_reduce_is_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["strengthreducepow2"]));
    assert_eq!(
        on, vm,
        "AOT mask/shift diverged from the VM oracle (non-negativity misproof?)"
    );
    assert_eq!(
        on, off,
        "StrengthReducePow2 ON changed output vs OFF — not behavior-preserving"
    );
}

#[test]
fn a_remainder_of_a_negative_dividend_is_not_proven_nonnegative() {
    for (src, reference) in [
        (SIGNED_REMAINDER, "-75000\n-75000\n-75000"),
        (SIGNED_REMAINDER_COMPARED, "12500\n37500\n12500"),
    ] {
        let vm = vm_output(src);
        assert_eq!(
            vm, reference,
            "the reference: a remainder takes its dividend's sign\n{src}"
        );
        assert_eq!(
            aot_output(src),
            vm,
            "AOT masked or shifted a remainder that can be negative\n{src}"
        );
        if common::require_tool(&common::c_compiler()) {
            assert_eq!(
                direct_c_output(src).trim(),
                vm,
                "direct C masked or shifted a remainder that can be negative\n{src}"
            );
        }
    }
}

#[test]
fn direct_c_exact_pow2_fallback_matches_vm_for_promoted_nonnegative_int() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    let c = compile_to_c(PROMOTED_NONNEGATIVE).expect("promoted program compiles");
    assert!(
        c.contains("logos_lint_div_pow2_nonnegative")
            && c.contains("logos_lint_mod_pow2_nonnegative"),
        "direct C did not consume the shared proof for an exact nonnegative Int:\n{c}"
    );
    assert_eq!(
        direct_c_output(PROMOTED_NONNEGATIVE).trim(),
        vm_output(PROMOTED_NONNEGATIVE),
        "direct C's promoted power-of-two fallback diverged from the exact VM"
    );
}
