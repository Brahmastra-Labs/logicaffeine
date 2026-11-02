//! ★ THE C EXACT NUMERIC TOWER LOCK — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! Phase 2 of the ALL-TYPES-ALL-TIERS campaign: the exact numeric tower above `Int`/`BigInt` —
//! `Rational` (p/q), `Decimal` (exact base-10), `Complex` (Gaussian, with rational parts), and
//! `Modular` (ℤ/nℤ) — must run on the AOT-C backend byte-for-byte identical to the tree-walker/VM,
//! exactly as it already does on the tree-walker, VM, AOT-native, and WASM-linked tiers. The C
//! runtime already carries `LRat`/`LDec`/`LComplex`/`LMod` (`codegen_c/runtime.rs`); this lock is the
//! proof that codegen EMITS them correctly for the whole surface.
//!
//! `Rational` and `Decimal` share one C representation — `LDec` (`{int is_rat; LBig a,b; int scale;}`):
//! `is_rat` selects an exact p/q rational, else a scale-tracked base-10 decimal. Decimal ÷ Decimal
//! widens to a Rational (never a float). `Complex` carries `LRat` real/imag parts, so `(3+4i)/(1+2i)`
//! is the exact `11/5-2/5i`. `Modular` reduces on construction and wraps in the ring.
//!
//! Discipline (mirrors `c_bigint_power_lock`): the tree-walker is the ORACLE; a corpus program MUST
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

/// Compile a program to C, build with gcc, run, return trimmed stdout. `None` on any failure (a
/// compile error or a nonzero exit) — a `Supported` program must do both.
fn c_output(src: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_cnumtower_{}_{}_{}",
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

/// The full exact-numeric-tower corpus. Every program must run on C byte-for-byte identical to the
/// tree-walker oracle. Drawn from `e2e_codegen_{rational,decimal,complex,modular}.rs` (which only
/// proved TW/VM/AOT-Rust) plus robustness cases.
const CORPUS: &[(&str, &str)] = &[
    // ---------------- Rational (p/q) — the `: Rational` annotation makes `/` exact ----------------
    ("rat_half", "## Main\nLet x: Rational be 7 / 2.\nShow x.\n"),
    ("rat_third", "## Main\nLet x: Rational be 1 / 3.\nShow x.\n"),
    (
        "rat_downsize",
        "## Main\nLet x: Rational be 6 / 2.\nShow x.\n",
    ), // 3 — downsizes to an integer
    ("rat_whole", "## Main\nLet x: Rational be 5.\nShow x.\n"),
    (
        "rat_add",
        "## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.\n",
    ), // 1/2
    (
        "rat_mixed_add",
        "## Main\nLet r: Rational be 1 / 2.\nLet s be r + 3.\nShow s.\n",
    ), // 7/2
    (
        "rat_int_plus_rat",
        "## Main\nLet x: Rational be 3 + 1 / 2.\nShow x.\n",
    ), // 7/2
    (
        "rat_mul",
        "## Main\nLet r: Rational be 1 / 2.\nLet a be r * 4.\nShow a.\n",
    ), // 2
    (
        "rat_div",
        "## Main\nLet r: Rational be 1 / 2.\nLet q be r / 2.\nShow q.\n",
    ), // 1/4
    (
        "rat_int_over_rat",
        "## Main\nLet r: Rational be 1 / 2.\nLet q be 3 / r.\nShow q.\n",
    ), // 6
    (
        "rat_sub",
        "## Main\nLet r: Rational be 3 / 4.\nLet d be r - 1 / 4.\nShow d.\n",
    ), // 1/2
    (
        "rat_floor",
        "## Main\nLet r: Rational be 7 / 2.\nShow floor(r).\n",
    ), // 3
    (
        "rat_ceil",
        "## Main\nLet r: Rational be 7 / 2.\nShow ceil(r).\n",
    ), // 4
    (
        "rat_round",
        "## Main\nLet r: Rational be 7 / 2.\nShow round(r).\n",
    ), // 4
    (
        "rat_floor_neg",
        "## Main\nLet r: Rational be -7 / 2.\nShow floor(r).\n",
    ), // -4
    (
        "rat_ceil_neg",
        "## Main\nLet r: Rational be -7 / 2.\nShow ceil(r).\n",
    ), // -3
    (
        "rat_abs",
        "## Main\nLet r: Rational be -7 / 2.\nShow abs(r).\n",
    ), // 7/2
    // ---------------- Decimal (exact base-10) — scale tracked, ÷ widens to Rational ---------------
    (
        "dec_mul",
        "## Main\nLet price be decimal(\"19.99\").\nLet total be price * 3.\nShow total.\n",
    ), // 59.97
    (
        "dec_add_no_drift",
        "## Main\nLet a be decimal(\"0.1\").\nLet b be decimal(\"0.2\").\nShow a + b.\n",
    ), // 0.3
    (
        "dec_scale",
        "## Main\nLet x be decimal(\"19.99\").\nShow x + decimal(\"0.01\").\n",
    ), // 20.00
    (
        "dec_sub",
        "## Main\nLet a be decimal(\"5.50\").\nShow a - decimal(\"0.50\").\n",
    ), // 5.00
    (
        "dec_div_widens",
        "## Main\nLet a be decimal(\"1\").\nLet b be decimal(\"8\").\nShow a / b.\n",
    ), // 1/8
    (
        "dec_cmp",
        "## Main\nLet price be decimal(\"19.99\").\nShow price > decimal(\"10\").\n",
    ), // true
    // ---------------- Complex (Gaussian, rational parts) ------------------------------------------
    (
        "cplx_i_squared",
        "## Main\nLet i be complex(0, 1).\nShow i * i.\n",
    ), // -1
    (
        "cplx_conj_real",
        "## Main\nLet a be complex(1, 1).\nLet b be complex(1, -1).\nShow a * b.\n",
    ), // 2
    (
        "cplx_add",
        "## Main\nLet a be complex(2, 3).\nLet b be complex(1, -1).\nShow a + b.\n",
    ), // 3+2i
    (
        "cplx_sub",
        "## Main\nLet a be complex(5, 5).\nLet b be complex(2, 3).\nShow a - b.\n",
    ), // 3+2i
    (
        "cplx_div_exact",
        "## Main\nLet a be complex(3, 4).\nLet b be complex(1, 2).\nShow a / b.\n",
    ), // 11/5-2/5i
    // ---------------- Modular (ℤ/nℤ) — reduce on construction, ring + field + pow -----------------
    ("mod_reduce", "## Main\nLet a be modular(10, 7).\nShow a.\n"), // 3 (mod 7)
    (
        "mod_mul",
        "## Main\nLet a be modular(3, 7).\nLet b be modular(5, 7).\nShow a * b.\n",
    ), // 1 (mod 7)
    (
        "mod_pow",
        "## Main\nLet g be modular(3, 5).\nShow pow(g, 4).\n",
    ), // 1 (mod 5)
    (
        "mod_div_inverse",
        "## Main\nLet a be modular(1, 7).\nLet b be modular(3, 7).\nShow a / b.\n",
    ), // 5 (mod 7)
    (
        "mod_add_direct",
        "## Main\nShow modular(5, 7) + modular(4, 7).\n",
    ), // 2 (mod 7)
    (
        "mod_sub_direct",
        "## Main\nShow modular(2, 7) - modular(5, 7).\n",
    ), // 4 (mod 7)
];

/// The oracle (tree-walker) runs every corpus program cleanly, and the VM agrees — so the byte-target
/// is unambiguous.
#[test]
fn oracle_agrees_on_every_numeric_tower_program() {
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

/// Headline values pinned to their exact literal, so a silent ORACLE regression (both tiers drifting
/// together) is still caught.
#[test]
fn oracle_values_are_the_known_exact_numbers() {
    let cases = [
        ("## Main\nLet x: Rational be 7 / 2.\nShow x.\n", "7/2"),
        (
            "## Main\nLet x: Rational be 1 / 3 + 1 / 6.\nShow x.\n",
            "1/2",
        ),
        (
            "## Main\nLet a be decimal(\"0.1\").\nLet b be decimal(\"0.2\").\nShow a + b.\n",
            "0.3",
        ),
        (
            "## Main\nLet a be decimal(\"1\").\nLet b be decimal(\"8\").\nShow a / b.\n",
            "1/8",
        ),
        ("## Main\nLet i be complex(0, 1).\nShow i * i.\n", "-1"),
        (
            "## Main\nLet a be complex(3, 4).\nLet b be complex(1, 2).\nShow a / b.\n",
            "11/5-2/5i",
        ),
        ("## Main\nLet a be modular(10, 7).\nShow a.\n", "3 (mod 7)"),
        (
            "## Main\nLet g be modular(3, 5).\nShow pow(g, 4).\n",
            "1 (mod 5)",
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
/// oracle. This is the gap-closing proof for the exact numeric tower on the AOT-C backend.
#[test]
fn c_backend_matches_the_oracle_on_every_numeric_tower_program() {
    if !have_gcc() {
        eprintln!(
            "SKIP c_backend_matches_the_oracle_on_every_numeric_tower_program: gcc unavailable"
        );
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
        "C EXACT NUMERIC TOWER broke ({} case(s)):\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}
