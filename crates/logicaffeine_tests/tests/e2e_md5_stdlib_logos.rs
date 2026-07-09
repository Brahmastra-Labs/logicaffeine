//! MD5 promoted to the Logos STDLIB: `md5Digest` (full multi-block MD5 with little-endian padding,
//! written in Logos over Word32) drives `uuidV3` — so name-based v3 UUIDs are derived in-language, with
//! NO native `md5` kernel on the path. Proven byte-exact against the native oracle (`Uuid::new_v3`, which
//! is validated against the `md-5` crate) on the tree-walker == VM AND the AOT tier.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn md5_digest_stdlib_matches_reference_all_tiers() {
    // md5Digest("abc") — the 16 raw digest bytes, one per line — must equal the reference MD5.
    let src = "## Main\n\
        \x20   Let msg be text_bytes(\"abc\").\n\
        \x20   Let d be md5Digest(msg).\n\
        \x20   Repeat for b in d:\n\
        \x20       Show b.";
    let expected: Vec<String> = logicaffeine_base::hash::md5(b"abc")
        .iter()
        .map(|b| b.to_string())
        .collect();
    let joined = expected.join("\n");
    common::assert_interpreter_output(src, &joined);
    let refs: Vec<&str> = expected.iter().map(|s| s.as_str()).collect();
    common::assert_output_lines(src, &refs);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn uuid_v3_written_in_logos_matches_reference_all_tiers() {
    // uuidV3 now derives its digest from the Logos md5Digest. Compare the canonical string against the
    // native oracle for the RFC DNS namespace + "www.example.com".
    let ns = logicaffeine_base::Uuid::parse("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap();
    let expected = logicaffeine_base::Uuid::new_v3(ns, b"www.example.com").to_string();
    let src = "## Main\n\
        \x20   Let ns be uuidParse(\"6ba7b810-9dad-11d1-80b4-00c04fd430c8\").\n\
        \x20   Let u be uuidV3(ns, \"www.example.com\").\n\
        \x20   Show uuidFormat(u).";
    common::assert_interpreter_output(src, &expected);
    common::assert_output_lines(src, &[&expected]);
}
