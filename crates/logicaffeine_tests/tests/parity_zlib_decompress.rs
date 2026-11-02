//! Stdlib-parity — `zlibDecompress` (Python zlib.decompress): a complete from-scratch DEFLATE
//! inflater over the zlib wrapper — STORED, FIXED-Huffman, and DYNAMIC-Huffman blocks (LZ77
//! back-references, canonical-Huffman decoding). Verified against CPython zlib.compress across all
//! three block types. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Inflate a zlib stream: stored, fixed, and dynamic Huffman. py: zlib.decompress
#[test]
fn parity_zlib_decompress() {
    let src = "## Main\n\
        Show text_from_bytes(zlibDecompress([120, 156, 203, 72, 205, 201, 201, 7, 0, 6, 44, 2, 21])).\n\
        Show text_from_bytes(zlibDecompress([120, 156, 75, 76, 132, 1, 0, 20, 225, 3, 203])).\n\
        Show text_from_bytes(zlibDecompress([120, 1, 1, 5, 0, 250, 255, 104, 101, 108, 108, 111, 6, 44, 2, 21])).\n\
        Show length of zlibDecompress([120, 156, 13, 207, 183, 1, 4, 49, 8, 0, 193, 90, 145, 119, 232, 4, 200, 82, 253, 127, 182, 193, 36, 235, 69, 159, 205, 110, 188, 111, 44, 212, 232, 6, 96, 159, 239, 193, 249, 242, 213, 36, 174, 2, 192, 98, 192, 29, 251, 5, 74, 127, 198, 169, 165, 253, 143, 2, 157, 151, 11, 235, 22, 119, 235, 61, 212, 105, 199, 82, 100, 16, 138, 25, 233, 42, 246, 29, 26, 159, 183, 111, 179, 223, 38, 247, 2, 97, 27, 23, 134, 41, 103, 138, 224, 10, 129, 18, 188, 200, 156, 144, 154, 180, 47, 111, 158, 23, 80, 47, 169, 167, 199, 177, 155, 209, 132, 35, 245, 209, 122, 3, 230, 169, 179, 126, 19, 52, 173, 192, 18, 172, 178, 102, 179, 173, 133, 15, 222, 203, 41, 59, 157, 161, 21, 27, 66, 166, 176, 243, 58, 229, 59, 117, 12, 7, 5, 107, 215, 152, 93, 190, 20, 103, 135, 4, 232, 205, 13, 223, 161, 221, 57, 45, 61, 244, 37, 90, 255, 65, 209, 186, 87, 55, 183, 248, 104, 138, 181, 165, 220, 208, 37, 123, 96, 35, 81, 190, 240, 206, 36, 131, 177, 185, 40, 187, 75, 28, 110, 99, 161, 1, 117, 98, 129, 16, 171, 138, 250, 218, 99, 222, 14, 185, 109, 30, 143, 147, 189, 198, 250, 16, 56, 230, 87, 39, 229, 86, 171, 43, 105, 190, 51, 188, 240, 115, 213, 116, 139, 234, 125, 117, 83, 20, 173, 112, 18, 155, 91, 249, 1, 159, 179, 171, 57]).\n";
    assert_all_tiers(src, "hello\naaaaaaaaaa\nhello\n400");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "hello\naaaaaaaaaa\nhello\n400");
}
