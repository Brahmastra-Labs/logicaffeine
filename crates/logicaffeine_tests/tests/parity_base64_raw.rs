//! Stdlib-parity — unpadded base64: base64RawEncode/base64RawUrlEncode (Go base64.RawStdEncoding/
//! RawURLEncoding = std/url base64 with '=' padding stripped). Values AUTHORITATIVE (Python).
#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_all_tiers, assert_c_output};

const PROG: &str = "## Main
Show base64RawEncode(\"A\").
Show base64RawEncode(\"hello\").
Show base64RawUrlEncode(\"hello\").
";
const EXPECT: &str = "QQ\naGVsbG8\naGVsbG8";

#[test]
fn parity_base64_raw() {
    assert_all_tiers(PROG, EXPECT);
}
#[test]
fn base64_raw_on_c() {
    assert_c_output(PROG, EXPECT);
}
