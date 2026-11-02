//! Stdlib-parity north-star corpus — domain: bytes
//!
//! Sequence-level byte operations of the Go `bytes` package (and Rust slice methods),
//! realized in pure LOGOS (`bytes.lg`) over `Seq of Int` and proven byte-identical on every
//! execution tier. These are the operations the element-wise iterators (`iter.lg`) do not
//! cover — subsequence search, prefix/suffix tests, lexicographic comparison, byte trimming.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Element-wise sequence equality. go: bytes.Equal · rust: slice PartialEq
#[test]
fn parity_bytes_equal() {
    assert_all_tiers(
        "## Main\nShow bytesEqual([1, 2, 3], [1, 2, 3]).\nShow bytesEqual([1, 2], [1, 3]).\nShow bytesEqual([1], [1, 2]).\n",
        "true\nfalse\nfalse",
    );
}

/// Lexicographic three-way comparison (-1/0/1). go: bytes.Compare
#[test]
fn parity_bytes_compare() {
    assert_all_tiers(
        "## Main\nShow bytesCompare([1, 2], [1, 3]).\nShow bytesCompare([1, 2, 3], [1, 2]).\nShow bytesCompare([5], [5]).\n",
        "-1\n1\n0",
    );
}

/// 1-based index of the first occurrence of a subsequence (0 if absent). go: bytes.Index
#[test]
fn parity_bytes_index_sub() {
    assert_all_tiers(
        "## Main\nShow bytesIndexSub([1, 2, 3, 4], [3, 4]).\nShow bytesIndexSub([1, 2, 3, 4], [5]).\n",
        "3\n0",
    );
}

/// Whether a subsequence occurs anywhere. go: bytes.Contains
#[test]
fn parity_bytes_contains_sub() {
    assert_all_tiers(
        "## Main\nShow bytesContainsSub([1, 2, 3], [2, 3]).\nShow bytesContainsSub([1, 2, 3], [3, 2]).\n",
        "true\nfalse",
    );
}

/// Whether the sequence begins with a given prefix. go: bytes.HasPrefix
#[test]
fn parity_bytes_has_prefix() {
    assert_all_tiers(
        "## Main\nShow bytesHasPrefix([1, 2, 3], [1, 2]).\nShow bytesHasPrefix([1, 2, 3], [2]).\n",
        "true\nfalse",
    );
}

/// Whether the sequence ends with a given suffix. go: bytes.HasSuffix
#[test]
fn parity_bytes_has_suffix() {
    assert_all_tiers(
        "## Main\nShow bytesHasSuffix([1, 2, 3], [2, 3]).\nShow bytesHasSuffix([1, 2, 3], [1]).\n",
        "true\nfalse",
    );
}

/// Count occurrences of a single byte. go: bytes.Count
#[test]
fn parity_bytes_count_byte() {
    assert_all_tiers(
        "## Main\nShow bytesCountByte([1, 2, 2, 3, 2], 2).\nShow bytesCountByte([1, 2, 3], 9).\n",
        "3\n0",
    );
}

/// Trim a leading/trailing byte from both ends. go: bytes.Trim
#[test]
fn parity_bytes_trim_byte() {
    assert_all_tiers(
        "## Main\nShow bytesTrimByte([0, 0, 5, 7, 0], 0).\nShow bytesTrimByte([5, 7], 0).\n",
        "[5, 7]\n[5, 7]",
    );
}

/// 1-based index of a single byte (0 if absent). go: bytes.IndexByte
#[test]
fn parity_bytes_index_byte() {
    assert_all_tiers(
        "## Main\nShow bytesIndexByte([5, 7, 9], 9).\nShow bytesIndexByte([5, 7], 3).\n",
        "3\n0",
    );
}

/// 1-based index of the LAST subsequence occurrence (0 if absent). go: bytes.LastIndex
#[test]
fn parity_bytes_last_index_sub() {
    assert_all_tiers(
        "## Main\nShow bytesLastIndexSub([1, 2, 1, 2], [1, 2]).\nShow bytesLastIndexSub([1, 2, 3], [9]).\n",
        "3\n0",
    );
}

/// Remove a leading prefix if present (else unchanged). go: bytes.TrimPrefix
#[test]
fn parity_bytes_trim_prefix() {
    assert_all_tiers(
        "## Main\nShow bytesTrimPrefix([1, 2, 3], [1, 2]).\nShow bytesTrimPrefix([1, 2, 3], [9]).\n",
        "[3]\n[1, 2, 3]",
    );
}

/// Remove a trailing suffix if present (else unchanged). go: bytes.TrimSuffix
#[test]
fn parity_bytes_trim_suffix() {
    assert_all_tiers(
        "## Main\nShow bytesTrimSuffix([1, 2, 3], [2, 3]).\nShow bytesTrimSuffix([1, 2, 3], [9]).\n",
        "[1]\n[1, 2, 3]",
    );
}

/// Replace every occurrence of one byte with another. go: bytes.Replace / bytes.Map
#[test]
fn parity_bytes_replace_byte() {
    assert_all_tiers(
        "## Main\nShow bytesReplaceByte([1, 2, 1, 3], 1, 9).\n",
        "[9, 2, 9, 3]",
    );
}

/// Concatenate a byte sequence with itself n times. go: bytes.Repeat
#[test]
fn parity_bytes_repeat() {
    assert_all_tiers(
        "## Main\nShow bytesRepeat([1, 2], 3).\nShow bytesRepeat([5], 0).\n",
        "[1, 2, 1, 2, 1, 2]\n[]",
    );
}

/// Uppercase every ASCII byte. go: bytes.ToUpper
#[test]
fn parity_bytes_to_upper() {
    assert_all_tiers(
        "## Main\nShow bytesToUpper([104, 105, 33]).\n",
        "[72, 73, 33]",
    );
}

/// Lowercase every ASCII byte. go: bytes.ToLower
#[test]
fn parity_bytes_to_lower() {
    assert_all_tiers(
        "## Main\nShow bytesToLower([72, 73, 33]).\n",
        "[104, 105, 33]",
    );
}

/// Trim leading/trailing whitespace bytes. go: bytes.TrimSpace
#[test]
fn parity_bytes_trim_space() {
    assert_all_tiers(
        "## Main\nShow bytesTrimSpace([32, 9, 65, 66, 10]).\nShow bytesTrimSpace([65]).\n",
        "[65, 66]\n[65]",
    );
}

/// Trim a byte from the left only. go: bytes.TrimLeft
#[test]
fn parity_bytes_trim_left() {
    assert_all_tiers(
        "## Main\nShow bytesTrimLeft([48, 48, 53, 48], 48).\n",
        "[53, 48]",
    );
}

/// Trim a byte from the right only. go: bytes.TrimRight
#[test]
fn parity_bytes_trim_right() {
    assert_all_tiers(
        "## Main\nShow bytesTrimRight([48, 53, 48, 48], 48).\n",
        "[48, 53]",
    );
}

/// Lowercase hex encoding of a byte sequence. go: encoding/hex.EncodeToString · py: bytes.hex / binascii.b2a_hex
#[test]
fn parity_bytes_to_hex() {
    assert_all_tiers(
        "## Main\nShow bytesToHex([255, 0, 171]).\nShow bytesToHex([72, 105]).\n",
        "ff00ab\n4869",
    );
}

/// Decode a lowercase/uppercase hex string to bytes. go: encoding/hex.DecodeString · py: bytes.fromhex / binascii.a2b_hex
#[test]
fn parity_bytes_from_hex() {
    assert_all_tiers(
        "## Main\nShow bytesFromHex(\"ff00ab\").\nShow bytesFromHex(\"48656C6C6F\").\n",
        "[255, 0, 171]\n[72, 101, 108, 108, 111]",
    );
}

/// Whether every byte is an ASCII digit (empty → false). py: bytes.isdigit
#[test]
fn parity_bytes_is_digit() {
    assert_all_tiers(
        "## Main\nShow bytesIsDigit([49, 50, 51]).\nShow bytesIsDigit([49, 97]).\nShow bytesIsDigit([]).\n",
        "true\nfalse\nfalse",
    );
}

/// Whether every byte is an ASCII letter. py: bytes.isalpha
#[test]
fn parity_bytes_is_alpha() {
    assert_all_tiers(
        "## Main\nShow bytesIsAlpha([72, 105]).\nShow bytesIsAlpha([72, 49]).\n",
        "true\nfalse",
    );
}

/// Whether every byte is ASCII whitespace. py: bytes.isspace
#[test]
fn parity_bytes_is_space() {
    assert_all_tiers(
        "## Main\nShow bytesIsSpace([32, 9, 10]).\nShow bytesIsSpace([32, 65]).\n",
        "true\nfalse",
    );
}

/// Whether all cased bytes are uppercase and at least one is cased. py: bytes.isupper
#[test]
fn parity_bytes_is_upper() {
    assert_all_tiers(
        "## Main\nShow bytesIsUpper([72, 73]).\nShow bytesIsUpper([72, 105]).\nShow bytesIsUpper([49]).\n",
        "true\nfalse\nfalse",
    );
}

/// Whether all cased bytes are lowercase and at least one is cased. py: bytes.islower
#[test]
fn parity_bytes_is_lower() {
    assert_all_tiers(
        "## Main\nShow bytesIsLower([104, 105]).\nShow bytesIsLower([72, 105]).\n",
        "true\nfalse",
    );
}

/// Uppercase the first byte, lowercase the rest. py: bytes.capitalize
#[test]
fn parity_bytes_capitalize() {
    assert_all_tiers(
        "## Main\nShow bytesCapitalize([104, 101, 108, 108, 111]).\n",
        "[72, 101, 108, 108, 111]",
    );
}

/// Swap the case of every ASCII letter. py: bytes.swapcase
#[test]
fn parity_bytes_swapcase() {
    assert_all_tiers("## Main\nShow bytesSwapCase([72, 105]).\n", "[104, 73]");
}

/// Title-case: uppercase each word's first letter, lowercase the rest. go: bytes.Title · py: bytes.title
#[test]
fn parity_bytes_title() {
    assert_all_tiers(
        "## Main\nShow bytesTitle([104, 105, 32, 116, 104, 101, 114, 101]).\n",
        "[72, 105, 32, 84, 104, 101, 114, 101]",
    );
}

/// Left-justify to a width, padding on the right. py: bytes.ljust / bytearray.ljust
#[test]
fn parity_bytes_ljust() {
    assert_all_tiers(
        "## Main\nShow bytesLjust([65, 66], 5, 46).\nShow bytesLjust([65, 66, 67], 2, 46).\n",
        "[65, 66, 46, 46, 46]\n[65, 66, 67]",
    );
}

/// Right-justify to a width, padding on the left. py: bytes.rjust / bytearray.rjust
#[test]
fn parity_bytes_rjust() {
    assert_all_tiers(
        "## Main\nShow bytesRjust([65, 66], 5, 46).\n",
        "[46, 46, 46, 65, 66]",
    );
}

/// Center in a width (CPython tie-break: extra pad right unless both margin and width are odd). py: bytes.center
#[test]
fn parity_bytes_center() {
    assert_all_tiers(
        "## Main\nShow bytesCenter([65, 66, 67], 7, 42).\nShow bytesCenter([65, 66], 5, 42).\n",
        "[42, 42, 65, 66, 67, 42, 42]\n[42, 42, 65, 66, 42]",
    );
}

/// Zero-fill to a width, keeping a leading sign first. py: bytes.zfill / bytearray.zfill
#[test]
fn parity_bytes_zfill() {
    assert_all_tiers(
        "## Main\nShow bytesZfill([52, 50], 5).\nShow bytesZfill([45, 52, 50], 6).\n",
        "[48, 48, 48, 52, 50]\n[45, 48, 48, 48, 52, 50]",
    );
}

/// Expand tabs to the next tab stop. py: bytes.expandtabs / bytearray.expandtabs
#[test]
fn parity_bytes_expandtabs() {
    assert_all_tiers(
        "## Main\nShow bytesExpandtabs([65, 9, 66], 4).\n",
        "[65, 32, 32, 32, 66]",
    );
}

/// Whether every byte is ASCII alphanumeric (empty → false). py: bytes.isalnum
#[test]
fn parity_bytes_is_alnum() {
    assert_all_tiers(
        "## Main\nShow bytesIsAlnum([65, 49, 122]).\nShow bytesIsAlnum([65, 32]).\n",
        "true\nfalse",
    );
}

/// Whether every byte is ASCII (< 128; empty → true). py: bytes.isascii
#[test]
fn parity_bytes_is_ascii() {
    assert_all_tiers(
        "## Main\nShow bytesIsAscii([65, 127]).\nShow bytesIsAscii([65, 200]).\n",
        "true\nfalse",
    );
}

/// Whether the bytes are title-cased. py: bytes.istitle
#[test]
fn parity_bytes_is_title() {
    assert_all_tiers(
        "## Main\nShow bytesIsTitle([72, 105]).\nShow bytesIsTitle([72, 73]).\n",
        "true\nfalse",
    );
}
