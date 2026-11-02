//! ★ ML-DSA AOT-Rust LOCK — the post-quantum signing surface lowers to Rust that COMPILES. ★
//!
//! The ML-DSA signing path emits monomorphized `mldsaSubRange_s*_s*` slice helpers, one
//! per distinct `(offset, length)` the program actually asks for. Each helper's declared
//! return type is INFERRED from its body: a plain slice-to-owned returns `Vec<i64>`, a
//! LogosSeq-wrapping return returns `LogosSeq<i64>`, and the Word32 twin returns
//! `LogosSeq<Word32>`. A regression that let the declared signature drift from what the
//! body returns is an `E0308` — but only rustc sees it, and only when the helper is
//! IN-PATH. The normal ML-DSA callers never exercise every specialization, so the
//! reachable-code tree-shaker prunes the offending helper and the mismatch stays hidden
//! (dead in-path). This lock keeps it honest by constructing a program that FORCES the
//! full set of specializations to be reachable — a real `pqAuthHandshake` over ML-DSA
//! keypairs — then BUILDS the emitted Rust with a real `cargo build`. The type checker is
//! the oracle: if any specialized helper's return type ever stops matching its body, the
//! ML-DSA AOT surface fails to compile and this test goes RED with the exact `E0308`.

use std::process::Command;

/// A 32-byte deterministic `Seq of Int` seed named `name`, every byte `val`.
fn seed(name: &str, val: u8) -> String {
    format!("Let mutable {name} be a new Seq of Int.\nRepeat for i from 1 to 32:\n    Push {val} to {name}.\n")
}

/// The ML-DSA signing surface, exercised end to end so EVERY `mldsaSubRange_*`
/// specialization the handshake needs is reachable — none is tree-shaken away, so a
/// return-type drift in any one of them surfaces as an `E0308` at cargo-build time.
#[test]
fn mldsa_sign_surface_lowers_to_aot_rust_that_compiles() {
    let mut p = String::from("## Main\n");
    p.push_str(&seed("seedI", 1));
    p.push_str(&seed("seedR", 2));
    p.push_str(&seed("kemD", 16));
    p.push_str(&seed("kemZ", 32));
    p.push_str(&seed("kemM", 48));
    p.push_str("Let ctx be a new Seq of Int.\n");
    p.push_str("Let idi be mldsaKeypairLogos(seedI).\n");
    p.push_str("Let idiPk be mlkemSubseq(idi, 0, 1952).\n");
    p.push_str("Let idiSk be mlkemSubseq(idi, 1952, 4032).\n");
    p.push_str("Let idr be mldsaKeypairLogos(seedR).\n");
    p.push_str("Let idrPk be mlkemSubseq(idr, 0, 1952).\n");
    p.push_str("Let idrSk be mlkemSubseq(idr, 1952, 4032).\n");
    p.push_str("Let ss be pqAuthHandshake(idiPk, idiSk, idrPk, idrSk, kemD, kemZ, kemM, ctx).\n");
    p.push_str("Show length of ss.\n");

    let out = logicaffeine_compile::compile::compile_program_full(&p)
        .expect("the ML-DSA signing program lowers to Rust");

    // Build a real cargo project so rustc resolves the workspace deps and any
    // return-type drift on a specialized helper surfaces as the real `E0308`.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let ws_root = std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let cargo_toml = format!(
        r#"[package]
name = "mldsa_aot_lock_gen"
version = "0.1.0"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "{root}/crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "{root}/crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
serde = {{ version = "1", features = ["derive"] }}
rayon = "1"
memchr = "2"
"#,
        root = ws_root.display()
    );
    std::fs::write(dir.path().join("Cargo.toml"), cargo_toml).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), &out.rust_code).unwrap();

    // A shared, cached target dir keeps repeat runs cheap (the deps compile once).
    let target = ws_root.join("target/mldsa_aot_lock_target");
    let output = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", &target)
        .env("CARGO_INCREMENTAL", "0")
        .output()
        .expect("cargo build spawns");

    if !output.status.success() {
        // Surface the specialized signatures + the type error so a RED run is diagnosable.
        for line in out.rust_code.lines() {
            if line.contains("mldsaSubRange") && line.contains("fn ") {
                eprintln!("SIG: {}", line.trim());
            }
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        for chunk in stderr.split("\n\n") {
            if chunk.contains("E0308")
                || chunk.contains("expected")
                || chunk.contains("mldsaSubRange")
            {
                eprintln!("=== ERR ===\n{chunk}");
            }
        }
    }
    assert!(
        output.status.success(),
        "the ML-DSA signing surface must lower to AOT-Rust that COMPILES — a specialized \
         mldsaSubRange_* helper's return type drifted from its body (E0308, see above)"
    );
}
