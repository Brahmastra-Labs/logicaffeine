//! Stdlib-parity — crcHqx: CRC-CCITT (XMODEM, poly 0x1021), Python binascii.crc_hqx.
//! Bit-by-bit over Int arithmetic (xor + shift + mask). Values AUTHORITATIVE (Python binascii).
#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_all_tiers, assert_c_output};

const PROG: &str = "## Main
Let mutable a be a new Seq of Int.
Push 65 to a.
Show crcHqx(a, 0).
Let mutable nine be a new Seq of Int.
Repeat for c from 49 to 57:
    Push c to nine.
Show crcHqx(nine, 0).
Let mutable h be a new Seq of Int.
Push 104 to h.
Push 101 to h.
Push 108 to h.
Push 108 to h.
Push 111 to h.
Show crcHqx(h, 0).
";
// crc_hqx: "A"=22757, "123456789"=12739, "hello"=50018
const EXPECT: &str = "22757\n12739\n50018";

#[test]
fn parity_crc_hqx() {
    assert_all_tiers(PROG, EXPECT);
}

#[test]
fn crc_hqx_on_c() {
    assert_c_output(PROG, EXPECT);
}
