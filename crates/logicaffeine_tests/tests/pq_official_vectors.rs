//! The native ML-DSA-65 and ML-KEM-768 kernels (`logicaffeine_system::{mldsa, mlkem}`) against the
//! official answers, every case of each file:
//!
//! * NIST ACVP (`tests/data/acvp/`, usnistgov/ACVP-Server, public domain): ML-DSA signing under every
//!   context NIST draws, deterministic and hedged; ML-DSA verification verdicts; and FIPS 203's
//!   encapsulation-key modulus check and decapsulation-key hash check.
//! * Project Wycheproof (`tests/data/wycheproof/`, C2SP/wycheproof, Apache-2.0): ML-DSA verification
//!   (too-long contexts, malformed hints, norm violations, wrong lengths, zero keys, many-iteration
//!   signatures), ML-DSA signing from a seed and from an expanded key, and ML-KEM encapsulation,
//!   decapsulation and key handling, including unreduced keys and malformed ciphertexts.
//!
//! A case the kernels cannot express (a signature over an external μ, FIPS 204's internal interface)
//! is counted and its count asserted, so a fixture refresh that adds or drops one is noticed.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_system::{mldsa, mlkem};
use serde_json::Value;

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex byte"))
        .collect()
}

fn load(dir: &str, name: &str) -> Value {
    let p = format!("{}/tests/data/{dir}/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}")))
        .unwrap_or_else(|e| panic!("parse {p}: {e}"))
}

fn field(v: &Value, k: &str) -> Vec<u8> {
    hex(v[k]
        .as_str()
        .unwrap_or_else(|| panic!("missing hex field {k} in {v}")))
}

fn optional(v: &Value, k: &str) -> Vec<u8> {
    v.get(k)
        .and_then(Value::as_str)
        .map(hex)
        .unwrap_or_default()
}

fn flags(t: &Value) -> Vec<&str> {
    t["flags"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Every case of a Wycheproof file, with the group it belongs to.
fn wycheproof_cases(file: &Value) -> Vec<(&Value, &Value)> {
    let cases: Vec<(&Value, &Value)> = file["testGroups"]
        .as_array()
        .expect("testGroups")
        .iter()
        .flat_map(|g| {
            g["tests"]
                .as_array()
                .expect("tests")
                .iter()
                .map(move |t| (g, t))
        })
        .collect();
    assert_eq!(
        cases.len() as u64,
        file["numberOfTests"].as_u64().expect("numberOfTests"),
        "every case of the file is read"
    );
    cases
}

fn valid(t: &Value) -> bool {
    match t["result"].as_str() {
        Some("valid") => true,
        Some("invalid") => false,
        other => panic!("tcId {}: unexpected result {other:?}", t["tcId"]),
    }
}

fn report(what: &str, wrong: Vec<String>, seen: usize) {
    assert!(
        wrong.is_empty(),
        "{what}: {} of {seen} cases disagree:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// ─── ML-DSA-65 ────────────────────────────────────────────────────────────────────────────────

#[test]
fn mldsa65_verifies_every_wycheproof_vector_as_expected() {
    let file = load("wycheproof", "mldsa65_verify.json");
    let cases = wycheproof_cases(&file);
    let mut wrong = Vec::new();
    for (g, t) in &cases {
        let pk = field(g, "publicKey");
        let got = mldsa::verify(&pk, &field(t, "msg"), &optional(t, "ctx"), &field(t, "sig"));
        if got != valid(t) {
            wrong.push(format!(
                "tcId {} ({}; {:?}): verify said {got}, Wycheproof says {}",
                t["tcId"],
                t["comment"],
                flags(t),
                t["result"]
            ));
        }
    }
    report("Wycheproof ML-DSA-65 verify", wrong, cases.len());
}

/// Sign one Wycheproof or ACVP case: hedged when it gives `rnd`, else deterministic.
fn sign_case(sk: &[u8], msg: &[u8], ctx: &[u8], rnd: &[u8]) -> Result<Vec<u8>, mldsa::SignError> {
    if rnd.is_empty() {
        mldsa::sign(sk, msg, ctx)
    } else {
        let rnd: [u8; 32] = rnd.try_into().expect("rnd is 32 bytes");
        mldsa::sign_randomized(sk, msg, ctx, &rnd)
    }
}

/// Judge one signing case: a valid one signs to exactly its signature (which also verifies), an
/// invalid one is refused.
fn judge_signing(
    t: &Value,
    signed: Result<Vec<u8>, mldsa::SignError>,
    pk: Option<&[u8]>,
    wrong: &mut Vec<String>,
) {
    let id = format!("tcId {} ({}; {:?})", t["tcId"], t["comment"], flags(t));
    match (valid(t), signed) {
        (true, Ok(sig)) => {
            if sig != field(t, "sig") {
                wrong.push(format!("{id}: the signature differs from Wycheproof's"));
            }
            if let Some(pk) = pk {
                if !mldsa::verify(pk, &optional(t, "msg"), &optional(t, "ctx"), &sig) {
                    wrong.push(format!("{id}: our own signature does not verify"));
                }
            }
        }
        (true, Err(e)) => wrong.push(format!("{id}: refused a valid case: {e}")),
        (false, Ok(_)) => wrong.push(format!("{id}: signed a case Wycheproof calls invalid")),
        (false, Err(_)) => {}
    }
}

#[test]
fn mldsa65_signs_every_wycheproof_vector_from_a_seed() {
    let file = load("wycheproof", "mldsa65_sign_seed.json");
    let cases = wycheproof_cases(&file);
    let (mut wrong, mut external_mu) = (Vec::new(), 0);
    for (g, t) in &cases {
        if flags(t).contains(&"Internal") {
            external_mu += 1;
            continue;
        }
        let seed = field(g, "privateSeed");
        let Ok(seed) = <[u8; 32]>::try_from(seed.as_slice()) else {
            // KeyGen takes a 32-byte ξ; a seed of any other length is refused by its type.
            if valid(t) {
                wrong.push(format!(
                    "tcId {}: a {}-byte seed called valid",
                    t["tcId"],
                    seed.len()
                ));
            }
            continue;
        };
        let (pk, sk) = mldsa::keygen(&seed);
        if pk != field(g, "publicKey") {
            wrong.push(format!(
                "tcId {}: the public key KeyGen derives differs",
                t["tcId"]
            ));
        }
        let signed = sign_case(
            &sk,
            &optional(t, "msg"),
            &optional(t, "ctx"),
            &optional(t, "rnd"),
        );
        judge_signing(t, signed, Some(&pk), &mut wrong);
    }
    report("Wycheproof ML-DSA-65 sign (seed)", wrong, cases.len());
    assert_eq!(
        external_mu, 17,
        "the external-μ cases, which sign through Sign_internal alone"
    );
}

#[test]
fn mldsa65_signs_every_wycheproof_vector_from_an_expanded_key() {
    let file = load("wycheproof", "mldsa65_sign_noseed.json");
    let cases = wycheproof_cases(&file);
    let (mut wrong, mut external_mu) = (Vec::new(), 0);
    for (g, t) in &cases {
        if flags(t).contains(&"Internal") {
            external_mu += 1;
            continue;
        }
        let pk = g.get("publicKey").and_then(Value::as_str).map(hex);
        let signed = sign_case(
            &field(g, "privateKey"),
            &optional(t, "msg"),
            &optional(t, "ctx"),
            &optional(t, "rnd"),
        );
        judge_signing(t, signed, pk.as_deref(), &mut wrong);
    }
    report(
        "Wycheproof ML-DSA-65 sign (expanded key)",
        wrong,
        cases.len(),
    );
    assert_eq!(
        external_mu, 10,
        "the external-μ cases, which sign through Sign_internal alone"
    );
}

#[test]
fn mldsa65_signs_every_acvp_vector_under_its_context() {
    let cases = load("acvp", "mldsa65_siggen_context.json");
    let cases = cases["cases"].as_array().expect("cases");
    let mut wrong = Vec::new();
    let (mut hedged, mut long_context) = (0, 0);
    for c in cases {
        let ctx = field(c, "context");
        hedged += c.get("rnd").is_some() as usize;
        long_context += (ctx.len() == 255) as usize;
        match sign_case(
            &field(c, "sk"),
            &field(c, "message"),
            &ctx,
            &optional(c, "rnd"),
        ) {
            Ok(sig) if sig == field(c, "signature") => {}
            Ok(_) => wrong.push(format!(
                "tcId {} (|ctx| = {}): the signature differs from NIST's",
                c["tcId"],
                ctx.len()
            )),
            Err(e) => wrong.push(format!(
                "tcId {} (|ctx| = {}): refused: {e}",
                c["tcId"],
                ctx.len()
            )),
        }
    }
    report("ACVP ML-DSA-65 sigGen (external, pure)", wrong, cases.len());
    assert_eq!(cases.len(), 30, "15 deterministic and 15 hedged NIST cases");
    assert_eq!(hedged, 15, "half of them hedged");
    assert!(
        long_context >= 2,
        "NIST draws the 255-byte context in both groups"
    );
}

#[test]
fn mldsa65_verifies_every_acvp_vector_as_nist_judges_it() {
    let cases = load("acvp", "mldsa65_sigver.json");
    let cases = cases["cases"].as_array().expect("cases");
    let mut wrong = Vec::new();
    for c in cases {
        let want = c["testPassed"].as_bool().expect("testPassed");
        let got = mldsa::verify(
            &field(c, "pk"),
            &field(c, "message"),
            &field(c, "context"),
            &field(c, "signature"),
        );
        if got != want {
            wrong.push(format!(
                "tcId {} ({}): verify said {got}, NIST says {want}",
                c["tcId"], c["reason"]
            ));
        }
    }
    report("ACVP ML-DSA-65 sigVer (external, pure)", wrong, cases.len());
    assert_eq!(cases.len(), 15);
}

// ─── ML-KEM-768 ───────────────────────────────────────────────────────────────────────────────

/// `(d, z)` from a Wycheproof seed `d ‖ z`, or `None` for a seed of another length.
fn kem_seeds(seed: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    (seed.len() == 64).then(|| {
        (
            seed[..32].try_into().unwrap(),
            seed[32..].try_into().unwrap(),
        )
    })
}

#[test]
fn mlkem768_decapsulates_every_wycheproof_vector() {
    let file = load("wycheproof", "mlkem768_decaps.json");
    let cases = wycheproof_cases(&file);
    let mut wrong = Vec::new();
    for (_, t) in &cases {
        let id = format!("tcId {} ({}; {:?})", t["tcId"], t["comment"], flags(t));
        let Some((d, z)) = kem_seeds(&field(t, "seed")) else {
            // KeyGen takes 32-byte d and z; any other seed is refused by its type.
            if valid(t) {
                wrong.push(format!("{id}: a seed of the wrong length called valid"));
            }
            continue;
        };
        let (ek, dk) = mlkem::keygen(&d, &z);
        if t.get("ek").is_some() && ek != field(t, "ek") {
            wrong.push(format!(
                "{id}: the encapsulation key KeyGen derives differs"
            ));
        }
        match (valid(t), mlkem::decaps(&dk, &field(t, "c"))) {
            (true, Ok(k)) if k.as_slice() == field(t, "K") => {}
            (true, Ok(_)) => wrong.push(format!("{id}: decapsulated a different key")),
            (true, Err(e)) => wrong.push(format!("{id}: refused a valid case: {e}")),
            (false, Ok(_)) => wrong.push(format!(
                "{id}: decapsulated a case Wycheproof calls invalid"
            )),
            (false, Err(_)) => {}
        }
    }
    report("Wycheproof ML-KEM-768 decaps", wrong, cases.len());
}

#[test]
fn mlkem768_encapsulates_every_wycheproof_vector() {
    let file = load("wycheproof", "mlkem768_encaps.json");
    let cases = wycheproof_cases(&file);
    let mut wrong = Vec::new();
    for (_, t) in &cases {
        let id = format!("tcId {} ({}; {:?})", t["tcId"], t["comment"], flags(t));
        let m: [u8; 32] = field(t, "m").try_into().expect("m is 32 bytes");
        match (valid(t), mlkem::encaps(&field(t, "ek"), &m)) {
            (true, Ok((c, k))) if c == field(t, "c") && k.as_slice() == field(t, "K") => {}
            (true, Ok(_)) => wrong.push(format!("{id}: a different ciphertext or key")),
            (true, Err(e)) => wrong.push(format!("{id}: refused a valid key: {e}")),
            (false, Ok(_)) => wrong.push(format!("{id}: encapsulated to a key FIPS 203 refuses")),
            (false, Err(_)) => {}
        }
    }
    report("Wycheproof ML-KEM-768 encaps", wrong, cases.len());
}

#[test]
fn mlkem768_decapsulates_every_wycheproof_expanded_key_vector() {
    let file = load("wycheproof", "mlkem768_semi_expanded_decaps.json");
    let cases = wycheproof_cases(&file);
    let mut wrong = Vec::new();
    for (_, t) in &cases {
        let id = format!("tcId {} ({}; {:?})", t["tcId"], t["comment"], flags(t));
        match (valid(t), mlkem::decaps(&field(t, "dk"), &field(t, "c"))) {
            (true, Ok(k)) if k.as_slice() == field(t, "K") => {}
            (true, Ok(_)) => wrong.push(format!("{id}: decapsulated a different key")),
            (true, Err(e)) => wrong.push(format!("{id}: refused a valid case: {e}")),
            (false, Ok(_)) => wrong.push(format!(
                "{id}: decapsulated under a key or ciphertext FIPS 203 refuses"
            )),
            (false, Err(_)) => {}
        }
    }
    report(
        "Wycheproof ML-KEM-768 semi-expanded decaps",
        wrong,
        cases.len(),
    );
}

#[test]
fn mlkem768_checks_its_keys_as_nist_judges_them() {
    // ACVP's FIPS 203 §7 key checks: an encapsulation key passes the modulus check exactly when NIST
    // says so, and a decapsulation key passes the hash check exactly when NIST says so.
    let cases = load("acvp", "mlkem768_keycheck.json");
    let cases = cases["cases"].as_array().expect("cases");
    let mut wrong = Vec::new();
    let (honest_ek, _) = mlkem::keygen(&[0x5a; 32], &[0xa5; 32]);
    let (honest_ct, _) = mlkem::encaps(&honest_ek, &[0x3c; 32]).expect("an honest key");
    for c in cases {
        let want = c["testPassed"].as_bool().expect("testPassed");
        let got = match c["function"].as_str() {
            Some("encapsulationKeyCheck") => mlkem::encaps(&field(c, "ek"), &[0x3c; 32]).is_ok(),
            Some("decapsulationKeyCheck") => match mlkem::decaps(&field(c, "dk"), &honest_ct) {
                Err(mlkem::KemError::DecapsulationKeyCorrupt) => false,
                Ok(_) => true,
                Err(e) => panic!("tcId {}: an unexpected refusal: {e}", c["tcId"]),
            },
            other => panic!("unknown function {other:?}"),
        };
        if got != want {
            wrong.push(format!(
                "tcId {} ({}): passed = {got}, NIST says {want}",
                c["tcId"], c["reason"]
            ));
        }
    }
    report("ACVP ML-KEM-768 key checks", wrong, cases.len());
    assert_eq!(cases.len(), 20);
}
