//! P14a — per-function codegen slice (HOTSWAP Axis-3). The AOT-native tier compiles
//! one annotated function into a loadable artifact, so it needs that function plus its
//! transitive callees and nothing else. `function_slice` reduces the program to that
//! closure (via the shared call graph), dropping `Main` and unrelated functions — the
//! input the per-function codegen + cdylib build (P14b/P16) consume.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::HashSet;

use logicaffeine_compile::codegen::{codegen_native_tier_export, function_slice};
use logicaffeine_compile::compile::compile_function_to_native_rust;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_language::ast::Stmt;

// f → g → h is the reachable chain; `k` is unrelated, and `Main` must be dropped.
const PROG: &str = "\
## To h (x: Int) -> Int:
    Return x.

## To g (x: Int) -> Int:
    Return h(x).

## To f (x: Int) -> Int:
    Return g(x).

## To k (x: Int) -> Int:
    Return x.

## Main
Show f(5).
";

#[test]
fn slice_keeps_target_and_transitive_callees_only() {
    with_parsed_program(PROG, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("program parses");
        let target = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "f" => Some(*name),
                _ => None,
            })
            .expect("function f exists");

        let slice = function_slice(stmts, target, interner);
        let names: HashSet<String> = slice
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(interner.resolve(*name).to_string()),
                _ => None,
            })
            .collect();

        assert_eq!(
            names,
            HashSet::from(["f".to_string(), "g".to_string(), "h".to_string()]),
            "slice = target f + transitive callees g,h; unrelated k and Main dropped"
        );
        // Only function definitions survive — no Main / top-level statements leak in.
        assert!(
            slice.iter().all(|s| matches!(s, Stmt::FunctionDef { .. })),
            "slice contains only FunctionDefs"
        );
    });
}

// P14b — the Rust-native export shim. A scalar function gets a thin `#[no_mangle]
// extern "C"` wrapper crossing values BY VALUE (no CString/handle marshaling); a
// function with a non-scalar (Seq/Map/Text) param gets NO shim and falls through.
const P14B_PROG: &str = "\
## To add (a: Int, b: Int) -> Int:
    Return a.

## To total (xs: Seq of Int) -> Int:
    Return 0.

## Main
Show add(1, 2).
";

#[test]
fn native_tier_export_emits_scalar_shim_and_skips_nonscalar() {
    with_parsed_program(P14B_PROG, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.expect("program parses");
        let get = |nm: &str| {
            stmts.iter().find_map(|s| match s {
                Stmt::FunctionDef { name, params, return_type, .. }
                    if interner.resolve(*name) == nm =>
                {
                    let ps: Vec<_> = params.iter().map(|(s, t)| (*s, *t)).collect();
                    Some((*name, ps, *return_type))
                }
                _ => None,
            })
        };

        // Scalar function → a value-crossing shim, no C marshaling.
        let (an, ap, ar) = get("add").expect("add exists");
        let shim = codegen_native_tier_export(an, &ap, ar, interner).expect("scalar fn gets a shim");
        assert!(shim.contains("#[no_mangle]"), "shim:\n{shim}");
        assert!(shim.contains("extern \"C\""));
        assert!(shim.contains("logos_native_add"));
        assert!(shim.contains(" -> "), "scalar return present");
        assert!(
            !shim.contains("c_char") && !shim.contains("Handle") && !shim.contains("CString"),
            "the Rust-native shim must NOT use C marshaling:\n{shim}"
        );

        // Seq param → outside the sound scalar subset → no shim (fall through to VM+JIT).
        let (tn, tp, tr) = get("total").expect("total exists");
        assert!(
            codegen_native_tier_export(tn, &tp, tr, interner).is_none(),
            "a non-scalar param must yield no AOT-native shim"
        );
    });
}

#[test]
fn native_module_source_assembles_for_scalar_target() {
    // Full source → AOT-native cdylib module: the sliced function + the export shim.
    let module = compile_function_to_native_rust(P14B_PROG, "add")
        .expect("source compiles")
        .expect("scalar target yields a module");
    assert!(module.rust.contains("logos_native_add"), "export shim symbol present:\n{}", module.rust);
    assert!(module.rust.contains("#[no_mangle]"), "shim is no_mangle");
    assert!(module.rust.contains("fn add"), "the sliced inner function is emitted");
    assert_eq!(module.symbol, "logos_native_add", "exact export symbol");
    assert_eq!(module.arity, 2, "two scalar params");

    // Non-scalar target → no module (caller keeps it on VM+JIT).
    assert!(
        compile_function_to_native_rust(P14B_PROG, "total")
            .expect("source compiles")
            .is_none(),
        "non-scalar function → no AOT-native module"
    );
    // Absent target → no module.
    assert!(
        compile_function_to_native_rust(P14B_PROG, "does_not_exist")
            .expect("source compiles")
            .is_none()
    );
}

// P16 (build) — the AOT-native PROOF: the generated module must compile to a real
// loadable cdylib. `#[ignore]` because it shells out to `cargo` (compiles the runtime
// crates from scratch — minutes); run on demand with `--run-ignored`.
#[test]
#[ignore = "builds a cdylib via cargo (slow) — the AOT-native build proof"]
fn aot_native_source_compiles_to_a_cdylib() {
    use logicaffeine_compile::compile::build_native_cdylib;
    let module = compile_function_to_native_rust(P14B_PROG, "add")
        .expect("source compiles")
        .expect("scalar target yields a module");
    let tmp = std::env::temp_dir().join(format!("logos_aot_p14_{}", std::process::id()));
    let so = build_native_cdylib(&module.rust, "logos_aot_add", &tmp)
        .unwrap_or_else(|e| panic!("AOT-native Rust must compile to a cdylib:\n{e}"));
    assert!(so.exists(), "cdylib produced at {}", so.display());
    let _ = std::fs::remove_dir_all(&tmp);
}
