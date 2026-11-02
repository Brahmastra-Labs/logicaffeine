//! Stdlib-parity — `b2a_qp` (Python binascii.b2a_qp: quoted-printable encode) and `a2b_qp`
//! (decode). CPython-verified: `a=b`→`a=3Db`, `café`→`caf=C3=A9`. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Quoted-printable encode: escape `=` and non-printable/high bytes as `=XX`. py: binascii.b2a_qp
#[test]
fn parity_b2a_qp() {
    let src = "## Main\n\
        Show b2a_qp(text_bytes(\"a=b\")).\n\
        Show b2a_qp(text_bytes(\"café\")).\n";
    assert_all_tiers(src, "a=3Db\ncaf=C3=A9");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a=3Db\ncaf=C3=A9");
}

/// Quoted-printable decode via round-trip through the encoder. py: binascii.a2b_qp
#[test]
fn parity_a2b_qp() {
    let src = "## Main\n\
        Show text_from_bytes(a2b_qp(b2a_qp(text_bytes(\"a=b 100%\")))).\n\
        Show text_from_bytes(a2b_qp(b2a_qp(text_bytes(\"café\")))).\n";
    assert_all_tiers(src, "a=b 100%\ncafé");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "a=b 100%\ncafé");
}
