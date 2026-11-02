//! The symmetric primitives against the official answers, every case of each file:
//!
//! * Project Wycheproof (`tests/data/wycheproof/`, C2SP/wycheproof, Apache-2.0): ChaCha20-Poly1305 and
//!   XChaCha20-Poly1305 (Poly1305's carry and modular edge cases, every tag bit flipped, truncated and
//!   empty frames), HMAC-SHA256 (keys of every block-relative length, truncated and altered tags) and
//!   PBKDF2-HMAC-SHA256 (iteration counts to 80,000, salts and passwords of every length).
//! * draft-irtf-cfrg-xchacha-03: the HChaCha20 subkey of §2.2.1 and the AEAD example of Appendix A.1.
//!
//! Each primitive is checked natively (`logicaffeine_system::aead`, `logicaffeine_compile::crypto_accel`)
//! and, where `crypto.lg` writes it, compiled from LOGOS to C. A case an interface cannot express (a
//! nonce of another length, a PBKDF2 output longer than one SHA-256 block) is counted and its count
//! asserted, so a fixture refresh that adds or drops one is noticed.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::crypto_accel;
use logicaffeine_system::aead;
use serde_json::Value;

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex length");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex byte"))
        .collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load(name: &str) -> Value {
    let p = format!(
        "{}/tests/data/wycheproof/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {p}: {e}")))
        .unwrap_or_else(|e| panic!("parse {p}: {e}"))
}

fn field(v: &Value, k: &str) -> Vec<u8> {
    hex(v[k]
        .as_str()
        .unwrap_or_else(|| panic!("missing hex field {k} in {v}")))
}

fn text(v: &Value, k: &str) -> String {
    v[k].as_str()
        .unwrap_or_else(|| panic!("missing field {k} in {v}"))
        .to_string()
}

/// Every case of a Wycheproof file with its group, the count checked against the file's own.
fn cases(file: &Value) -> Vec<(&Value, &Value)> {
    let all: Vec<(&Value, &Value)> = file["testGroups"]
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
        all.len() as u64,
        file["numberOfTests"].as_u64().unwrap(),
        "every case is read"
    );
    all
}

fn valid(t: &Value) -> bool {
    match t["result"].as_str().unwrap() {
        "valid" | "acceptable" => true,
        "invalid" => false,
        other => panic!("result {other}"),
    }
}

/// `program` compiled to C, built as a release C build builds it, and run: its standard output.
fn run_c(label: &str, program: &str) -> String {
    let c = logicaffeine_compile::compile::compile_to_c(program)
        .unwrap_or_else(|e| panic!("{label}: the program compiles to C: {e:?}"));
    let dir = tempfile::Builder::new()
        .prefix(&format!("logos_vectors_{label}_"))
        .tempdir()
        .expect("a run directory");
    let (file, bin) = (dir.path().join("prog.c"), dir.path().join("prog"));
    std::fs::write(&file, &c).expect("write the C");
    let built = common::build_c_release(&file, &bin).expect("the C compiler starts");
    assert!(
        built.status.success(),
        "{label}: the C builds\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("the program starts");
    assert!(
        run.status.success(),
        "{label}: the program runs\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

// ─── ChaCha20-Poly1305 (RFC 8439) ────────────────────────────────────────────────────────────

/// The ChaCha20-Poly1305 cases a 96-bit nonce can carry, and how many carry another.
fn chacha_cases(file: &Value) -> (Vec<&Value>, usize) {
    let (mut ours, mut other) = (Vec::new(), 0);
    for (g, t) in cases(file) {
        assert_eq!(g["keySize"], 256);
        if g["ivSize"] == 96 {
            ours.push(t);
        } else {
            assert!(
                !valid(t),
                "tc {}: a nonce that is not 96 bits is no RFC 8439 nonce",
                t["tcId"]
            );
            other += 1;
        }
    }
    (ours, other)
}

#[test]
fn chacha20_poly1305_answers_every_wycheproof_case() {
    let file = load("chacha20_poly1305.json");
    let (ours, other) = chacha_cases(&file);
    assert_eq!(
        (ours.len(), other),
        (316, 9),
        "every 96-bit case checked; the other nonce sizes counted"
    );
    for t in ours {
        let id = &t["tcId"];
        let key: [u8; 32] = field(t, "key").try_into().unwrap();
        let iv: [u8; 12] = field(t, "iv").try_into().unwrap();
        let (aad, msg) = (field(t, "aad"), field(t, "msg"));
        let mut sealed = field(t, "ct");
        sealed.extend(field(t, "tag"));
        if valid(t) {
            assert_eq!(
                aead::chacha20poly1305_seal(&key, &iv, &aad, &msg),
                sealed,
                "tc {id}: seal"
            );
            assert_eq!(
                aead::chacha20poly1305_open(&key, &iv, &aad, &sealed),
                Some(msg),
                "tc {id}: open"
            );
        } else {
            assert_eq!(
                aead::chacha20poly1305_open(&key, &iv, &aad, &sealed),
                None,
                "tc {id}: refused"
            );
        }
    }
}

/// `crypto.lg`'s AEAD over every 96-bit case, as one program. A frame `aeadOpen` refuses opens to the
/// empty sequence, which is also what a valid frame of an empty plaintext opens to; an invalid case
/// whose plaintext is empty therefore shows only that the refusal is empty. (Its ChaCha20 runs on the
/// eight-lane `Word32` vectors, which the C tier does not lower.)
fn chacha20_poly1305_program() -> String {
    let file = load("chacha20_poly1305.json");
    let (ours, _) = chacha_cases(&file);
    let mut program = String::from(
        "## To checkSeal (id: Int) and (k: Text) and (n: Text) and (a: Text) and (m: Text) and (s: Text) and (ok: Bool):\n    \
         Let key be bytesFromHex(k).\n    Let nonce be bytesFromHex(n).\n    Let aad be bytesFromHex(a).\n    \
         If ok is equal to true:\n        \
         If bytesToHex(aeadSeal(key, nonce, aad, bytesFromHex(m))) is not equal to s:\n            Show \"seal \" + format(id).\n        \
         If bytesToHex(aeadOpen(key, nonce, aad, bytesFromHex(s))) is not equal to m:\n            Show \"open \" + format(id).\n    \
         If ok is equal to false:\n        \
         If length of aeadOpen(key, nonce, aad, bytesFromHex(s)) is not equal to 0:\n            Show \"accepted \" + format(id).\n\n## Main\n",
    );
    for t in &ours {
        let sealed = format!("{}{}", text(t, "ct"), text(t, "tag"));
        program.push_str(&format!(
            "checkSeal({}, \"{}\", \"{}\", \"{}\", \"{}\", \"{sealed}\", {}).\n",
            t["tcId"],
            text(t, "key"),
            text(t, "iv"),
            text(t, "aad"),
            text(t, "msg"),
            valid(t)
        ));
    }
    program.push_str("Show \"checked\".\n");
    program
}

#[test]
fn the_standard_librarys_chacha20_poly1305_answers_every_wycheproof_case() {
    let run = logicaffeine_compile::compile::vm_outcome(&chacha20_poly1305_program());
    assert_eq!(run.error, None, "the program runs");
    assert_eq!(
        run.output.trim(),
        "checked",
        "every case answers as Wycheproof does"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — crypto.lg's AEAD over every Wycheproof case, compiled"]
fn the_compiled_standard_librarys_chacha20_poly1305_answers_every_wycheproof_case() {
    let run = common::run_logos_with_args(&chacha20_poly1305_program(), &[]);
    assert!(run.success, "the compiled program runs\n{}", run.stderr);
    assert_eq!(
        run.stdout.trim(),
        "checked",
        "every case answers as Wycheproof does"
    );
}

// ─── XChaCha20-Poly1305 (draft-irtf-cfrg-xchacha-03) ─────────────────────────────────────────

#[test]
fn hchacha20_derives_the_drafts_subkey() {
    let key: [u8; 32] = std::array::from_fn(|i| i as u8);
    let nonce: [u8; 16] = hex("000000090000004a0000000031415927").try_into().unwrap();
    assert_eq!(
        to_hex(&aead::hchacha20(&key, &nonce)),
        "82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc",
        "draft-irtf-cfrg-xchacha-03 §2.2.1"
    );
}

#[test]
fn xchacha20_poly1305_seals_the_drafts_example() {
    let key: [u8; 32] = std::array::from_fn(|i| 0x80 + i as u8);
    let nonce: [u8; 24] = std::array::from_fn(|i| 0x40 + i as u8);
    let aad = hex("50515253c0c1c2c3c4c5c6c7");
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let sealed = aead::xchacha20poly1305_seal(&key, &nonce, &aad, plaintext);
    assert_eq!(
        to_hex(&sealed),
        "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb\
         731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b452\
         2f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff9\
         21f9664c97637da9768812f615c68b13b52e\
         c0875924c1c7987947deafd8780acf49",
        "draft-irtf-cfrg-xchacha-03 Appendix A.1"
    );
    assert_eq!(
        aead::xchacha20poly1305_open(&key, &nonce, &aad, &sealed).as_deref(),
        Some(&plaintext[..])
    );
}

#[test]
fn xchacha20_poly1305_answers_every_wycheproof_case() {
    let file = load("xchacha20_poly1305.json");
    let (mut checked, mut other) = (0, 0);
    for (g, t) in cases(&file) {
        let id = &t["tcId"];
        assert_eq!(g["keySize"], 256);
        if g["ivSize"] != 192 {
            assert!(
                !valid(t),
                "tc {id}: a nonce that is not 192 bits is no XChaCha20 nonce"
            );
            other += 1;
            continue;
        }
        let key: [u8; 32] = field(t, "key").try_into().unwrap();
        let iv: [u8; 24] = field(t, "iv").try_into().unwrap();
        let (aad, msg) = (field(t, "aad"), field(t, "msg"));
        let mut sealed = field(t, "ct");
        sealed.extend(field(t, "tag"));
        if valid(t) {
            assert_eq!(
                aead::xchacha20poly1305_seal(&key, &iv, &aad, &msg),
                sealed,
                "tc {id}: seal"
            );
            assert_eq!(
                aead::xchacha20poly1305_open(&key, &iv, &aad, &sealed),
                Some(msg),
                "tc {id}: open"
            );
        } else {
            assert_eq!(
                aead::xchacha20poly1305_open(&key, &iv, &aad, &sealed),
                None,
                "tc {id}: refused"
            );
        }
        checked += 1;
    }
    assert_eq!(
        (checked, other),
        (306, 9),
        "every 192-bit case checked; the other nonce sizes counted"
    );
}

// ─── HMAC-SHA256 (RFC 2104) ──────────────────────────────────────────────────────────────────

/// Each HMAC case with the hex length of its tag: Wycheproof truncates some to 128 bits.
fn hmac_cases(file: &Value) -> Vec<(&Value, usize)> {
    cases(file)
        .into_iter()
        .map(|(g, t)| (t, g["tagSize"].as_u64().unwrap() as usize / 4))
        .collect()
}

#[test]
fn hmac_sha256_answers_every_wycheproof_case() {
    let file = load("hmac_sha256.json");
    for (t, tag_hex) in hmac_cases(&file) {
        let full = to_hex(&crypto_accel::hmac_sha256(
            &field(t, "key"),
            &field(t, "msg"),
        ));
        assert_eq!(
            full[..tag_hex] == text(t, "tag"),
            valid(t),
            "tc {}: the tag {} checks as Wycheproof says",
            t["tcId"],
            text(t, "tag")
        );
    }
}

#[test]
fn the_standard_librarys_hmac_sha256_answers_every_wycheproof_case() {
    let file = load("hmac_sha256.json");
    let mut program = String::from(
        "## To checkTag (id: Int) and (k: Text) and (m: Text) and (tag: Text) and (ok: Bool):\n    \
         Let full be bytesToHex(hmacSha256(bytesFromHex(k), bytesFromHex(m))).\n    \
         If startsWith(full, tag) is not equal to ok:\n        Show \"tag \" + format(id).\n\n## Main\n",
    );
    for (t, _) in hmac_cases(&file) {
        program.push_str(&format!(
            "checkTag({}, \"{}\", \"{}\", \"{}\", {}).\n",
            t["tcId"],
            text(t, "key"),
            text(t, "msg"),
            text(t, "tag"),
            valid(t)
        ));
    }
    program.push_str("Show \"checked\".\n");
    assert_eq!(
        run_c("hmac_sha256", &program),
        "checked\n",
        "every case answers as Wycheproof does"
    );
}

// ─── PBKDF2-HMAC-SHA256 (RFC 8018) ───────────────────────────────────────────────────────────

/// The PBKDF2 cases one 32-byte block answers, and how many ask for more.
fn pbkdf2_cases(file: &Value) -> (Vec<&Value>, usize) {
    let (mut ours, mut longer) = (Vec::new(), 0);
    for (_, t) in cases(file) {
        assert!(valid(t), "tc {}: every PBKDF2 case is valid", t["tcId"]);
        if t["dkLen"].as_u64().unwrap() <= 32 {
            ours.push(t);
        } else {
            longer += 1;
        }
    }
    (ours, longer)
}

#[test]
fn pbkdf2_sha256_answers_every_wycheproof_case_of_one_block() {
    let file = load("pbkdf2_hmac_sha256.json");
    let (ours, longer) = pbkdf2_cases(&file);
    assert_eq!(
        (ours.len(), longer),
        (26, 34),
        "every one-block case checked; the longer outputs counted"
    );
    for t in ours {
        let iters = u32::try_from(t["iterationCount"].as_u64().unwrap()).unwrap();
        let dk = crypto_accel::pbkdf2_sha256(&field(t, "password"), &field(t, "salt"), iters);
        let want = field(t, "dk");
        assert_eq!(&dk[..want.len()], &want[..], "tc {}", t["tcId"]);
    }
}

#[test]
fn the_standard_librarys_pbkdf2_sha256_answers_every_wycheproof_case_of_one_block() {
    let file = load("pbkdf2_hmac_sha256.json");
    let (ours, _) = pbkdf2_cases(&file);
    let mut program = String::from(
        "## To checkKey (id: Int) and (p: Text) and (s: Text) and (iters: Int) and (dk: Text):\n    \
         Let full be bytesToHex(pbkdf2Sha256(bytesFromHex(p), bytesFromHex(s), iters)).\n    \
         If startsWith(full, dk) is not equal to true:\n        Show \"key \" + format(id).\n\n## Main\n",
    );
    for t in ours {
        program.push_str(&format!(
            "checkKey({}, \"{}\", \"{}\", {}, \"{}\").\n",
            t["tcId"],
            text(t, "password"),
            text(t, "salt"),
            t["iterationCount"],
            text(t, "dk")
        ));
    }
    program.push_str("Show \"checked\".\n");
    assert_eq!(
        run_c("pbkdf2_hmac_sha256", &program),
        "checked\n",
        "every case answers as Wycheproof does"
    );
}
