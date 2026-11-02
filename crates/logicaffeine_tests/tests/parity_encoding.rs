//! Stdlib-parity north-star corpus — domain: encoding
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Parse JSON text into a value tree.
/// provenance — python: json.loads · go: encoding/json.Unmarshal
/// LOGOS surface: `the JSON parsed from s`
#[test]
fn parity_json_parse() {
    assert_all_tiers(
        "## Main
Show jsonSerialize(jsonParse(\"[1,2,3]\")).
Show jsonSerialize(jsonParse(\"42\")).
Show jsonSerialize(jsonParse(\"[true,null]\")).
",
        "[1,2,3]
42
[true,null]",
    );
}

/// Serialize a value tree to JSON text.
/// provenance — python: json.dumps · go: encoding/json.Marshal
/// LOGOS surface: `the JSON text of v`
#[test]
fn parity_json_serialize() {
    assert_all_tiers(
        "## Main
Let arr be a new Seq of Json.
Push (a new JNum with n 1) to arr.
Push (a new JNum with n 2) to arr.
Push (a new JBool with b true) to arr.
Let j be a new JArr with elems arr.
Show jsonSerialize(j).
",
        "[1,2,true]",
    );
}

/// Hex-encode a string's bytes (lowercase). py: binascii.hexlify · go: encoding/hex.EncodeToString
#[test]
fn parity_encoding_hex_encode() {
    assert_all_tiers(
        "## Main\nShow toHex(\"AB\").\nShow toHex(\"hi!\").\n",
        "4142\n686921",
    );
}
/// Hex-decode. py: binascii.unhexlify · go: encoding/hex.DecodeString
#[test]
fn parity_encoding_hex_decode() {
    assert_all_tiers(
        "## Main\nShow fromHex(\"4142\").\nShow fromHex(toHex(\"round!\")).\n",
        "AB\nround!",
    );
}

/// Standard Base64 encoding (RFC 4648, with `=` padding). py: base64.b64encode · go: base64.StdEncoding
#[test]
fn parity_encoding_base64() {
    assert_all_tiers(
        "## Main\nShow base64Encode(\"Man\").\nShow base64Encode(\"Ma\").\nShow base64Encode(\"M\").\nShow base64Encode(\"Hello\").\n",
        "TWFu\nTWE=\nTQ==\nSGVsbG8=",
    );
}

/// Standard Base64 decoding (RFC 4648). py: base64.b64decode
#[test]
fn parity_encoding_base64_decode() {
    assert_all_tiers(
        "## Main\nShow base64Decode(\"TWFu\").\nShow base64Decode(\"TWE=\").\nShow base64Decode(\"TQ==\").\nShow base64Decode(base64Encode(\"round-trip!\")).\n",
        "Man\nMa\nM\nround-trip!",
    );
}

/// Standard Base32 encoding (RFC 4648, with `=` padding). py: base64.b32encode · go: encoding/base32.StdEncoding
/// The five RFC 4648 §10 test vectors exercise every padding length (0/1/3/4/6 `=`).
#[test]
fn parity_encoding_base32() {
    assert_all_tiers(
        "## Main\nShow base32Encode(\"f\").\nShow base32Encode(\"fo\").\nShow base32Encode(\"foo\").\nShow base32Encode(\"foob\").\nShow base32Encode(\"fooba\").\nShow base32Encode(\"foobar\").\n",
        "MY======\nMZXQ====\nMZXW6===\nMZXW6YQ=\nMZXW6YTB\nMZXW6YTBOI======",
    );
}

/// Standard Base32 decoding (RFC 4648). py: base64.b32decode · go: encoding/base32.DecodeString
#[test]
fn parity_encoding_base32_decode() {
    assert_all_tiers(
        "## Main\nShow base32Decode(\"MY======\").\nShow base32Decode(\"MZXW6===\").\nShow base32Decode(\"MZXW6YTBOI======\").\nShow base32Decode(base32Encode(\"round-trip base32!\")).\n",
        "f\nfoo\nfoobar\nround-trip base32!",
    );
}

/// Extended-hex Base32 encoding (RFC 4648 §7, alphabet `0-9A-V` — order-preserving). py: base64.b32hexencode
#[test]
fn parity_encoding_base32hex() {
    assert_all_tiers(
        "## Main\nShow base32hexEncode(\"f\").\nShow base32hexEncode(\"foo\").\nShow base32hexEncode(\"foob\").\nShow base32hexEncode(\"foobar\").\nShow base32hexEncode(\"Hello, World!\").\n",
        "CO======\nCPNMU===\nCPNMUOG=\nCPNMUOJ1E8======\n91IMOR3F5GG5ERRIDHI22===",
    );
}

/// Extended-hex Base32 decoding (RFC 4648 §7). py: base64.b32hexdecode
#[test]
fn parity_encoding_base32hex_decode() {
    assert_all_tiers(
        "## Main\nShow base32hexDecode(\"CO======\").\nShow base32hexDecode(\"CPNMUOJ1E8======\").\nShow base32hexDecode(base32hexEncode(\"b32hex round-trip!\")).\n",
        "f\nfoobar\nb32hex round-trip!",
    );
}

/// Ascii85 encoding (Adobe/`btoa` radix-85, `z` for a zero group; no `<~ ~>` wrapper).
/// Reference outputs are `python3 base64.a85encode`. py: base64.a85encode · go: encoding/ascii85.Encode
#[test]
fn parity_encoding_ascii85() {
    assert_all_tiers(
        "## Main\nShow ascii85Encode(\"f\").\nShow ascii85Encode(\"fo\").\nShow ascii85Encode(\"foob\").\nShow ascii85Encode(\"fooba\").\nShow ascii85Encode(\"foobar\").\nShow ascii85Encode(\"Hello, World!\").\n",
        "Ac\nAo@\nAoDTs\nAoDTs@/\nAoDTs@<)\n87cURD_*#4DfTZ)+T",
    );
}

/// Ascii85 decoding — reverse of `ascii85Encode`, expanding `z` to four zero bytes.
/// py: base64.a85decode · go: encoding/ascii85.Decode
#[test]
fn parity_encoding_ascii85_decode() {
    assert_all_tiers(
        "## Main\nShow ascii85Decode(\"AoDTs\").\nShow ascii85Decode(\"87cURD_*#4DfTZ)+T\").\nShow ascii85Decode(ascii85Encode(\"ascii85 round-trip!\")).\n",
        "foob\nHello, World!\nascii85 round-trip!",
    );
}

/// Base85 encoding (RFC 1924 / `base64.b85encode` alphabet — no `z`, no padding chars).
/// Reference outputs are `python3 base64.b85encode`. py: base64.b85encode
#[test]
fn parity_encoding_base85() {
    assert_all_tiers(
        "## Main\nShow base85Encode(\"f\").\nShow base85Encode(\"fo\").\nShow base85Encode(\"foob\").\nShow base85Encode(\"fooba\").\nShow base85Encode(\"foobar\").\nShow base85Encode(\"Hello, World!\").\n",
        "W&\nW^V\nW^Zp|\nW^Zp|VE\nW^Zp|VR8\nNM&qnZ!92JZ*pv8Ap",
    );
}

/// Base85 decoding — reverse of `base85Encode`. py: base64.b85decode
#[test]
fn parity_encoding_base85_decode() {
    assert_all_tiers(
        "## Main\nShow base85Decode(\"W^Zp|\").\nShow base85Decode(\"NM&qnZ!92JZ*pv8Ap\").\nShow base85Decode(base85Encode(\"base85 round-trip!\")).\n",
        "foob\nHello, World!\nbase85 round-trip!",
    );
}

/// Base16 encoding (UPPERCASE hex, RFC 4648 §8). py: base64.b16encode
#[test]
fn parity_encoding_base16() {
    assert_all_tiers(
        "## Main\nShow base16Encode(\"f\").\nShow base16Encode(\"foo\").\nShow base16Encode(\"Hello?>\").\n",
        "66\n666F6F\n48656C6C6F3F3E",
    );
}

/// URL-safe Base64 (RFC 4648 §5 — `-`/`_` for `+`/`/`, `=` padding kept). py: base64.urlsafe_b64encode · go: encoding/base64.URLEncoding
#[test]
fn parity_encoding_base64url() {
    assert_all_tiers(
        "## Main\nShow base64UrlEncode(\"foo\").\nShow base64UrlEncode(\">>>\").\nShow base64UrlEncode(\"???\").\nShow base64UrlEncode(\"Hello?>\").\n",
        "Zm9v\nPj4-\nPz8_\nSGVsbG8_Pg==",
    );
}

/// URL-safe Base64 decoding. py: base64.urlsafe_b64decode
#[test]
fn parity_encoding_base64url_decode() {
    assert_all_tiers(
        "## Main\nShow base64UrlDecode(\"Pj4-\").\nShow base64UrlDecode(\"Pz8_\").\nShow base64UrlDecode(base64UrlEncode(\"url-safe round-trip?>\")).\n",
        ">>>\n???\nurl-safe round-trip?>",
    );
}
