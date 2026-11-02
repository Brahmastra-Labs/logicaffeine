//! File-level `## No <opt>` / `## Require <opt>` decorators must be honored through
//! the REAL `largo` compile path (`compile_project` → `compile_to_rust_with_registry_full`),
//! not only through the traced `compile_to_rust` API that the unit tests use.
//!
//! The bug this locks: the production path built the optimization config with a bare
//! `OptimizationConfig::from_env()` and never merged `parser.program_opt_flags()`, so
//! every file-level `## No`/`## Require` decorator was silently dropped for `largo build`
//! and `largo emit`. That made `## Require <opt>` — our whole answer to Ray Myers' wish
//! ("declare a dependence on an optimization so a missing one is an error") — a no-op in
//! practice, and made `## No <opt>` (needed to *prove* the optimizer via same-program
//! on/off) do nothing. A passing traced-path test certified a feature that didn't work
//! for real users. These exercise the real path so it can't regress.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::compile::{
    compile_project, compile_to_c, compile_to_isa, compile_to_wasm,
};
use logicaffeine_compile::ui_bridge::interpret_unsatisfiable_require;

/// Compile a single-file program through the real `largo`/`compile_project` path and
/// return the generated Rust (or the error string).
fn emit(src: &str) -> Result<String, String> {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, src).unwrap();
    compile_project(&main)
        .map(|o| o.rust_code)
        .map_err(|e| format!("{e:?}"))
}

/// A dense-integer-keyed `Map` read across its full range so the DenseMap optimizer can
/// prove the keys are dense and collapse the hashmap to a direct-addressed array (the
/// `collect` benchmark shape). This is the sentinel we toggle.
const DENSE_MAP_PROGRAM: &str = "## To native args () -> Seq of Text\n\
    ## To native parseInt (s: Text) -> Int\n\
    \n\
    ## Main\n\
    Let arguments be args().\n\
    Let n be parseInt(item 2 of arguments).\n\
    Let mutable m be a new Map of Int to Int with capacity n.\n\
    Let mutable i be 1.\n\
    While i is less than n + 1:\n\
    \x20   Set item i of m to i * 2.\n\
    \x20   Set i to i + 1.\n\
    Let mutable found be 0.\n\
    Set i to 1.\n\
    While i is less than n + 1:\n\
    \x20   If item i of m equals i * 2:\n\
    \x20       Set found to found + 1.\n\
    \x20   Set i to i + 1.\n\
    Show found.\n";

/// Baseline: by default the DenseMap collapse FIRES (proves the sentinel is meaningful).
#[test]
fn densemap_fires_by_default_through_largo() {
    let rust = emit(DENSE_MAP_PROGRAM).expect("dense-map program should compile");
    assert!(
        rust.contains("LogosDense"),
        "DenseMap should fire by default (direct-addressed array), but no dense map type \
         appears in the generated Rust:\n{rust}"
    );
}

/// A program-wide `## No densemap` (placed immediately before `## Main`, the documented
/// spot for a file-level decorator) must DISABLE the collapse through the real path — the
/// map stays a real hashmap. RED before the fix (the production path dropped the decorator
/// entirely, dense map still emitted).
#[test]
fn no_densemap_reverts_to_hashmap_through_largo() {
    let src = DENSE_MAP_PROGRAM.replace("## Main", "## No densemap\n## Main");
    let rust = emit(&src).expect("`## No densemap` program should compile");
    assert!(
        !rust.contains("LogosDense"),
        "`## No densemap` must disable the DenseMap collapse through the real largo path \
         (map should stay a hashmap), but a dense map type is still in the generated Rust:\n{rust}"
    );
}

/// `## Require memo` on a program where memoization CANNOT fire (a plain loop, no
/// recursion) must FAIL the compile. RED before the fix (Require was dropped → compiled OK).
#[test]
fn require_memo_errors_when_it_cannot_fire_through_largo() {
    let src = "## Require memo\n## Main\n\
        Let mutable s be 0.\n\
        Let mutable i be 1.\n\
        While i is at most 10:\n\
        \x20   Set s to s + i.\n\
        \x20   Set i to i + 1.\n\
        Show s.\n";
    let result = emit(src);
    assert!(
        result.is_err(),
        "`## Require memo` on a non-memoizable program must fail the compile through the \
         real largo path, but it compiled successfully:\n{}",
        result.unwrap_or_default()
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("memo") && err.to_lowercase().contains("require"),
        "the error should name the unfired `memo` requirement, got: {err}"
    );
}

/// The C backend (`largo emit c` → `compile_to_c`) must ALSO honor `## Require`. A demanded
/// optimization that cannot fire has to fail the compile there too, exactly like the Rust
/// path — not silently succeed. RED before the fix: `compile_to_c` built the optimization
/// config with a bare `OptimizationConfig::from_env()` and never merged the parser's
/// decorators, so `## Require memo` was dropped and the C program compiled with the
/// requirement quietly ignored (`largo emit c` is a real user command — the benchmark
/// backend). `memo` is a Rust-codegen construct the C backend does not emit, so the honest
/// answer for `## Require memo` under `emit c` is a loud error, never a silent pass.
#[test]
fn require_memo_errors_through_emit_c() {
    let src = "## Require memo\n## Main\n\
        Let mutable s be 0.\n\
        Let mutable i be 1.\n\
        While i is at most 10:\n\
        \x20   Set s to s + i.\n\
        \x20   Set i to i + 1.\n\
        Show s.\n";
    let result = compile_to_c(src).map_err(|e| format!("{e:?}"));
    assert!(
        result.is_err(),
        "`## Require memo` must fail the compile through `largo emit c` (compile_to_c), but \
         it compiled successfully:\n{}",
        result.unwrap_or_default()
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("memo") && err.to_lowercase().contains("require"),
        "the C-path error should name the unfired `memo` requirement, got: {err}"
    );
}

/// `## Require memo` on a genuinely memoizable program (naive recursive fib) must COMPILE —
/// the requirement is satisfied because memo fires. Guards against over-erroring.
#[test]
fn require_memo_succeeds_when_it_fires_through_largo() {
    let src = "## Require memo\n\
        ## To fib (n: Int) -> Int:\n\
        \x20   If n is less than 2:\n\
        \x20       Return n.\n\
        \x20   Return fib(n - 1) + fib(n - 2).\n\
        ## Main\n\
        Show fib(10).\n";
    let rust = emit(src).expect("`## Require memo` on recursive fib should compile (memo fires)");
    assert!(
        rust.contains("fib"),
        "the memoized fib should still be in the generated Rust:\n{rust}"
    );
}

// ---------------------------------------------------------------------------
// VM-tier uniformity: the raw VM/wasm/isa seam (`with_parsed_program`) used to
// build the run config but never install it, so `## No <vm-opt>` was ignored and
// `## Require` was unchecked. The seam now `set_active_config`s the merged config
// (`## No` reaches the VM opts) and the `_checked` variant enforces `## Require`.
// ---------------------------------------------------------------------------

/// A loop that divides and mods by a non-power-of-two constant, so `FastDiv` lowers `/`/`%` to
/// magic-reciprocal VM ops — the sentinel we toggle to prove `## No` reaches the VM tier.
const DIV_PROGRAM: &str = "## To native args () -> Seq of Text\n\
    ## To native parseInt (s: Text) -> Int\n\
    \n\
    ## Main\n\
    Let arguments be args().\n\
    Let n be parseInt(item 2 of arguments).\n\
    Let mutable total be 0.\n\
    Let mutable i be 1.\n\
    While i is at most n:\n\
    \x20   Set total to total + (i / 7) + (i % 7).\n\
    \x20   Set i to i + 1.\n\
    Show total.\n";

/// `## No fastdiv` must reach the VM tier (`largo emit isa`/`wasm`, `largo run --interpret`) — RED
/// before the fix: the raw seam built the merged config but never `set_active_config`'d it, so the
/// VM compiler's `FastDiv` gate read the default (all-on) and kept magic division regardless.
#[test]
fn no_fastdiv_reaches_the_vm_tier_isa() {
    let on = compile_to_isa(DIV_PROGRAM)
        .expect("div program compiles to ISA")
        .to_uppercase();
    assert!(
        on.contains("MAGICDIVU") || on.contains("DIVPOW2"),
        "default: fastdiv should lower `/`/`%` to magic-reciprocal ops in the ISA, got:\n{on}"
    );
    let off_src = DIV_PROGRAM.replace("## Main", "## No fastdiv\n## Main");
    let off = compile_to_isa(&off_src)
        .expect("`## No fastdiv` div program compiles")
        .to_uppercase();
    assert!(
        !off.contains("MAGICDIVU") && !off.contains("DIVPOW2"),
        "`## No fastdiv` must reach the VM tier now (set_active_config on the raw seam) and keep \
         plain division ops, but magic-reciprocal ops are still in the ISA:\n{off}"
    );
}

/// `## Require fastdiv` is satisfiable on the VM tier (fastdiv fires there), so it must COMPILE
/// through `emit isa` — guards against over-erroring the new `## Require` check on the VM tiers.
#[test]
fn require_fastdiv_succeeds_through_emit_isa() {
    let src = DIV_PROGRAM.replace("## Main", "## Require fastdiv\n## Main");
    assert!(
        compile_to_isa(&src).is_ok(),
        "`## Require fastdiv` should succeed on the VM tier (fastdiv fires): {:?}",
        compile_to_isa(&src).err()
    );
}

/// A plain loop, no recursion — nothing for `memo` to apply to. `memo` is an AOT/JIT-codegen
/// construct the VM tiers never emit, so `## Require memo` must ERROR through the VM emit backends,
/// matching the `emit c` precedent. RED before the fix (the raw seam dropped `## Require`).
const NON_MEMOIZABLE: &str = "## Require memo\n## Main\n\
    Let mutable s be 0.\n\
    Let mutable i be 1.\n\
    While i is at most 10:\n\
    \x20   Set s to s + i.\n\
    \x20   Set i to i + 1.\n\
    Show s.\n";

#[test]
fn require_memo_errors_through_emit_isa() {
    let result = compile_to_isa(NON_MEMOIZABLE).map_err(|e| format!("{e:?}"));
    assert!(
        result.is_err(),
        "`## Require memo` must fail through `emit isa` (the VM tier can't memoize), but it \
         compiled:\n{}",
        result.unwrap_or_default()
    );
    let err = result.err().unwrap();
    assert!(
        err.contains("memo") && err.to_lowercase().contains("require"),
        "the ISA-path error should name the unfired `memo` requirement, got: {err}"
    );
}

#[test]
fn require_memo_errors_through_emit_wasm() {
    let result = compile_to_wasm(NON_MEMOIZABLE).map_err(|e| format!("{e:?}"));
    assert!(
        result.is_err(),
        "`## Require memo` must fail through `emit wasm` (the VM tier can't memoize), but it \
         compiled to a module of {} bytes",
        result.map(|b| b.len()).unwrap_or(0)
    );
    assert!(
        result.err().unwrap().contains("memo"),
        "the wasm-path error should name the unfired `memo` requirement"
    );
}

/// `largo run --interpret` (tree-walk tier) applies no optimizations, so any `## Require` is
/// unsatisfiable there and must be rejected before execution (the `run.rs` interpret arm surfaces
/// this). A decorator-free program yields no error.
#[test]
fn require_on_interpret_is_rejected() {
    let msg = interpret_unsatisfiable_require(NON_MEMOIZABLE);
    assert!(
        msg.is_some(),
        "`## Require memo` on `--interpret` must be rejected (the interpreter optimizes nothing)"
    );
    let msg = msg.unwrap();
    assert!(
        msg.contains("memo") && msg.contains("interpret"),
        "the interpret rejection should name `memo` and explain the interpreter tier, got: {msg}"
    );
    assert!(
        interpret_unsatisfiable_require("## Main\nShow 1.\n").is_none(),
        "a program with no `## Require` must run on `--interpret` without error"
    );
}
