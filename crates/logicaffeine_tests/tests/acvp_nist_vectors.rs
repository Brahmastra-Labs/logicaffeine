//! Bit-exact gates vs the OFFICIAL NIST ACVP test vectors (FIPS 203 ML-KEM, FIPS 204 ML-DSA).
//!
//! The strongest correctness claim we can make for the standardized PQC schemes: our LOGOS
//! `mlkem768*` / `mldsa*Logos` programs, compiled the full AOT way, must reproduce NIST's own
//! published expected outputs BYTE-FOR-BYTE on NIST's own seeds — not a third-party oracle. The
//! vectors are vendored under `tests/data/acvp/` from `usnistgov/ACVP-Server` @ `a7f283c` (a
//! US-Government work, public domain); regenerate with `scripts/test/fetch-acvp-vectors.sh`.
//!
//! Each gate runs every NIST vector for its operation inside a SINGLE compiled program (one rustc
//! invocation), so the whole vector set costs one AOT compile.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_logos_with_args;

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex byte"))
        .collect()
}
fn lit(v: &[u8]) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
fn load(name: &str) -> Vec<serde_json::Value> {
    let p = format!("{}/tests/data/acvp/{}", env!("CARGO_MANIFEST_DIR"), name);
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}")),
    )
    .unwrap();
    v["cases"].as_array().expect("cases array").clone()
}
fn field(c: &serde_json::Value, k: &str) -> Vec<u8> {
    hex(c[k]
        .as_str()
        .unwrap_or_else(|| panic!("missing hex field {k}")))
}
/// Run a LOGOS program that prints one integer (byte) per line; collect exactly `n` bytes.
fn run_bytes(prog: &str, n: usize) -> Vec<u8> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(
        aot.success,
        "AOT compile/run failed:\n--- stderr ---\n{}",
        aot.stderr
    );
    let got: Vec<u8> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse::<i64>().expect("byte line") as u8)
        .collect();
    assert_eq!(got.len(), n, "expected {n} output bytes, got {}", got.len());
    got
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-KEM-768 keyGen vs official NIST ACVP"]
fn logos_mlkem768_keygen_matches_official_nist_acvp() {
    // FIPS 203: ek (encapsulation/public key) is derived from d alone, so `mlkem768Keygen(d)` must
    // reproduce NIST's ek for every ACVP seed.
    let cases = load("mlkem768_keygen.json");
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        expected.extend_from_slice(&field(c, "ek"));
        prog += &format!(
            "Let d{i} be [{}].\nLet ek{i} be mlkem768Keygen(d{i}).\nRepeat for j from 1 to 1184:\n    Show item j of ek{i}.\n",
            lit(&field(c, "d"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got,
        expected,
        "LOGOS ML-KEM-768 keyGen ek must equal NIST ACVP over all {} vectors",
        cases.len()
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-DSA-65 keyGen vs official NIST ACVP"]
fn logos_mldsa65_keygen_matches_official_nist_acvp() {
    // FIPS 204: `mldsaKeygenLogos(seed)` must reproduce NIST's pk for every ACVP seed.
    let cases = load("mldsa65_keygen.json");
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        expected.extend_from_slice(&field(c, "pk"));
        prog += &format!(
            "Let seed{i} be [{}].\nLet pk{i} be mldsaKeygenLogos(seed{i}).\nRepeat for j from 1 to 1952:\n    Show item j of pk{i}.\n",
            lit(&field(c, "seed"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got,
        expected,
        "LOGOS ML-DSA-65 keyGen pk must equal NIST ACVP over all {} vectors",
        cases.len()
    );
}

/// encaps/decaps/sign embed full keys (1184/2400/4032 B) as program literals, so we gate a
/// representative slice of NIST's vectors to bound the single compiled program's size.
const SLICE: usize = 6;

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-KEM-768 encaps vs official NIST ACVP"]
fn logos_mlkem768_encaps_matches_official_nist_acvp() {
    // FIPS 203: `mlkem768Encaps(ek, m)` returns c ‖ K; both must equal NIST's ciphertext and
    // shared secret for its (ek, m) inputs.
    let cases = load("mlkem768_encaps.json");
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().take(SLICE).enumerate() {
        expected.extend_from_slice(&field(c, "c"));
        expected.extend_from_slice(&field(c, "k"));
        prog += &format!(
            "Let ek{i} be [{}].\nLet m{i} be [{}].\nLet out{i} be mlkem768Encaps(ek{i}, m{i}).\nRepeat for j from 1 to 1120:\n    Show item j of out{i}.\n",
            lit(&field(c, "ek")),
            lit(&field(c, "m"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got, expected,
        "LOGOS ML-KEM-768 encaps (c‖K) must equal NIST ACVP over {SLICE} vectors"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-KEM-768 decaps vs official NIST ACVP"]
fn logos_mlkem768_decaps_matches_official_nist_acvp() {
    // FIPS 203: `mlkem768Decaps(dk, c)` must recover NIST's shared secret K.
    let cases = load("mlkem768_decaps.json");
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().take(SLICE).enumerate() {
        expected.extend_from_slice(&field(c, "k"));
        prog += &format!(
            "Let dk{i} be [{}].\nLet c{i} be [{}].\nLet k{i} be mlkem768Decaps(dk{i}, c{i}).\nRepeat for j from 1 to 32:\n    Show item j of k{i}.\n",
            lit(&field(c, "dk")),
            lit(&field(c, "c"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got, expected,
        "LOGOS ML-KEM-768 decaps K must equal NIST ACVP over {SLICE} vectors"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-DSA-65 sign under every NIST context"]
fn logos_mldsa65_sign_under_every_acvp_context_matches_official_nist_acvp() {
    // FIPS 204's external, pure, deterministic interface under each context NIST draws (0 to 255
    // bytes): `mldsaSignCtxLogos(sk, M, ctx)` must reproduce NIST's signature, and verify under it.
    let cases: Vec<serde_json::Value> = load("mldsa65_siggen_context.json")
        .into_iter()
        .filter(|c| c.get("rnd").is_none())
        .collect();
    assert_eq!(
        cases.len(),
        15,
        "NIST's deterministic ML-DSA-65 context cases"
    );
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        expected.extend_from_slice(&field(c, "signature"));
        let ctx = field(c, "context");
        let ctx = if ctx.is_empty() {
            "a new Seq of Int".to_string()
        } else {
            format!("[{}]", lit(&ctx))
        };
        prog += &format!(
            "Let sk{i} be [{}].\nLet msg{i} be [{}].\nLet ctx{i} be {ctx}.\nLet sig{i} be mldsaSignCtxLogos(sk{i}, msg{i}, ctx{i}).\nRepeat for j from 1 to 3309:\n    Show item j of sig{i}.\n",
            lit(&field(c, "sk")),
            lit(&field(c, "message"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got, expected,
        "LOGOS ML-DSA-65 signatures under every context must equal NIST ACVP"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — ML-DSA-65 sign vs official NIST ACVP"]
fn logos_mldsa65_sign_matches_official_nist_acvp() {
    // FIPS 204: `mldsaSignLogos(sk, M)` is the external-interface, pure, empty-context,
    // deterministic signer; it must reproduce NIST's signature for that exact variant.
    let cases = load("mldsa65_siggen.json");
    assert!(
        !cases.is_empty(),
        "no matching ML-DSA sign vector in the fixture"
    );
    let mut prog = String::from("## Main\n");
    let mut expected = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        expected.extend_from_slice(&field(c, "signature"));
        prog += &format!(
            "Let sk{i} be [{}].\nLet msg{i} be [{}].\nLet sig{i} be mldsaSignLogos(sk{i}, msg{i}).\nRepeat for j from 1 to 3309:\n    Show item j of sig{i}.\n",
            lit(&field(c, "sk")),
            lit(&field(c, "message"))
        );
    }
    let got = run_bytes(&prog, expected.len());
    assert_eq!(
        got, expected,
        "LOGOS ML-DSA-65 signature must equal NIST ACVP"
    );
}
