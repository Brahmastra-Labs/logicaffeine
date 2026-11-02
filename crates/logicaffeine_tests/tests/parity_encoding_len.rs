//! Stdlib-parity — base64/hex output-length calculators (Go encoding/base64.EncodedLen/DecodedLen,
//! encoding/hex.EncodedLen/DecodedLen). Pure Int arithmetic; byte-identical all tiers.
#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_all_tiers, assert_c_output};

#[test]
fn parity_base64_encoded_len() {
    // 3 bytes -> 4, 1 -> 4, 4 -> 8, 0 -> 0
    assert_all_tiers("## Main\nShow base64EncodedLen(3).\nShow base64EncodedLen(1).\nShow base64EncodedLen(4).\nShow base64EncodedLen(0).\n", "4\n4\n8\n0");
}
#[test]
fn parity_base64_decoded_len() {
    assert_all_tiers("## Main\nShow base64DecodedLen(4).\nShow base64DecodedLen(8).\nShow base64DecodedLen(0).\n", "3\n6\n0");
}
#[test]
fn parity_hex_encoded_len() {
    assert_all_tiers(
        "## Main\nShow hexEncodedLen(3).\nShow hexEncodedLen(0).\n",
        "6\n0",
    );
}
#[test]
fn parity_hex_decoded_len() {
    assert_all_tiers(
        "## Main\nShow hexDecodedLen(6).\nShow hexDecodedLen(0).\n",
        "3\n0",
    );
}
#[test]
fn encoding_len_on_c() {
    assert_c_output(
        "## Main\nShow base64EncodedLen(5).\nShow hexEncodedLen(4).\nShow base64DecodedLen(12).\n",
        "8\n8\n9",
    );
}
