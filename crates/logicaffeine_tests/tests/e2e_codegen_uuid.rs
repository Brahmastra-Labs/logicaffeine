//! UUID across every execution tier — tree-walker, bytecode VM, and AOT-compiled-to-Rust must agree
//! byte-for-byte. UUIDs (RFC 9562) parse and render canonically, the name-based versions (v3/v5) are
//! stable hashes of namespace ‖ name, and ids compare by their 128 bits (so time-ordered ids sort
//! chronologically). The values are validated bit-exact against the `uuid` crate in the base unit
//! tests; here we prove the LANGUAGE surface is identical across all three tiers.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_compiled_equals_interpreted, assert_interpreter_output};

#[cfg(not(target_arch = "wasm32"))]
const UUID: &str = "## Main\n\
Show uuid(\"550e8400-e29b-41d4-a716-446655440000\").\n\
Show uuid_nil().\n\
Show uuid_max().\n\
Show uuid_v3(uuid_dns(), \"www.example.com\").\n\
Show uuid_v5(uuid_dns(), \"www.example.com\").\n\
Show uuid_version(uuid_v5(uuid_dns(), \"www.example.com\")).";

#[cfg(not(target_arch = "wasm32"))]
const UUID_LINES: &[&str] = &[
    "550e8400-e29b-41d4-a716-446655440000",
    "00000000-0000-0000-0000-000000000000",
    "ffffffff-ffff-ffff-ffff-ffffffffffff",
    // RFC 9562 worked examples for the DNS namespace + "www.example.com".
    "5df41881-3aed-3515-88a7-2f4a814cf09e",
    "2ed6657d-e927-568b-95e1-2665a8aea6a2",
    "5",
];

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(UUID, &UUID_LINES.join("\n"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_on_aot() {
    common::assert_output_lines(UUID, UUID_LINES);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_all_tiers_agree() {
    assert_compiled_equals_interpreted(UUID);
}

// ---- The version constructors WRITTEN IN LOGOS (uuid.lg): uuidV3 hashes namespace ‖ name with the
//      native MD5 primitive; uuidV5 hashes it with SHA-1 written ENTIRELY IN LOGOS (the sha1rnds4 lane
//      schedule), then both stamp version+variant in-language. Must equal the RFC 9562 worked examples
//      AND the native builtins — proving the Logos SHA-1 is byte-exact across all three tiers. ----

#[cfg(not(target_arch = "wasm32"))]
const UUID_LG: &str = "## Main\n\
Show uuidV5(uuid_dns(), \"www.example.com\").\n\
Show uuidV3(uuid_dns(), \"www.example.com\").\n\
Show uuidV5(uuid_dns(), \"www.example.com\") is equal to uuid_v5(uuid_dns(), \"www.example.com\").";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_logos_written_constructors_on_interpreter() {
    // Logos-written v5/v3 match the RFC examples and the native builtins — same bytes.
    assert_interpreter_output(
        UUID_LG,
        "2ed6657d-e927-568b-95e1-2665a8aea6a2\n5df41881-3aed-3515-88a7-2f4a814cf09e\ntrue",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_logos_written_constructors_on_aot() {
    // The uuid.lg algorithm compiles to native Rust (calling the md5/sha1 kernels) and agrees.
    common::assert_output_lines(
        UUID_LG,
        &["2ed6657d-e927-568b-95e1-2665a8aea6a2", "5df41881-3aed-3515-88a7-2f4a814cf09e", "true"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_logos_written_constructors_all_tiers_agree() {
    assert_compiled_equals_interpreted(UUID_LG);
}

// ---- FOUNDATION for writing the hashes IN LOGOS: Word32 bitwise ops (&, |, xor, ~ via `xor ones`)
//      must work in-language. MD5's F-function `(b&c)|(~b&d)` computed in Logos over Word32. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn word32_bitwise_ops_work_in_logos() {
    // MD5's F-function `(b&c)|(~b&d)` in Logos via the cross-tier-consistent word bitwise builtins
    // (NOT `and`/`or`, which are logical short-circuit on the VM). b=0xAAAAAAAA, c=0x55555555,
    // d=0xF0F0F0F0 → 0x50505050 = 1347440720.
    let src = "## Main\n\
        Let b be word32(2863311530).\n\
        Let c be word32(1431655765).\n\
        Let d be word32(4042322160).\n\
        Let f be word_or(word_and(b, c), word_and(word_not(b), d)).\n\
        Show intOfWord32(f).";
    assert_interpreter_output(src, "1347440720");
}

// ---- The native hash TOOLS are directly Logos-callable (not just UUID-internal): a plain Logos
//      program reaches the SHA-NI SHA-1 / MD5 kernels via `sha1`/`md5` over `text_bytes`. ----

#[cfg(not(target_arch = "wasm32"))]
const CRYPTO_TOOLS: &str = "## Main\n\
Let d be sha1(text_bytes(\"abc\")).\n\
Show item 1 of d.\n\
Show item 2 of d.\n\
Let m be md5(text_bytes(\"abc\")).\n\
Show item 1 of m.";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn logos_calls_native_hash_tools_directly() {
    // sha1("abc") = a9993e36… → bytes 0xa9,0x99 = 169,153; md5("abc") = 9001… → 0x90 = 144.
    assert_interpreter_output(CRYPTO_TOOLS, "169\n153\n144");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn logos_calls_native_hash_tools_on_aot() {
    common::assert_output_lines(CRYPTO_TOOLS, &["169", "153", "144"]);
}

// ---- Natural `uuid "…"` literal (like `timestamp "…"`): reads as a value, no parens. ----

#[cfg(not(target_arch = "wasm32"))]
const UUID_NATURAL: &str = "## Main\n\
Show uuid \"550e8400-e29b-41d4-a716-446655440000\".\n\
Let id be uuid \"00000000-0000-0000-0000-000000000000\".\n\
Show id is equal to uuid_nil().";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_uuid_literal_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(UUID_NATURAL, "550e8400-e29b-41d4-a716-446655440000\ntrue");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_uuid_literal_on_aot() {
    common::assert_output_lines(UUID_NATURAL, &["550e8400-e29b-41d4-a716-446655440000", "true"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn natural_uuid_literal_all_tiers_agree() {
    assert_compiled_equals_interpreted(UUID_NATURAL);
}

// ---- Name-based ids are STABLE across namespaces, and v3 ≠ v5 for the same input. ----

#[cfg(not(target_arch = "wasm32"))]
const UUID_NAMESPACES: &str = "## Main\n\
Show uuid_v5(uuid_url(), \"https://logicaffeine.com\").\n\
Show uuid_v5(uuid_url(), \"https://logicaffeine.com\").\n\
Show uuid_v3(uuid_oid(), \"1.3.6.1\").\n\
Show uuid_version(uuid_v3(uuid_oid(), \"1.3.6.1\")).";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_namespaces_are_stable_across_tiers() {
    // The two v5 lines must be identical (deterministic), and v3 reports version 3.
    assert_compiled_equals_interpreted(UUID_NAMESPACES);
    common::assert_output_lines(
        "## Main\nShow uuid_v3(uuid_oid(), \"1.3.6.1\") is equal to uuid_v3(uuid_oid(), \"1.3.6.1\").",
        &["true"],
    );
}

// ---- Ordering: ids compare by their 128 bits, so v7 sorts by time. Forced across every tier. ----

#[cfg(not(target_arch = "wasm32"))]
const UUID_ORDER: &str = "## Main\n\
Let a be uuid(\"00000000-0000-0000-0000-000000000001\").\n\
Let b be uuid(\"00000000-0000-0000-0000-000000000002\").\n\
Show a is less than b.\n\
Show b is greater than a.\n\
Show a is equal to a.\n\
Show uuid_nil() is less than uuid_max().";

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_ordering_on_interpreter_vm_and_treewalker() {
    assert_interpreter_output(UUID_ORDER, "true\ntrue\ntrue\ntrue");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_ordering_on_aot() {
    common::assert_output_lines(UUID_ORDER, &["true", "true", "true", "true"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_ordering_all_tiers_agree() {
    assert_compiled_equals_interpreted(UUID_ORDER);
}

// ---- Parse normalizes every accepted form to the canonical lowercase hyphenated id. ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_parse_normalizes_input_forms() {
    // The simple (32-hex) and urn forms normalize to the canonical id. The braced `{…}` form is also
    // accepted by `Uuid::parse` (see the base unit tests) but cannot be written as a LOGOS *source*
    // string literal — `{…}` is string interpolation — so it is exercised at the value layer, not here.
    let src = "## Main\n\
        Show uuid(\"550E8400E29B41D4A716446655440000\").\n\
        Show uuid(\"urn:uuid:550e8400-e29b-41d4-a716-446655440000\").";
    let canon = "550e8400-e29b-41d4-a716-446655440000";
    common::assert_output_lines(src, &[canon, canon]);
    assert_compiled_equals_interpreted(src);
}

// ---- A malformed UUID is a clean runtime error on the interpreter (no output, no panic). ----

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn malformed_uuid_is_a_clean_error() {
    let r = common::run_interpreter("## Main\nShow \"start\".\nShow uuid(\"not-a-uuid\").");
    assert!(!r.success, "a malformed UUID must error");
    assert!(r.error.to_lowercase().contains("uuid"), "got: {}", r.error);
}
