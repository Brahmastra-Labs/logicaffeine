//! P12 — tier cache (HOTSWAP §P12). A compiled `FnBytecode` is serialized and stored
//! keyed by `(source, optimization-config, tier)`; a re-run loads it instead of
//! re-optimizing. Proofs: a store→load round-trips byte-identically (sound to install),
//! and any change to source / config / tier changes the key, invalidating the entry.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::fn_bytecode::slice_function;
use logicaffeine_compile::vm::tier_cache;
use logicaffeine_compile::vm::Compiler;
use logicaffeine_language::ast::Stmt;

const PROG: &str = "\
## To f (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return f(n - 1) + f(n - 2).

## Main
Show f(10).
";

/// Slice function `name` from `PROG` into a FnBytecode (the unit the cache stores).
fn body_of(name: &str) -> logicaffeine_compile::vm::fn_bytecode::FnBytecode {
    with_parsed_program(PROG, |parsed, interner| {
        let (stmts, types, _policies) = parsed.expect("parses");
        let program = Compiler::compile_with_types(stmts, interner, Some(types)).expect("compiles");
        let sym = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::FunctionDef { name: nm, .. } if interner.resolve(*nm) == name => Some(*nm),
                _ => None,
            })
            .expect("function defined");
        let fi = *program.fn_index.get(&sym).expect("in fn_index") as usize;
        slice_function(&program, fi)
    })
}

#[test]
fn encode_decode_round_trips_byte_identically() {
    let body = body_of("f");
    let wire = tier_cache::encode(&body);
    let back = tier_cache::decode(&wire).expect("decodes");
    // FnBytecode has no PartialEq (Op doesn't); re-encoding the decoded body must
    // reproduce the exact same wire bytes — a bit-exact round-trip.
    assert_eq!(tier_cache::encode(&back), wire, "decode∘encode is the identity");
    assert_eq!(back.code.len(), body.code.len());
    assert_eq!(back.register_count, body.register_count);
    assert_eq!(back.param_count, body.param_count);
}

#[test]
fn store_then_load_recovers_the_body() {
    let dir = std::env::temp_dir().join(format!("logos_tiercache_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let body = body_of("f");

    assert!(tier_cache::load(&dir, PROG, 0xABCD, 3).is_none(), "cold cache misses");
    tier_cache::store(&dir, PROG, 0xABCD, 3, &body).expect("stores");
    let loaded = tier_cache::load(&dir, PROG, 0xABCD, 3).expect("hot cache hits");
    assert_eq!(tier_cache::encode(&loaded), tier_cache::encode(&body), "loaded == stored");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn key_invalidates_on_source_config_or_tier_change() {
    let base = tier_cache::cache_key(PROG, 0xABCD, 3);
    // Editing the source invalidates.
    let edited = format!("{PROG}-- a trailing comment changes the bytes\n");
    assert_ne!(tier_cache::cache_key(&edited, 0xABCD, 3), base, "source edit ⇒ new key");
    // A different optimization config invalidates.
    assert_ne!(tier_cache::cache_key(PROG, 0x1234, 3), base, "config change ⇒ new key");
    // A different tier invalidates (m8).
    assert_ne!(tier_cache::cache_key(PROG, 0xABCD, 2), base, "tier change ⇒ new key");
    // Same inputs are stable.
    assert_eq!(tier_cache::cache_key(PROG, 0xABCD, 3), base, "deterministic");
}

#[test]
fn decode_rejects_corruption_so_a_bad_entry_is_just_a_miss() {
    let body = body_of("f");
    let wire = tier_cache::encode(&body);
    assert!(tier_cache::decode(&wire).is_some(), "valid wire decodes");

    let (_sum, json) = wire.split_once('\n').expect("checksum line present");

    // A valid-JSON payload with the WRONG checksum (a tamper / bit-flip that survived
    // JSON parsing) must be rejected, not installed — this is the "corrupt = miss"
    // guarantee the warm/cache path relies on.
    let wrong_sum = format!("0000000000000000\n{json}");
    assert!(tier_cache::decode(&wrong_sum).is_none(), "checksum mismatch is a miss");

    // Missing checksum line / garbage / empty all miss cleanly.
    assert!(tier_cache::decode(json).is_none(), "no checksum line → miss");
    assert!(tier_cache::decode("not json at all").is_none());
    assert!(tier_cache::decode("").is_none());

    // A truncated payload (checksum kept, JSON cut) → checksum mismatch → miss.
    let truncated = format!("{}\n{}", _sum, &json[..json.len() / 2]);
    assert!(tier_cache::decode(&truncated).is_none(), "truncation is a miss");
}
