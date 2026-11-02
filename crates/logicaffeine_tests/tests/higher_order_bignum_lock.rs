//! ★ HIGHER-ORDER BIGNUM LOCK — arbitrary-precision integer promotion must flow through closures / higher-order
//! functions on EVERY backend, not just the VM.
//!
//! LOGOS integers are EXACT. A closure `(n) -> n * 2` applied to `i64::MAX` overflows, so the VM promotes to a
//! bignum (`18446744073709551614`). But the shared promotion analyses — `bigint_promote` (Rust) and
//! `int_exceeds_i64` / `codegen_closure` (AST-C) — are first-order and intra-procedural: they type a closure's
//! integer params/return as raw i64, never crossing the closure boundary, so the exact result gets WRAPPED
//! (AST-C → `-2`) or the checked-i64 runtime PANICS (Rust). This lock pins the fix: on both AOT backends the
//! closure body's arithmetic is exact and the bignum propagates through the higher-order call == the VM oracle.
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_program, program_outcome, run_logos_source,
};

/// Programs whose exact result only appears when bignum promotion reaches through the closure boundary.
const HIGHER_ORDER_BIGNUM: &[(&str, &str, &str)] = &[
    // A closure doubling `i64::MAX` — the product overflows i64; VM = 18446744073709551614.
    (
        "closure_mul_overflow",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 9223372036854775807).\n",
        "18446744073709551614",
    ),
    // A closure squaring a large value — n*n overflows; VM = 100000000000000000000.
    (
        "closure_square_overflow",
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * n, 10000000000).\n",
        "100000000000000000000",
    ),
];

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn gcc_run(c: &str, label: &str) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("hob_{}_{}", std::process::id(), label));
    let _ = std::fs::create_dir_all(&dir);
    let cf = dir.join("p.c");
    let bin = dir.join("p");
    std::fs::write(&cf, c).map_err(|e| e.to_string())?;
    let b = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cf)
        .arg("-lm")
        .output()
        .map_err(|e| e.to_string())?;
    if !b.status.success() {
        return Err(format!(
            "gcc failed:\n{}",
            String::from_utf8_lossy(&b.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let r = std::process::Command::new(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(String::from_utf8_lossy(&r.stdout).trim().to_string())
}

#[test]
fn the_vm_oracle_promotes_bignum_through_closures() {
    // Sanity: the oracle itself is exact (guards the expected outputs).
    for (label, src, want) in HIGHER_ORDER_BIGNUM {
        let vm =
            program_outcome(&compile_to_program(src).unwrap_or_else(|e| panic!("[{label}] {e:?}")));
        assert_eq!(vm.output.trim(), *want, "[{label}] VM oracle mismatch");
    }
}

/// ★ FIXED (2026-09-01) via the SELECTIVE higher-order-overflow analysis (`is_exact_hof` in `codegen_c/mod.rs`):
/// a SCALAR higher-order fn (a `fn`-param but NO seq-param, like `apply(f, x)`) has its integer params/return +
/// closure param exactified to `LInt`, so an overflowing closure promotes == the VM. A seq-HOF (`map`/`fold`/
/// `reduceSeq`) is EXCLUDED — exactifying it breaks passing an i64 seq element to an `LInt` closure param. The
/// naive "exact everywhere" version regressed `reduceSeq` (TDD caught it); this excludes seq-HOFs and stays green
/// on both (`c_prelude_probe::c_backend_closures_compile_and_run`).
#[test]
fn ast_c_promotes_bignum_through_closures() {
    if !have_gcc() {
        eprintln!("SKIP: gcc absent");
        return;
    }
    for (label, src, want) in HIGHER_ORDER_BIGNUM {
        let c = compile_to_c(src).unwrap_or_else(|e| panic!("[{label}] compile_to_c: {e:?}"));
        let got = gcc_run(&c, label).unwrap_or_else(|e| panic!("[{label}] AST-C run: {e}"));
        assert_eq!(
            got, *want,
            "[{label}] AST-C != VM — arbitrary-precision promotion must reach through the closure boundary (it \
             currently WRAPS because the closure's integers are typed raw i64)"
        );
    }
}

/// ★ FIXED (2026-09-01) — the Rust MIRROR of the AST-C selective analysis, identical `is_exact_hof` rule (a
/// scalar HOF: `Int`/`Nat` return + a `fn`-typed param + NO seq param). `codegen/bigint_promote::exact_hof_fns`
/// classifies them; `codegen/program.rs` folds them into `bigint_fns` (return `-> LogosInt`), exactifies the
/// `fn`-param + integer params (`impl Fn(LogosInt) -> LogosInt`, `i64` → `LogosInt`), and threads the set to
/// `codegen/expr.rs`, whose `Expr::Closure` arm types the passed closure's integer params `LogosInt` (exact,
/// tolerant body) and whose call emit promotes integer arguments to `LogosInt`. So `(n) -> n * 2` on `i64::MAX`
/// routes through `logos_mul_exact` and promotes to a bignum == the VM, instead of the raw `i64` that panicked.
/// Seq-HOFs are EXCLUDED (`map`/`fold`/`reduceSeq` closures stay `i64`), so there is no regression.
#[test]
fn rust_promotes_bignum_through_closures() {
    for (label, src, want) in HIGHER_ORDER_BIGNUM {
        let got = run_logos_source(src).unwrap_or_else(|e| {
            panic!("[{label}] Rust run failed (likely the checked-i64 overflow PANIC): {e}")
        });
        assert_eq!(
            got.trim(), *want,
            "[{label}] Rust != VM — arbitrary-precision promotion must reach through the closure boundary (the \
             closure is typed `impl Fn(i64)->i64`, so the checked-i64 runtime panics on overflow)"
        );
    }
}
