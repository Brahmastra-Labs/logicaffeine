//! Stdlib-parity — integer byte packing (`packUintBE`/`packUintLE`/`unpackUintBE`/`unpackUintLE`):
//! the integer case of Python `struct.pack`/`struct.unpack` with `>`/`<` byte order. All-integer
//! (shift + mask), verified against CPython `struct`. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Big-endian integer pack. py: struct.pack(">…", v)
#[test]
fn parity_pack_uint_be() {
    // struct.pack(">I", 1000) = [0,0,3,232]; struct.pack(">H", 255) = [0,255]
    let src = "## Main\n\
        Show item 1 of packUintBE(1000, 4).\n\
        Show item 2 of packUintBE(1000, 4).\n\
        Show item 3 of packUintBE(1000, 4).\n\
        Show item 4 of packUintBE(1000, 4).\n\
        Show item 1 of packUintBE(255, 2).\n\
        Show item 2 of packUintBE(255, 2).\n";
    assert_all_tiers(src, "0\n0\n3\n232\n0\n255");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "0\n0\n3\n232\n0\n255");
}

/// Little-endian integer pack. py: struct.pack("<…", v)
#[test]
fn parity_pack_uint_le() {
    // struct.pack("<I", 1000) = [232,3,0,0]; struct.pack("<H", 258) = [2,1]
    let src = "## Main\n\
        Show item 1 of packUintLE(1000, 4).\n\
        Show item 2 of packUintLE(1000, 4).\n\
        Show item 3 of packUintLE(1000, 4).\n\
        Show item 4 of packUintLE(1000, 4).\n\
        Show item 1 of packUintLE(258, 2).\n\
        Show item 2 of packUintLE(258, 2).\n";
    assert_all_tiers(src, "232\n3\n0\n0\n2\n1");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "232\n3\n0\n0\n2\n1");
}

/// Big-endian integer unpack + round-trip. py: struct.unpack(">…", …)
#[test]
fn parity_unpack_uint_be() {
    // struct.unpack(">I", b"\x01\x02\x03\x04") = 16909060
    let src = "## Main\n\
        Show unpackUintBE([1, 2, 3, 4]).\n\
        Show unpackUintBE(packUintBE(70000, 4)).\n";
    assert_all_tiers(src, "16909060\n70000");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "16909060\n70000");
}

/// Little-endian integer unpack + round-trip. py: struct.unpack("<…", …)
#[test]
fn parity_unpack_uint_le() {
    // struct.unpack("<I", b"\x01\x02\x03\x04") = 67305985
    let src = "## Main\n\
        Show unpackUintLE([1, 2, 3, 4]).\n\
        Show unpackUintLE(packUintLE(70000, 4)).\n";
    assert_all_tiers(src, "67305985\n70000");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "67305985\n70000");
}
