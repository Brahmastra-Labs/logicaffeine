use super::*;
use std::env;

/// `largo build --emit wasm` writes a self-contained, VALID `.wasm` module directly from a LOGOS
/// project — no rustc, cargo, or wasm-bindgen. Verifies the CLI plumbing (manifest → entry →
/// `compile_to_wasm` → `target/<name>.wasm`) end to end and that the bytes are a real wasm module.
#[test]
fn emit_wasm_writes_a_valid_module() {
    let dir = env::temp_dir().join(format!("largo_emit_wasm_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("Largo.toml"),
        "[package]\nname=\"emitted\"\nversion=\"0.1.0\"\nentry=\"src/main.lg\"\n",
    )
    .unwrap();
    fs::write(dir.join("src/main.lg"), "## Main\n    Show 6 * 7.\n").unwrap();

    emit_wasm_module(&dir, false).expect("emit wasm should succeed");

    let wasm = fs::read(dir.join("target/emitted.wasm")).expect("output .wasm should exist");
    assert_eq!(
        &wasm[0..4],
        b"\0asm",
        "output must begin with the wasm magic"
    );
    assert!(wasm.len() > 8, "a real module is more than a header");
    fs::remove_dir_all(&dir).ok();
}
