//! P17 — browser pre-bundled wasm (HOTSWAP §P17). The browser AOT-equivalent tier: a
//! scalar function compiled to a `wasm32-unknown-unknown` cdylib module, which the
//! browser then `WebAssembly.instantiate`s and hot-swaps in through the warm-bytecode
//! indirection (the desktop cdylib path is P14–P16/P18). This proves the BUILD half —
//! a LOGOS function becomes a real `.wasm` module exporting its scalar entry. The
//! browser load/instantiate is JS glue over this artifact. `#[ignore]` because it
//! shells out to `cargo` for the `wasm32` target (slow, and requires that target).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{build_native_wasm, compile_function_to_native_rust};

const PROG: &str = "\
## To add (a: Int, b: Int) -> Int:
    Return a + b.

## Main
Show add(2, 3).
";

#[test]
#[ignore = "builds a wasm32-unknown-unknown module via cargo (slow; needs the wasm target)"]
fn scalar_function_builds_to_a_wasm_module() {
    let module = compile_function_to_native_rust(PROG, "add")
        .expect("parses/types")
        .expect("add is in the scalar ABI subset");

    let dir = std::env::temp_dir().join(format!("logos_wasm_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let wasm = build_native_wasm(&module.rust, "logos_add_wasm", &dir)
        .expect("scalar function builds to a wasm module");

    assert!(wasm.exists(), "wasm module exists at {wasm:?}");
    assert_eq!(wasm.extension().and_then(|e| e.to_str()), Some("wasm"));
    let bytes = std::fs::read(&wasm).expect("readable");
    assert!(bytes.starts_with(b"\0asm"), "a real wasm module (magic header)");

    let _ = std::fs::remove_dir_all(&dir);
}
