//! ★ THE C EXACT-INTEGER-POWER LOCK — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! LOGOS `Int` is arbitrary precision: it promotes to `BigInt` on overflow and downsizes back when a
//! value fits `i64` again, on EVERY tier. Integer exponentiation (`a ** b` and `pow(a, b)`) is the
//! sharpest overflow source in the language — `2 ** 100` is 31 digits — so it is the last integer
//! operation the AOT-C backend had to make exact. It was the sole surviving `("bigint", "c")` gap in
//! `type_tier_coverage_lock.rs`.
//!
//! Two emission sites were wrong before this lock:
//!   * the `pow(...)` CALL form computed exponentiation-by-squaring in wrapping `uint64_t`, so
//!     `pow(2, 100)` wrapped to `0` (2^100 ≡ 0 mod 2^64);
//!   * the `**` OPERATOR form used `llround(pow((double)a, (double)b))`, which is float-LOSSY even
//!     inside i64 range (`3 ** 34` needs 54 bits, more than an f64 mantissa).
//! Both now route through the runtime's exact tagged integer `LInt` (`lint_pow` → `lbig_pow` +
//! `lint_norm`), so a fitting result stays a fast small and an overflow promotes — byte-for-byte the
//! tree-walker/VM.
//!
//! Discipline (mirrors `c_temporal_lock`): the tree-walker is the ORACLE; a `Supported` program MUST
//! compile to C and run byte-for-byte identical to it. RED tests are the spec — if a case diverges,
//! fix `codegen_c`, NEVER weaken this file. The execution half is gated on `gcc` and SKIPS when absent.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, tw_outcome, vm_outcome};

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Compile a program to C, build with gcc, run, and return trimmed stdout. `None` if the C fails to
/// compile or the binary exits non-zero (a genuine LOGOS runtime abort — e.g. a negative Int exponent).
fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_cbigpow_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).ok()?;
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .ok()?;
    if !build.status.success() {
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

/// Every program that must run on C byte-for-byte identical to the tree-walker oracle. The exponent is
/// always non-negative (a negative Int exponent is a loud error — covered separately). Both the `**`
/// operator and the `pow(...)` call surface appear, because they are two distinct emission paths.
const CORPUS: &[(&str, &str)] = &[
    // ---- the headline gap: exact overflow past i64, both surfaces ----
    ("call_2_pow_100", "## Main\nShow pow(2, 100).\n"),
    ("op_2_pow_100", "## Main\nShow 2 ** 100.\n"),
    ("call_10_pow_30", "## Main\nShow pow(10, 30).\n"),
    ("op_2_pow_64", "## Main\nShow 2 ** 64.\n"),
    ("op_2_pow_63", "## Main\nShow 2 ** 63.\n"), // i64::MAX + 1 — overflows by exactly one
    // ---- fits i64 but NOT f64: catches the lossy llround path even in range ----
    ("call_3_pow_34", "## Main\nShow pow(3, 34).\n"),
    ("op_3_pow_34", "## Main\nShow 3 ** 34.\n"),
    // ---- small results must DOWNSIZE and print like a plain int (sticky fast path) ----
    ("call_2_pow_10", "## Main\nShow pow(2, 10).\n"),
    ("op_2_pow_10", "## Main\nShow 2 ** 10.\n"),
    ("call_5_pow_0", "## Main\nShow pow(5, 0).\n"),
    ("op_2_pow_0", "## Main\nShow 2 ** 0.\n"),
    ("op_9999999999_pow_1", "## Main\nShow 9999999999 ** 1.\n"),
    // ---- signed base: odd exponent keeps the sign, even exponent clears it ----
    ("neg_base_odd", "## Main\nLet b be 0 - 3.\nShow b ** 3.\n"), // -27
    (
        "neg_base_even",
        "## Main\nLet b be 0 - 2.\nShow b ** 100.\n",
    ), // +2^100
    // ---- variable operands that overflow at RUNTIME: the acid test that promotion is dynamic,
    //      not a constant-fold trick (a wrapping-uint64 path would silently give the wrong answer) ----
    (
        "var_7_pow_30",
        "## Main\nLet b be 7.\nLet e be 30.\nShow b ** e.\n",
    ),
    (
        "var_pow_call",
        "## Main\nLet b be 12.\nLet e be 20.\nShow pow(b, e).\n",
    ),
    // ---- the power result flows onward: bound to a Let, then arithmetic / comparison / Show ----
    ("let_then_add", "## Main\nLet x be 2 ** 100.\nShow x + 1.\n"),
    (
        "let_then_mul",
        "## Main\nLet x be pow(2, 100).\nShow x * 2.\n",
    ),
    ("let_then_sub", "## Main\nLet x be 10 ** 30.\nShow x - 1.\n"),
    ("power_in_sum", "## Main\nShow (2 ** 100) + (2 ** 100).\n"),
    // ---- exact division / modulo of a power (stays exact through the whole chain) ----
    ("power_div", "## Main\nShow (10 ** 30) / 1000000000.\n"),
    ("power_mod", "## Main\nShow (2 ** 100) mod 1000000.\n"),
    // ---- a function whose Int return overflows must WIDEN to carry the exact bignum out ----
    (
        "fn_ret_op_power",
        "## To big () -> Int:\n    Return 2 ** 100.\n## Main\nShow big().\n",
    ),
    (
        "fn_ret_call_power",
        "## To big () -> Int:\n    Return pow(2, 100).\n## Main\nShow big().\n",
    ),
    (
        "fn_ret_var_power",
        "## To pw (e: Int) -> Int:\n    Return 2 ** e.\n## Main\nShow pw(100).\n",
    ),
    // ---- a mutable local REASSIGNED an overflowing power must be promoted at its declaration ----
    (
        "mut_pow_reassign",
        "## Main\nLet mutable x be 2.\nSet x to x ** 100.\nShow x.\n",
    ),
];

/// The oracle (tree-walker) must run every corpus program cleanly — otherwise the target is undefined.
/// The VM must agree with it too (a second independent oracle), so the byte-target is unambiguous.
#[test]
fn oracle_agrees_on_every_power_program() {
    for (label, src) in CORPUS {
        let tw = tw_outcome(src);
        assert!(
            tw.error.is_none(),
            "[{label}] the tree-walker ORACLE must run cleanly, but errored: {}\nSource:\n{src}",
            tw.error.as_deref().unwrap_or("")
        );
        let vm = vm_outcome(src);
        assert!(
            vm.error.is_none() && vm.output.trim() == tw.output.trim(),
            "[{label}] VM must agree with the tree-walker oracle.\n tw: {:?}\n vm: {:?}\nSource:\n{src}",
            tw.output.trim(),
            vm
        );
    }
}

/// A few headline values pinned to their exact literal, so a silent ORACLE regression (both tiers
/// drifting together) is still caught — the oracle-agreement test alone can't see that.
#[test]
fn oracle_values_are_the_known_exact_integers() {
    let cases = [
        (
            "## Main\nShow pow(2, 100).\n",
            "1267650600228229401496703205376",
        ),
        (
            "## Main\nShow 2 ** 100.\n",
            "1267650600228229401496703205376",
        ),
        ("## Main\nShow 3 ** 34.\n", "16677181699666569"),
        ("## Main\nShow 2 ** 63.\n", "9223372036854775808"),
        (
            "## Main\nShow 10 ** 30.\n",
            "1000000000000000000000000000000",
        ),
        ("## Main\nShow 2 ** 10.\n", "1024"),
        ("## Main\nLet b be 0 - 3.\nShow b ** 3.\n", "-27"),
        (
            "## Main\nLet b be 7.\nLet e be 30.\nShow b ** e.\n",
            "22539340290692258087863249",
        ),
    ];
    for (src, want) in cases {
        assert_eq!(
            tw_outcome(src).output.trim(),
            want,
            "oracle drifted for:\n{src}"
        );
    }
}

/// THE BICONDITIONAL: every corpus program compiles to C and runs byte-for-byte identical to the
/// oracle. This is the gap-closing proof — `pow(2, 100)` on C must now print the exact 31-digit value.
#[test]
fn c_backend_matches_the_oracle_on_every_power_program() {
    if !have_gcc() {
        eprintln!("SKIP c_backend_matches_the_oracle_on_every_power_program: gcc unavailable");
        return;
    }
    let mut failures = Vec::new();
    for (label, src) in CORPUS {
        let want = tw_outcome(src).output.trim().to_string();
        match c_output(src, label) {
            Some(got) if got == want => {}
            other => failures.push(format!(
                "[{label}] want {want:?}, C produced {other:?}\nSource:\n{src}"
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "C EXACT-INTEGER-POWER broke ({} case(s)):\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

/// A negative Int exponent has no exact integer value — the tree-walker rejects it loudly. The AOT-C
/// binary MUST also refuse (abort, non-zero exit — `c_output` returns `None`), never silently return 1
/// or a wrapped value. Correctness at the error boundary is as load-bearing as at a success.
#[test]
fn negative_int_exponent_is_a_loud_error_on_c() {
    let src = "## Main\nShow 2 ** -1.\n";
    assert!(
        tw_outcome(src).error.is_some(),
        "the tree-walker oracle must reject a negative Int exponent"
    );
    if !have_gcc() {
        eprintln!("SKIP negative_int_exponent_is_a_loud_error_on_c: gcc unavailable");
        return;
    }
    assert_eq!(
        c_output(src, "neg_exp"),
        None,
        "a negative Int exponent must abort the C binary loudly, not print a value"
    );
}
