//! SHA-2 family lock: pure-LOGOS `sha256`, `hmacSha256`, and `pbkdf2Sha256` in the
//! stdlib (`assets/std/crypto.lg`), pinned to the official vectors — FIPS 180-4 for
//! SHA-256, RFC 4231 for HMAC-SHA256, and the RFC 7677 SCRAM-SHA-256 example for
//! PBKDF2 (`Hi("pencil", …, 4096)`). These are the primitives SCRAM-SHA-256 Postgres
//! authentication is built on, so every digest here is compared byte-for-byte (as
//! lowercase hex via `bytesToHex`) across the tree-walker AND the VM, with the same
//! vectors locked through the C AOT tier at the end. Every expected value below was
//! independently recomputed with python3 `hashlib`/`hmac` before being pinned.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_all_tiers, assert_c_output};

// ---------------------------------------------------------------------------
// SHA-256 — FIPS 180-4 vectors
// ---------------------------------------------------------------------------

/// FIPS 180-4: SHA-256 of the empty message.
#[test]
fn sha256_empty_matches_fips() {
    assert_all_tiers(
        "## Main\n\
         Let msg be a new Seq of Int.\n\
         Show bytesToHex(sha256(msg)).\n",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
}

/// FIPS 180-4: SHA-256("abc") — the one-block reference vector.
#[test]
fn sha256_abc_matches_fips() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(sha256(text_bytes(\"abc\"))).\n",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
}

/// FIPS 180-4: the 56-byte (448-bit) two-block vector — padding spills into a second block.
#[test]
fn sha256_two_block_matches_fips() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(sha256(text_bytes(\"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq\"))).\n",
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
}

/// 55 bytes of 'a': the largest message whose padding (0x80 + 8-length bytes) still
/// fits in ONE block — the exact edge below the 448-bit boundary. Expected value
/// computed with python3 hashlib.
#[test]
fn sha256_55_byte_single_block_boundary() {
    assert_all_tiers(
        "## Main\n\
         Let mutable msg be a new Seq of Int.\n\
         Repeat for i from 1 to 55:\n    \
             Push 97 to msg.\n\
         Show bytesToHex(sha256(msg)).\n",
        "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
    );
}

/// 64 bytes of 'a': a message that fills a block EXACTLY, forcing an all-padding
/// second block. Expected value computed with python3 hashlib.
#[test]
fn sha256_64_byte_exact_block() {
    assert_all_tiers(
        "## Main\n\
         Let mutable msg be a new Seq of Int.\n\
         Repeat for i from 1 to 64:\n    \
             Push 97 to msg.\n\
         Show bytesToHex(sha256(msg)).\n",
        "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
    );
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 — RFC 4231 test cases
// ---------------------------------------------------------------------------

/// RFC 4231 test case 1: key = 0x0b × 20, data = "Hi There".
#[test]
fn hmac_sha256_rfc4231_case1() {
    assert_all_tiers(
        "## Main\n\
         Let mutable key be a new Seq of Int.\n\
         Repeat for i from 1 to 20:\n    \
             Push 11 to key.\n\
         Show bytesToHex(hmacSha256(key, text_bytes(\"Hi There\"))).\n",
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
    );
}

/// RFC 4231 test case 2: key = "Jefe" (shorter than the block), data = the question.
#[test]
fn hmac_sha256_rfc4231_case2() {
    assert_all_tiers(
        "## Main\n\
         Show bytesToHex(hmacSha256(text_bytes(\"Jefe\"), text_bytes(\"what do ya want for nothing?\"))).\n",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    );
}

/// RFC 4231 test case 3: key = 0xaa × 20, data = 0xdd × 50.
#[test]
fn hmac_sha256_rfc4231_case3() {
    assert_all_tiers(
        "## Main\n\
         Let mutable key be a new Seq of Int.\n\
         Repeat for i from 1 to 20:\n    \
             Push 170 to key.\n\
         Let mutable data be a new Seq of Int.\n\
         Repeat for i from 1 to 50:\n    \
             Push 221 to data.\n\
         Show bytesToHex(hmacSha256(key, data)).\n",
        "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
    );
}

/// RFC 4231 test case 4: key = bytes 0x01..0x19, data = 0xcd × 50.
#[test]
fn hmac_sha256_rfc4231_case4() {
    assert_all_tiers(
        "## Main\n\
         Let mutable key be a new Seq of Int.\n\
         Repeat for i from 1 to 25:\n    \
             Push i to key.\n\
         Let mutable data be a new Seq of Int.\n\
         Repeat for i from 1 to 50:\n    \
             Push 205 to data.\n\
         Show bytesToHex(hmacSha256(key, data)).\n",
        "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
    );
}

/// RFC 4231 test case 6: key = 0xaa × 131 — LONGER than the 64-byte block, so HMAC
/// must first hash the key down to 32 bytes.
#[test]
fn hmac_sha256_rfc4231_case6_key_longer_than_block() {
    assert_all_tiers(
        "## Main\n\
         Let mutable key be a new Seq of Int.\n\
         Repeat for i from 1 to 131:\n    \
             Push 170 to key.\n\
         Show bytesToHex(hmacSha256(key, text_bytes(\"Test Using Larger Than Block-Size Key - Hash Key First\"))).\n",
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
    );
}

// ---------------------------------------------------------------------------
// PBKDF2-HMAC-SHA256 — SCRAM's Hi(), dkLen fixed at 32
// ---------------------------------------------------------------------------

/// The RFC 7677 salt `W22ZaJ0SNY7soEsUEjb6gQ==` base64-decoded to bytes, as a LOGOS
/// literal list.
const SCRAM_SALT: &str =
    "[91, 109, 153, 104, 157, 18, 53, 142, 236, 160, 75, 20, 18, 54, 250, 129]";

/// PBKDF2-HMAC-SHA256("pencil", RFC 7677 salt, 1 iteration) — U1 alone. Expected
/// value computed with python3 hashlib.pbkdf2_hmac.
#[test]
fn pbkdf2_sha256_one_iteration() {
    assert_all_tiers(
        &format!(
            "## Main\n\
             Let salt be {SCRAM_SALT}.\n\
             Show bytesToHex(pbkdf2Sha256(text_bytes(\"pencil\"), salt, 1)).\n"
        ),
        "fcfc563c4b0220bf424e06d9117416df9c73092e7fabe25a0aad51586527408e",
    );
}

/// PBKDF2-HMAC-SHA256("pencil", RFC 7677 salt, 2 iterations) — the first real XOR
/// chain step. Expected value computed with python3 hashlib.pbkdf2_hmac.
#[test]
fn pbkdf2_sha256_two_iterations() {
    assert_all_tiers(
        &format!(
            "## Main\n\
             Let salt be {SCRAM_SALT}.\n\
             Show bytesToHex(pbkdf2Sha256(text_bytes(\"pencil\"), salt, 2)).\n"
        ),
        "dfa991a6c2725659ae5f5d73579e02e10f5d295f8700c4db23a76c1ff0739216",
    );
}

/// The full RFC 7677 SCRAM-SHA-256 example: SaltedPassword = Hi("pencil", salt, 4096).
/// Expected value confirmed with python3 hashlib.pbkdf2_hmac (matches the RFC).
#[test]
fn pbkdf2_sha256_rfc7677_scram_pencil_4096() {
    assert_all_tiers(
        &format!(
            "## Main\n\
             Let salt be {SCRAM_SALT}.\n\
             Show bytesToHex(pbkdf2Sha256(text_bytes(\"pencil\"), salt, 4096)).\n"
        ),
        "c4a49510323ab4f952cac1fa99441939e78ea74d6be81ddf7096e87513dc615d",
    );
}

// ---------------------------------------------------------------------------
// AOT coverage — the compiled tier here is the C backend, whose unit-reachability
// tree-shake emits exactly the functions the program calls, and the digest is
// locked to the SAME official vector bytes as the interpreted tiers above.
// ---------------------------------------------------------------------------

/// SHA-256 two-block FIPS vector through the C AOT tier.
#[test]
fn sha256_two_block_c_aot_matches_fips() {
    assert_c_output(
        "## Main\n\
         Show bytesToHex(sha256(text_bytes(\"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq\"))).\n",
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
    );
}

/// HMAC-SHA256 RFC 4231 case 6 (key longer than the block) through the C AOT tier.
#[test]
fn hmac_sha256_long_key_c_aot_matches_rfc4231() {
    assert_c_output(
        "## Main\n\
         Let mutable key be a new Seq of Int.\n\
         Repeat for i from 1 to 131:\n    \
             Push 170 to key.\n\
         Show bytesToHex(hmacSha256(key, text_bytes(\"Test Using Larger Than Block-Size Key - Hash Key First\"))).\n",
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
    );
}

/// PBKDF2-HMAC-SHA256 (2 iterations) through the C AOT tier.
#[test]
fn pbkdf2_sha256_c_aot_matches_python_vector() {
    assert_c_output(
        &format!(
            "## Main\n\
             Let salt be {SCRAM_SALT}.\n\
             Show bytesToHex(pbkdf2Sha256(text_bytes(\"pencil\"), salt, 2)).\n"
        ),
        "dfa991a6c2725659ae5f5d73579e02e10f5d295f8700c4db23a76c1ff0739216",
    );
}
